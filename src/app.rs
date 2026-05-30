use crate::{
    capture::{
        available_screen_infos, capture_once, validate_capture_mode_for_screens, CaptureReport,
        CaptureScreenInfo,
    },
    config::{
        cloned_config, load_config, save_config, save_current_config, CaptureMode, Config, Language,
    },
    i18n::Text,
    logging::Logger,
    paths::AppPaths,
    platform,
    temp::AtomicFlagGuard,
    tray::{self, AppCommand, TrayControls, TrayState},
    video::generate_today_video,
};
use std::{
    any::Any,
    error::Error,
    panic::{self, AssertUnwindSafe},
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tray_icon::menu::MenuEvent;
#[cfg(target_os = "macos")]
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
use winit::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
};

pub(crate) type AppResult<T> = Result<T, Box<dyn Error>>;

enum AppEvent {
    CaptureSucceeded { report: CaptureReport, notify: bool },
    CaptureFailed { message: String, notify: bool },
}

enum LastCapture {
    Saved(CaptureReport),
    Failed(String),
}

#[derive(Clone, Default)]
struct ThreadRegistry {
    handles: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
}

impl ThreadRegistry {
    fn spawn<F>(&self, task_name: &'static str, language: Language, logger: Logger, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let handle = thread::spawn(move || {
            if let Err(panic_payload) = panic::catch_unwind(AssertUnwindSafe(task)) {
                report_thread_panic(task_name, panic_payload.as_ref(), &logger, language);
            }
        });
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
    app_events: mpsc::Sender<AppEvent>,
    workers: ThreadRegistry,
    logger: Logger,
}

