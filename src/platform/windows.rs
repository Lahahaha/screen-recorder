use crate::{
    app::AppResult,
    logging::Logger,
    platform::{find_ffmpeg_binary, rename_with_context},
};
use std::{
    ffi::c_void,
    fs, io, mem,
    path::{Path, PathBuf},
    process::Command,
    ptr, thread,
};

type Bool = i32;
type Dword = u32;
type Handle = isize;
type WtsInfoClass = i32;

const WTS_CURRENT_SERVER_HANDLE: Handle = 0;
const WTS_CURRENT_SESSION: Dword = Dword::MAX;
const WTS_SESSION_INFO_EX: WtsInfoClass = 25;
const WTS_INFO_EX_LEVEL1: Dword = 1;
const WTS_SESSIONSTATE_LOCK: i32 = 0;
const WTS_SESSIONSTATE_UNLOCK: i32 = 1;
const WTS_SESSIONSTATE_UNKNOWN: i32 = -1;

#[repr(C)]
struct WtsInfoExW {
    level: Dword,
    data: WtsInfoExLevelW,
}

#[repr(C)]
union WtsInfoExLevelW {
    level1: WtsInfoExLevel1W,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
struct WtsInfoExLevel1W {
    session_id: Dword,
    session_state: i32,
    session_flags: i32,
    win_station_name: [u16; 33],
    user_name: [u16; 21],
    domain_name: [u16; 18],
    logon_time: i64,
    connect_time: i64,
    disconnect_time: i64,
    last_input_time: i64,
    current_time: i64,
    incoming_bytes: Dword,
    outgoing_bytes: Dword,
    incoming_frames: Dword,
    outgoing_frames: Dword,
    incoming_compressed_bytes: Dword,
    outgoing_compressed_bytes: Dword,
}

#[link(name = "wtsapi32")]
extern "system" {
    fn WTSFreeMemory(memory: *mut c_void);
    fn WTSQuerySessionInformationW(
        server: Handle,
        session_id: Dword,
        info_class: WtsInfoClass,
        buffer: *mut *mut u16,
        bytes_returned: *mut Dword,
    ) -> Bool;
}

pub(crate) fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    if !destination.exists() {
        rename_with_context(source, destination, "替换文件失败")?;
        return Ok(());
    }

    let backup = backup_path(destination);
    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("删除旧备份文件失败 {}: {error}", backup.display()),
            )
        })?;
    }

    rename_with_context(destination, &backup, "备份目标文件失败")?;

    match rename_with_context(source, destination, "替换文件失败") {
        Ok(()) => {
            fs::remove_file(&backup).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("删除备份文件失败 {}: {error}", backup.display()),
                )
            })?;
            Ok(())
        }
        Err(error) => {
            let restore_error = rename_with_context(&backup, destination, "恢复备份失败").err();
            let restore_message = restore_error
                .map(|restore_error| format!("；{restore_error}"))
                .unwrap_or_default();
            Err(io::Error::new(error.kind(), format!("{error}{restore_message}")).into())
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|file_name| file_name.to_string_lossy())
        .unwrap_or_else(|| "backup".into());
    path.with_file_name(format!("{file_name}.bak"))
}

pub(crate) fn find_ffmpeg() -> AppResult<PathBuf> {
    find_ffmpeg_binary("ffmpeg.exe", &[])
}

pub(crate) fn notify(title: &str, message: &str, logger: Logger) {
    let title = title.to_string();
    let message = message.to_string();
    thread::spawn(move || {
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show('{}', 'Screen Recorder - {}') | Out-Null",
            escape_powershell_single_quoted(&message),
            escape_powershell_single_quoted(&title)
        );
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", &script]);
        hide_console(&mut cmd);
        let result = cmd.status();

        if let Err(error) = result {
            logger.error(format!("发送通知失败: {error}"));
        }
    });
}

pub(crate) fn open_path(path: &Path) -> AppResult<()> {
    let mut cmd = Command::new("explorer");
    cmd.arg(path);
    hide_console(&mut cmd);
    cmd.spawn()?;
    Ok(())
}

pub(crate) fn screen_locked() -> AppResult<bool> {
    let Some(session_flags) = query_session_flags() else {
        return Ok(true);
    };
    Ok(session_flags_indicate_locked(session_flags))
}

fn query_session_flags() -> Option<i32> {
    let mut buffer = ptr::null_mut::<u16>();
    let mut bytes_returned = 0;
    let query_ok = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            WTS_CURRENT_SESSION,
            WTS_SESSION_INFO_EX,
            &mut buffer,
            &mut bytes_returned,
        )
    };
    if query_ok == 0 || buffer.is_null() {
        return None;
    }

    let _memory = WtsMemory(buffer.cast::<c_void>());
    if (bytes_returned as usize) < mem::size_of::<WtsInfoExW>() {
        return None;
    }

    let info = unsafe { &*(buffer.cast::<WtsInfoExW>()) };
    if info.level != WTS_INFO_EX_LEVEL1 {
        return None;
    }

    Some(unsafe { info.data.level1.session_flags })
}

fn session_flags_indicate_locked(session_flags: i32) -> bool {
    match session_flags {
        WTS_SESSIONSTATE_UNLOCK => false,
        WTS_SESSIONSTATE_LOCK | WTS_SESSIONSTATE_UNKNOWN => true,
        _ => true,
    }
}

struct WtsMemory(*mut c_void);

impl Drop for WtsMemory {
    fn drop(&mut self) {
        unsafe {
            WTSFreeMemory(self.0);
        }
    }
}

pub(crate) fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000);
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_flags_treat_lock_as_locked() {
        assert!(session_flags_indicate_locked(WTS_SESSIONSTATE_LOCK));
    }

    #[test]
    fn session_flags_treat_unlock_as_unlocked() {
        assert!(!session_flags_indicate_locked(WTS_SESSIONSTATE_UNLOCK));
    }

    #[test]
    fn session_flags_treat_unknown_as_locked() {
        assert!(session_flags_indicate_locked(WTS_SESSIONSTATE_UNKNOWN));
    }

    #[test]
    fn session_flags_treat_unexpected_value_as_locked() {
        assert!(session_flags_indicate_locked(42));
    }
}
