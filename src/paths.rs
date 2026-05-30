use crate::app::AppResult;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(crate) struct AppPaths {
    pub(crate) root: PathBuf,
    pub(crate) config: PathBuf,
    pub(crate) screenshots: PathBuf,
    pub(crate) videos: PathBuf,
}

impl AppPaths {
    pub(crate) fn new() -> AppResult<Self> {
        // 按优先级查找可用的视频目录
        let root = Self::find_data_dir()?;
        let screenshots = root.join("screenshots");
        let videos = root.join("videos");
        let config = root.join("config.json");

        fs::create_dir_all(&screenshots)?;
        fs::create_dir_all(&videos)?;

        Ok(Self {
            root,
            config,
            screenshots,
            videos,
        })
    }

    fn find_data_dir() -> AppResult<PathBuf> {
        // 1. 优先使用系统视频目录
        if let Some(video_dir) = dirs::video_dir() {
            let root = video_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return Ok(root);
            }
        }

        // 2. 使用文档目录（macOS/Windows 都存在）
        if let Some(doc_dir) = dirs::document_dir() {
            let root = doc_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return Ok(root);
            }
        }

        // 3. 使用用户主目录
        if let Some(home_dir) = dirs::home_dir() {
            let root = home_dir.join("ScreenRecorder");
            if Self::ensure_dir(&root).is_ok() {
                return Ok(root);
            }
        }

        Err(io::Error::new(io::ErrorKind::NotFound, "无法找到可用的数据存储目录").into())
    }

    fn ensure_dir(path: &Path) -> AppResult<()> {
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
