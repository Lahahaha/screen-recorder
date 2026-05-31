mod dedup;
mod geometry;

use self::{
    dedup::{
        clear_capture_fingerprint, duplicate_capture_report, screen_fingerprints,
        store_capture_fingerprint, CaptureBatchFingerprint,
    },
    geometry::{
        captured_geometry, geometry_inputs_for_targets, scaled_geometries,
        single_screen_geometries, SavedGeometry,
    },
};
use crate::{
    app::AppResult,
    config::{CaptureMode, Config, ScreenshotFormat},
    logging::Logger,
    paths::AppPaths,
    platform,
    screenshot_naming::{
        multi_screen_metadata_file_name, parse_screenshot_file_name, screenshot_file_name,
        MultiScreenCaptureMetadata, ScreenCaptureMetadata,
    },
    temp::TempFileCleanup,
};
use chrono::{Local, SecondsFormat};
use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use screenshots::Screen;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CaptureScreenInfo {
    pub(crate) index: u32,
    pub(crate) id: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale_factor: f64,
    pub(crate) is_primary: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CaptureReport {
    pub(crate) saved_paths: Vec<PathBuf>,
    pub(crate) previous_paths: Vec<PathBuf>,
    pub(crate) metadata_path: Option<PathBuf>,
    pub(crate) target_screen_count: usize,
    pub(crate) failed_screen_count: usize,
    pub(crate) skipped_duplicate: bool,
}

impl CaptureReport {
    pub(crate) fn saved_count(&self) -> usize {
        self.saved_paths.len()
    }

    pub(crate) fn display_paths(&self) -> &[PathBuf] {
        if self.saved_paths.is_empty() {
            &self.previous_paths
        } else {
            &self.saved_paths
        }
    }

    pub(crate) fn output_dir(&self) -> Option<&Path> {
        self.display_paths()
            .first()
            .and_then(|path| path.parent())
            .or_else(|| self.metadata_path.as_deref().and_then(Path::parent))
    }
}

pub(crate) struct CaptureSession {
    capture_gate: Mutex<()>,
    state: Mutex<CaptureState>,
}

impl Default for CaptureSession {
    fn default() -> Self {
        Self {
            capture_gate: Mutex::new(()),
            state: Mutex::new(CaptureState::default()),
        }
    }
}

#[derive(Default)]
struct CaptureState {
    next_sequence_by_dir: BTreeMap<PathBuf, u64>,
    last_capture: Option<CaptureBatchFingerprint>,
}

#[derive(Clone)]
struct IndexedScreen {
    info: CaptureScreenInfo,
    screen: Screen,
}

#[derive(Clone)]
struct CapturedScreen {
    info: CaptureScreenInfo,
    image: DynamicImage,
}

pub(crate) fn available_screen_infos() -> AppResult<Vec<CaptureScreenInfo>> {
    let screens = Screen::all()?;
    let infos = assign_screen_indices_to_entries(
        screens
            .iter()
            .map(|screen| ((), screen_info_without_index(screen)))
            .collect::<Vec<_>>(),
    )
    .into_iter()
    .map(|(_, info)| info)
    .collect();
    Ok(infos)
}

impl CaptureSession {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn lock_state(&self) -> AppResult<std::sync::MutexGuard<'_, CaptureState>> {
        self.state
            .lock()
            .map_err(|error| io::Error::other(error.to_string()).into())
    }

    fn lock_capture_gate(&self) -> std::sync::MutexGuard<'_, ()> {
        match self.capture_gate.lock() {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        }
    }

    pub(crate) fn capture_once(
        &self,
        paths: &AppPaths,
        config: &Config,
        logger: &Logger,
    ) -> AppResult<CaptureReport> {
        let _capture_gate = self.lock_capture_gate();
        let timestamp = Local::now();
        let today = timestamp.format("%Y-%m-%d").to_string();
        let now = timestamp.format("%H-%M-%S%.3f").to_string();
        let captured_at = timestamp.to_rfc3339_opts(SecondsFormat::Millis, false);
        let output_dir = paths.screenshots_dir_for_date(&today);
        fs::create_dir_all(&output_dir)?;

        let screens = indexed_screens().inspect_err(|_| {
            platform::notify_screen_capture_failure(logger, config.language);
        })?;
        let targets = select_capture_targets(&screens, config.capture_mode)?;
        let target_screen_count = targets.len();
        let use_multi_names =
            matches!(config.capture_mode, CaptureMode::Auto) && target_screen_count > 1;

        let mut captured = Vec::new();
        let mut failed_screen_count = 0_usize;
        for target in &targets {
            match capture_target_screen(target, config.scale) {
                Ok(capture) => captured.push(capture),
                Err(error) if use_multi_names => {
                    failed_screen_count += 1;
                    logger.warn(format!(
                        "跳过截屏失败的屏幕 screen-{:02}: {error}",
                        target.info.index
                    ));
                }
                Err(error) => {
                    platform::notify_screen_capture_failure(logger, config.language);
                    return Err(io::Error::other(format!(
                        "屏幕 screen-{:02} 截屏失败: {error}",
                        target.info.index
                    ))
                    .into());
                }
            }
        }

        if captured.is_empty() {
            platform::notify_screen_capture_failure(logger, config.language);
            return Err(io::Error::other("所有目标屏幕截屏失败").into());
        }

        let geometries = if use_multi_names {
            let inputs = geometry_inputs_for_targets(&targets, &captured, config.scale);
            scaled_geometries(&inputs)
        } else {
            single_screen_geometries(&captured)
        };

        let (sequence, screen_fingerprints) = {
            let mut state = self.lock_state()?;
            let screen_fingerprints = if config.dedup {
                let screen_fingerprints = screen_fingerprints(&captured, &geometries);
                if let Some(report) = duplicate_capture_report(
                    &mut state,
                    &output_dir,
                    &screen_fingerprints,
                    target_screen_count,
                    failed_screen_count,
                ) {
                    logger.info("跳过重复截图批次");
                    return Ok(report);
                }
                Some(screen_fingerprints)
            } else {
                clear_capture_fingerprint(&mut state);
                None
            };
            let sequence = reserve_capture_sequence(
                &mut state,
                &output_dir,
                &now,
                use_multi_names,
                config.image_format,
                &captured,
            )?;
            (sequence, screen_fingerprints)
        };

        let report = save_capture_batch(SaveCaptureBatch {
            output_dir: &output_dir,
            timestamp: &now,
            captured_at: &captured_at,
            sequence,
            format: config.image_format,
            use_multi_names,
            captured: &captured,
            geometries: &geometries,
            target_screen_count,
            failed_screen_count,
            logger,
        })?;

        if let Some(screen_fingerprints) = screen_fingerprints {
            let mut state = self.lock_state()?;
            store_capture_fingerprint(
                &mut state,
                CaptureBatchFingerprint {
                    output_dir,
                    screens: screen_fingerprints,
                    paths: report.saved_paths.clone(),
                    metadata_path: report.metadata_path.clone(),
                },
            );
        }

        logger.info(format!(
            "已保存截图批次: {} 张，失败屏幕: {}",
            report.saved_count(),
            report.failed_screen_count
        ));
        Ok(report)
    }
}

