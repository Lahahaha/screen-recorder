use crate::{
    app::AppResult,
    config::{Config, ScreenshotFormat},
    logging::Logger,
    paths::AppPaths,
    platform,
    temp::TempFileCleanup,
};
use chrono::Local;
use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use screenshots::Screen;
use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    io::{self, BufWriter, Write},
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
            platform::notify_screen_capture_failure(logger, config.language);
        })?
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "没有可用屏幕"))?;
    let screenshot = screen.capture().inspect_err(|_| {
        platform::notify_screen_capture_failure(logger, config.language);
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
    save_screenshot_atomic(&image, format, &output)?;

    store_screenshot_fingerprint(screenshot_hash, output.clone())?;
    logger.info(format!("已保存截图: {}", output.display()));
    Ok(output)
}

fn save_screenshot_atomic(
    image: &DynamicImage,
    format: ScreenshotFormat,
    output: &Path,
) -> AppResult<()> {
    let parent = output.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("截图路径缺少父目录: {}", output.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let file_name = output
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("screenshot");
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        Local::now().format("%Y%m%d%H%M%S%.3f")
    ));
    let mut temp_cleanup = TempFileCleanup::new(temp_path.clone());
    {
        let file = fs::File::create(&temp_path)?;
        let mut writer = BufWriter::new(file);
        write_screenshot_image(image, format, &mut writer)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
    }

    platform::replace_file(&temp_path, output)?;
    temp_cleanup.disarm();
    Ok(())
}

fn write_screenshot_image<W: Write + std::io::Seek>(
    image: &DynamicImage,
    format: ScreenshotFormat,
    writer: &mut W,
) -> AppResult<()> {
    match format {
        ScreenshotFormat::Png => {
            image.write_to(writer, ImageFormat::Png)?;
        }
        ScreenshotFormat::Jpg => {
            DynamicImage::ImageRgb8(image.to_rgb8()).write_to(writer, ImageFormat::Jpeg)?;
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-capture-test-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f")
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn screenshot_format_for_path_accepts_supported_extensions() {
        assert_eq!(
            screenshot_format_for_path(Path::new("screen.PNG")),
            Some(ScreenshotFormat::Png)
        );
        assert_eq!(
            screenshot_format_for_path(Path::new("screen.jpg")),
            Some(ScreenshotFormat::Jpg)
        );
        assert_eq!(
            screenshot_format_for_path(Path::new("screen.JPEG")),
            Some(ScreenshotFormat::Jpg)
        );
    }

    #[test]
    fn screenshot_format_for_path_rejects_unknown_extensions() {
        assert_eq!(screenshot_format_for_path(Path::new("screen.gif")), None);
        assert_eq!(screenshot_format_for_path(Path::new("screen")), None);
    }

    #[test]
    fn prepare_screenshot_image_scales_dimensions() {
        let image = RgbaImage::new(10, 6);
        let scaled = prepare_screenshot_image(image, 0.5);

        assert_eq!(scaled.width(), 5);
        assert_eq!(scaled.height(), 3);
    }

    #[test]
    fn prepare_screenshot_image_keeps_minimum_one_pixel() {
        let image = RgbaImage::new(1, 1);
        let scaled = prepare_screenshot_image(image, 0.1);

        assert_eq!(scaled.width(), 1);
        assert_eq!(scaled.height(), 1);
    }

    #[test]
    fn save_screenshot_atomic_writes_final_file() {
        let dir = test_dir();
        let output = dir.join("screen.png");
        let image = DynamicImage::ImageRgba8(RgbaImage::new(2, 2));

        save_screenshot_atomic(&image, ScreenshotFormat::Png, &output).expect("save screenshot");

        assert!(output.exists());
        assert!(fs::read_dir(&dir)
            .expect("read test dir")
            .all(|entry| !entry
                .expect("read entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")));
    }
}
