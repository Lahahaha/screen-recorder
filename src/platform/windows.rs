use crate::{
    app::AppResult,
    logging::Logger,
    platform::{find_in_path, is_usable_executable},
};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    thread,
};

pub(crate) fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    if !destination.exists() {
        fs::rename(source, destination).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "替换文件失败 {} -> {}: {error}",
                    source.display(),
                    destination.display()
                ),
            )
        })?;
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

    fs::rename(destination, &backup).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "备份目标文件失败 {} -> {}: {error}",
                destination.display(),
                backup.display()
            ),
        )
    })?;

    match fs::rename(source, destination) {
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
            let restore_error = fs::rename(&backup, destination).err();
            let restore_message = restore_error
                .map(|restore_error| {
                    format!(
                        "；恢复备份失败 {} -> {}: {restore_error}",
                        backup.display(),
                        destination.display()
                    )
                })
                .unwrap_or_default();
            Err(io::Error::new(
                error.kind(),
                format!(
                    "替换文件失败 {} -> {}: {error}{restore_message}",
                    source.display(),
                    destination.display()
                ),
            )
            .into())
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
    let executable = env::current_exe()?.canonicalize()?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法定位当前可执行文件所在目录"))?;

    let same_dir = executable_dir.join("ffmpeg.exe");
    if is_usable_executable(&same_dir) {
        return Ok(same_dir);
    }

    find_in_path("ffmpeg.exe").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "找不到 ffmpeg，请将 ffmpeg 放入程序所在目录或安装到 PATH",
        )
        .into()
    })
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
