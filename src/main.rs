#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use chrono::Local;
use image::{imageops::FilterType, DynamicImage, ImageFormat, RgbaImage};
use screenshots::Screen;
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    env,
    error::Error,
    fs,
    hash::{Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tray_icon::{
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    Icon, TrayIcon, TrayIconBuilder,
};
use winit::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
};

type AppResult<T> = Result<T, Box<dyn Error>>;

const APP_NAME: &str = "Screen Recorder";
const SUPPORTED_INTERVALS: [u64; 4] = [10, 30, 60, 120];
const MAX_SCALE: f32 = 4.0;

static SCREENSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static LAST_SCREENSHOT: Mutex<Option<ScreenshotFingerprint>> = Mutex::new(None);

#[derive(Clone, Debug)]
struct AppPaths {
    root: PathBuf,
    config: PathBuf,
    screenshots: PathBuf,
    videos: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
struct Config {
    interval: u64,
    fps: u32,
    image_format: String,
    scale: f32,
    dedup: bool,
    auto_start: bool,
    video_codec: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval: 30,
            fps: 10,
            image_format: "png".to_string(),
            scale: 1.0,
            dedup: false,
            auto_start: false,
            video_codec: "h264".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct ScreenshotFingerprint {
    hash: u64,
    path: PathBuf,
}

struct TempFileCleanup {
    path: PathBuf,
    disarmed: bool,
}

impl TempFileCleanup {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
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

struct AtomicFlagGuard {
    flag: Arc<AtomicBool>,
}

impl AtomicFlagGuard {
    fn new(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}

impl Drop for AtomicFlagGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VideoCodec {
    H264,
    H265,
}

impl VideoCodec {
    fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "h265" | "h.265" | "hevc" | "libx265" => Self::H265,
            _ => Self::H264,
        }
    }

    fn config_value(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::H265 => "h265",
        }
    }

    fn encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::H265 => "libx265",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScreenshotFormat {
    Png,
    Jpg,
}

impl ScreenshotFormat {
    fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Self::Jpg,
            _ => Self::Png,
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
        }
    }
}

struct TrayControls {
    menu: Menu,
    capture_now: MenuItem,
    start_pause: MenuItem,
    interval_menu: Submenu,
    interval_items: Vec<(u64, CheckMenuItem)>,
    generate_video: MenuItem,
    quit: MenuItem,
}

struct TrayState {
    _tray_icon: TrayIcon,
    controls: TrayControls,
}

#[derive(Clone)]
struct Logger {
    file: Arc<Mutex<fs::File>>,
}

impl Logger {
    fn new(paths: &AppPaths) -> AppResult<Self> {
        fs::create_dir_all(&paths.root)?;
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(paths.root.join("app.log"))?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    fn info(&self, message: impl AsRef<str>) {
        self.write("INFO", message.as_ref());
    }

    fn warn(&self, message: impl AsRef<str>) {
        self.write("WARN", message.as_ref());
    }

    fn error(&self, message: impl AsRef<str>) {
        self.write("ERROR", message.as_ref());
    }

    fn write(&self, level: &str, message: &str) {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        if let Ok(mut file) = self.file.lock() {
            let _ = writeln!(file, "{timestamp} [{level}] {message}");
        }
    }
}

#[derive(Clone, Default)]
struct ThreadRegistry {
    handles: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
}

impl ThreadRegistry {
    fn spawn<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let handle = thread::spawn(task);
        if let Ok(mut handles) = self.handles.lock() {
            handles.push(handle);
        }
    }

    fn join_all(&self, logger: &Logger) {
        let handles = match self.handles.lock() {
            Ok(mut handles) => handles.drain(..).collect::<Vec<_>>(),
            Err(error) => {
                logger.error(format!("读取后台线程列表失败: {error}"));
                return;
            }
        };

        for handle in handles {
            if let Err(error) = handle.join() {
                logger.error(format!("后台任务退出异常: {error:?}"));
            }
        }
    }
}

struct AppState {
    paths: AppPaths,
    config: Arc<Mutex<Config>>,
    running: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    generating_video: Arc<AtomicBool>,
    workers: ThreadRegistry,
    logger: Logger,
}

