use crate::{app::AppResult, logging::Logger, paths::AppPaths, platform, temp::TempFileCleanup};
use chrono::Local;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fs, io,
    sync::{Arc, Mutex},
};

pub(crate) const SUPPORTED_INTERVALS: [u64; 4] = [10, 30, 60, 120];
const MAX_SCALE: f32 = 4.0;

pub(crate) struct ConfigStore;

impl ConfigStore {
    pub(crate) fn load(paths: &AppPaths, logger: &Logger) -> AppResult<Config> {
        load_config(paths, logger)
    }

    pub(crate) fn save_current(paths: &AppPaths, config: &Arc<Mutex<Config>>, logger: &Logger) {
        save_current_config(paths, config, logger);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Config {
    pub(crate) interval: u64,
    pub(crate) fps: u32,
    pub(crate) image_format: ScreenshotFormat,
    pub(crate) scale: f32,
    pub(crate) dedup: bool,
    pub(crate) auto_start: bool,
    pub(crate) video_codec: VideoCodec,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval: 30,
            fps: 10,
            image_format: ScreenshotFormat::Png,
            scale: 1.0,
            dedup: false,
            auto_start: false,
            video_codec: VideoCodec::H264,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum VideoCodec {
    #[default]
    H264,
    H265,
}

impl VideoCodec {
    pub(crate) fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "h265" | "h.265" | "hevc" | "libx265" => Self::H265,
            _ => Self::H264,
        }
    }

    pub(crate) fn config_value(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::H265 => "h265",
        }
    }

    pub(crate) fn encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::H265 => "libx265",
        }
    }
}

impl Serialize for VideoCodec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.config_value())
    }
}

impl<'de> Deserialize<'de> for VideoCodec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self::from_config(&value))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ScreenshotFormat {
    #[default]
    Png,
    Jpg,
}

impl ScreenshotFormat {
    pub(crate) fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Self::Jpg,
            _ => Self::Png,
        }
    }

    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
        }
    }
}

impl Serialize for ScreenshotFormat {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.extension())
    }
}

impl<'de> Deserialize<'de> for ScreenshotFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self::from_config(&value))
    }
}

pub(crate) fn load_config(paths: &AppPaths, logger: &Logger) -> AppResult<Config> {
    if !paths.config.exists() {
        let config = Config::default();
        save_config(paths, &config)?;
        return Ok(config);
    }

    let content = fs::read_to_string(&paths.config)?;
    let mut config: Config = match serde_json::from_str(&content) {
        Ok(config) => config,
        Err(_) => {
            backup_corrupted_config(paths)?;
            let config = Config::default();
            save_config(paths, &config)?;
            config
        }
    };
    normalize_config(&mut config, logger);
    Ok(config)
}

pub(crate) fn normalize_config(config: &mut Config, logger: &Logger) {
    if !SUPPORTED_INTERVALS.contains(&config.interval) {
        config.interval = Config::default().interval;
    }
    if config.fps == 0 {
        config.fps = Config::default().fps;
    }
    if !config.scale.is_finite() || config.scale <= 0.0 {
        config.scale = Config::default().scale;
    } else if config.scale > MAX_SCALE {
        logger.warn(format!(
            "scale 配置过大，已从 {} 限制为 {}",
            config.scale, MAX_SCALE
        ));
        config.scale = MAX_SCALE;
    }
}

fn backup_corrupted_config(paths: &AppPaths) -> AppResult<()> {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S%.3f");
    let backup = paths.root.join(format!("config.json.corrupt.{timestamp}"));
    fs::rename(&paths.config, backup)?;
    Ok(())
}

pub(crate) fn save_config(paths: &AppPaths, config: &Config) -> AppResult<()> {
    fs::create_dir_all(&paths.root)?;
    let content = serde_json::to_string_pretty(config)?;
    let temp_path = paths.root.join(format!(
        ".config.json.{}.{}.tmp",
        std::process::id(),
        Local::now().format("%Y%m%d%H%M%S%.3f")
    ));
    let mut temp_cleanup = TempFileCleanup::new(temp_path.clone());
    {
        let mut file = fs::File::create(&temp_path)?;
        use std::io::Write;
        file.write_all(format!("{content}\n").as_bytes())?;
        file.sync_all()?;
    }
    platform::replace_file(&temp_path, &paths.config)?;
    temp_cleanup.disarm();
    Ok(())
}

pub(crate) fn save_current_config(paths: &AppPaths, config: &Arc<Mutex<Config>>, logger: &Logger) {
    match config.lock() {
        Ok(config) => {
            if let Err(error) = save_config(paths, &config) {
                logger.error(format!("保存配置失败: {error}"));
            }
        }
        Err(error) => logger.error(format!("读取配置失败: {error}")),
    }
}

pub(crate) fn cloned_config(config: &Arc<Mutex<Config>>) -> AppResult<Config> {
    let config = config
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(config.clone())
}