fn indexed_screens() -> AppResult<Vec<IndexedScreen>> {
    let screens = Screen::all()?;
    let entries = assign_screen_indices_to_entries(
        screens
            .into_iter()
            .map(|screen| {
                let info = screen_info_without_index(&screen);
                (screen, info)
            })
            .collect::<Vec<_>>(),
    )
    .into_iter()
    .map(|(screen, info)| IndexedScreen { info, screen })
    .collect::<Vec<_>>();
    if entries.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "没有可用屏幕").into());
    }
    Ok(entries)
}

fn screen_info_without_index(screen: &Screen) -> CaptureScreenInfo {
    let info = screen.display_info;
    CaptureScreenInfo {
        index: 0,
        id: info.id,
        x: info.x,
        y: info.y,
        width: info.width,
        height: info.height,
        scale_factor: f64::from(info.scale_factor),
        is_primary: info.is_primary,
    }
}

#[cfg(test)]
fn assign_screen_indices(infos: Vec<CaptureScreenInfo>) -> Vec<CaptureScreenInfo> {
    assign_screen_indices_to_entries(infos.into_iter().map(|info| ((), info)).collect())
        .into_iter()
        .map(|(_, info)| info)
        .collect()
}

fn assign_screen_indices_to_entries<T>(
    mut entries: Vec<(T, CaptureScreenInfo)>,
) -> Vec<(T, CaptureScreenInfo)> {
    entries.sort_by_key(|(_, info)| (!info.is_primary, info.x, info.y, info.id));
    for (index, (_, info)) in entries.iter_mut().enumerate() {
        info.index = (index + 1) as u32;
    }
    entries
}