fn main() -> AppResult<()> {
    let paths = AppPaths::new()?;
    let logger = Logger::new(&paths)?;
    let initial_config = load_config(&paths, &logger)?;
    let auto_start = initial_config.auto_start;
    let config = Arc::new(Mutex::new(initial_config));
    let running = Arc::new(AtomicBool::new(auto_start));
    let shutdown = Arc::new(AtomicBool::new(false));
    let generating_video = Arc::new(AtomicBool::new(false));
    let workers = ThreadRegistry::default();

    let mut capture_thread = Some(spawn_capture_loop(
        paths.clone(),
        Arc::clone(&running),
        Arc::clone(&shutdown),
        Arc::clone(&config),
        logger.clone(),
    ));
    let app_state = AppState {
        paths,
        config,
        running,
        shutdown,
        generating_video,
        workers,
        logger,
    };

    let event_loop = EventLoopBuilder::<()>::with_user_event().build()?;
    let (menu_tx, menu_rx) = mpsc::channel::<MenuEvent>();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_tx.send(event);
    }));

    let mut tray_state: Option<TrayState> = None;
    let mut saved_on_quit = false;

    event_loop.run(move |event, event_loop| {
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(100),
        ));

        match event {
            Event::NewEvents(StartCause::Init) => {
                if tray_state.is_none() {
                    match create_tray_state(
                        &app_state.config,
                        app_state.running.load(Ordering::SeqCst),
                    ) {
                        Ok(state) => {
                            tray_state = Some(state);
                        }
                        Err(error) => {
                            app_state.logger.error(format!("创建系统托盘失败: {error}"));
                            app_state.shutdown.store(true, Ordering::SeqCst);
                            event_loop.exit();
                        }
                    }
                }
            }
            Event::AboutToWait => {
                if let Some(state) = tray_state.as_ref() {
                    for event in menu_rx.try_iter() {
                        handle_menu_event(
                            event,
                            &state.controls,
                            &app_state,
                            event_loop,
                            &mut saved_on_quit,
                        );
                    }
                }
            }
            Event::LoopExiting => {
                app_state.shutdown.store(true, Ordering::SeqCst);
                if !saved_on_quit {
                    save_current_config(&app_state.paths, &app_state.config, &app_state.logger);
                    saved_on_quit = true;
                }
                if let Some(capture_thread) = capture_thread.take() {
                    if let Err(error) = capture_thread.join() {
                        app_state
                            .logger
                            .error(format!("后台截屏线程退出异常: {error:?}"));
                    }
                }
                app_state.workers.join_all(&app_state.logger);
            }
            _ => {}
        }
    })?;

    Ok(())
}

impl AppPaths {
    fn new() -> AppResult<Self> {
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

    fn screenshots_dir_for_date(&self, date: &str) -> PathBuf {
        self.screenshots.join(date)
    }

    fn video_path_for_date(&self, date: &str) -> PathBuf {
        self.videos.join(format!("{date}.mp4"))
    }
}

fn load_config(paths: &AppPaths, logger: &Logger) -> AppResult<Config> {
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

fn normalize_config(config: &mut Config, logger: &Logger) {
    if !SUPPORTED_INTERVALS.contains(&config.interval) {
        config.interval = Config::default().interval;
    }
    if config.fps == 0 {
        config.fps = Config::default().fps;
    }
    config.image_format = ScreenshotFormat::from_config(&config.image_format)
        .extension()
        .to_string();
    if !config.scale.is_finite() || config.scale <= 0.0 {
        config.scale = Config::default().scale;
    } else if config.scale > MAX_SCALE {
        logger.warn(format!(
            "scale 配置过大，已从 {} 限制为 {}",
            config.scale, MAX_SCALE
        ));
        config.scale = MAX_SCALE;
    }
    config.video_codec = VideoCodec::from_config(&config.video_codec)
        .config_value()
        .to_string();
}

fn backup_corrupted_config(paths: &AppPaths) -> AppResult<()> {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S%.3f");
    let backup = paths.root.join(format!("config.json.corrupt.{timestamp}"));
    fs::rename(&paths.config, backup)?;
    Ok(())
}

fn save_config(paths: &AppPaths, config: &Config) -> AppResult<()> {
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
        file.write_all(format!("{content}\n").as_bytes())?;
        file.sync_all()?;
    }
    replace_file(&temp_path, &paths.config)?;
    temp_cleanup.disarm();
    Ok(())
}

fn save_current_config(paths: &AppPaths, config: &Arc<Mutex<Config>>, logger: &Logger) {
    match config.lock() {
        Ok(config) => {
            if let Err(error) = save_config(paths, &config) {
                logger.error(format!("保存配置失败: {error}"));
            }
        }
        Err(error) => logger.error(format!("读取配置失败: {error}")),
    }
}

fn create_tray_state(config: &Arc<Mutex<Config>>, is_running: bool) -> AppResult<TrayState> {
    let interval = config
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?
        .interval;
    let controls = build_menu(interval)?;
    update_running_menu(&controls, is_running);
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(controls.menu.clone()))
        .with_tooltip(APP_NAME)
        .with_icon(create_tray_icon()?)
        .build()?;
    tray_icon.set_icon_as_template(true);