pub(crate) fn run() -> AppResult<()> {
    let paths = AppPaths::new()?;
    let logger = Logger::new(&paths)?;
    let mut initial_config = load_config(&paths, &logger)?;
    normalize_startup_capture_mode(&paths, &mut initial_config, &logger);
    platform::request_screen_capture_permission(&logger);
    let auto_start = initial_config.auto_start;
    let config = Arc::new(Mutex::new(initial_config));
    let running = Arc::new(AtomicBool::new(auto_start));
    let shutdown = Arc::new(AtomicBool::new(false));
    let generating_video = Arc::new(AtomicBool::new(false));
    let workers = ThreadRegistry::default();
    let (app_event_tx, app_event_rx) = mpsc::channel::<AppEvent>();

    let mut capture_thread = Some(spawn_capture_loop(
        paths.clone(),
        Arc::clone(&running),
        Arc::clone(&shutdown),
        Arc::clone(&config),
        app_event_tx.clone(),
        logger.clone(),
    ));
    let app_state = AppState {
        paths,
        config,
        running,
        shutdown,
        generating_video,
        app_events: app_event_tx,
        workers,
        logger,
    };

    let mut event_loop_builder = EventLoopBuilder::<()>::with_user_event();
    #[cfg(target_os = "macos")]
    event_loop_builder.with_activation_policy(ActivationPolicy::Accessory);

    #[allow(deprecated)]
    let event_loop = event_loop_builder.build()?;
    let (menu_tx, menu_rx) = mpsc::channel::<MenuEvent>();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_tx.send(event);
    }));

    let mut tray_state: Option<TrayState> = None;
    let mut saved_on_quit = false;
    let mut screenshot_count = 0_u64;
    let mut last_capture: Option<LastCapture> = None;

    #[allow(deprecated)]
    event_loop.run(move |event, event_loop| {
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(100),
        ));

        match event {
            Event::NewEvents(StartCause::Init) => {
                if tray_state.is_none() {
                    match tray::create_tray_state(
                        &app_state.config,
                        app_state.running.load(Ordering::SeqCst),
                    ) {
                        Ok(state) => {
                            tray_state = Some(state);
                            if let Some(state) = tray_state.as_ref() {
                                update_tray_tooltip(
                                    state,
                                    &app_state,
                                    screenshot_count,
                                    last_capture.as_ref(),
                                );
                            }
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
                if let Some(state) = tray_state.as_mut() {
                    for event in app_event_rx.try_iter() {
                        handle_app_event(
                            event,
                            &mut screenshot_count,
                            &mut last_capture,
                            &app_state.config,
                            &app_state.logger,
                        );
                        update_tray_tooltip(
                            state,
                            &app_state,
                            screenshot_count,
                            last_capture.as_ref(),
                        );
                    }

                    for event in menu_rx.try_iter() {
                        if let Some(command) = tray::command_for_event(&event, &state.controls) {
                            handle_app_command(
                                command,
                                &mut state.controls,
                                &app_state,
                                event_loop,
                                &mut saved_on_quit,
                            );
                            update_tray_tooltip(
                                state,
                                &app_state,
                                screenshot_count,
                                last_capture.as_ref(),
                            );
                        }
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

fn handle_app_command(
    command: AppCommand,
    controls: &mut TrayControls,
    state: &AppState,
    event_loop: &winit::event_loop::EventLoopWindowTarget<()>,
    saved_on_quit: &mut bool,
) {
    match command {
        AppCommand::CaptureNow => capture_once_in_thread(
            state.paths.clone(),
            Arc::clone(&state.config),
            state.app_events.clone(),
            &state.workers,
            state.logger.clone(),
        ),
        AppCommand::ToggleRunning => {
            let next = !state.running.load(Ordering::SeqCst);
            state.running.store(next, Ordering::SeqCst);
            let language = current_language(&state.config, &state.logger);
            tray::update_running_menu(controls, next, language);
        }
        AppCommand::SetInterval(seconds) => {
            set_interval(
                seconds,
                controls,
                &state.paths,
                &state.config,
                &state.logger,
            );
        }
        AppCommand::SetCaptureMode(capture_mode) => {
            set_capture_mode(
                capture_mode,
                controls,
                &state.paths,
                &state.config,
                &state.logger,
            );
        }
        AppCommand::RefreshCaptureSources => {
            refresh_capture_sources(controls, &state.paths, &state.config, &state.logger);
        }
        AppCommand::SetLanguage(language) => {
            set_language(
                language,
                controls,
                state.running.load(Ordering::SeqCst),
                &state.paths,
                &state.config,
                &state.logger,
            );
        }
        AppCommand::GenerateTodayVideo => generate_today_video_in_thread(
            state.paths.clone(),
            Arc::clone(&state.config),
            Arc::clone(&state.generating_video),
            &state.workers,
            state.logger.clone(),
        ),
        AppCommand::OpenHistoryVideos => {
            if let Err(error) = open_history_window() {
                state.logger.error(format!("打开历史视频窗口失败: {error}"));
                let text = Text::new(current_language(&state.config, &state.logger));
                platform::notify(
                    text.video_failed_title(),
                    &format!("{error}"),
                    state.logger.clone(),
                );
            }
        }
        AppCommand::OpenAbout => {
            if let Err(error) = open_about_window() {
                state.logger.error(format!("打开关于窗口失败: {error}"));
                let text = Text::new(current_language(&state.config, &state.logger));
                platform::notify(
                    text.about_failed_title(),
                    &format!("{error}"),
                    state.logger.clone(),
                );
            }
        }
        AppCommand::OpenOutputDir => {
            if let Err(error) = platform::open_path(&state.paths.root) {
                let text = Text::new(current_language(&state.config, &state.logger));
                state.logger.error(format!("打开保存目录失败: {error}"));
                platform::notify(
                    text.output_dir_failed_title(),
                    &format!("{error}"),
                    state.logger.clone(),
                );
            }
        }
        AppCommand::Quit => {
            state.shutdown.store(true, Ordering::SeqCst);
            save_current_config(&state.paths, &state.config, &state.logger);
            *saved_on_quit = true;
            event_loop.exit();
        }
    }
}

fn open_history_window() -> AppResult<()> {
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command.arg("--history");
    platform::hide_console(&mut command);
    command.spawn()?;
    Ok(())
}

fn open_about_window() -> AppResult<()> {
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command.arg("--about");
    platform::hide_console(&mut command);
    command.spawn()?;
    Ok(())
}

fn normalize_startup_capture_mode(paths: &AppPaths, config: &mut Config, logger: &Logger) {
    let screen_infos = match available_screen_infos() {
        Ok(screen_infos) => screen_infos,
        Err(error) => {
            logger.warn(format!("启动时刷新截屏范围失败，保留当前配置: {error}"));
            return;
        }
    };
    if reconcile_capture_mode_with_screens(config, &screen_infos, logger) {
        if let Err(error) = save_config(paths, config) {
            logger.error(format!("保存启动截屏范围回退配置失败: {error}"));
        }
    }
}

fn reconcile_capture_mode_with_screens(
    config: &mut Config,
    screen_infos: &[CaptureScreenInfo],
    logger: &Logger,
) -> bool {
    let configured_mode = config.capture_mode;
    let effective_mode = validate_capture_mode_for_screens(configured_mode, screen_infos);
    if effective_mode == configured_mode {
        return false;
    }

    logger.warn(format!(
        "启动时所选截屏范围 {} 不可用，已回退到 {}",
        configured_mode.config_value(),
        effective_mode.config_value()
    ));
    config.capture_mode = effective_mode;
    true
}

fn set_interval(
    seconds: u64,
    controls: &TrayControls,
    paths: &AppPaths,
    config: &Arc<Mutex<Config>>,
    logger: &Logger,
) {
    match config.lock() {
        Ok(mut config_guard) => {
            config_guard.interval = seconds;
            let language = config_guard.language;
            tray::update_interval_menu(controls, seconds, language);
            drop(config_guard);
            save_current_config(paths, config, logger);
        }
        Err(error) => logger.error(format!("更新间隔失败: {error}")),
    }
}

fn set_language(
    language: Language,
    controls: &TrayControls,
    is_running: bool,
    paths: &AppPaths,
    config: &Arc<Mutex<Config>>,
    logger: &Logger,
) {
    match config.lock() {
        Ok(mut config_guard) => {
            config_guard.language = language;
            let interval = config_guard.interval;
            let capture_mode = config_guard.capture_mode;
            tray::update_menu_labels(controls, is_running, interval, capture_mode, language);
            drop(config_guard);
            save_current_config(paths, config, logger);
        }
        Err(error) => logger.error(format!("更新语言失败: {error}")),
    }
}

fn capture_once_in_thread(
    paths: AppPaths,
    config: Arc<Mutex<Config>>,
    app_events: mpsc::Sender<AppEvent>,
    workers: &ThreadRegistry,
    logger: Logger,
) {
    let language = current_language(&config, &logger);
    workers.spawn(
        "手动截屏任务",
        language,
        logger.clone(),
        move || match cloned_config(&config) {
            Ok(config) => match capture_once(&paths, &config, &logger) {
                Ok(report) => {
                    let _ = app_events.send(AppEvent::CaptureSucceeded {
                        report,
                        notify: true,
                    });
                }
                Err(error) => {
                    logger.error(format!("手动截屏失败: {error}"));
                    let _ = app_events.send(AppEvent::CaptureFailed {
                        message: error.to_string(),
                        notify: true,
                    });
                }
            },
            Err(error) => {
                let text = Text::new(Language::default());
                logger.error(format!("读取配置失败: {error}"));
                let _ = app_events.send(AppEvent::CaptureFailed {
                    message: text.config_read_failed(&error),
                    notify: true,
                });
            }
        },
    );
}

fn generate_today_video_in_thread(
    paths: AppPaths,
    config: Arc<Mutex<Config>>,
    generating_video: Arc<AtomicBool>,
    workers: &ThreadRegistry,
    logger: Logger,
) {
    if generating_video
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        let text = Text::new(current_language(&config, &logger));
        logger.warn("已有视频生成任务正在运行，跳过本次请求");
        platform::notify(
            text.video_generating_title(),
            text.video_generating_body(),
            logger,
        );
        return;
    }

    let language = current_language(&config, &logger);
    workers.spawn("生成视频任务", language, logger.clone(), move || {
        let _generating_video_guard = AtomicFlagGuard::new(generating_video);
        match cloned_config(&config) {
            Ok(config) => {
                let text = Text::new(config.language);
                match generate_today_video(
                    &paths,
                    config.fps,
                    config.image_format,
                    config.video_codec,
                    &logger,
                ) {
                    Ok(output) => {
                        platform::notify(
                            text.video_success_title(),
                            &text.saved_to(&output),
                            logger.clone(),
                        );
                    }
                    Err(error) => {
                        logger.error(format!("生成今日视频失败: {error}"));
                        platform::notify(
                            text.video_failed_title(),
                            &format!("{error}"),
                            logger.clone(),
                        );
                    }
                }
            }
            Err(error) => {
                let text = Text::new(Language::default());
                logger.error(format!("读取配置失败: {error}"));
                platform::notify(
                    text.video_failed_title(),
                    &text.config_read_failed(&error),
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
    app_events: mpsc::Sender<AppEvent>,
    logger: Logger,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let panic_logger = logger.clone();
        let panic_config = Arc::clone(&config);
        if let Err(panic_payload) = panic::catch_unwind(AssertUnwindSafe(move || {
            run_capture_loop(paths, running, shutdown, config, app_events, logger);
        })) {
            report_thread_panic(
                "定时截屏线程",
                panic_payload.as_ref(),
                &panic_logger,
                current_language(&panic_config, &panic_logger),
            );
        }
    })
}

fn run_capture_loop(
    paths: AppPaths,
    running: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    config: Arc<Mutex<Config>>,
    app_events: mpsc::Sender<AppEvent>,
    logger: Logger,
) {
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
                        match capture_once(&paths, &config, &logger) {
                            Ok(report) => {
                                let _ = app_events.send(AppEvent::CaptureSucceeded {
                                    report,
                                    notify: false,
                                });
                            }
                            Err(error) => {
                                logger.error(format!("定时截屏失败: {error}"));
                                let _ = app_events.send(AppEvent::CaptureFailed {
                                    message: error.to_string(),
                                    notify: false,
                                });
                            }
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
}

fn report_thread_panic(
    task_name: &str,
    panic_payload: &(dyn Any + Send),
    logger: &Logger,
    language: Language,
) {
    let message = format!(
        "{task_name} panic: {}",
        panic_payload_message(panic_payload)
    );
    logger.error(&message);
    platform::notify(
        Text::new(language).background_task_failed_title(),
        &message,
        logger.clone(),
    );
}

fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "未知 panic payload".to_string()
}

fn handle_app_event(
    event: AppEvent,
    screenshot_count: &mut u64,
    last_capture: &mut Option<LastCapture>,
    config: &Arc<Mutex<Config>>,
    logger: &Logger,
) {
    match event {
        AppEvent::CaptureSucceeded { report, notify } => {
            *screenshot_count += report.saved_count() as u64;
            *last_capture = Some(LastCapture::Saved(report.clone()));
            if notify {
                let text = Text::new(current_language(config, logger));
                let message = capture_report_message(&text, &report);
                platform::notify(text.capture_success_title(), &message, logger.clone());
            }
        }
        AppEvent::CaptureFailed { message, notify } => {
            *last_capture = Some(LastCapture::Failed(message.clone()));
            if notify {
                let text = Text::new(current_language(config, logger));
                platform::notify(text.capture_failed_title(), &message, logger.clone());
            }
        }
    }
}

fn update_tray_tooltip(
    tray_state: &TrayState,
    app_state: &AppState,
    screenshot_count: u64,
    last_capture: Option<&LastCapture>,
) {
    let config = cloned_config(&app_state.config).unwrap_or_else(|error| {
        app_state.logger.error(format!("读取配置失败: {error}"));
        Config::default()
    });
    let text = Text::new(config.language);
    let last_capture = last_capture.map(|capture| match capture {
        LastCapture::Saved(report) => capture_report_message(&text, report),
        LastCapture::Failed(message) => text.failed_capture(message),
    });
    let tooltip = tray::status_tooltip(
        config.language,
        app_state.running.load(Ordering::SeqCst),
        config.interval,
        screenshot_count,
        last_capture.as_deref(),
    );
    if let Err(error) = tray::update_tooltip(tray_state, &tooltip) {
        app_state
            .logger
            .error(format!("更新托盘状态提示失败: {error}"));
    }
}

fn set_capture_mode(
    capture_mode: CaptureMode,
    controls: &mut TrayControls,
    paths: &AppPaths,
    config: &Arc<Mutex<Config>>,
    logger: &Logger,
) {
    match config.lock() {
        Ok(mut config_guard) => {
            let language = config_guard.language;
            let effective_mode =
                refresh_capture_sources_for_mode(controls, capture_mode, language, logger);
            if effective_mode != capture_mode {
                logger.warn(format!(
                    "所选截屏范围不可用，已回退到 {}",
                    effective_mode.config_value()
                ));
            }
            config_guard.capture_mode = effective_mode;
            let language = config_guard.language;
            tray::update_capture_source_menu(controls, effective_mode, language);
            drop(config_guard);
            save_current_config(paths, config, logger);
        }
        Err(error) => logger.error(format!("更新截屏范围失败: {error}")),
    }
}

fn refresh_capture_sources(
    controls: &mut TrayControls,
    paths: &AppPaths,
    config: &Arc<Mutex<Config>>,
    logger: &Logger,
) {
    match config.lock() {
        Ok(mut config_guard) => {
            let capture_mode = config_guard.capture_mode;
            let language = config_guard.language;
            let effective_mode =
                refresh_capture_sources_for_mode(controls, capture_mode, language, logger);
            if effective_mode != capture_mode {
                config_guard.capture_mode = effective_mode;
                drop(config_guard);
                save_current_config(paths, config, logger);
            }
        }
        Err(error) => logger.error(format!("刷新截屏范围失败: {error}")),
    }
}

fn refresh_capture_sources_for_mode(
    controls: &mut TrayControls,
    capture_mode: CaptureMode,
    language: Language,
    logger: &Logger,
) -> CaptureMode {
    match tray::refresh_capture_source_menu(controls, capture_mode, language) {
        Ok(effective_mode) => effective_mode,
        Err(error) => {
            logger.error(format!("刷新截屏范围失败: {error}"));
            tray::update_capture_source_menu(controls, capture_mode, language);
            capture_mode
        }
    }
}

fn capture_report_message(text: &Text, report: &CaptureReport) -> String {
    if report.skipped_duplicate {
        return text.skipped_duplicate_capture().to_string();
    }
    let mut message = if report.saved_count() == 1 {
        report
            .saved_paths
            .first()
            .map(|path| text.saved_capture(path))
            .unwrap_or_else(|| text.skipped_duplicate_capture().to_string())
    } else {
        let dir = report.output_dir().unwrap_or_else(|| Path::new("."));
        text.saved_screenshots(report.saved_count(), dir)
    };
    if report.failed_screen_count > 0 {
        message.push_str("; ");
        message.push_str(
            &text.partial_capture_failed(report.failed_screen_count, report.target_screen_count),
        );
    }
    message
}

fn current_language(config: &Arc<Mutex<Config>>, logger: &Logger) -> Language {
    current_config_or_default(config, logger).language
}

fn current_config_or_default(config: &Arc<Mutex<Config>>, logger: &Logger) -> Config {
    cloned_config(config).unwrap_or_else(|error| {
        logger.error(format!("读取配置失败: {error}"));
        Config::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-app-test-{}-{}",
            std::process::id(),
            sequence
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn test_logger(root: &Path) -> Logger {
        let paths = AppPaths {
            root: root.to_path_buf(),
            config: root.join("config.json"),
            screenshots: root.join("screenshots"),
            videos: root.join("videos"),
        };
        Logger::new(&paths).expect("create logger")
    }

    #[test]
    fn panic_payload_message_reads_str_payload() {
        let payload: &(dyn Any + Send) = &"boom";

        assert_eq!(panic_payload_message(payload), "boom");
    }

    #[test]
    fn panic_payload_message_reads_string_payload() {
        let payload: &(dyn Any + Send) = &"boom".to_string();

        assert_eq!(panic_payload_message(payload), "boom");
    }

    #[test]
    fn startup_capture_mode_falls_back_when_saved_screen_is_missing() {
        let root = test_dir();
        let logger = test_logger(&root);
        let mut config = Config {
            capture_mode: CaptureMode::Screen(2),
            ..Config::default()
        };
        let screen_infos = vec![CaptureScreenInfo {
            index: 1,
            id: 10,
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
            scale_factor: 1.0,
            is_primary: true,
        }];

        let changed = reconcile_capture_mode_with_screens(&mut config, &screen_infos, &logger);

        assert!(changed);
        assert_eq!(config.capture_mode, CaptureMode::Auto);
    }

    #[test]
    fn startup_capture_mode_keeps_available_saved_screen() {
        let root = test_dir();
        let logger = test_logger(&root);
        let mut config = Config {
            capture_mode: CaptureMode::Screen(2),
            ..Config::default()
        };
        let screen_infos = vec![CaptureScreenInfo {
            index: 2,
            id: 20,
            x: 1280,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            is_primary: false,
        }];

        let changed = reconcile_capture_mode_with_screens(&mut config, &screen_infos, &logger);

        assert!(!changed);
        assert_eq!(config.capture_mode, CaptureMode::Screen(2));
    }
}
