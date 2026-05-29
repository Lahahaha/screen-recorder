#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use chrono::Local;
use image::{DynamicImage, ImageFormat, RgbaImage};
use screenshots::Screen;
use serde::{Deserialize, Serialize};
use std::{
    env,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
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

fn main() -> AppResult<()> {
    let paths = AppPaths::new()?;
    let initial_config = load_config(&paths)?;
    let config = Arc::new(Mutex::new(initial_config));
    let running = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::new(AtomicBool::new(false));

    let mut capture_thread = Some(spawn_capture_loop(
        paths.clone(),
        Arc::clone(&running),
        Arc::clone(&shutdown),
        Arc::clone(&config),
    ));

    let event_loop = EventLoopBuilder::<()>::with_user_event().build()?;
    let (menu_tx, menu_rx) = mpsc::channel::<MenuEvent>();
    MenuEvent::set_event_handler(Some(move |event| {
        if let Err(error) = menu_tx.send(event) {
            eprintln!("发送菜单事件失败: {error}");
        }
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
                    match create_tray_state(&config) {
                        Ok(state) => tray_state = Some(state),
                        Err(error) => {
                            eprintln!("创建系统托盘失败: {error}");
                            shutdown.store(true, Ordering::SeqCst);
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
                            &paths,
                            &config,
                            &running,
                            &shutdown,
                            event_loop,
                            &mut saved_on_quit,
                        );
                    }
                }
            }
            Event::LoopExiting => {
                shutdown.store(true, Ordering::SeqCst);
                if !saved_on_quit {
                    save_current_config(&paths, &config);
                    saved_on_quit = true;
                }
                if let Some(capture_thread) = capture_thread.take() {
                    if let Err(error) = capture_thread.join() {
                        eprintln!("后台截屏线程退出异常: {error:?}");
                    }
                }
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
        if !path.exists() {
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

fn load_config(paths: &AppPaths) -> AppResult<Config> {
    if !paths.config.exists() {
        let config = Config::default();
        save_config(paths, &config)?;
        return Ok(config);
    }

    let content = fs::read_to_string(&paths.config)?;
    let mut config: Config = serde_json::from_str(&content)?;
    normalize_config(&mut config);
    Ok(config)
}

fn normalize_config(config: &mut Config) {
    if !SUPPORTED_INTERVALS.contains(&config.interval) {
        config.interval = Config::default().interval;
    }
    if config.fps == 0 {
        config.fps = Config::default().fps;
    }
    config.image_format = ScreenshotFormat::from_config(&config.image_format)
        .extension()
        .to_string();
    if config.scale <= 0.0 {
        config.scale = Config::default().scale;
    }
}

fn save_config(paths: &AppPaths, config: &Config) -> AppResult<()> {
    fs::create_dir_all(&paths.root)?;
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&paths.config, format!("{content}\n"))?;
    Ok(())
}

fn save_current_config(paths: &AppPaths, config: &Arc<Mutex<Config>>) {
    match config.lock() {
        Ok(config) => {
            if let Err(error) = save_config(paths, &config) {
                eprintln!("保存配置失败: {error}");
            }
        }
        Err(error) => eprintln!("读取配置失败: {error}"),
    }
}

fn create_tray_state(config: &Arc<Mutex<Config>>) -> AppResult<TrayState> {
    let interval = config
        .lock()
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?
        .interval;
    let controls = build_menu(interval)?;
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
    paths: &AppPaths,
    config: &Arc<Mutex<Config>>,
    running: &Arc<AtomicBool>,
    shutdown: &Arc<AtomicBool>,
    event_loop: &winit::event_loop::EventLoopWindowTarget<()>,
    saved_on_quit: &mut bool,
) {
    if event.id == controls.capture_now.id() {
        capture_once_in_thread(paths.clone(), Arc::clone(config));
        return;
    }

    if event.id == controls.start_pause.id() {
        let next = !running.load(Ordering::SeqCst);
        running.store(next, Ordering::SeqCst);
        update_running_menu(controls, next);
        return;
    }

    for (seconds, item) in &controls.interval_items {
        if event.id == item.id() {
            set_interval(*seconds, controls, config);
            return;
        }
    }

    if event.id == controls.generate_video.id() {
        generate_today_video_in_thread(paths.clone(), Arc::clone(config));
        return;
    }

    if event.id == controls.quit.id() {
        shutdown.store(true, Ordering::SeqCst);
        save_current_config(paths, config);
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

fn set_interval(seconds: u64, controls: &TrayControls, config: &Arc<Mutex<Config>>) {
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
        Err(error) => eprintln!("更新间隔失败: {error}"),
    }
}

fn capture_once_in_thread(paths: AppPaths, config: Arc<Mutex<Config>>) {
    thread::spawn(move || match cloned_config(&config) {
        Ok(config) => {
            if let Err(error) = capture_once(&paths, &config) {
                eprintln!("手动截屏失败: {error}");
            }
        }
        Err(error) => eprintln!("读取配置失败: {error}"),
    });
}

fn generate_today_video_in_thread(paths: AppPaths, config: Arc<Mutex<Config>>) {
    thread::spawn(move || match cloned_config(&config) {
        Ok(config) => match generate_today_video(&paths, config.fps) {
            Ok(output) => {
                notify("视频生成成功", &format!("已保存到: {}", output.display()));
            }
            Err(error) => {
                eprintln!("生成今日视频失败: {error}");
                notify("视频生成失败", &format!("{error}"));
            }
        },
        Err(error) => {
            eprintln!("读取配置失败: {error}");
            notify("视频生成失败", &format!("读取配置失败: {error}"));
        }
    });
}

fn spawn_capture_loop(
    paths: AppPaths,
    running: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    config: Arc<Mutex<Config>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut next_capture = Instant::now();

        while !shutdown.load(Ordering::SeqCst) {
            if running.load(Ordering::SeqCst) {
                let now = Instant::now();
                if now >= next_capture {
                    match cloned_config(&config) {
                        Ok(config) => {
                            let interval = config.interval;
                            if let Err(error) = capture_once(&paths, &config) {
                                eprintln!("定时截屏失败: {error}");
                            }
                            next_capture = now + Duration::from_secs(interval);
                        }
                        Err(error) => {
                            eprintln!("读取配置失败: {error}");
                            next_capture = now + Duration::from_secs(Config::default().interval);
                        }
                    }
                }
            } else {
                next_capture = Instant::now();
            }

            thread::sleep(Duration::from_secs(1));
        }
    })
}

fn cloned_config(config: &Arc<Mutex<Config>>) -> AppResult<Config> {
    let config = config
        .lock()
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
    Ok(config.clone())
}

fn capture_once(paths: &AppPaths, config: &Config) -> AppResult<PathBuf> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let now = Local::now().format("%H-%M-%S").to_string();
    let format = ScreenshotFormat::from_config(&config.image_format);
    let output_dir = paths.screenshots_dir_for_date(&today);
    fs::create_dir_all(&output_dir)?;

    let screen = Screen::all()?
        .into_iter()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "没有可用屏幕"))?;
    let screenshot = screen.capture()?;
    let width = screenshot.width();
    let height = screenshot.height();
    let rgba = screenshot.into_raw();
    let image = RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "截屏像素数据尺寸不匹配"))?;

    let output = output_dir.join(format!("{now}.{}", format.extension()));
    match format {
        ScreenshotFormat::Png => {
            DynamicImage::ImageRgba8(image).save_with_format(&output, ImageFormat::Png)?;
        }
        ScreenshotFormat::Jpg => {
            DynamicImage::ImageRgba8(image)
                .to_rgb8()
                .save_with_format(&output, ImageFormat::Jpeg)?;
        }
    }

    println!("已保存截图: {}", output.display());
    Ok(output)
}