    Ok(TrayState {
        _tray_icon: tray_icon,
        controls,
    })
}

fn build_menu(current_interval: u64) -> AppResult<TrayControls> {
    let menu = Menu::new();
    let capture_now = MenuItem::new("📷 截一张", true, None);
    let start_pause = MenuItem::new("▶ 开始", true, None);
    let interval_menu = Submenu::new(format!("⏱ 间隔 当前：{current_interval}s"), true);
    let generate_video = MenuItem::new("🎬 生成今日视频", true, None);
    let quit = MenuItem::new("❌ 退出", true, None);

    let interval_items = SUPPORTED_INTERVALS
        .iter()
        .map(|seconds| {
            (
                *seconds,
                CheckMenuItem::new(
                    format!("{seconds}s"),
                    true,
                    *seconds == current_interval,
                    None,
                ),
            )
        })
        .collect::<Vec<_>>();

    let interval_item_refs = interval_items
        .iter()
        .map(|(_, item)| item as &dyn IsMenuItem)
        .collect::<Vec<_>>();
    interval_menu.append_items(&interval_item_refs)?;

    menu.append_items(&[
        &capture_now as &dyn IsMenuItem,
        &PredefinedMenuItem::separator(),
        &start_pause,
        &PredefinedMenuItem::separator(),
        &interval_menu,
        &PredefinedMenuItem::separator(),
        &generate_video,
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;

    Ok(TrayControls {
        menu,
        capture_now,
        start_pause,
        interval_menu,
        interval_items,
        generate_video,
        quit,
    })
}

fn handle_menu_event(
    event: MenuEvent,
    controls: &TrayControls,
    state: &AppState,
    event_loop: &winit::event_loop::EventLoopWindowTarget<()>,
    saved_on_quit: &mut bool,
) {
    if event.id == controls.capture_now.id() {
        capture_once_in_thread(
            state.paths.clone(),
            Arc::clone(&state.config),
            state.workers.clone(),
            state.logger.clone(),
        );
        return;
    }

    if event.id == controls.start_pause.id() {
        let next = !state.running.load(Ordering::SeqCst);
        state.running.store(next, Ordering::SeqCst);
        update_running_menu(controls, next);
        return;
    }

    for (seconds, item) in &controls.interval_items {
        if event.id == item.id() {
            set_interval(*seconds, controls, &state.config, &state.logger);
            return;
        }
    }

    if event.id == controls.generate_video.id() {
        generate_today_video_in_thread(
            state.paths.clone(),
            Arc::clone(&state.config),
            Arc::clone(&state.generating_video),
            state.workers.clone(),
            state.logger.clone(),
        );
        return;
    }

    if event.id == controls.quit.id() {
        state.shutdown.store(true, Ordering::SeqCst);
        save_current_config(&state.paths, &state.config, &state.logger);
        *saved_on_quit = true;
        event_loop.exit();
    }
}

fn update_running_menu(controls: &TrayControls, is_running: bool) {
    let text = if is_running {
        "⏸ 暂停"
    } else {
        "▶ 开始"
    };
    controls.start_pause.set_text(text);
}

fn set_interval(
    seconds: u64,
    controls: &TrayControls,
    config: &Arc<Mutex<Config>>,
    logger: &Logger,
) {
    match config.lock() {
        Ok(mut config) => {
            config.interval = seconds;
            controls
                .interval_menu
                .set_text(format!("⏱ 间隔 当前：{seconds}s"));
            for (value, item) in &controls.interval_items {
                item.set_checked(*value == seconds);
            }
        }
        Err(error) => logger.error(format!("更新间隔失败: {error}")),
    }
}

fn capture_once_in_thread(
    paths: AppPaths,
    config: Arc<Mutex<Config>>,
    workers: ThreadRegistry,
    logger: Logger,
) {
    workers.clone().spawn(move || match cloned_config(&config) {
        Ok(config) => {
            if let Err(error) = capture_once(&paths, &config, &logger) {
                logger.error(format!("手动截屏失败: {error}"));
            }
        }
        Err(error) => logger.error(format!("读取配置失败: {error}")),
    });
}

fn generate_today_video_in_thread(
    paths: AppPaths,
    config: Arc<Mutex<Config>>,
    generating_video: Arc<AtomicBool>,
    workers: ThreadRegistry,
    logger: Logger,
) {
    if generating_video
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        logger.warn("已有视频生成任务正在运行，跳过本次请求");
        notify("视频生成中", "已有视频生成任务正在运行。", logger);
        return;
    }

    workers.clone().spawn(move || {
        let _generating_video_guard = AtomicFlagGuard::new(generating_video);
        match cloned_config(&config) {
            Ok(config) => {
                match generate_today_video(
                    &paths,
                    config.fps,
                    &config.image_format,
                    &config.video_codec,
                    &logger,
                ) {
                    Ok(output) => {
                        notify(
                            "视频生成成功",
                            &format!("已保存到: {}", output.display()),
                            logger.clone(),
                        );
                    }
                    Err(error) => {
                        logger.error(format!("生成今日视频失败: {error}"));
                        notify("视频生成失败", &format!("{error}"), logger.clone());
                    }
                }
            }
            Err(error) => {
                logger.error(format!("读取配置失败: {error}"));
                notify(
                    "视频生成失败",
                    &format!("读取配置失败: {error}"),
                    logger.clone(),
                );
            }
        }
    });
}

fn spawn_capture_loop(
    paths: AppPaths,
    running: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    config: Arc<Mutex<Config>>,
    logger: Logger,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut next_capture = Instant::now();
        let mut last_interval = cloned_config(&config)
            .map(|config| config.interval)
            .unwrap_or_else(|error| {
                logger.error(format!("读取配置失败: {error}"));
                Config::default().interval
            });

        while !shutdown.load(Ordering::SeqCst) {
            if running.load(Ordering::SeqCst) {
                let now = Instant::now();
                match cloned_config(&config) {
                    Ok(config) => {
                        let interval = config.interval;
                        if interval != last_interval {
                            last_interval = interval;
                            next_capture = now + Duration::from_secs(interval);
                        }

                        if now >= next_capture {
                            if let Err(error) = capture_once(&paths, &config, &logger) {
                                logger.error(format!("定时截屏失败: {error}"));
                            }
                            next_capture = now + Duration::from_secs(interval);
                        }
                    }
                    Err(error) => {
                        logger.error(format!("读取配置失败: {error}"));
                        last_interval = Config::default().interval;
                        next_capture = now + Duration::from_secs(last_interval);
                    }
                }
            } else {
                next_capture = Instant::now();
                if let Ok(config) = cloned_config(&config) {
                    last_interval = config.interval;
                }
            }

            thread::sleep(Duration::from_secs(1));
        }
    })
}

