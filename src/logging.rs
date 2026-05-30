use crate::{app::AppResult, paths::AppPaths};
use chrono::Local;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const MAX_ROTATED_LOGS: u8 = 3;

#[derive(Clone)]
pub(crate) struct Logger {
    file: Arc<Mutex<fs::File>>,
}

impl Logger {
    pub(crate) fn new(paths: &AppPaths) -> AppResult<Self> {
        fs::create_dir_all(&paths.root)?;
        rotate_logs(&paths.root)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths.root.join("app.log"))?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub(crate) fn info(&self, message: impl AsRef<str>) {
        self.write("INFO", message.as_ref());
    }

    pub(crate) fn warn(&self, message: impl AsRef<str>) {
        self.write("WARN", message.as_ref());
    }

    pub(crate) fn error(&self, message: impl AsRef<str>) {
        self.write("ERROR", message.as_ref());
    }

    fn write(&self, level: &str, message: &str) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{timestamp} [{level}] {message}");
        }
    }
}

fn rotate_logs(root: &Path) -> AppResult<()> {
    let current = root.join("app.log");
    if !should_rotate_log(&current)? {
        return Ok(());
    }

    rotate_log_files(root)
}

fn rotate_log_files(root: &Path) -> AppResult<()> {
    let current = root.join("app.log");
    let oldest = rotated_log_path(root, MAX_ROTATED_LOGS);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }

    for index in (1..MAX_ROTATED_LOGS).rev() {
        let source = rotated_log_path(root, index);
        if source.exists() {
            fs::rename(source, rotated_log_path(root, index + 1))?;
        }
    }

    fs::rename(current, rotated_log_path(root, 1))?;
    Ok(())
}

fn should_rotate_log(path: &Path) -> AppResult<bool> {
    if !path.exists() {
        return Ok(false);
    }

    Ok(path.metadata()?.len() > MAX_LOG_BYTES)
}

fn rotated_log_path(root: &Path, index: u8) -> PathBuf {
    root.join(format!("app.log.{index}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-logging-test-{}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f"),
            sequence
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn should_rotate_log_only_when_file_exceeds_limit() {
        let dir = test_dir();
        let log = dir.join("app.log");

        assert!(!should_rotate_log(&log).expect("missing log"));
        fs::write(&log, vec![0_u8; MAX_LOG_BYTES as usize]).expect("write log at limit");
        assert!(!should_rotate_log(&log).expect("log at limit"));
        fs::write(&log, vec![0_u8; MAX_LOG_BYTES as usize + 1]).expect("write oversized log");
        assert!(should_rotate_log(&log).expect("oversized log"));
    }

    #[test]
    fn rotate_logs_keeps_bounded_history() {
        let dir = test_dir();
        fs::write(dir.join("app.log"), b"current").expect("write current");
        fs::write(dir.join("app.log.1"), b"one").expect("write one");
        fs::write(dir.join("app.log.2"), b"two").expect("write two");
        fs::write(dir.join("app.log.3"), b"three").expect("write three");

        rotate_log_files(&dir).expect("rotate logs");

        assert!(!dir.join("app.log").exists());
        assert_eq!(
            fs::read_to_string(dir.join("app.log.1")).expect("read one"),
            "current"
        );
        assert_eq!(
            fs::read_to_string(dir.join("app.log.2")).expect("read two"),
            "one"
        );
        assert_eq!(
            fs::read_to_string(dir.join("app.log.3")).expect("read three"),
            "two"
        );
        assert!(!dir.join("app.log.4").exists());
    }
}
