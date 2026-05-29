use crate::{app::AppResult, paths::AppPaths};
use chrono::Local;
use std::{
    fs,
    io::Write,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub(crate) struct Logger {
    file: Arc<Mutex<fs::File>>,
}

impl Logger {
    pub(crate) fn new(paths: &AppPaths) -> AppResult<Self> {
        fs::create_dir_all(&paths.root)?;
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
