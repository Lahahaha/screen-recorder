use crate::{
    app::AppResult,
    logging::Logger,
    platform::{find_ffmpeg_binary, rename_with_context},
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    thread,
};

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

pub(crate) fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000);
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}