fn cloned_config(config: &Arc<Mutex<Config>>) -> AppResult<Config> {
    let config = config
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(config.clone())
}

fn capture_once(paths: &AppPaths, config: &Config, logger: &Logger) -> AppResult<PathBuf> {
    let timestamp = Local::now();
    let today = timestamp.format("%Y-%m-%d").to_string();
    let now = timestamp.format("%H-%M-%S%.3f").to_string();
    let format = ScreenshotFormat::from_config(&config.image_format);
    let output_dir = paths.screenshots_dir_for_date(&today);
    fs::create_dir_all(&output_dir)?;

    let screen = Screen::all()
        .inspect_err(|_| {
            notify_screen_capture_failure(logger);
        })?
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "没有可用屏幕"))?;
    let screenshot = screen.capture().inspect_err(|_| {
        notify_screen_capture_failure(logger);
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

fn notify_screen_capture_failure(logger: &Logger) {
    if cfg!(target_os = "macos") {
        notify(
            "截屏权限不足",
            "无法读取屏幕内容，请在系统设置中允许屏幕录制权限。",
            logger.clone(),
        );
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

struct TempDir {
    path: PathBuf,
    logger: Logger,
}

impl TempDir {
    fn new(parent: &Path, prefix: &str, logger: Logger) -> AppResult<Self> {
        fs::create_dir_all(parent)?;
        let path = parent.join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f")
        ));
        fs::create_dir(&path)?;
        Ok(Self { path, logger })
    }

    fn path(&self) -> &Path {
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

fn generate_today_video(
    paths: &AppPaths,
    fps: u32,
    image_format: &str,
    video_codec: &str,
    logger: &Logger,
) -> AppResult<PathBuf> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let screenshot_dir = paths.screenshots_dir_for_date(&today);
    if !screenshot_dir.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "今天还没有截图").into());
    }

    let mut images = fs::read_dir(&screenshot_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_supported_image(path))
        .collect::<Vec<_>>();
    images.sort();

    if images.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "今天还没有截图").into());
    }

    let (images, frame_format) = choose_video_images(images, image_format)?;
    fs::create_dir_all(&paths.videos)?;
    let work_dir = TempDir::new(&paths.videos, ".video-work", logger.clone())?;
    prepare_frame_sequence(work_dir.path(), &images, frame_format)?;

    let output = paths.video_path_for_date(&today);
    let temp_output = work_dir.path().join("output.mp4");
    let fps_value = fps.max(1).to_string();
    let ffmpeg = find_ffmpeg()?;
    let codec = VideoCodec::from_config(video_codec);
    let input_pattern = work_dir
        .path()
        .join(format!("frame_%06d.{}", frame_format.extension()));
    let mut cmd = Command::new(&ffmpeg);
    cmd.args(["-y", "-framerate", &fps_value, "-start_number", "0", "-i"])
        .arg(&input_pattern)
        .args([
            "-c:v",
            codec.encoder(),
            "-pix_fmt",
            "yuv420p",
            "-vf",
            "scale=trunc(iw/2)*2:trunc(ih/2)*2",
            "-r",
            &fps_value,
        ]);
    if codec == VideoCodec::H265 {
        cmd.args(["-tag:v", "hvc1"]);
    }
    cmd.arg(&temp_output);

    // Windows: 隐藏命令行窗口
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let status = cmd.status()?;
    if !status.success() {
        return Err(io::Error::other(format!("ffmpeg 退出码: {status}")).into());
    }

    replace_file(&temp_output, &output)?;
    logger.info(format!("已生成视频: {}", output.display()));
    Ok(output)
}

