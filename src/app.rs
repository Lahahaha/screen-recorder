use crate::{
    capture::capture_once,
    config::{cloned_config, load_config, save_current_config, Config},
    logging::Logger,
    paths::AppPaths,
    platform,
    temp::AtomicFlagGuard,
    tray::{self, AppCommand, TrayControls, TrayState},
    video::generate_today_video,
};
use std::{
    error::Error,
    path::PathBuf,
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
    CaptureSucceeded { path: PathBuf, notify: bool },
    CaptureFailed { message: String, notify: bool },
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
    app_events: mpsc::Sender<AppEvent>,
    workers: ThreadRegistry,
    logger: Logger,
}

pub(crate) fn run() -> AppResult<()> {
    let paths = AppPaths::new()?;
    let logger = Logger::new(&paths)?;
    let initial_config = load_config(&paths, &logger)?;
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
    let mut last_capture: Option<String> = None;

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
                                    last_capture.as_deref(),
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
                if let Some(state) = tray_state.as_ref() {
                    for event in app_event_rx.try_iter() {
                        handle_app_event(
                            event,
                            &mut screenshot_count,
                            &mut last_capture,
                            &app_state.logger,
                        );
                        update_tray_tooltip(
                            state,
                            &app_state,
                            screenshot_count,
                            last_capture.as_deref(),
                        );
                    }

                    for event in menu_rx.try_iter() {
                        if let Some(command) = tray::command_for_event(&event, &state.controls) {
                            handle_app_command(
                                command,
                                &state.controls,
                                &app_state,
                                event_loop,
                                &mut saved_on_quit,
                            );
                            update_tray_tooltip(
                                state,
                                &app_state,
                                screenshot_count,
                                last_capture.as_deref(),
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
    controls: &TrayControls,
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
            tray::update_running_menu(controls, next);
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
        AppCommand::GenerateTodayVideo => generate_today_video_in_thread(
            state.paths.clone(),
            Arc::clone(&state.config),
            Arc::clone(&state.generating_video),
            &state.workers,
            state.logger.clone(),
        ),
        AppCommand::OpenOutputDir => {
            if let Err(error) = platform::open_path(&state.paths.root) {
                state.logger.error(format!("打开保存目录失败: {error}"));
                platform::notify(
                    "打开保存目录失败",
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
            tray::update_interval_menu(controls, seconds);
            drop(config_guard);
            save_current_config(paths, config, logger);
        }
        Err(error) => logger.error(format!("更新间隔失败: {error}")),
    }
}

fn capture_once_in_thread(
    paths: AppPaths,
    config: Arc<Mutex<Config>>,
    app_events: mpsc::Sender<AppEvent>,
    workers: &ThreadRegistry,
    logger: Logger,
) {
    workers.spawn(move || match cloned_config(&config) {
        Ok(config) => match capture_once(&paths, &config, &logger) {
            Ok(path) => {
                let _ = app_events.send(AppEvent::CaptureSucceeded { path, notify: true });
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
            logger.error(format!("读取配置失败: {error}"));
            let _ = app_events.send(AppEvent::CaptureFailed {
                message: format!("读取配置失败: {error}"),
                notify: true,
            });
        }
    });
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
        logger.warn("已有视频生成任务正在运行，跳过本次请求");
        platform::notify("视频生成中", "已有视频生成任务正在运行。", logger);
        return;
    }

    workers.spawn(move || {
        let _generating_video_guard = AtomicFlagGuard::new(generating_video);
        match cloned_config(&config) {
            Ok(config) => {
                match generate_today_video(
                    &paths,
                    config.fps,
                    config.image_format,
                    config.video_codec,
                    &logger,
                ) {
                    Ok(output) => {
                        platform::notify(
                            "视频生成成功",
                            &format!("已保存到: {}", output.display()),
                            logger.clone(),
                        );
                    }
                    Err(error) => {
                        logger.error(format!("生成今日视频失败: {error}"));
                        platform::notify("视频生成失败", &format!("{error}"), logger.clone());
                    }
                }
            }
            Err(error) => {
                logger.error(format!("读取配置失败: {error}"));
                platform::notify(
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
    app_events: mpsc::Sender<AppEvent>,
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
                            match capture_once(&paths, &config, &logger) {
                                Ok(path) => {
                                    let _ = app_events.send(AppEvent::CaptureSucceeded {
                                        path,
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
    })
}

fn handle_app_event(
    event: AppEvent,
    screenshot_count: &mut u64,
    last_capture: &mut Option<String>,
    logger: &Logger,
) {
    match event {
        AppEvent::CaptureSucceeded { path, notify } => {
            *screenshot_count += 1;
            *last_capture = Some(format!("已保存 {}", path.display()));
            if notify {
                platform::notify(
                    "截图成功",
                    &format!("已保存到: {}", path.display()),
                    logger.clone(),
                );
            }
        }
        AppEvent::CaptureFailed { message, notify } => {
            *last_capture = Some(format!("失败: {message}"));
            if notify {
                platform::notify("截图失败", &message, logger.clone());
            }
        }
    }
}

fn update_tray_tooltip(
    tray_state: &TrayState,
    app_state: &AppState,
    screenshot_count: u64,
    last_capture: Option<&str>,
) {
    let interval = cloned_config(&app_state.config)
        .map(|config| config.interval)
        .unwrap_or_else(|error| {
            app_state.logger.error(format!("读取配置失败: {error}"));
            Config::default().interval
        });
    let tooltip = tray::status_tooltip(
        app_state.running.load(Ordering::SeqCst),
        interval,
        screenshot_count,
        last_capture,
    );
    if let Err(error) = tray::update_tooltip(tray_state, &tooltip) {
        app_state
            .logger
            .error(format!("更新托盘状态提示失败: {error}"));
    }
}
