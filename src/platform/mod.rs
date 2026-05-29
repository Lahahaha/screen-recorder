use crate::logging::Logger;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(not(target_os = "windows"))]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

trait Platform {
    fn replace_file(source: &Path, destination: &Path) -> crate::app::AppResult<()>;
    fn find_ffmpeg() -> crate::app::AppResult<PathBuf>;
    fn notify(title: &str, message: &str, logger: Logger);
    fn hide_console(cmd: &mut Command);
}

#[cfg(not(target_os = "windows"))]
struct CurrentPlatform;

#[cfg(target_os = "windows")]
struct CurrentPlatform;

#[cfg(not(target_os = "windows"))]
impl Platform for CurrentPlatform {
    fn replace_file(source: &Path, destination: &Path) -> crate::app::AppResult<()> {
        macos::replace_file(source, destination)
    }

    fn find_ffmpeg() -> crate::app::AppResult<PathBuf> {
        macos::find_ffmpeg()
    }

    fn notify(title: &str, message: &str, logger: Logger) {
        macos::notify(title, message, logger);
    }

    fn hide_console(_cmd: &mut Command) {}
}

#[cfg(target_os = "windows")]
impl Platform for CurrentPlatform {
    fn replace_file(source: &Path, destination: &Path) -> crate::app::AppResult<()> {
        windows::replace_file(source, destination)
    }

    fn find_ffmpeg() -> crate::app::AppResult<PathBuf> {
        windows::find_ffmpeg()
    }

    fn notify(title: &str, message: &str, logger: Logger) {
        windows::notify(title, message, logger);
    }

    fn hide_console(cmd: &mut Command) {
        windows::hide_console(cmd);
    }
}

pub(crate) fn replace_file(source: &Path, destination: &Path) -> crate::app::AppResult<()> {
    CurrentPlatform::replace_file(source, destination)
}

pub(crate) fn find_ffmpeg() -> crate::app::AppResult<PathBuf> {
    CurrentPlatform::find_ffmpeg()
}

pub(crate) fn notify(title: &str, message: &str, logger: Logger) {
    CurrentPlatform::notify(title, message, logger);
}

pub(crate) fn hide_console(cmd: &mut Command) {
    CurrentPlatform::hide_console(cmd);
}

pub(crate) fn notify_screen_capture_failure(logger: &Logger) {
    #[cfg(target_os = "macos")]
    notify(
        "截屏权限不足",
        "无法读取屏幕内容，请在系统设置中允许屏幕录制权限。",
        logger.clone(),
    );

    #[cfg(not(target_os = "macos"))]
    let _ = logger;
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(binary))
            .find(|path| is_usable_executable(path))
    })
}

fn is_usable_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}
