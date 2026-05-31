#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod about;
mod app;
mod capture;
mod config;
mod history;
mod i18n;
mod logging;
mod paths;
mod platform;
mod profile;
mod screenshot_naming;
#[cfg(debug_assertions)]
mod simulate;
mod single_instance;
mod temp;
mod tray;
mod video;
mod workdirs;

fn main() -> app::AppResult<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let workdir = arg_value(&args, "--workdir").map(std::path::PathBuf::from);
    if let Some(index) = args.iter().position(|arg| arg == "--profile-video-dir") {
        let input = args.get(index + 1).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "缺少 --profile-video-dir 的输入目录",
            )
        })?;
        let output = args.get(index + 2).map(std::path::PathBuf::from);
        return profile::profile_video_dir(std::path::PathBuf::from(input), output);
    }

    #[cfg(debug_assertions)]
    {
        if args.iter().any(|arg| arg == "--simulate-multiscreen-video") {
            return simulate::run();
        }
    }

    if args.iter().any(|arg| arg == "--history") {
        return history::run(workdir);
    }
    if args.iter().any(|arg| arg == "--about") {
        return about::run(workdir);
    }
    if args.iter().any(|arg| arg == "--workdirs") {
        return workdirs::run(workdir);
    }
    app::run()
}

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
