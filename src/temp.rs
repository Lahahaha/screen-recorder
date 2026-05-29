use crate::{app::AppResult, logging::Logger};
use chrono::Local;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub(crate) struct TempFileCleanup {
    path: PathBuf,
    disarmed: bool,
}

impl TempFileCleanup {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            disarmed: false,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        if !self.disarmed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(crate) struct AtomicFlagGuard {
    flag: Arc<AtomicBool>,
}

impl AtomicFlagGuard {
    pub(crate) fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

impl Drop for AtomicFlagGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct TempDir {
    path: PathBuf,
    logger: Logger,
}

impl TempDir {
    pub(crate) fn new(parent: &Path, prefix: &str, logger: Logger) -> AppResult<Self> {
        fs::create_dir_all(parent)?;
        let path = parent.join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f")
        ));
        fs::create_dir(&path)?;
        Ok(Self { path, logger })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            self.logger
                .error(format!("清理临时目录失败 {}: {error}", self.path.display()));
        }
    }
}
