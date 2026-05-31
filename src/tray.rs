use crate::{
    app::AppResult,
    capture::{available_screen_infos, validate_capture_mode_for_screens, CaptureScreenInfo},
    config::{CaptureMode, Config, Language, SUPPORTED_INTERVALS},
    i18n::{Text, APP_NAME},
    logging::Logger,
};
use std::{
    io,
    sync::{Arc, Mutex},
};
use tray_icon::{
    menu::{
        CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    },
    Icon as TrayIconImage, TrayIcon, TrayIconBuilder,
};

pub(crate) struct TrayControls {
    menu: Menu,
    capture_now: MenuItem,
    start_pause: MenuItem,
    interval_menu: Submenu,
    interval_items: Vec<(u64, CheckMenuItem)>,
    capture_source_menu: Submenu,
    capture_source_auto: CheckMenuItem,
    capture_source_refresh: MenuItem,
    capture_source_items: Vec<(u32, CheckMenuItem)>,
    capture_source_screens: Vec<CaptureScreenInfo>,
    generate_video: MenuItem,
    history_videos: MenuItem,
    open_output_dir: MenuItem,
    language_menu: Submenu,
    language_items: Vec<(Language, CheckMenuItem)>,
    about: MenuItem,
    quit: MenuItem,
    command_ids: TrayCommandIds,
}

pub(crate) struct TrayState {
    tray_icon: TrayIcon,
    pub(crate) controls: TrayControls,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppCommand {
    CaptureNow,
    ToggleRunning,
    SetInterval(u64),
    SetCaptureMode(CaptureMode),
    RefreshCaptureSources,
    SetLanguage(Language),
    GenerateTodayVideo,
    OpenHistoryVideos,
    OpenOutputDir,
    OpenAbout,
    Quit,
}

#[derive(Clone, Debug)]
struct TrayCommandIds {
    capture_now: MenuId,
    start_pause: MenuId,
    interval_items: Vec<(u64, MenuId)>,
    capture_source_auto: MenuId,
    capture_source_refresh: MenuId,
    capture_source_items: Vec<(u32, MenuId)>,
    generate_video: MenuId,
    history_videos: MenuId,
    open_output_dir: MenuId,
    language_items: Vec<(Language, MenuId)>,
    about: MenuId,
    quit: MenuId,
}

pub(crate) fn create_tray_state(
    config: &Arc<Mutex<Config>>,
    is_running: bool,
    logger: &Logger,
) -> AppResult<TrayState> {
    let config = config
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?
        .clone();
    let screen_infos = match available_screen_infos() {
        Ok(screen_infos) => screen_infos,
        Err(error) => {
            logger.warn(format!(
                "启动时枚举屏幕列表失败，托盘屏幕列表暂为空，可点击刷新重试: {error}"
            ));
            Vec::new()
        }
    };
    let controls = build_menu(
        config.interval,
        config.language,
        config.capture_mode,
        screen_infos,
    )?;
    update_menu_labels(
        &controls,
        is_running,
        config.interval,
        config.capture_mode,
        config.language,
    );
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(controls.menu.clone()))
        .with_tooltip(APP_NAME)
        .with_icon(create_tray_icon()?)
        .build()?;
    tray_icon.set_icon_as_template(true);

    Ok(TrayState {
        tray_icon,
        controls,
    })
}

