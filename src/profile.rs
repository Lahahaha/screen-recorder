use crate::{
    app::AppResult, config::load_config, logging::Logger, paths::AppPaths,
    video::generate_video_from_dir,
};
use chrono::Local;
use std::{path::PathBuf, time::Instant};

pub(crate) fn profile_video_dir(input_dir: PathBuf, output: Option<PathBuf>) -> AppResult<()> {
    let paths = AppPaths::new()?;
    let logger = Logger::new(&paths)?;
    let config = load_config(&paths, &logger)?;
    let output = output.unwrap_or_else(|| {
        paths.videos.join(format!(
            "_profile-{}.mp4",
            Local::now().format("%Y%m%d-%H%M%S")
        ))
    });

    let started = Instant::now();
    let report = generate_video_from_dir(
        &input_dir,
        &output,
        config.fps,
        config.image_format,
        config.video_codec,
        &logger,
    )?;

    println!("Profile input: {}", input_dir.display());
    println!("Profile output: {}", report.output.display());
    println!("Frames: {}", report.frame_count);
    println!("Skipped: {}", report.skipped_images.len());
    println!("Elapsed: {:.3}s", started.elapsed().as_secs_f64());

    Ok(())
}
