use crate::{app::AppResult, logging::Logger, paths::AppPaths, platform, temp::TempFileCleanup};
use chrono::Local;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fs, io,
    sync::{Arc, Mutex},
};

pub(crate) const SUPPORTED_INTERVALS: [u64; 4] = [10, 30, 60, 120];
const MAX_SCALE: f32 = 4.0;

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
    pub(crate) language: Language,
    pub(crate) capture_mode: CaptureMode,
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
            language: Language::ZhCn,
            capture_mode: CaptureMode::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CaptureMode {
    #[default]
    Auto,
    Screen(u32),
}

impl CaptureMode {
    pub(crate) fn from_config(value: &str) -> Self {
        let value = value.trim().to_ascii_lowercase();
        if value == "auto" {
            return Self::Auto;
        }
        let Some(index) = value
            .strip_prefix("screen-")
            .and_then(|index| index.parse::<u32>().ok())
        else {
            return Self::Auto;
        };
        if index == 0 {
            Self::Auto
        } else {
            Self::Screen(index)
        }
    }

    pub(crate) fn config_value(self) -> String {
        match self {
            Self::Auto => "auto".to_string(),
            Self::Screen(index) => format!("screen-{index:02}"),
        }
    }
}

impl Serialize for CaptureMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.config_value())
    }
}

impl<'de> Deserialize<'de> for CaptureMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self::from_config(&value))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Language {
    #[default]
    ZhCn,
    En,
}

impl Language {
    pub(crate) const ALL: &'static [Language] = &[Language::ZhCn, Language::En];

    pub(crate) fn from_config(value: &str) -> Self {
        let value = value.trim().to_ascii_lowercase();
        for language in Self::ALL {
            if language
                .config_aliases()
                .iter()
                .any(|alias| *alias == value)
            {
                return *language;
            }
        }
        Self::default()
    }

    pub(crate) fn config_value(self) -> &'static str {
        self.config_aliases()[0]
    }

    pub(crate) fn config_aliases(self) -> &'static [&'static str] {
        match self {
            Self::ZhCn => &["zh-CN", "zh", "zh-cn", "zh_cn", "cn"],
            Self::En => &["en", "en-US", "en-us", "en_us"],
        }
    }

    pub(crate) fn menu_label(self) -> &'static str {
        match self {
            Self::ZhCn => "中文",
            Self::En => "English",
        }
    }
}

impl Serialize for Language {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.config_value())
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self::from_config(&value))
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
        Err(error) => {
            logger.error(format!("解析配置失败，将重置为默认配置: {error}"));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_logger() -> Logger {
        let root = std::env::temp_dir().join(format!(
            "screen-recorder-test-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f")
        ));
        let paths = AppPaths {
            config: root.join("config.json"),
            screenshots: root.join("screenshots"),
            videos: root.join("videos"),
            root,
        };
        Logger::new(&paths).expect("create test logger")
    }

    #[test]
    fn default_config_is_conservative() {
        let config = Config::default();

        assert_eq!(config.interval, 30);
        assert_eq!(config.fps, 10);
        assert_eq!(config.image_format, ScreenshotFormat::Png);
        assert_eq!(config.scale, 1.0);
        assert!(!config.dedup);
        assert!(!config.auto_start);
        assert_eq!(config.video_codec, VideoCodec::H264);
        assert_eq!(config.language, Language::ZhCn);
        assert_eq!(config.capture_mode, CaptureMode::Auto);
    }

    #[test]
    fn capture_mode_parses_known_values() {
        assert_eq!(CaptureMode::from_config("auto"), CaptureMode::Auto);
        assert_eq!(
            CaptureMode::from_config("screen-01"),
            CaptureMode::Screen(1)
        );
        assert_eq!(CaptureMode::from_config("screen-2"), CaptureMode::Screen(2));
        assert_eq!(CaptureMode::from_config("screen-00"), CaptureMode::Auto);
        assert_eq!(CaptureMode::from_config("unknown"), CaptureMode::Auto);
        assert_eq!(CaptureMode::Screen(12).config_value(), "screen-12");
    }

    #[test]
    fn screenshot_format_parses_known_values() {
        assert_eq!(ScreenshotFormat::from_config("jpg"), ScreenshotFormat::Jpg);
        assert_eq!(ScreenshotFormat::from_config("JPEG"), ScreenshotFormat::Jpg);
        assert_eq!(ScreenshotFormat::from_config("png"), ScreenshotFormat::Png);
        assert_eq!(
            ScreenshotFormat::from_config("unknown"),
            ScreenshotFormat::Png
        );
    }

    #[test]
    fn video_codec_parses_known_values() {
        assert_eq!(VideoCodec::from_config("h265"), VideoCodec::H265);
        assert_eq!(VideoCodec::from_config("hevc"), VideoCodec::H265);
        assert_eq!(VideoCodec::from_config("libx265"), VideoCodec::H265);
        assert_eq!(VideoCodec::from_config("h264"), VideoCodec::H264);
        assert_eq!(VideoCodec::from_config("unknown"), VideoCodec::H264);
    }

    #[test]
    fn language_parses_known_values() {
        assert_eq!(Language::from_config("zh-CN"), Language::ZhCn);
        assert_eq!(Language::from_config("zh"), Language::ZhCn);
        assert_eq!(Language::from_config("en"), Language::En);
        assert_eq!(Language::from_config("unknown"), Language::ZhCn);
    }

    #[test]
    fn language_metadata_is_complete() {
        for language in Language::ALL {
            assert!(!language.config_value().is_empty());
            assert!(!language.menu_label().is_empty());
            assert!(language.config_aliases().contains(&language.config_value()));
        }
    }

    #[test]
    fn language_config_values_are_unique() {
        for (index, language) in Language::ALL.iter().enumerate() {
            for other in &Language::ALL[index + 1..] {
                assert_ne!(language.config_value(), other.config_value());
            }
        }
    }

    #[test]
    fn normalize_config_resets_invalid_values() {
        let logger = test_logger();
        let mut config = Config {
            interval: 999,
            fps: 0,
            scale: -1.0,
            ..Config::default()
        };

        normalize_config(&mut config, &logger);

        assert_eq!(config.interval, Config::default().interval);
        assert_eq!(config.fps, Config::default().fps);
        assert_eq!(config.scale, Config::default().scale);
    }

    #[test]
    fn normalize_config_caps_large_scale() {
        let logger = test_logger();
        let mut config = Config {
            scale: 99.0,
            ..Config::default()
        };

        normalize_config(&mut config, &logger);

        assert_eq!(config.scale, MAX_SCALE);
    }

    #[test]
    fn cloned_config_returns_snapshot() {
        let config = Arc::new(Mutex::new(Config {
            interval: 10,
            ..Config::default()
        }));

        let cloned = cloned_config(&config).expect("clone config");

        assert_eq!(cloned.interval, 10);
    }
}
