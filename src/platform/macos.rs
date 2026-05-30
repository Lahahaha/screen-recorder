use crate::{
    app::AppResult,
    logging::Logger,
    platform::{find_ffmpeg_binary, rename_with_context},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    thread,
};

pub(crate) fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    rename_with_context(source, destination, "替换文件失败")?;
    Ok(())
}

pub(crate) fn find_ffmpeg() -> AppResult<PathBuf> {
    find_ffmpeg_binary("ffmpeg", &["../Resources/ffmpeg"])
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

pub(crate) fn open_path(path: &Path) -> AppResult<()> {
    let status = Command::new("open").arg(path).status()?;
    if !status.success() {
        return Err(
            std::io::Error::other(format!("打开路径失败 {}: {status}", path.display())).into(),
        );
    }
    Ok(())
}

fn escape_applescript_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
