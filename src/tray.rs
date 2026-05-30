use crate::{
    app::AppResult,
    config::{Config, SUPPORTED_INTERVALS},
};
use std::{
    io,
    sync::{Arc, Mutex},
};
use tray_icon::{
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
    Icon, TrayIcon, TrayIconBuilder,
};

const APP_NAME: &str = "Screen Recorder";

pub(crate) struct TrayControls {
    menu: Menu,
    capture_now: MenuItem,
    start_pause: MenuItem,
    interval_menu: Submenu,
    interval_items: Vec<(u64, CheckMenuItem)>,
    generate_video: MenuItem,
    open_output_dir: MenuItem,
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
    GenerateTodayVideo,
    OpenOutputDir,
    Quit,
}

pub(crate) fn create_tray_state(
    config: &Arc<Mutex<Config>>,
    is_running: bool,
) -> AppResult<TrayState> {
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
        tray_icon,
        controls,
    })
}

fn build_menu(current_interval: u64) -> AppResult<TrayControls> {
    let menu = Menu::new();
    let capture_now = MenuItem::new("📷 截一张", true, None);
    let start_pause = MenuItem::new("▶ 开始", true, None);
    let interval_menu = Submenu::new(format!("⏱ 间隔 当前：{current_interval}s"), true);
    let generate_video = MenuItem::new("🎬 生成今日视频", true, None);
    let open_output_dir = MenuItem::new("📁 打开保存目录", true, None);
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
        &open_output_dir,
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

pub(crate) fn update_running_menu(controls: &TrayControls, is_running: bool) {
    let text = if is_running {
        "⏸ 暂停"
    } else {
        "▶ 开始"
    };
    controls.start_pause.set_text(text);
}

pub(crate) fn update_interval_menu(controls: &TrayControls, seconds: u64) {
    controls
        .interval_menu
        .set_text(format!("⏱ 间隔 当前：{seconds}s"));
    for (value, item) in &controls.interval_items {
        item.set_checked(*value == seconds);
    }
}

pub(crate) fn update_tooltip(state: &TrayState, tooltip: &str) -> AppResult<()> {
    state.tray_icon.set_tooltip(Some(tooltip))?;
    Ok(())
}

pub(crate) fn status_tooltip(
    is_running: bool,
    interval: u64,
    screenshot_count: u64,
    last_capture: Option<&str>,
) -> String {
    let status = if is_running { "运行中" } else { "已暂停" };
    let last_capture = last_capture.unwrap_or("暂无截图");
    format!(
        "{APP_NAME}\n状态: {status}\n间隔: {interval}s\n本次截图: {screenshot_count}\n最近: {last_capture}"
    )
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
        let tooltip = status_tooltip(true, 30, 2, Some("已保存 screen.png"));

        assert!(tooltip.contains("状态: 运行中"));
        assert!(tooltip.contains("间隔: 30s"));
        assert!(tooltip.contains("本次截图: 2"));
        assert!(tooltip.contains("最近: 已保存 screen.png"));
    }
}
