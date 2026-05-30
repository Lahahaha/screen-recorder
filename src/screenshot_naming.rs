use crate::config::ScreenshotFormat;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScreenshotName {
    Single {
        timestamp: String,
        sequence: u64,
    },
    MultiScreen {
        timestamp: String,
        screen_index: u32,
        sequence: u64,
    },
}

impl ScreenshotName {
    pub(crate) fn timestamp(&self) -> &str {
        match self {
            Self::Single { timestamp, .. } | Self::MultiScreen { timestamp, .. } => timestamp,
        }
    }

    pub(crate) fn sequence(&self) -> u64 {
        match self {
            Self::Single { sequence, .. } | Self::MultiScreen { sequence, .. } => *sequence,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct MultiScreenCaptureMetadata {
    pub(crate) captured_at: String,
    pub(crate) sequence: u64,
    pub(crate) screens: Vec<ScreenCaptureMetadata>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ScreenCaptureMetadata {
    pub(crate) screen_index: u32,
    pub(crate) file: String,
    pub(crate) x: Option<i32>,
    pub(crate) y: Option<i32>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_factor: Option<f64>,
}

pub(crate) fn screenshot_file_name(
    timestamp: &str,
    screen_index: Option<u32>,
    sequence: u64,
    format: ScreenshotFormat,
) -> String {
    if let Some(screen_index) = screen_index {
        return multi_screen_screenshot_file_name(timestamp, screen_index, sequence, format);
    }
    single_screenshot_file_name(timestamp, sequence, format)
}

fn single_screenshot_file_name(timestamp: &str, sequence: u64, format: ScreenshotFormat) -> String {
    format!("{timestamp}-{sequence:06}.{}", format.extension())
}

fn multi_screen_screenshot_file_name(
    timestamp: &str,
    screen_index: u32,
    sequence: u64,
    format: ScreenshotFormat,
) -> String {
    format!(
        "{timestamp}-screen-{screen_index:02}-{sequence:06}.{}",
        format.extension()
    )
}

pub(crate) fn multi_screen_metadata_file_name(timestamp: &str, sequence: u64) -> String {
    format!("{timestamp}-{sequence:06}.screens.json")
}

pub(crate) fn screenshot_format_for_path(path: &Path) -> Option<ScreenshotFormat> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| match extension.to_ascii_lowercase().as_str() {
            "png" => Some(ScreenshotFormat::Png),
            "jpg" | "jpeg" => Some(ScreenshotFormat::Jpg),
            _ => None,
        })
}

pub(crate) fn parse_screenshot_file_name(path: &Path) -> Option<ScreenshotName> {
    screenshot_format_for_path(path)?;
    let stem = path.file_stem()?.to_str()?;
    let parts = stem.split('-').collect::<Vec<_>>();
    if parts.len() < 4 {
        return None;
    }

    if parts.len() == 6 && parts[3] == "screen" {
        let timestamp = parts[0..3].join("-");
        let screen_index = parts[4].parse::<u32>().ok()?;
        let sequence = parts[5].parse::<u64>().ok()?;
        if parts[4].len() != 2 || parts[5].len() != 6 || screen_index == 0 {
            return None;
        }
        return Some(ScreenshotName::MultiScreen {
            timestamp,
            screen_index,
            sequence,
        });
    }

    if parts.len() == 4 {
        let timestamp = parts[0..3].join("-");
        let sequence = parts[3].parse::<u64>().ok()?;
        if parts[3].len() != 6 {
            return None;
        }
        return Some(ScreenshotName::Single {
            timestamp,
            sequence,
        });
    }

    None
}

pub(crate) fn metadata_path_for_screenshot(path: &Path) -> Option<PathBuf> {
    let parsed = parse_screenshot_file_name(path)?;
    let file_name = multi_screen_metadata_file_name(parsed.timestamp(), parsed.sequence());
    path.parent().map(|parent| parent.join(file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_screenshot_file_name_keeps_existing_format() {
        assert_eq!(
            screenshot_file_name("14-03-22.184", None, 512, ScreenshotFormat::Png),
            "14-03-22.184-000512.png"
        );
    }

    #[test]
    fn multi_screen_screenshot_file_name_uses_screen_index() {
        assert_eq!(
            screenshot_file_name("14-03-22.184", Some(2), 512, ScreenshotFormat::Jpg),
            "14-03-22.184-screen-02-000512.jpg"
        );
    }

    #[test]
    fn multi_screen_metadata_file_name_uses_batch_sequence() {
        assert_eq!(
            multi_screen_metadata_file_name("14-03-22.184", 512),
            "14-03-22.184-000512.screens.json"
        );
    }

    #[test]
    fn parse_screenshot_file_name_accepts_legacy_single_screen() {
        assert_eq!(
            parse_screenshot_file_name(Path::new("14-03-22.184-000512.png")),
            Some(ScreenshotName::Single {
                timestamp: "14-03-22.184".to_string(),
                sequence: 512,
            })
        );
    }

    #[test]
    fn parse_screenshot_file_name_accepts_multi_screen() {
        assert_eq!(
            parse_screenshot_file_name(Path::new("14-03-22.184-screen-02-000512.jpg")),
            Some(ScreenshotName::MultiScreen {
                timestamp: "14-03-22.184".to_string(),
                screen_index: 2,
                sequence: 512,
            })
        );
    }

    #[test]
    fn parse_screenshot_file_name_rejects_nonstandard_names() {
        assert_eq!(parse_screenshot_file_name(Path::new("external.png")), None);
        assert_eq!(
            parse_screenshot_file_name(Path::new("14-03-22.184-screen-00-000512.png")),
            None
        );
        assert_eq!(
            parse_screenshot_file_name(Path::new("14-03-22.184-screen-1-000512.png")),
            None
        );
        assert_eq!(
            parse_screenshot_file_name(Path::new("14-03-22.184-screen-01-000512-extra.png")),
            None
        );
    }

    #[test]
    fn metadata_path_for_screenshot_uses_matching_batch() {
        assert_eq!(
            metadata_path_for_screenshot(Path::new("/tmp/14-03-22.184-screen-02-000512.png")),
            Some(PathBuf::from("/tmp/14-03-22.184-000512.screens.json"))
        );
    }
}
