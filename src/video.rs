use crate::{
    app::AppResult,
    capture::screenshot_format_for_path,
    config::{ScreenshotFormat, VideoCodec},
    logging::Logger,
    paths::AppPaths,
    platform,
    temp::TempDir,
};
use chrono::Local;
use image::ImageFormat;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn generate_today_video(
    paths: &AppPaths,
    fps: u32,
    image_format: ScreenshotFormat,
    video_codec: VideoCodec,
    logger: &Logger,
) -> AppResult<PathBuf> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let screenshot_dir = paths.screenshots_dir_for_date(&today);
    if !screenshot_dir.exists() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "今天还没有截图").into());
    }

    let images = fs::read_dir(&screenshot_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;

    let (images, frame_format) = choose_video_images(images, image_format)?;
    fs::create_dir_all(&paths.videos)?;
    let work_dir = TempDir::new(&paths.videos, ".video-work", logger.clone())?;
    prepare_frame_sequence(work_dir.path(), &images, frame_format)?;

    let output = paths.video_path_for_date(&today);
    let temp_output = work_dir.path().join("output.mp4");
    let fps_value = fps.max(1).to_string();
    let ffmpeg = platform::find_ffmpeg()?;
    let input_pattern = work_dir
        .path()
        .join(format!("frame_%06d.{}", frame_format.extension()));
    let mut cmd = Command::new(&ffmpeg);
    cmd.args(["-y", "-framerate", &fps_value, "-start_number", "0", "-i"])
        .arg(&input_pattern)
        .args([
            "-c:v",
            video_codec.encoder(),
            "-pix_fmt",
            "yuv420p",
            "-vf",
            "scale=trunc(iw/2)*2:trunc(ih/2)*2",
            "-r",
            &fps_value,
        ]);
    if video_codec == VideoCodec::H265 {
        cmd.args(["-tag:v", "hvc1"]);
    }
    cmd.arg(&temp_output);
    platform::hide_console(&mut cmd);

    let status = cmd.status()?;
    if !status.success() {
        return Err(io::Error::other(format!("ffmpeg 退出码: {status}")).into());
    }

    platform::replace_file(&temp_output, &output)?;
    logger.info(format!("已生成视频: {}", output.display()));
    Ok(output)
}

fn choose_video_images(
    mut images: Vec<PathBuf>,
    image_format: ScreenshotFormat,
) -> AppResult<(Vec<PathBuf>, ScreenshotFormat)> {
    images.retain(|path| screenshot_format_for_path(path).is_some());
    images.sort();

    if images.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "今天还没有可用截图").into());
    }

    Ok((images, image_format))
}

fn prepare_frame_sequence(
    work_dir: &Path,
    images: &[PathBuf],
    frame_format: ScreenshotFormat,
) -> AppResult<()> {
    for (index, image) in images.iter().enumerate() {
        let frame = work_dir.join(format!("frame_{index:06}.{}", frame_format.extension()));
        if screenshot_format_for_path(image) == Some(frame_format) {
            if fs::hard_link(image, &frame).is_ok() {
                continue;
            }
            fs::copy(image, &frame)?;
            continue;
        }

        let image = image::open(image)?;
        match frame_format {
            ScreenshotFormat::Png => image.save_with_format(&frame, ImageFormat::Png)?,
            ScreenshotFormat::Jpg => image
                .to_rgb8()
                .save_with_format(&frame, ImageFormat::Jpeg)?,
        }
    }
    Ok(())
}