fn select_capture_targets(
    screens: &[IndexedScreen],
    capture_mode: CaptureMode,
) -> AppResult<Vec<IndexedScreen>> {
    match capture_mode {
        CaptureMode::Auto => Ok(screens.to_vec()),
        CaptureMode::Screen(index) => screens
            .iter()
            .find(|screen| screen.info.index == index)
            .cloned()
            .map(|screen| vec![screen])
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("所选屏幕 screen-{index:02} 不可用"),
                )
                .into()
            }),
    }
}

pub(crate) fn validate_capture_mode_for_screens(
    capture_mode: CaptureMode,
    screen_infos: &[CaptureScreenInfo],
) -> CaptureMode {
    match capture_mode {
        CaptureMode::Auto => CaptureMode::Auto,
        CaptureMode::Screen(index) if screen_infos.iter().any(|screen| screen.index == index) => {
            capture_mode
        }
        CaptureMode::Screen(_) => CaptureMode::Auto,
    }
}

fn capture_target_screen(target: &IndexedScreen, scale: f32) -> AppResult<CapturedScreen> {
    let screenshot = target.screen.capture()?;
    let width = screenshot.width();
    let height = screenshot.height();
    let rgba = screenshot.into_raw();
    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "截屏像素数据尺寸不匹配"))?;
    let image = prepare_screenshot_image(image, scale);
    Ok(CapturedScreen {
        info: target.info,
        image,
    })
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

fn save_metadata_atomic(metadata: &MultiScreenCaptureMetadata, output: &Path) -> AppResult<()> {
    let parent = output.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("metadata 路径缺少父目录: {}", output.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    let file_name = output
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .unwrap_or("screens.json");
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        Local::now().format("%Y%m%d%H%M%S%.3f")
    ));
    let mut temp_cleanup = TempFileCleanup::new(temp_path.clone());
    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(serde_json::to_string_pretty(metadata)?.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    platform::replace_file(&temp_path, output)?;
    temp_cleanup.disarm();
    Ok(())
}

struct SaveCaptureBatch<'a> {
    output_dir: &'a Path,
    timestamp: &'a str,
    captured_at: &'a str,
    sequence: u64,
    format: ScreenshotFormat,
    use_multi_names: bool,
    captured: &'a [CapturedScreen],
    geometries: &'a BTreeMap<u32, SavedGeometry>,
    target_screen_count: usize,
    failed_screen_count: usize,
    logger: &'a Logger,
}

