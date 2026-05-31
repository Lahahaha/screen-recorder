use crate::{app::AppResult, workdirs};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(crate) struct AppPaths {
    pub(crate) control: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) config: PathBuf,
    pub(crate) screenshots: PathBuf,
    pub(crate) videos: PathBuf,
}

impl AppPaths {
    pub(crate) fn new() -> AppResult<Self> {
        let control = Self::control_dir()?;
        let default_root = Self::find_default_data_dir()?;
        let root = workdirs::startup_root(&control, &default_root)?;
        let paths = Self::from_control_and_root(control, root)?;
        workdirs::record_startup_root(&paths.control, &paths.root)?;
        Ok(paths)
    }

    pub(crate) fn from_root(root: PathBuf) -> AppResult<Self> {
        Self::from_control_and_root(Self::control_dir()?, root)
    }

    fn from_control_and_root(control: PathBuf, root: PathBuf) -> AppResult<Self> {
        Self::ensure_dir(&control)?;
        Self::ensure_dir(&root)?;
        let control = canonicalize_dir(&control)?;
        let root = canonicalize_dir(&root)?;
        let screenshots = root.join("screenshots");
        let videos = root.join("videos");
        let config = root.join("config.json");

        Self::ensure_dir(&screenshots)?;
        Self::ensure_dir(&videos)?;

        Ok(Self {
            control,
            root,
            config,
            screenshots,
            videos,
        })
    }

    pub(crate) fn control_dir() -> AppResult<PathBuf> {
        if let Some(config_dir) = dirs::config_dir() {
            let root = config_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return canonicalize_dir(&root);
            }
        }

        if let Some(data_dir) = dirs::data_local_dir() {
            let root = data_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return canonicalize_dir(&root);
            }
        }

        Self::find_default_data_dir()
    }

    fn find_default_data_dir() -> AppResult<PathBuf> {
        // 1. 优先使用系统视频目录
        if let Some(video_dir) = dirs::video_dir() {
            let root = video_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return canonicalize_dir(&root);
            }
        }

        // 2. 使用文档目录（macOS/Windows 都存在）
        if let Some(doc_dir) = dirs::document_dir() {
            let root = doc_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return canonicalize_dir(&root);
            }
        }

        // 3. 使用用户主目录
        if let Some(home_dir) = dirs::home_dir() {
            let root = home_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return canonicalize_dir(&root);
            }
        }

        Err(io::Error::new(io::ErrorKind::NotFound, "无法找到可用的数据存储目录").into())
    }

    pub(crate) fn ensure_dir(path: &Path) -> AppResult<()> {
        if path.exists() {
            if !path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("路径已存在但不是目录: {}", path.display()),
                )
                .into());
            }
        } else {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }

    pub(crate) fn screenshots_dir_for_date(&self, date: &str) -> PathBuf {
        self.screenshots.join(date)
    }

    pub(crate) fn video_path_for_date(&self, date: &str) -> PathBuf {
        self.videos.join(format!("{date}.mp4"))
    }
}

pub(crate) fn display_path(path: &Path) -> String {
    format_path_for_display(&path.display().to_string())
}

fn format_path_for_display(value: &str) -> String {
    const WINDOWS_VERBATIM_UNC_PREFIX: &str = "\\\\?\\UNC\\";
    const WINDOWS_VERBATIM_PREFIX: &str = "\\\\?\\";

    if let Some(stripped) = value.strip_prefix(WINDOWS_VERBATIM_UNC_PREFIX) {
        return format!("\\\\{stripped}");
    }
    if let Some(stripped) = value.strip_prefix(WINDOWS_VERBATIM_PREFIX) {
        return stripped.to_string();
    }

    value.to_string()
}

fn canonicalize_dir(path: &Path) -> AppResult<PathBuf> {
    path.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("规范化目录失败 {}: {error}", path.display()),
        )
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-paths-test-{name}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f")
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn from_control_and_root_creates_work_subdirectories_without_config() {
        let control = test_dir("control");
        let root = test_dir("root");

        let paths = AppPaths::from_control_and_root(control, root.clone()).expect("create paths");

        assert!(paths.control.is_dir());
        assert!(paths.root.is_dir());
        assert!(paths.screenshots.is_dir());
        assert!(paths.videos.is_dir());
        assert!(!paths.config.exists());
        assert_eq!(paths.screenshots, paths.root.join("screenshots"));
        assert_eq!(paths.videos, paths.root.join("videos"));
    }

    #[test]
    fn display_path_strips_windows_verbatim_drive_prefix() {
        assert_eq!(
            format_path_for_display(r"\\?\C:\Users\hang\ScreenRecorder"),
            r"C:\Users\hang\ScreenRecorder"
        );
    }

    #[test]
    fn display_path_strips_windows_verbatim_unc_prefix() {
        assert_eq!(
            format_path_for_display(r"\\?\UNC\server\share\ScreenRecorder"),
            r"\\server\share\ScreenRecorder"
        );
    }
}
