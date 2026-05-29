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
    Ok(())
}

pub(crate) fn find_ffmpeg() -> AppResult<PathBuf> {
    let executable = env::current_exe()?.canonicalize()?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法定位当前可执行文件所在目录"))?;

    let same_dir = executable_dir.join("ffmpeg");
    if is_usable_executable(&same_dir) {
        return Ok(same_dir);
    }

    let bundled = executable_dir.join("../Resources/ffmpeg");
    if let Ok(bundled) = bundled.canonicalize() {
        if is_usable_executable(&bundled) {
            return Ok(bundled);
        }
    }

    find_in_path("ffmpeg").ok_or_else(|| {
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
            "display notification \"{}\" with title \"Screen Recorder\" subtitle \"{}\"",
            escape_applescript_string(&message),
            escape_applescript_string(&title)
        );
        let result = Command::new("osascript").args(["-e", &script]).status();

        if let Err(error) = result {
            logger.error(format!("发送通知失败: {error}"));
        }
    });
}

fn escape_applescript_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