fn save_capture_batch(input: SaveCaptureBatch<'_>) -> AppResult<CaptureReport> {
    let mut saved_paths = Vec::new();
    let mut metadata_screens = Vec::new();

    for capture in input.captured {
        let file_name = screenshot_file_name(
            input.timestamp,
            input.use_multi_names.then_some(capture.info.index),
            input.sequence,
            input.format,
        );
        let output = input.output_dir.join(&file_name);
        if let Err(error) = save_screenshot_atomic(&capture.image, input.format, &output) {
            cleanup_saved_files(&saved_paths, None, input.logger);
            return Err(error);
        }

        if input.use_multi_names {
            let geometry = input
                .geometries
                .get(&capture.info.index)
                .copied()
                .unwrap_or_else(|| captured_geometry(capture));
            metadata_screens.push(ScreenCaptureMetadata {
                screen_index: capture.info.index,
                file: file_name,
                x: Some(geometry.x),
                y: Some(geometry.y),
                width: geometry.width,
                height: geometry.height,
                scale_factor: Some(capture.info.scale_factor),
            });
        }
        saved_paths.push(output);
    }

    let metadata_path = if input.use_multi_names {
        let metadata = MultiScreenCaptureMetadata {
            captured_at: input.captured_at.to_string(),
            sequence: input.sequence,
            screens: metadata_screens,
        };
        let path = input.output_dir.join(multi_screen_metadata_file_name(
            input.timestamp,
            input.sequence,
        ));
        if let Err(error) = save_metadata_atomic(&metadata, &path) {
            cleanup_saved_files(&saved_paths, Some(&path), input.logger);
            return Err(error);
        }
        Some(path)
    } else {
        None
    };

    Ok(CaptureReport {
        saved_paths,
        previous_paths: Vec::new(),
        metadata_path,
        target_screen_count: input.target_screen_count,
        failed_screen_count: input.failed_screen_count,
        skipped_duplicate: false,
    })
}

fn reserve_capture_sequence(
    state: &mut CaptureState,
    output_dir: &Path,
    timestamp: &str,
    use_multi_names: bool,
    format: ScreenshotFormat,
    captured: &[CapturedScreen],
) -> AppResult<u64> {
    let mut sequence = match state.next_sequence_by_dir.get(output_dir) {
        Some(sequence) => *sequence,
        None => next_sequence_after_existing_files(output_dir)?,
    };

    loop {
        if !capture_batch_paths_exist(
            output_dir,
            timestamp,
            sequence,
            use_multi_names,
            format,
            captured,
        ) {
            state.next_sequence_by_dir.insert(
                output_dir.to_path_buf(),
                sequence
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("截图序号已达到最大值"))?,
            );
            return Ok(sequence);
        }
        sequence = sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("截图序号已达到最大值"))?;
    }
}

fn next_sequence_after_existing_files(output_dir: &Path) -> AppResult<u64> {
    let mut next_sequence = 0_u64;
    if !output_dir.exists() {
        return Ok(next_sequence);
    }

    for entry in fs::read_dir(output_dir)? {
        let path = entry?.path();
        let Some(name) = parse_screenshot_file_name(&path) else {
            continue;
        };
        let candidate = name
            .sequence()
            .checked_add(1)
            .ok_or_else(|| io::Error::other("截图序号已达到最大值"))?;
        next_sequence = next_sequence.max(candidate);
    }

    Ok(next_sequence)
}

fn capture_batch_paths_exist(
    output_dir: &Path,
    timestamp: &str,
    sequence: u64,
    use_multi_names: bool,
    format: ScreenshotFormat,
    captured: &[CapturedScreen],
) -> bool {
    let image_exists = captured.iter().any(|capture| {
        let file_name = screenshot_file_name(
            timestamp,
            use_multi_names.then_some(capture.info.index),
            sequence,
            format,
        );
        output_dir.join(file_name).exists()
    });

    image_exists
        || (use_multi_names
            && output_dir
                .join(multi_screen_metadata_file_name(timestamp, sequence))
                .exists())
}

