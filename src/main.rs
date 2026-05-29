#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod capture;
mod config;
mod logging;
mod paths;
mod platform;
mod temp;
mod tray;
mod video;

fn main() -> app::AppResult<()> {
    app::run()
}
