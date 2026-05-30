use crate::{
    app::AppResult,
    config::{Config, ScreenshotFormat},
    logging::Logger,
    paths::AppPaths,
    platform,
};
use chrono::Local;
use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use screenshots::Screen;
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

static SCREENSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LAST_SCREENSHOT: Mutex<Option<ScreenshotFingerprint>> = Mutex::new(None);

#[derive(Clone, Debug)]
struct ScreenshotFingerprint {
    hash: u64,
    path: PathBuf,
}

pub(crate) fn capture_once(
    paths: &AppPaths,
    config: &Config,
    logger: &Logger,
) -> AppResult<PathBuf> {
    let timestamp = Local::now();
    let today = timestamp.format("%Y-%m-%d").to_string();
    let now = timestamp.format("%H-%M-%S%.3f").to_string();
    let format = config.image_format;
    let output_dir = paths.screenshots_dir_for_date(&today);
    fs::create_dir_all(&output_dir)?;

    let screen = Screen::all()
        .inspect_err(|_| {
            platform::notify_screen_capture_failure(logger);
        })?
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "没有可用屏幕"))?;
    let screenshot = screen.capture().inspect_err(|_| {
        platform::notify_screen_capture_failure(logger);
    })?;
    let width = screenshot.width();
    let height = screenshot.height();
    let rgba = screenshot.into_raw();
    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "截屏像素数据尺寸不匹配"))?;

    let image = prepare_screenshot_image(image, config.scale);
    let screenshot_hash = rgba_buffer_hash(&image);
    if config.dedup {
        if let Some(previous) = duplicate_screenshot_path(&output_dir, screenshot_hash)? {
            logger.info(format!("跳过重复截图: {}", previous.display()));
            return Ok(previous);
        }
    }

    let sequence = SCREENSHOT_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let output = output_dir.join(format!("{now}-{sequence:06}.{}", format.extension()));
    match format {
        ScreenshotFormat::Png => {
            image.save_with_format(&output, ImageFormat::Png)?;
        }
        ScreenshotFormat::Jpg => {
            image
                .to_rgb8()
                .save_with_format(&output, ImageFormat::Jpeg)?;
        }
    }

    store_screenshot_fingerprint(screenshot_hash, output.clone())?;
    logger.info(format!("已保存截图: {}", output.display()));
    Ok(output)
}

fn prepare_screenshot_image(image: RgbaImage, scale: f32) -> DynamicImage {
    let image = DynamicImage::ImageRgba8(image);
    if (scale - 1.0).abs() < f32::EPSILON {
        return image;
    }

    let width = ((image.width() as f32 * scale).round() as u32).max(1);
    let height = ((image.height() as f32 * scale).round() as u32).max(1);
    image.resize_exact(width, height, FilterType::Lanczos3)
}

fn duplicate_screenshot_path(
    output_dir: &Path,
    screenshot_hash: u64,
) -> AppResult<Option<PathBuf>> {
    let last_screenshot = LAST_SCREENSHOT
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?;
    if let Some(previous) = last_screenshot.as_ref() {
        let same_dir = previous.path.parent() == Some(output_dir);
        if same_dir && previous.hash == screenshot_hash && previous.path.exists() {
            return Ok(Some(previous.path.clone()));
        }
    }
    Ok(None)
}

fn store_screenshot_fingerprint(screenshot_hash: u64, path: PathBuf) -> AppResult<()> {
    let mut last_screenshot = LAST_SCREENSHOT
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?;
    *last_screenshot = Some(ScreenshotFingerprint {
        hash: screenshot_hash,
        path,
    });
    Ok(())
}

fn rgba_buffer_hash(image: &DynamicImage) -> u64 {
    let rgba = image.to_rgba8();
    let mut hasher = DefaultHasher::new();
    rgba.width().hash(&mut hasher);
    rgba.height().hash(&mut hasher);
    rgba.as_raw().hash(&mut hasher);
    hasher.finish()
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