fn cleanup_saved_files(paths: &[PathBuf], metadata_path: Option<&Path>, logger: &Logger) {
    for path in paths {
        if let Err(error) = fs::remove_file(path) {
            logger.warn(format!("清理失败截图 {}: {error}", path.display()));
        }
    }
    if let Some(path) = metadata_path {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != io::ErrorKind::NotFound {
                logger.warn(format!("清理失败 metadata {}: {error}", path.display()));
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screenshot_naming::screenshot_format_for_path;
    use chrono::Local;
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            mpsc, Arc,
        },
        thread,
        time::Duration,
    };

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-capture-test-{}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f"),
            sequence
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn test_logger(root: &Path) -> Logger {
        let paths = AppPaths {
            config: root.join("config.json"),
            screenshots: root.join("screenshots"),
            videos: root.join("videos"),
            root: root.to_path_buf(),
        };
        Logger::new(&paths).expect("create test logger")
    }

    fn screen_info(index: u32, x: i32, y: i32, width: u32, height: u32) -> CaptureScreenInfo {
        CaptureScreenInfo {
            index,
            id: index,
            x,
            y,
            width,
            height,
            scale_factor: 1.0,
            is_primary: false,
        }
    }

    fn captured_screen(index: u32, width: u32, height: u32) -> CapturedScreen {
        CapturedScreen {
            info: screen_info(index, 0, 0, width, height),
            image: DynamicImage::ImageRgba8(RgbaImage::new(width, height)),
        }
    }

    #[test]
    fn capture_session_gate_serializes_capture_work() {
        let session = Arc::new(CaptureSession::new());
        let gate = session.lock_capture_gate();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let worker_session = Arc::clone(&session);
        let handle = thread::spawn(move || {
            started_tx.send(()).expect("send worker start");
            let _gate = worker_session.lock_capture_gate();
            acquired_tx.send(()).expect("send gate acquired");
        });

        started_rx.recv().expect("worker started");
        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());

        drop(gate);
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker acquired gate after release");
        handle.join().expect("worker joined");
    }

    #[test]
    fn capture_session_gate_recovers_after_panic() {
        let session = Arc::new(CaptureSession::new());
        let poison_session = Arc::clone(&session);
        let result = thread::spawn(move || {
            let _gate = poison_session.lock_capture_gate();
            panic!("poison capture gate");
        })
        .join();

        assert!(result.is_err());
        let _gate = session.lock_capture_gate();
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

    #[test]
    fn save_single_screen_batch_keeps_legacy_name() {
        let dir = test_dir();
        let logger = test_logger(&dir);
        let captured = vec![captured_screen(1, 4, 3)];
        let geometries = BTreeMap::from([(
            1,
            SavedGeometry {
                x: 0,
                y: 0,
                width: 4,
                height: 3,
            },
        )]);

        let report = save_capture_batch(SaveCaptureBatch {
            output_dir: &dir,
            timestamp: "01-02-03.004",
            captured_at: "2026-05-30T01:02:03.004+08:00",
            sequence: 7,
            format: ScreenshotFormat::Png,
            use_multi_names: false,
            captured: &captured,
            geometries: &geometries,
            target_screen_count: 1,
            failed_screen_count: 0,
            logger: &logger,
        })
        .expect("save batch");

        assert_eq!(
            report.saved_paths,
            vec![dir.join("01-02-03.004-000007.png")]
        );
        assert!(report.metadata_path.is_none());
        assert_eq!(report.saved_count(), 1);
    }

    #[test]
    fn save_multi_screen_batch_writes_metadata_for_successful_screens() {
        let dir = test_dir();
        let logger = test_logger(&dir);
        let mut captured = vec![captured_screen(2, 20, 10)];
        captured[0].info.x = 100;
        let geometries = BTreeMap::from([(
            2,
            SavedGeometry {
                x: 200,
                y: 0,
                width: 20,
                height: 10,
            },
        )]);

        let report = save_capture_batch(SaveCaptureBatch {
            output_dir: &dir,
            timestamp: "01-02-03.004",
            captured_at: "2026-05-30T01:02:03.004+08:00",
            sequence: 9,
            format: ScreenshotFormat::Png,
            use_multi_names: true,
            captured: &captured,
            geometries: &geometries,
            target_screen_count: 2,
            failed_screen_count: 1,
            logger: &logger,
        })
        .expect("save batch");

        let expected_image = dir.join("01-02-03.004-screen-02-000009.png");
        let expected_metadata = dir.join("01-02-03.004-000009.screens.json");
        assert_eq!(report.saved_paths, vec![expected_image]);
        assert_eq!(report.metadata_path, Some(expected_metadata.clone()));
        assert_eq!(report.target_screen_count, 2);
        assert_eq!(report.failed_screen_count, 1);

        let metadata = fs::read_to_string(expected_metadata).expect("read metadata");
        let metadata: MultiScreenCaptureMetadata =
            serde_json::from_str(&metadata).expect("parse metadata");
        assert_eq!(metadata.sequence, 9);
        assert_eq!(metadata.screens.len(), 1);
        assert_eq!(metadata.screens[0].screen_index, 2);
        assert_eq!(
            metadata.screens[0].file,
            "01-02-03.004-screen-02-000009.png"
        );
        assert_eq!(metadata.screens[0].x, Some(200));
        assert_eq!(metadata.screens[0].width, 20);
    }

    #[test]
    fn reserve_sequence_starts_at_zero_for_empty_dir() {
        let dir = test_dir();
        let mut state = CaptureState::default();
        let captured = vec![captured_screen(1, 4, 3)];

        let sequence = reserve_capture_sequence(
            &mut state,
            &dir,
            "01-02-03.004",
            false,
            ScreenshotFormat::Png,
            &captured,
        )
        .expect("reserve sequence");

        assert_eq!(sequence, 0);
    }

    #[test]
    fn reserve_sequence_continues_after_existing_single_screen_files() {
        let dir = test_dir();
        fs::write(dir.join("01-02-03.004-000041.png"), b"image").expect("write image");
        fs::write(dir.join("not-a-capture.png"), b"image").expect("write noise");
        let mut state = CaptureState::default();
        let captured = vec![captured_screen(1, 4, 3)];

        let sequence = reserve_capture_sequence(
            &mut state,
            &dir,
            "02-03-04.005",
            false,
            ScreenshotFormat::Png,
            &captured,
        )
        .expect("reserve sequence");

        assert_eq!(sequence, 42);
    }

    #[test]
    fn reserve_sequence_continues_after_existing_multi_screen_files() {
        let dir = test_dir();
        fs::write(dir.join("01-02-03.004-screen-02-000010.jpg"), b"image").expect("write image");
        let mut state = CaptureState::default();
        let captured = vec![captured_screen(2, 4, 3)];

        let sequence = reserve_capture_sequence(
            &mut state,
            &dir,
            "02-03-04.005",
            true,
            ScreenshotFormat::Png,
            &captured,
        )
        .expect("reserve sequence");

        assert_eq!(sequence, 11);
    }

    #[test]
    fn reserve_sequence_skips_existing_targets_and_metadata() {
        let dir = test_dir();
        fs::write(
            dir.join("01-02-03.004-screen-02-000007.png"),
            b"existing image",
        )
        .expect("write image");
        fs::write(
            dir.join("01-02-03.004-000008.screens.json"),
            b"existing metadata",
        )
        .expect("write metadata");
        let mut state = CaptureState {
            next_sequence_by_dir: BTreeMap::from([(dir.clone(), 7)]),
            last_capture: None,
        };
        let captured = vec![captured_screen(2, 4, 3)];

        let sequence = reserve_capture_sequence(
            &mut state,
            &dir,
            "01-02-03.004",
            true,
            ScreenshotFormat::Png,
            &captured,
        )
        .expect("reserve sequence");

        assert_eq!(sequence, 9);
    }

    #[test]
    fn screen_indices_put_primary_first_then_position() {
        let mut primary = screen_info(10, 100, 0, 100, 100);
        primary.is_primary = true;
        let infos = assign_screen_indices(vec![
            screen_info(2, 200, 0, 100, 100),
            primary,
            screen_info(1, -100, 0, 100, 100),
        ]);

        assert_eq!(infos[0].id, 10);
        assert_eq!(infos[0].index, 1);
        assert_eq!(infos[1].id, 1);
        assert_eq!(infos[1].index, 2);
        assert_eq!(infos[2].id, 2);
        assert_eq!(infos[2].index, 3);
    }

    #[test]
    fn screen_index_assignment_keeps_entries_attached_to_infos() {
        let mut primary = screen_info(10, 100, 0, 100, 100);
        primary.is_primary = true;
        let entries = assign_screen_indices_to_entries(vec![
            ("right", screen_info(2, 200, 0, 100, 100)),
            ("primary", primary),
            ("left", screen_info(1, -100, 0, 100, 100)),
        ]);

        let labels = entries
            .iter()
            .map(|(label, info)| (*label, info.id, info.index))
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![("primary", 10, 1), ("left", 1, 2), ("right", 2, 3)]
        );
    }

    #[test]
    #[ignore = "requires a real multi-screen desktop session and screen capture permission"]
    fn real_multi_screen_capture_smoke() {
        let dir = test_dir();
        let paths = AppPaths {
            config: dir.join("config.json"),
            screenshots: dir.join("screenshots"),
            videos: dir.join("videos"),
            root: dir.clone(),
        };
        fs::create_dir_all(&paths.screenshots).expect("create screenshots dir");
        fs::create_dir_all(&paths.videos).expect("create videos dir");
        let logger = Logger::new(&paths).expect("create logger");
        let config = Config {
            capture_mode: CaptureMode::Auto,
            dedup: false,
            ..Config::default()
        };
        let screens = available_screen_infos().expect("list screens");
        assert!(
            screens.len() >= 2,
            "expected at least two screens, found {}",
            screens.len()
        );

        let session = CaptureSession::new();
        let report = session
            .capture_once(&paths, &config, &logger)
            .expect("capture once");

        assert_eq!(report.target_screen_count, screens.len());
        assert_eq!(report.failed_screen_count, 0);
        assert_eq!(report.saved_count(), screens.len());
        assert!(
            report.metadata_path.is_some(),
            "multi-screen capture should write metadata"
        );

        let mut saved_names = report
            .saved_paths
            .iter()
            .map(|path| {
                assert!(path.exists(), "missing screenshot {}", path.display());
                image::open(path).expect("read saved screenshot");
                path.file_name()
                    .and_then(|file_name| file_name.to_str())
                    .expect("screenshot file name")
                    .to_string()
            })
            .collect::<Vec<_>>();
        saved_names.sort();
        assert!(saved_names[0].contains("-screen-01-"));
        assert!(saved_names[1].contains("-screen-02-"));

        let metadata_path = report.metadata_path.expect("metadata path");
        let metadata = fs::read_to_string(&metadata_path).expect("read metadata");
        let metadata: MultiScreenCaptureMetadata =
            serde_json::from_str(&metadata).expect("parse metadata");
        assert_eq!(metadata.screens.len(), screens.len());
        assert_eq!(metadata.screens[0].screen_index, 1);
        assert_eq!(metadata.screens[1].screen_index, 2);
        for screen in metadata.screens {
            assert!(
                report.saved_paths.iter().any(|path| path
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    == Some(screen.file.as_str())),
                "metadata references missing file {}",
                screen.file
            );
            assert!(screen.width > 0);
            assert!(screen.height > 0);
            assert!(screen.x.is_some());
            assert!(screen.y.is_some());
        }
    }
}
