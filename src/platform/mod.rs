use crate::logging::Logger;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
compile_error!("screen-recorder only supports macOS and Windows");

trait Platform {
    fn replace_file(source: &Path, destination: &Path) -> crate::app::AppResult<()>;
    fn find_ffmpeg() -> crate::app::AppResult<PathBuf>;
    fn notify(title: &str, message: &str, logger: Logger);
    fn open_path(path: &Path) -> crate::app::AppResult<()>;
    fn hide_console(cmd: &mut Command);
}

#[cfg(target_os = "macos")]
struct CurrentPlatform;

#[cfg(target_os = "windows")]
struct CurrentPlatform;

#[cfg(target_os = "macos")]
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

    fn open_path(path: &Path) -> crate::app::AppResult<()> {
        macos::open_path(path)
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

    fn open_path(path: &Path) -> crate::app::AppResult<()> {
        windows::open_path(path)
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

pub(crate) fn open_path(path: &Path) -> crate::app::AppResult<()> {
    CurrentPlatform::open_path(path)
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

    #[cfg(target_os = "windows")]
    let _ = logger;
}

pub(crate) fn find_ffmpeg_binary(
    binary_name: &str,
    extra_paths: &[&str],
) -> crate::app::AppResult<PathBuf> {
    let executable = env::current_exe()?.canonicalize()?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法定位当前可执行文件所在目录"))?;

    let same_dir = executable_dir.join(binary_name);
    if is_usable_executable(&same_dir) {
        return Ok(same_dir);
    }

    for extra_path in extra_paths {
        let candidate = executable_dir.join(extra_path);
        if is_usable_executable(&candidate) {
            return Ok(candidate);
        }

        if let Ok(candidate) = candidate.canonicalize() {
            if is_usable_executable(&candidate) {
                return Ok(candidate);
            }
        }
    }

    find_in_path(binary_name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "找不到 ffmpeg，请将 ffmpeg 放入程序所在目录或安装到 PATH",
        )
        .into()
    })
}

pub(crate) fn rename_with_context(
    source: &Path,
    destination: &Path,
    action: &str,
) -> io::Result<()> {
    fs::rename(source, destination).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{action} {} -> {}: {error}",
                source.display(),
                destination.display()
            ),
        )
    })
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
