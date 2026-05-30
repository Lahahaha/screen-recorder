use crate::{
    app::AppResult,
    config::{Config, Language, SUPPORTED_INTERVALS},
    i18n::{Text, APP_NAME},
};
use std::{
    io,
    sync::{Arc, Mutex},
};
use tray_icon::{
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    Icon, TrayIcon, TrayIconBuilder,
};

pub(crate) struct TrayControls {
    menu: Menu,
    capture_now: MenuItem,
    start_pause: MenuItem,
    interval_menu: Submenu,
    interval_items: Vec<(u64, CheckMenuItem)>,
    generate_video: MenuItem,
    open_output_dir: MenuItem,
    language_menu: Submenu,
    language_items: Vec<(Language, CheckMenuItem)>,
    quit: MenuItem,
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
    SetLanguage(Language),
    GenerateTodayVideo,
    OpenOutputDir,
    Quit,
}

pub(crate) fn create_tray_state(
    config: &Arc<Mutex<Config>>,
    is_running: bool,
) -> AppResult<TrayState> {
    let config = config
        .lock()
        .map_err(|error| io::Error::other(error.to_string()))?
        .clone();
    let controls = build_menu(config.interval, config.language)?;
    update_menu_labels(&controls, is_running, config.interval, config.language);
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

fn build_menu(current_interval: u64, language: Language) -> AppResult<TrayControls> {
    let menu = Menu::new();
    let text = Text::new(language);
    let capture_now = MenuItem::new(text.capture_now(), true, None);
    let start_pause = MenuItem::new(text.start(), true, None);
    let interval_menu = Submenu::new(text.interval_menu(current_interval), true);
    let generate_video = MenuItem::new(text.generate_today_video(), true, None);
    let open_output_dir = MenuItem::new(text.open_output_dir(), true, None);
    let language_menu = Submenu::new(text.language_menu(), true);
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

    let interval_item_refs = interval_items
        .iter()
        .map(|(_, item)| item as &dyn IsMenuItem)
        .collect::<Vec<_>>();
    interval_menu.append_items(&interval_item_refs)?;

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
        &generate_video,
        &open_output_dir,
        &language_menu,
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
        open_output_dir,
        language_menu,
        language_items,
        quit,
    })
}

pub(crate) fn command_for_event(event: &MenuEvent, controls: &TrayControls) -> Option<AppCommand> {
    if event.id == controls.capture_now.id() {
        return Some(AppCommand::CaptureNow);
    }

    if event.id == controls.start_pause.id() {
        return Some(AppCommand::ToggleRunning);
    }

    for (seconds, item) in &controls.interval_items {
        if event.id == item.id() {
            return Some(AppCommand::SetInterval(*seconds));
        }
    }

    for (language, item) in &controls.language_items {
        if event.id == item.id() {
            return Some(AppCommand::SetLanguage(*language));
        }
    }

    if event.id == controls.generate_video.id() {
        return Some(AppCommand::GenerateTodayVideo);
    }

    if event.id == controls.open_output_dir.id() {
        return Some(AppCommand::OpenOutputDir);
    }

    if event.id == controls.quit.id() {
        return Some(AppCommand::Quit);
    }

    None
}

pub(crate) fn update_menu_labels(
    controls: &TrayControls,
    is_running: bool,
    interval: u64,
    language: Language,
) {
    let text = Text::new(language);
    controls.capture_now.set_text(text.capture_now());
    update_running_menu(controls, is_running, language);
    update_interval_menu(controls, interval, language);
    controls
        .generate_video
        .set_text(text.generate_today_video());
    controls.open_output_dir.set_text(text.open_output_dir());
    controls.language_menu.set_text(text.language_menu());
    for (value, item) in &controls.language_items {
        item.set_checked(*value == language);
    }
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
    controls
        .interval_menu
        .set_text(Text::new(language).interval_menu(seconds));
    for (value, item) in &controls.interval_items {
        item.set_checked(*value == seconds);
    }
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

fn put_pixel(rgba: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 4]) {
    let index = ((y * width + x) * 4) as usize;
    rgba[index..index + 4].copy_from_slice(&color);
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let controls = build_menu(30, Language::ZhCn).expect("build menu");
        assert_eq!(controls.language_items.len(), Language::ALL.len());

        let english_item = controls
            .language_items
            .iter()
            .find(|(language, _)| *language == Language::En)
            .map(|(_, item)| item)
            .expect("english item");
        let event = MenuEvent {
            id: english_item.id().clone(),
        };

        assert_eq!(
            command_for_event(&event, &controls),
            Some(AppCommand::SetLanguage(Language::En))
        );
    }
}