fn build_menu(
    current_interval: u64,
    language: Language,
    capture_mode: CaptureMode,
    screen_infos: Vec<CaptureScreenInfo>,
) -> AppResult<TrayControls> {
    let menu = Menu::new();
    let text = Text::new(language);
    let capture_now = MenuItem::new(text.capture_now(), true, None);
    let start_pause = MenuItem::new(text.start(), true, None);
    let interval_menu = Submenu::new(text.interval_menu(current_interval), true);
    let capture_source_menu = Submenu::new(text.capture_source_menu(), true);
    let capture_source_auto = CheckMenuItem::new(
        text.capture_source_auto(),
        true,
        capture_mode == CaptureMode::Auto,
        None,
    );
    let capture_source_refresh = MenuItem::new(text.refresh_capture_sources(), true, None);
    let generate_video = MenuItem::new(text.generate_today_video(), true, None);
    let history_videos = MenuItem::new(text.history_videos(), true, None);
    let open_output_dir = MenuItem::new(text.open_output_dir(), true, None);
    let language_menu = Submenu::new(text.language_menu(), true);
    let about = MenuItem::new(text.about(), true, None);
    let quit = MenuItem::new(text.quit(), true, None);

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

    let language_items = Language::ALL
        .iter()
        .map(|value| {
            (
                *value,
                CheckMenuItem::new(value.menu_label(), true, *value == language, None),
            )
        })
        .collect::<Vec<_>>();

    let capture_source_items = screen_infos
        .iter()
        .map(|screen| {
            let (width, height) = screen_label_size(screen);
            (
                screen.index,
                CheckMenuItem::new(
                    text.screen_label(screen.index, screen.is_primary, width, height),
                    true,
                    capture_mode == CaptureMode::Screen(screen.index),
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

    let mut capture_source_item_refs = vec![&capture_source_auto as &dyn IsMenuItem];
    for (_, item) in &capture_source_items {
        capture_source_item_refs.push(item as &dyn IsMenuItem);
    }
    capture_source_item_refs.push(&capture_source_refresh as &dyn IsMenuItem);
    capture_source_menu.append_items(&capture_source_item_refs)?;

    let language_item_refs = language_items
        .iter()
        .map(|(_, item)| item as &dyn IsMenuItem)
        .collect::<Vec<_>>();
    language_menu.append_items(&language_item_refs)?;

    menu.append_items(&[
        &capture_now as &dyn IsMenuItem,
        &PredefinedMenuItem::separator(),
        &start_pause,
        &PredefinedMenuItem::separator(),
        &interval_menu,
        &PredefinedMenuItem::separator(),
        &capture_source_menu,
        &PredefinedMenuItem::separator(),
        &generate_video,
        &history_videos,
        &open_output_dir,
        &language_menu,
        &PredefinedMenuItem::separator(),
        &about,
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;

    let command_ids = TrayCommandIds {
        capture_now: capture_now.id().clone(),
        start_pause: start_pause.id().clone(),
        interval_items: interval_items
            .iter()
            .map(|(seconds, item)| (*seconds, item.id().clone()))
            .collect(),
        capture_source_auto: capture_source_auto.id().clone(),
        capture_source_refresh: capture_source_refresh.id().clone(),
        capture_source_items: capture_source_items
            .iter()
            .map(|(screen_index, item)| (*screen_index, item.id().clone()))
            .collect(),
        generate_video: generate_video.id().clone(),
        history_videos: history_videos.id().clone(),
        open_output_dir: open_output_dir.id().clone(),
        language_items: language_items
            .iter()
            .map(|(language, item)| (*language, item.id().clone()))
            .collect(),
        about: about.id().clone(),
        quit: quit.id().clone(),
    };

    Ok(TrayControls {
        menu,
        capture_now,
        start_pause,
        interval_menu,
        interval_items,
        capture_source_menu,
        capture_source_auto,
        capture_source_refresh,
        capture_source_items,
        capture_source_screens: screen_infos,
        generate_video,
        history_videos,
        open_output_dir,
        language_menu,
        language_items,
        about,
        quit,
        command_ids,
    })
}

pub(crate) fn command_for_event(event: &MenuEvent, controls: &TrayControls) -> Option<AppCommand> {
    command_for_menu_id(&event.id, &controls.command_ids)
}

fn command_for_menu_id(id: &MenuId, command_ids: &TrayCommandIds) -> Option<AppCommand> {
    if id == &command_ids.capture_now {
        return Some(AppCommand::CaptureNow);
    }

    if id == &command_ids.start_pause {
        return Some(AppCommand::ToggleRunning);
    }

    for (seconds, item_id) in &command_ids.interval_items {
        if id == item_id {
            return Some(AppCommand::SetInterval(*seconds));
        }
    }

    if id == &command_ids.capture_source_auto {
        return Some(AppCommand::SetCaptureMode(CaptureMode::Auto));
    }

    for (screen_index, item_id) in &command_ids.capture_source_items {
        if id == item_id {
            return Some(AppCommand::SetCaptureMode(CaptureMode::Screen(
                *screen_index,
            )));
        }
    }

    if id == &command_ids.capture_source_refresh {
        return Some(AppCommand::RefreshCaptureSources);
    }

    for (language, item_id) in &command_ids.language_items {
        if id == item_id {
            return Some(AppCommand::SetLanguage(*language));
        }
    }

    if id == &command_ids.generate_video {
        return Some(AppCommand::GenerateTodayVideo);
    }

    if id == &command_ids.history_videos {
        return Some(AppCommand::OpenHistoryVideos);
    }

    if id == &command_ids.open_output_dir {
        return Some(AppCommand::OpenOutputDir);
    }

    if id == &command_ids.about {
        return Some(AppCommand::OpenAbout);
    }

    if id == &command_ids.quit {
        return Some(AppCommand::Quit);
    }

    None
}

pub(crate) fn update_menu_labels(
    controls: &TrayControls,
    is_running: bool,
    interval: u64,
    capture_mode: CaptureMode,
    language: Language,
) {
    let text = Text::new(language);
    controls.capture_now.set_text(text.capture_now());
    update_running_menu(controls, is_running, language);
    update_interval_menu(controls, interval, language);
    update_capture_source_menu(controls, capture_mode, language);
    controls
        .generate_video
        .set_text(text.generate_today_video());
    controls.history_videos.set_text(text.history_videos());
    controls.open_output_dir.set_text(text.open_output_dir());
    controls.language_menu.set_text(text.language_menu());
    for (value, item) in &controls.language_items {
        item.set_checked(*value == language);
    }
    controls.about.set_text(text.about());
    controls.quit.set_text(text.quit());
}

pub(crate) fn update_running_menu(controls: &TrayControls, is_running: bool, language: Language) {
    let text = Text::new(language);
    let label = if is_running {
        text.pause()
    } else {
        text.start()
    };
    controls.start_pause.set_text(label);
}

pub(crate) fn update_interval_menu(controls: &TrayControls, seconds: u64, language: Language) {
    let label = Text::new(language).interval_menu(seconds);
    controls.interval_menu.set_text(label);
    for (value, item) in &controls.interval_items {
        item.set_checked(*value == seconds);
    }
}

pub(crate) fn update_capture_source_menu(
    controls: &TrayControls,
    capture_mode: CaptureMode,
    language: Language,
) {
    let text = Text::new(language);
    controls
        .capture_source_menu
        .set_text(text.capture_source_menu());
    controls
        .capture_source_auto
        .set_text(text.capture_source_auto());
    controls
        .capture_source_auto
        .set_checked(capture_mode == CaptureMode::Auto);
    controls
        .capture_source_refresh
        .set_text(text.refresh_capture_sources());
    for ((screen_index, item), screen) in controls
        .capture_source_items
        .iter()
        .zip(controls.capture_source_screens.iter())
    {
        let (width, height) = screen_label_size(screen);
        item.set_text(text.screen_label(screen.index, screen.is_primary, width, height));
        item.set_checked(capture_mode == CaptureMode::Screen(*screen_index));
    }
}

pub(crate) fn refresh_capture_source_menu(
    controls: &mut TrayControls,
    capture_mode: CaptureMode,
    language: Language,
) -> AppResult<CaptureMode> {
    let screens = available_screen_infos()?;
    replace_capture_source_screens(controls, screens, capture_mode, language)
}

fn replace_capture_source_screens(
    controls: &mut TrayControls,
    screen_infos: Vec<CaptureScreenInfo>,
    capture_mode: CaptureMode,
    language: Language,
) -> AppResult<CaptureMode> {
    controls
        .capture_source_menu
        .remove(&controls.capture_source_refresh)?;
    for (_, item) in &controls.capture_source_items {
        controls.capture_source_menu.remove(item)?;
    }

    let text = Text::new(language);
    let effective_mode = validate_capture_mode_for_screens(capture_mode, &screen_infos);
    let capture_source_items = screen_infos
        .iter()
        .map(|screen| {
            let (width, height) = screen_label_size(screen);
            (
                screen.index,
                CheckMenuItem::new(
                    text.screen_label(screen.index, screen.is_primary, width, height),
                    true,
                    effective_mode == CaptureMode::Screen(screen.index),
                    None,
                ),
            )
        })
        .collect::<Vec<_>>();

    let mut item_refs = capture_source_items
        .iter()
        .map(|(_, item)| item as &dyn IsMenuItem)
        .collect::<Vec<_>>();
    item_refs.push(&controls.capture_source_refresh as &dyn IsMenuItem);
    controls.capture_source_menu.append_items(&item_refs)?;
    controls.capture_source_items = capture_source_items;
    controls.command_ids.capture_source_items = controls
        .capture_source_items
        .iter()
        .map(|(screen_index, item)| (*screen_index, item.id().clone()))
        .collect();
    controls.capture_source_screens = screen_infos;
    update_capture_source_menu(controls, effective_mode, language);
    Ok(effective_mode)
}

fn screen_label_size(screen: &CaptureScreenInfo) -> (u32, u32) {
    let scale_factor = screen.scale_factor.max(f64::EPSILON);
    (
        (f64::from(screen.width) * scale_factor).round().max(1.0) as u32,
        (f64::from(screen.height) * scale_factor).round().max(1.0) as u32,
    )
}

pub(crate) fn update_tooltip(state: &TrayState, tooltip: &str) -> AppResult<()> {
    state.tray_icon.set_tooltip(Some(tooltip))?;
    Ok(())
}

pub(crate) fn status_tooltip(
    language: Language,
    is_running: bool,
    interval: u64,
    screenshot_count: u64,
    last_capture: Option<&str>,
) -> String {
    Text::new(language).status_tooltip(is_running, interval, screenshot_count, last_capture)
}

fn create_tray_icon() -> AppResult<TrayIconImage> {
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

    Ok(TrayIconImage::from_rgba(rgba, SIZE, SIZE)?)
}

fn put_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = ((y * width + x) * 4) as usize;
    rgba[index..index + 4].copy_from_slice(&color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_command_ids() -> TrayCommandIds {
        TrayCommandIds {
            capture_now: MenuId::new("capture-now"),
            start_pause: MenuId::new("start-pause"),
            interval_items: vec![(30, MenuId::new("interval-30"))],
            capture_source_auto: MenuId::new("capture-source-auto"),
            capture_source_refresh: MenuId::new("capture-source-refresh"),
            capture_source_items: vec![(2, MenuId::new("capture-source-screen-2"))],
            generate_video: MenuId::new("generate-video"),
            history_videos: MenuId::new("history-videos"),
            open_output_dir: MenuId::new("open-output-dir"),
            language_items: vec![
                (Language::ZhCn, MenuId::new("language-zh-cn")),
                (Language::En, MenuId::new("language-en")),
            ],
            about: MenuId::new("about"),
            quit: MenuId::new("quit"),
        }
    }

    #[test]
    fn status_tooltip_includes_runtime_state() {
        let tooltip = status_tooltip(Language::ZhCn, true, 30, 2, Some("已保存 screen.png"));

        assert!(tooltip.contains("状态: 运行中"));
        assert!(tooltip.contains("间隔: 30s"));
        assert!(tooltip.contains("本次截图: 2"));
        assert!(tooltip.contains("最近: 已保存 screen.png"));
    }

    #[test]
    fn status_tooltip_supports_english() {
        let tooltip = status_tooltip(Language::En, false, 60, 3, Some("Saved screen.png"));

        assert!(tooltip.contains("Status: Paused"));
        assert!(tooltip.contains("Interval: 60s"));
        assert!(tooltip.contains("Screenshots this session: 3"));
        assert!(tooltip.contains("Latest: Saved screen.png"));
    }

    #[test]
    fn command_for_event_detects_language_items() {
        let command_ids = test_command_ids();

        assert_eq!(
            command_for_menu_id(&MenuId::new("language-en"), &command_ids),
            Some(AppCommand::SetLanguage(Language::En))
        );
    }

    #[test]
    fn command_for_event_detects_history_videos() {
        let command_ids = test_command_ids();

        assert_eq!(
            command_for_menu_id(&MenuId::new("history-videos"), &command_ids),
            Some(AppCommand::OpenHistoryVideos)
        );
    }

    #[test]
    fn command_for_event_detects_about() {
        let command_ids = test_command_ids();

        assert_eq!(
            command_for_menu_id(&MenuId::new("about"), &command_ids),
            Some(AppCommand::OpenAbout)
        );
    }

    #[test]
    fn command_for_event_detects_capture_source_items() {
        let command_ids = test_command_ids();
        assert_eq!(
            command_for_menu_id(&MenuId::new("capture-source-auto"), &command_ids),
            Some(AppCommand::SetCaptureMode(CaptureMode::Auto))
        );

        assert_eq!(
            command_for_menu_id(&MenuId::new("capture-source-screen-2"), &command_ids),
            Some(AppCommand::SetCaptureMode(CaptureMode::Screen(2)))
        );
    }

    #[test]
    fn command_for_event_detects_capture_source_refresh() {
        let command_ids = test_command_ids();

        assert_eq!(
            command_for_menu_id(&MenuId::new("capture-source-refresh"), &command_ids),
            Some(AppCommand::RefreshCaptureSources)
        );
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "muda 0.19 requires native menus to be created on the main thread on macOS"
    )]
    fn build_menu_with_empty_screens_keeps_refresh_command() {
        let controls =
            build_menu(30, Language::ZhCn, CaptureMode::Auto, Vec::new()).expect("build menu");

        assert!(controls.capture_source_items.is_empty());
        assert_eq!(
            command_for_menu_id(
                &controls.command_ids.capture_source_refresh,
                &controls.command_ids
            ),
            Some(AppCommand::RefreshCaptureSources)
        );
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "muda 0.19 requires native menus to be created on the main thread on macOS"
    )]
    fn replace_capture_source_screens_rebuilds_items_and_falls_back_when_selected_missing() {
        let initial_screens = vec![CaptureScreenInfo {
            index: 2,
            id: 20,
            x: 100,
            y: 0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            is_primary: false,
        }];
        let mut controls = build_menu(30, Language::ZhCn, CaptureMode::Screen(2), initial_screens)
            .expect("build menu");
        let replacement_screens = vec![CaptureScreenInfo {
            index: 1,
            id: 10,
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
            scale_factor: 1.0,
            is_primary: true,
        }];

        let effective_mode = replace_capture_source_screens(
            &mut controls,
            replacement_screens,
            CaptureMode::Screen(2),
            Language::ZhCn,
        )
        .expect("replace screens");

        assert_eq!(effective_mode, CaptureMode::Auto);
        assert_eq!(controls.capture_source_items.len(), 1);
        assert_eq!(controls.capture_source_items[0].0, 1);
        assert!(controls.capture_source_auto.is_checked());
        assert!(!controls.capture_source_items[0].1.is_checked());
    }
}