fn choose_video_images(
    mut images: Vec<PathBuf>,
    image_format: &str,
) -> AppResult<(Vec<PathBuf>, ScreenshotFormat)> {
    images.retain(|path| is_supported_image(path));
    images.sort();

    if images.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "今天还没有可用截图").into());
    }

    Ok((images, ScreenshotFormat::from_config(image_format)))
}

fn prepare_frame_sequence(
    work_dir: &Path,
    images: &[PathBuf],
    frame_format: ScreenshotFormat,
) -> AppResult<()> {
    for (index, image) in images.iter().enumerate() {
        let frame = work_dir.join(format!("frame_{index:06}.{}", frame_format.extension()));
        if screenshot_format_for_path(image) == Some(frame_format) {
            if fs::hard_link(image, &frame).is_ok() {
                continue;
            }
            fs::copy(image, &frame)?;
            continue;
        }

        let image = image::open(image)?;
        match frame_format {
            ScreenshotFormat::Png => image.save_with_format(&frame, ImageFormat::Png)?,
            ScreenshotFormat::Jpg => image
                .to_rgb8()
                .save_with_format(&frame, ImageFormat::Jpeg)?,
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    fs::rename(source, destination).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "替换文件失败 {} -> {}: {error}",
                source.display(),
                destination.display()
            ),
        )
    })?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    if !destination.exists() {
        fs::rename(source, destination).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "替换文件失败 {} -> {}: {error}",
                    source.display(),
                    destination.display()
                ),
            )
        })?;
        return Ok(());
    }

    let backup = backup_path(destination);
    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("删除旧备份文件失败 {}: {error}", backup.display()),
            )
        })?;
    }

    fs::rename(destination, &backup).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "备份目标文件失败 {} -> {}: {error}",
                destination.display(),
                backup.display()
            ),
        )
    })?;

    match fs::rename(source, destination) {
        Ok(()) => {
            fs::remove_file(&backup).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("删除备份文件失败 {}: {error}", backup.display()),
                )
            })?;
            Ok(())
        }
        Err(error) => {
            let restore_error = fs::rename(&backup, destination).err();
            let restore_message = restore_error
                .map(|restore_error| {
                    format!(
                        "；恢复备份失败 {} -> {}: {restore_error}",
                        backup.display(),
                        destination.display()
                    )
                })
                .unwrap_or_default();
            Err(io::Error::new(
                error.kind(),
                format!(
                    "替换文件失败 {} -> {}: {error}{restore_message}",
                    source.display(),
                    destination.display()
                ),
            )
            .into())
        }
    }
}

