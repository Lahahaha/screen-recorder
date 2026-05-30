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
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tray_icon::menu::MenuEvent;
use winit::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoopBuilder},
};

pub(crate) type AppResult<T> = Result<T, Box<dyn Error>>;

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

    #[allow(deprecated)]
    let event_loop = EventLoopBuilder::<()>::with_user_event().build()?;
    let (menu_tx, menu_rx) = mpsc::channel::<MenuEvent>();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = menu_tx.send(event);
    }));

    let mut tray_state: Option<TrayState> = None;
    let mut saved_on_quit = false;

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
                        if let Some(command) = tray::command_for_event(&event, &state.controls) {
                            handle_app_command(
                                command,
                                &state.controls,
                                &app_state,
                                event_loop,
                                &mut saved_on_quit,
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
            &state.workers,
            state.logger.clone(),
        ),
        AppCommand::ToggleRunning => {
            let next = !state.running.load(Ordering::SeqCst);
            state.running.store(next, Ordering::SeqCst);
            tray::update_running_menu(controls, next);
        }
        AppCommand::SetInterval(seconds) => {
            set_interval(seconds, controls, &state.config, &state.logger);
        }
        AppCommand::GenerateTodayVideo => generate_today_video_in_thread(
            state.paths.clone(),
            Arc::clone(&state.config),
            Arc::clone(&state.generating_video),
            &state.workers,
            state.logger.clone(),
        ),
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
    config: &Arc<Mutex<Config>>,
    logger: &Logger,
) {
    match config.lock() {
        Ok(mut config) => {
            config.interval = seconds;
            tray::update_interval_menu(controls, seconds);
        }
        Err(error) => logger.error(format!("更新间隔失败: {error}")),
    }
}

fn capture_once_in_thread(
    paths: AppPaths,
    config: Arc<Mutex<Config>>,
    workers: &ThreadRegistry,
    logger: Logger,
) {
    workers.spawn(move || match cloned_config(&config) {
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