fn generate_today_video(paths: &AppPaths, fps: u32) -> AppResult<PathBuf> {
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

    fs::create_dir_all(&paths.videos)?;
    let filelist = screenshot_dir.join("filelist.txt");
    write_ffmpeg_filelist(&filelist, &images)?;

    let output = paths.video_path_for_date(&today);
    let fps_value = fps.max(1).to_string();
    let ffmpeg = find_ffmpeg()?;
    let status = Command::new(&ffmpeg)
        .args(["-y", "-f", "concat", "-safe", "0", "-r", &fps_value, "-i"])
        .arg(&filelist)
        .args([
            "-c:v", "libx265", "-tag:v", "hvc1", "-pix_fmt", "yuv420p", "-r", &fps_value,
        ])
        .arg(&output)
        .status();

    let cleanup_result = fs::remove_file(&filelist);
    if let Err(error) = cleanup_result {
        eprintln!("清理临时文件失败: {error}");
    }

    let status = status?;
    if !status.success() {
        return Err(
            io::Error::new(io::ErrorKind::Other, format!("ffmpeg 退出码: {status}")).into(),
        );
    }

    println!("已生成视频: {}", output.display());
    Ok(output)
}

fn find_ffmpeg() -> AppResult<PathBuf> {
    let executable = env::current_exe()?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法定位当前可执行文件所在目录"))?;

    // 1. 优先查找 exe 同目录的 ffmpeg（Windows 打包方式）
    let same_dir = if cfg!(target_os = "windows") {
        executable_dir.join("ffmpeg.exe")
    } else {
        executable_dir.join("ffmpeg")
    };
    if same_dir.is_file() {
        return Ok(same_dir);
    }

    // 2. macOS: 查找 Resources 目录
    let bundled = executable_dir.join("../Resources/ffmpeg");
    if bundled.is_file() {
        return Ok(bundled);
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
            .find(|path| path.is_file())
    })
}

fn write_ffmpeg_filelist(filelist: &Path, images: &[PathBuf]) -> AppResult<()> {
    let mut content = String::new();
    for image in images {
        let escaped_path = escape_ffmpeg_concat_path(image);
        content.push_str("file '");
        content.push_str(&escaped_path);
        content.push_str("'\n");
    }
    fs::write(filelist, content)?;
    Ok(())
}

fn escape_ffmpeg_concat_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg"
            )
        })
        .unwrap_or(false)
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

fn notify(title: &str, message: &str) {
    if cfg!(target_os = "windows") {
        // Windows: 使用 PowerShell 消息框
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.MessageBox]::Show('{}', 'Screen Recorder - {}')",
            message.replace('\'', "''"),
            title.replace('\'', "''")
        );
        if let Err(error) = Command::new("powershell")
            .args(["-Command", &script])
            .status()
        {
            eprintln!("发送通知失败: {error}");
        }
    } else {
        // macOS: 使用 osascript
        let script = format!(
            "display notification \"{}\" with title \"Screen Recorder\" subtitle \"{}\"",
            message.replace('"', "\\\""),
            title.replace('"', "\\\"")
        );
        if let Err(error) = Command::new("osascript").args(["-e", &script]).status() {
            eprintln!("发送通知失败: {error}");
        }
    }
}

fn put_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = ((y * width + x) * 4) as usize;
    rgba[index..index + 4].copy_from_slice(&color);
}
