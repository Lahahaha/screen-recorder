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

    let command = format!("{cmd:?}");
    let output_result = cmd.output()?;
    if !output_result.status.success() {
        let details = ffmpeg_failure_details(
            &command,
            output_result.status.to_string(),
            &output_result.stdout,
            &output_result.stderr,
        );
        logger.error(format!("ffmpeg 执行失败: {details}"));
        return Err(io::Error::other(details).into());
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

fn ffmpeg_failure_details(command: &str, status: String, stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = summarize_process_output(stdout);
    let stderr = summarize_process_output(stderr);

    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("命令: {command}; 退出码: {status}; 无 stdout/stderr"),
        (true, false) => format!("命令: {command}; 退出码: {status}; stderr: {stderr}"),
        (false, true) => format!("命令: {command}; 退出码: {status}; stdout: {stdout}"),
        (false, false) => {
            format!("命令: {command}; 退出码: {status}; stderr: {stderr}; stdout: {stdout}")
        }
    }
}

fn summarize_process_output(output: &[u8]) -> String {
    const MAX_LEN: usize = 4000;
    let text = String::from_utf8_lossy(output).trim().to_string();
    if text.len() <= MAX_LEN {
        return text;
    }

    let mut summary = text
        .char_indices()
        .take_while(|(index, _)| *index < MAX_LEN)
        .map(|(_, ch)| ch)
        .collect::<String>();
    summary.push_str("...<truncated>");
    summary
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_video_images_keeps_supported_images_sorted() {
        let images = vec![
            PathBuf::from("z.txt"),
            PathBuf::from("10-00-30.jpg"),
            PathBuf::from("10-00-00.png"),
        ];

        let (images, format) =
            choose_video_images(images, ScreenshotFormat::Jpg).expect("choose images");

        assert_eq!(
            images,
            vec![PathBuf::from("10-00-00.png"), PathBuf::from("10-00-30.jpg")]
        );
        assert_eq!(format, ScreenshotFormat::Jpg);
    }

    #[test]
    fn choose_video_images_rejects_empty_supported_set() {
        let error = choose_video_images(vec![PathBuf::from("notes.txt")], ScreenshotFormat::Png)
            .expect_err("reject empty set");

        assert!(error.to_string().contains("今天还没有可用截图"));
    }

    #[test]
    fn ffmpeg_failure_details_prefers_stderr() {
        let details = ffmpeg_failure_details(
            "\"ffmpeg\" \"-i\" \"input\"",
            "exit status: 1".to_string(),
            b"",
            b"bad input",
        );

        assert!(details.contains("命令: \"ffmpeg\" \"-i\" \"input\""));
        assert!(details.contains("退出码: exit status: 1"));
        assert!(details.contains("stderr: bad input"));
    }

    #[test]
    fn summarize_process_output_truncates_long_text() {
        let long_output = "a".repeat(4100);

        let summary = summarize_process_output(long_output.as_bytes());

        assert!(summary.ends_with("...<truncated>"));
        assert!(summary.len() < long_output.len());
    }
}