#[cfg(target_os = "windows")]
fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|file_name| file_name.to_string_lossy())
        .unwrap_or_else(|| "backup".into());
    path.with_file_name(format!("{file_name}.bak"))
}

fn find_ffmpeg() -> AppResult<PathBuf> {
    let executable = env::current_exe()?.canonicalize()?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法定位当前可执行文件所在目录"))?;

    // 1. 优先查找 exe 同目录的 ffmpeg（Windows 打包方式）
    let same_dir = if cfg!(target_os = "windows") {
        executable_dir.join("ffmpeg.exe")
    } else {
        executable_dir.join("ffmpeg")
    };
    if is_usable_executable(&same_dir) {
        return Ok(same_dir);
    }

    // 2. macOS: 查找 Resources 目录
    let bundled = executable_dir.join("../Resources/ffmpeg");
    if let Ok(bundled) = bundled.canonicalize() {
        if is_usable_executable(&bundled) {
            return Ok(bundled);
        }
    }

    // 3. 查找系统 PATH
    let ffmpeg_name = if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    find_in_path(ffmpeg_name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "找不到 ffmpeg，请将 ffmpeg 放入程序所在目录或安装到 PATH",
        )
        .into()
    })
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|dir| dir.join(binary))
            .find(|path| is_usable_executable(path))
    })
}

fn is_usable_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn is_supported_image(path: &Path) -> bool {
    screenshot_format_for_path(path).is_some()
}

fn screenshot_format_for_path(path: &Path) -> Option<ScreenshotFormat> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| match extension.to_ascii_lowercase().as_str() {
            "png" => Some(ScreenshotFormat::Png),
            "jpg" | "jpeg" => Some(ScreenshotFormat::Jpg),
            _ => None,
        })
}

fn create_tray_icon() -> AppResult<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0; (SIZE * SIZE * 4) as usize];

    for y in 7..25 {
        for x in 5..27 {
            put_pixel(&mut rgba, SIZE, x, y, [255, 255, 255, 255]);
        }
    }
    for y in 10..22 {
        for x in 8..24 {
            put_pixel(&mut rgba, SIZE, x, y, [0, 0, 0, 0]);
        }
    }
    for y in 5..9 {
        for x in 11..21 {
            put_pixel(&mut rgba, SIZE, x, y, [255, 255, 255, 255]);
        }
    }
    for y in 12..20 {
        for x in 12..20 {
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            if dx * dx + dy * dy <= 16 {
                put_pixel(&mut rgba, SIZE, x, y, [255, 255, 255, 255]);
            }
        }
    }

    Ok(Icon::from_rgba(rgba, SIZE, SIZE)?)
}

fn notify(title: &str, message: &str, logger: Logger) {
    let title = title.to_string();
    let message = message.to_string();
    thread::spawn(move || {
        let result = if cfg!(target_os = "windows") {
            let script = format!(
                "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show('{}', 'Screen Recorder - {}') | Out-Null",
                escape_powershell_single_quoted(&message),
                escape_powershell_single_quoted(&title)
            );
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-Command", &script]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            cmd.status()
        } else {
            let script = format!(
                "display notification \"{}\" with title \"Screen Recorder\" subtitle \"{}\"",
                escape_applescript_string(&message),
                escape_applescript_string(&title)
            );
            Command::new("osascript").args(["-e", &script]).status()
        };

        if let Err(error) = result {
            logger.error(format!("发送通知失败: {error}"));
        }
    });
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn escape_applescript_string(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn put_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = ((y * width + x) * 4) as usize;
    rgba[index..index + 4].copy_from_slice(&color);
}
