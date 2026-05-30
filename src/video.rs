use crate::{
    app::AppResult,
    config::{ScreenshotFormat, VideoCodec},
    logging::Logger,
    paths::AppPaths,
    platform,
    screenshot_naming::{
        metadata_path_for_screenshot, parse_screenshot_file_name, screenshot_format_for_path,
        MultiScreenCaptureMetadata, ScreenshotName,
    },
    temp::TempDir,
};
use chrono::Local;
use image::{imageops::FilterType, DynamicImage, Rgb, RgbImage};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const BACKGROUND: Rgb<u8> = Rgb([24, 24, 24]);
const MAX_MULTI_WIDTH: u32 = 7680;
const MAX_MULTI_HEIGHT: u32 = 4320;

#[derive(Clone, Debug)]
pub(crate) struct VideoGenerationReport {
    pub(crate) output: PathBuf,
    pub(crate) frame_count: usize,
    pub(crate) skipped_images: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VideoGenerationProgress {
    Scanning,
    PreparingFrames { current: usize, total: usize },
    Encoding,
    Replacing,
}

#[derive(Clone, Default)]
pub(crate) struct VideoGenerationCancelToken {
    cancelled: Arc<AtomicBool>,
}

impl VideoGenerationCancelToken {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

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

    let output = paths.video_path_for_date(&today);
    let report = generate_video_from_dir(
        &screenshot_dir,
        &output,
        fps,
        image_format,
        video_codec,
        logger,
    )?;
    Ok(report.output)
}

pub(crate) fn generate_video_from_dir(
    input_dir: &Path,
    output: &Path,
    fps: u32,
    image_format: ScreenshotFormat,
    video_codec: VideoCodec,
    logger: &Logger,
) -> AppResult<VideoGenerationReport> {
    generate_video_from_dir_with_progress(
        input_dir,
        output,
        fps,
        image_format,
        video_codec,
        logger,
        |_| {},
    )
}

pub(crate) fn generate_video_from_dir_with_progress<F>(
    input_dir: &Path,
    output: &Path,
    fps: u32,
    _image_format: ScreenshotFormat,
    video_codec: VideoCodec,
    logger: &Logger,
    progress: F,
) -> AppResult<VideoGenerationReport>
where
    F: Fn(VideoGenerationProgress),
{
    let cancel_token = VideoGenerationCancelToken::default();
    generate_video_from_dir_with_control(
        input_dir,
        output,
        fps,
        video_codec,
        logger,
        progress,
        &cancel_token,
    )
}

pub(crate) fn generate_video_from_dir_with_control<F>(
    input_dir: &Path,
    output: &Path,
    fps: u32,
    video_codec: VideoCodec,
    logger: &Logger,
    progress: F,
    cancel_token: &VideoGenerationCancelToken,
) -> AppResult<VideoGenerationReport>
where
    F: Fn(VideoGenerationProgress),
{
    let total_start = Instant::now();
    check_cancelled(cancel_token)?;
    progress(VideoGenerationProgress::Scanning);
    let scan_start = Instant::now();
    let images = fs::read_dir(input_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    logger.info(format!(
        "视频生成计时: 扫描目录 {}，文件数: {}，耗时: {}",
        input_dir.display(),
        images.len(),
        format_duration(scan_start.elapsed())
    ));
    check_cancelled(cancel_token)?;
    let group_start = Instant::now();
    let frame_groups = choose_video_frame_groups(images)?;
    logger.info(format!(
        "视频生成计时: 解析/分组，帧组数: {}，耗时: {}",
        frame_groups.len(),
        format_duration(group_start.elapsed())
    ));

    let output_start = Instant::now();
    let output_dir = output.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("视频输出路径缺少父目录: {}", output.display()),
        )
    })?;
    fs::create_dir_all(output_dir)?;
    let work_dir = TempDir::new(output_dir, ".video-work", logger.clone())?;
    let temp_output = work_dir.path().join("output.mp4");
    logger.info(format!(
        "视频生成计时: 准备输出目录，耗时: {}",
        format_duration(output_start.elapsed())
    ));

    let encode_start = Instant::now();
    let sequence_report = encode_frame_sequence(
        &frame_groups,
        fps,
        video_codec,
        &temp_output,
        logger,
        &progress,
        cancel_token,
    )?;
    logger.info(format!(
        "视频生成计时: 生成临时视频，帧数: {}，跳过图片: {}，耗时: {}",
        sequence_report.frame_count,
        sequence_report.skipped_images.len(),
        format_duration(encode_start.elapsed())
    ));

    check_cancelled(cancel_token)?;
    progress(VideoGenerationProgress::Replacing);
    let replace_start = Instant::now();
    platform::replace_file(&temp_output, output)?;
    logger.info(format!(
        "视频生成计时: 替换输出文件，耗时: {}",
        format_duration(replace_start.elapsed())
    ));
    logger.info(format!(
        "已生成视频: {}，帧数: {}，跳过图片: {}，总耗时: {}",
        output.display(),
        sequence_report.frame_count,
        sequence_report.skipped_images.len(),
        format_duration(total_start.elapsed())
    ));
    Ok(VideoGenerationReport {
        output: output.to_path_buf(),
        frame_count: sequence_report.frame_count,
        skipped_images: sequence_report.skipped_images,
    })
}

fn encode_frame_sequence<F>(
    frame_groups: &[VideoFrameGroup],
    fps: u32,
    video_codec: VideoCodec,
    temp_output: &Path,
    logger: &Logger,
    progress: &F,
    cancel_token: &VideoGenerationCancelToken,
) -> AppResult<FrameSequenceReport>
where
    F: Fn(VideoGenerationProgress),
{
    let plan_start = Instant::now();
    let plan = plan_frame_sequence(frame_groups, logger)?;
    logger.info(format!(
        "视频生成计时: 规划帧序列，帧数: {}，输出尺寸: {}x{}，预跳过图片: {}，耗时: {}",
        plan.frames.len(),
        plan.output_size.width,
        plan.output_size.height,
        plan.skipped_images.len(),
        format_duration(plan_start.elapsed())
    ));
    let target = FfmpegTarget {
        fps,
        video_codec,
        temp_output,
    };
    if let Some(codec) = encoded_pipe_codec_for_plan(&plan) {
        logger.info(format!(
            "视频生成路径: 原图直通 image2pipe，codec: {:?}，帧数: {}",
            codec,
            plan.frames.len()
        ));
        return encode_frame_sequence_encoded_pipe(
            plan,
            codec,
            target,
            logger,
            progress,
            cancel_token,
        );
    }

    logger.info(format!(
        "视频生成路径: 原始 RGB 管道，帧数: {}，输出尺寸: {}x{}",
        plan.frames.len(),
        plan.output_size.width,
        plan.output_size.height
    ));
    encode_frame_sequence_raw_pipe(plan, target, logger, progress, cancel_token)
}

fn encode_frame_sequence_raw_pipe<F>(
    plan: SequencePlan,
    target: FfmpegTarget<'_>,
    logger: &Logger,
    progress: &F,
    cancel_token: &VideoGenerationCancelToken,
) -> AppResult<FrameSequenceReport>
where
    F: Fn(VideoGenerationProgress),
{
    let cmd = raw_video_ffmpeg_command(plan.output_size, target)?;
    run_ffmpeg_with_input(cmd, logger, cancel_token, |stdin| {
        let report = stream_frame_sequence_to_writer(plan, stdin, logger, progress, cancel_token)?;
        progress(VideoGenerationProgress::Encoding);
        Ok(report)
    })
}

fn encode_frame_sequence_encoded_pipe<F>(
    plan: SequencePlan,
    codec: EncodedPipeCodec,
    target: FfmpegTarget<'_>,
    logger: &Logger,
    progress: &F,
    cancel_token: &VideoGenerationCancelToken,
) -> AppResult<FrameSequenceReport>
where
    F: Fn(VideoGenerationProgress),
{
    let cmd = encoded_pipe_ffmpeg_command(codec, target)?;
    run_ffmpeg_with_input(cmd, logger, cancel_token, |stdin| {
        let report =
            stream_encoded_frame_sequence_to_writer(plan, stdin, logger, progress, cancel_token)?;
        progress(VideoGenerationProgress::Encoding);
        Ok(report)
    })
}

fn run_ffmpeg_with_input<W>(
    mut cmd: Command,
    logger: &Logger,
    cancel_token: &VideoGenerationCancelToken,
    write_input: W,
) -> AppResult<FrameSequenceReport>
where
    W: FnOnce(&mut dyn Write) -> AppResult<FrameSequenceReport>,
{
    check_cancelled(cancel_token)?;
    platform::hide_console(&mut cmd);

    let command = format!("{cmd:?}");
    let spawn_start = Instant::now();
    let mut child = cmd.spawn()?;
    logger.info(format!(
        "视频生成计时: 启动 ffmpeg，耗时: {}",
        format_duration(spawn_start.elapsed())
    ));
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("无法打开 ffmpeg stdin"))?;
    let write_start = Instant::now();
    let stream_result = write_input(&mut stdin);
    logger.info(format!(
        "视频生成计时: 写入 ffmpeg stdin，耗时: {}",
        format_duration(write_start.elapsed())
    ));
    drop(stdin);
    if stream_result.is_err() && cancel_token.is_cancelled() {
        let _ = child.kill();
    }

    let wait_start = Instant::now();
    let output_result = wait_for_ffmpeg_output(child, cancel_token)?;
    logger.info(format!(
        "视频生成计时: 等待 ffmpeg 编码完成，耗时: {}",
        format_duration(wait_start.elapsed())
    ));
    if cancel_token.is_cancelled() {
        return Err(cancelled_error());
    }
    match stream_result {
        Ok(report) if output_result.status.success() => Ok(report),
        Ok(_) => {
            let details = ffmpeg_failure_details(
                &command,
                output_result.status.to_string(),
                &output_result.stdout,
                &output_result.stderr,
            );
            logger.error(format!("ffmpeg 执行失败: {details}"));
            Err(io::Error::other(details).into())
        }
        Err(error) if output_result.status.success() => Err(error),
        Err(error) => {
            let details = ffmpeg_failure_details(
                &command,
                output_result.status.to_string(),
                &output_result.stdout,
                &output_result.stderr,
            );
            logger.error(format!("ffmpeg 执行失败: {details}"));
            Err(io::Error::other(format!("{error}; {details}")).into())
        }
    }
}

fn wait_for_ffmpeg_output(
    mut child: Child,
    cancel_token: &VideoGenerationCancelToken,
) -> io::Result<Output> {
    loop {
        if cancel_token.is_cancelled() {
            let _ = child.kill();
            return child.wait_with_output();
        }
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn raw_video_ffmpeg_command(size: Dimensions, target: FfmpegTarget<'_>) -> AppResult<Command> {
    let fps_value = target.fps.max(1).to_string();
    let ffmpeg = platform::find_ffmpeg()?;
    let video_size = format!("{}x{}", size.width, size.height);
    let mut cmd = Command::new(&ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "-s",
        &video_size,
        "-r",
        &fps_value,
        "-i",
        "pipe:0",
        "-an",
    ])
    .args([
        "-c:v",
        target.video_codec.encoder(),
        "-pix_fmt",
        "yuv420p",
        "-r",
        &fps_value,
    ]);
    if target.video_codec == VideoCodec::H265 {
        cmd.args(["-tag:v", "hvc1"]);
    }
    cmd.arg(target.temp_output);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(cmd)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EncodedPipeCodec {
    Png,
    Jpeg,
}

impl EncodedPipeCodec {
    fn ffmpeg_decoder(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "mjpeg",
        }
    }
}

fn encoded_pipe_ffmpeg_command(
    codec: EncodedPipeCodec,
    target: FfmpegTarget<'_>,
) -> AppResult<Command> {
    let fps_value = target.fps.max(1).to_string();
    let ffmpeg = platform::find_ffmpeg()?;
    let mut cmd = Command::new(&ffmpeg);
    cmd.args([
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "image2pipe",
        "-framerate",
        &fps_value,
        "-vcodec",
        codec.ffmpeg_decoder(),
        "-i",
        "pipe:0",
        "-an",
    ])
    .args([
        "-c:v",
        target.video_codec.encoder(),
        "-pix_fmt",
        "yuv420p",
        "-r",
        &fps_value,
    ]);
    if target.video_codec == VideoCodec::H265 {
        cmd.args(["-tag:v", "hvc1"]);
    }
    cmd.arg(target.temp_output);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(cmd)
}

#[cfg(test)]
fn choose_video_images(
    images: Vec<PathBuf>,
    image_format: ScreenshotFormat,
) -> AppResult<(Vec<PathBuf>, ScreenshotFormat)> {
    let frame_groups = choose_video_frame_groups(images)?;
    let images = frame_groups
        .into_iter()
        .flat_map(|group| group.images.into_iter().map(|image| image.path))
        .collect::<Vec<_>>();

    Ok((images, image_format))
}

#[derive(Clone, Debug, PartialEq)]
struct VideoFrameGroup {
    images: Vec<FrameImage>,
}

#[derive(Clone, Debug, PartialEq)]
struct FrameImage {
    path: PathBuf,
    screen_index: Option<u32>,
    geometry: Option<ScreenGeometry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Dimensions {
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScreenGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug)]
struct SequencePlan {
    frames: Vec<PlannedFrame>,
    output_size: Dimensions,
    skipped_images: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct PlannedFrame {
    placements: Vec<FramePlacement>,
}

#[derive(Clone, Debug)]
struct FramePlacement {
    path: PathBuf,
    slot: Rect,
    source_size: Dimensions,
}

struct FrameSequenceReport {
    frame_count: usize,
    skipped_images: Vec<PathBuf>,
}

#[derive(Clone, Copy)]
struct FfmpegTarget<'a> {
    fps: u32,
    video_codec: VideoCodec,
    temp_output: &'a Path,
}

fn choose_video_frame_groups(images: Vec<PathBuf>) -> AppResult<Vec<VideoFrameGroup>> {
    let mut images = images
        .into_iter()
        .filter(|path| screenshot_format_for_path(path).is_some())
        .collect::<Vec<_>>();
    images.sort();

    if images.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "今天还没有可用截图").into());
    }

    let mut groups = BTreeMap::<String, VideoFrameGroup>::new();
    let mut multi_screen = BTreeMap::<(String, u64), Vec<(u32, PathBuf)>>::new();

    for image in images {
        match parse_screenshot_file_name(&image) {
            Some(ScreenshotName::Single {
                timestamp,
                sequence,
            }) => {
                groups.insert(
                    format!("{timestamp}-{sequence:06}-single"),
                    VideoFrameGroup {
                        images: vec![FrameImage {
                            path: image,
                            screen_index: None,
                            geometry: None,
                        }],
                    },
                );
            }
            Some(ScreenshotName::MultiScreen {
                timestamp,
                screen_index,
                sequence,
            }) => {
                multi_screen
                    .entry((timestamp, sequence))
                    .or_default()
                    .push((screen_index, image));
            }
            None => {
                let key = image.to_string_lossy().to_string();
                groups.insert(
                    format!("external-{key}"),
                    VideoFrameGroup {
                        images: vec![FrameImage {
                            path: image,
                            screen_index: None,
                            geometry: None,
                        }],
                    },
                );
            }
        }
    }

    for ((timestamp, sequence), entries) in multi_screen {
        let images = ordered_multi_screen_images(entries);
        groups.insert(
            format!("{timestamp}-{sequence:06}-multi"),
            VideoFrameGroup { images },
        );
    }

    Ok(groups.into_values().collect())
}

fn ordered_multi_screen_images(mut entries: Vec<(u32, PathBuf)>) -> Vec<FrameImage> {
    entries.sort_by_key(|(screen_index, path)| (*screen_index, path.clone()));
    let fallback = || {
        entries
            .iter()
            .map(|(screen_index, path)| FrameImage {
                path: path.clone(),
                screen_index: Some(*screen_index),
                geometry: None,
            })
            .collect::<Vec<_>>()
    };
    let Some(metadata_path) = entries
        .first()
        .and_then(|(_, path)| metadata_path_for_screenshot(path))
    else {
        return fallback();
    };

    let Ok(content) = fs::read_to_string(metadata_path) else {
        return fallback();
    };
    let Ok(metadata) = serde_json::from_str::<MultiScreenCaptureMetadata>(&content) else {
        return fallback();
    };

    let mut by_name = entries
        .iter()
        .filter_map(|(screen_index, path)| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| (name.to_string(), (*screen_index, path.clone())))
        })
        .collect::<HashMap<_, _>>();
    let mut ordered = Vec::new();
    for screen in metadata.screens {
        if let Some((screen_index, path)) = by_name.remove(&screen.file) {
            ordered.push(FrameImage {
                path,
                screen_index: Some(screen_index),
                geometry: screen_geometry_from_metadata(&screen),
            });
        }
    }

    for (screen_index, path) in entries {
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .and_then(|name| by_name.remove(name))
            .is_some()
        {
            ordered.push(FrameImage {
                path,
                screen_index: Some(screen_index),
                geometry: None,
            });
        }
    }

    ordered
}

fn screen_geometry_from_metadata(
    screen: &crate::screenshot_naming::ScreenCaptureMetadata,
) -> Option<ScreenGeometry> {
    Some(ScreenGeometry {
        x: screen.x?,
        y: screen.y?,
        width: screen.width,
        height: screen.height,
    })
}

fn stream_frame_sequence_to_writer<W, F>(
    plan: SequencePlan,
    writer: &mut W,
    logger: &Logger,
    progress: &F,
    cancel_token: &VideoGenerationCancelToken,
) -> AppResult<FrameSequenceReport>
where
    W: Write + ?Sized,
    F: Fn(VideoGenerationProgress),
{
    let stream_start = Instant::now();
    let output_size = plan.output_size;
    let bytes_per_frame =
        u64::from(output_size.width) * u64::from(output_size.height) * u64::from(3_u8);
    let mut skipped_images = plan.skipped_images;
    let mut frame_count = 0_usize;
    let total_frames = plan.frames.len();
    let mut canvas_duration = Duration::ZERO;
    let mut render_duration = Duration::ZERO;
    let mut paste_duration = Duration::ZERO;
    let mut write_duration = Duration::ZERO;
    let mut placement_count = 0_usize;

    for (index, frame) in plan.frames.into_iter().enumerate() {
        check_cancelled(cancel_token)?;
        let canvas_start = Instant::now();
        let mut canvas = RgbImage::from_pixel(output_size.width, output_size.height, BACKGROUND);
        canvas_duration += canvas_start.elapsed();
        let mut rendered_any = false;
        for placement in frame.placements {
            placement_count += 1;
            let render_start = Instant::now();
            let render_result = render_placement(&placement);
            render_duration += render_start.elapsed();
            match render_result {
                Ok((image, x, y)) => {
                    let paste_start = Instant::now();
                    paste_rgb(&mut canvas, &image, x, y);
                    paste_duration += paste_start.elapsed();
                    rendered_any = true;
                }
                Err(error) => {
                    logger.warn(format!(
                        "跳过不可读图片 {}: {error}",
                        placement.path.display()
                    ));
                    skipped_images.push(placement.path);
                }
            }
        }

        if !rendered_any {
            progress(VideoGenerationProgress::PreparingFrames {
                current: index + 1,
                total: total_frames,
            });
            continue;
        }
        let write_start = Instant::now();
        writer.write_all(canvas.as_raw())?;
        write_duration += write_start.elapsed();
        frame_count += 1;
        progress(VideoGenerationProgress::PreparingFrames {
            current: index + 1,
            total: total_frames,
        });
    }

    if frame_count == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "没有可读图片").into());
    }

    logger.info(format!(
        "视频生成计时: 原始 RGB 帧准备，帧数: {}，画布: {}x{}，写入字节: {}，耗时: {}",
        frame_count,
        output_size.width,
        output_size.height,
        bytes_per_frame * frame_count as u64,
        format_duration(stream_start.elapsed())
    ));
    logger.info(format!(
        "视频生成计时: 原始 RGB 明细，placement: {}，初始化画布: {}，解码/缩放/转 RGB: {}，贴图: {}，写入 stdin: {}",
        placement_count,
        format_duration(canvas_duration),
        format_duration(render_duration),
        format_duration(paste_duration),
        format_duration(write_duration)
    ));

    Ok(FrameSequenceReport {
        frame_count,
        skipped_images,
    })
}

fn stream_encoded_frame_sequence_to_writer<W, F>(
    plan: SequencePlan,
    writer: &mut W,
    logger: &Logger,
    progress: &F,
    cancel_token: &VideoGenerationCancelToken,
) -> AppResult<FrameSequenceReport>
where
    W: Write + ?Sized,
    F: Fn(VideoGenerationProgress),
{
    let stream_start = Instant::now();
    let skipped_images = plan.skipped_images;
    let total_frames = plan.frames.len();
    let mut frame_count = 0_usize;
    let mut total_bytes = 0_u64;
    for (index, frame) in plan.frames.into_iter().enumerate() {
        check_cancelled(cancel_token)?;
        let source = encoded_pipe_frame_source(&frame, plan.output_size)
            .ok_or_else(|| io::Error::other("视频帧不适合直接编码管道"))?;
        let mut file = fs::File::open(source)?;
        total_bytes += io::copy(&mut file, writer)?;
        frame_count += 1;
        progress(VideoGenerationProgress::PreparingFrames {
            current: index + 1,
            total: total_frames,
        });
    }

    if frame_count == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "没有可读图片").into());
    }

    logger.info(format!(
        "视频生成计时: 原图直通 image2pipe，帧数: {}，输入字节: {}，耗时: {}",
        frame_count,
        total_bytes,
        format_duration(stream_start.elapsed())
    ));

    Ok(FrameSequenceReport {
        frame_count,
        skipped_images,
    })
}

fn encoded_pipe_codec_for_plan(plan: &SequencePlan) -> Option<EncodedPipeCodec> {
    let mut codec = None;
    for frame in &plan.frames {
        let source = encoded_pipe_frame_source(frame, plan.output_size)?;
        let current_codec = match screenshot_format_for_path(source)? {
            ScreenshotFormat::Png => EncodedPipeCodec::Png,
            ScreenshotFormat::Jpg => EncodedPipeCodec::Jpeg,
        };
        match codec {
            Some(previous) if previous != current_codec => return None,
            None => codec = Some(current_codec),
            _ => {}
        }
    }
    codec
}

fn encoded_pipe_frame_source(frame: &PlannedFrame, output_size: Dimensions) -> Option<&Path> {
    let [placement] = frame.placements.as_slice() else {
        return None;
    };
    if placement.slot.x != 0 || placement.slot.y != 0 {
        return None;
    }
    if placement.slot.width != output_size.width || placement.slot.height != output_size.height {
        return None;
    }
    if placement.source_size != output_size {
        return None;
    }
    Some(&placement.path)
}

fn check_cancelled(cancel_token: &VideoGenerationCancelToken) -> AppResult<()> {
    if cancel_token.is_cancelled() {
        return Err(cancelled_error());
    }
    Ok(())
}

fn cancelled_error() -> Box<dyn std::error::Error> {
    io::Error::new(io::ErrorKind::Interrupted, "视频生成已取消").into()
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs() > 0 {
        format!("{:.3}s", duration.as_secs_f64())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

fn plan_frame_sequence(
    frame_groups: &[VideoFrameGroup],
    logger: &Logger,
) -> AppResult<SequencePlan> {
    let mut readable_frames = Vec::<ReadableFrame>::new();
    let mut skipped_images = Vec::<PathBuf>::new();

    for group in frame_groups {
        let mut readable_images = Vec::new();
        for image in &group.images {
            match image::image_dimensions(&image.path) {
                Ok((width, height)) if width > 0 && height > 0 => {
                    readable_images.push(ReadableImage {
                        image: image.clone(),
                        source_size: Dimensions { width, height },
                    });
                }
                Ok(_) => skipped_images.push(image.path.clone()),
                Err(error) => {
                    logger.warn(format!("跳过不可读图片 {}: {error}", image.path.display()));
                    skipped_images.push(image.path.clone());
                }
            }
        }

        if readable_images.is_empty() {
            continue;
        }

        if readable_images
            .iter()
            .any(|image| image.image.screen_index.is_some())
        {
            readable_frames.push(ReadableFrame::Multi { readable_images });
        } else {
            readable_frames.push(ReadableFrame::Single {
                readable_image: readable_images.remove(0),
            });
        }
    }

    if readable_frames.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "没有可读图片").into());
    }

    let multi_layout = build_global_multi_layout(&readable_frames);
    let mut planned_frames = Vec::new();
    let mut output_size = Dimensions {
        width: 2,
        height: 2,
    };

    for frame in readable_frames {
        match frame {
            ReadableFrame::Single { readable_image } => {
                let size = even_dimensions(readable_image.source_size);
                output_size.width = output_size.width.max(size.width);
                output_size.height = output_size.height.max(size.height);
                planned_frames.push(PlannedFrame {
                    placements: vec![FramePlacement {
                        path: readable_image.image.path,
                        slot: Rect {
                            x: 0,
                            y: 0,
                            width: size.width,
                            height: size.height,
                        },
                        source_size: readable_image.source_size,
                    }],
                });
            }
            ReadableFrame::Multi { readable_images } => {
                let Some(layout) = multi_layout.as_ref() else {
                    continue;
                };
                output_size.width = output_size.width.max(layout.canvas.width);
                output_size.height = output_size.height.max(layout.canvas.height);
                let placements = readable_images
                    .into_iter()
                    .filter_map(|image| {
                        let screen_index = image.image.screen_index?;
                        let slot = *layout.slots.get(&screen_index)?;
                        Some(FramePlacement {
                            path: image.image.path,
                            slot,
                            source_size: image.source_size,
                        })
                    })
                    .collect::<Vec<_>>();
                if !placements.is_empty() {
                    planned_frames.push(PlannedFrame { placements });
                }
            }
        }
    }

    output_size = even_dimensions(output_size);
    for frame in &mut planned_frames {
        let frame_bounds = frame_bounds(frame);
        let offset_x = (output_size.width.saturating_sub(frame_bounds.width)) / 2;
        let offset_y = (output_size.height.saturating_sub(frame_bounds.height)) / 2;
        for placement in &mut frame.placements {
            placement.slot.x += offset_x;
            placement.slot.y += offset_y;
        }
    }

    Ok(SequencePlan {
        frames: planned_frames,
        output_size,
        skipped_images,
    })
}

#[derive(Clone, Debug)]
enum ReadableFrame {
    Single { readable_image: ReadableImage },
    Multi { readable_images: Vec<ReadableImage> },
}

#[derive(Clone, Debug)]
struct ReadableImage {
    image: FrameImage,
    source_size: Dimensions,
}

#[derive(Clone, Debug)]
struct MultiLayout {
    slots: BTreeMap<u32, Rect>,
    canvas: Dimensions,
}

#[derive(Clone, Debug)]
struct SlotSpec {
    screen_index: u32,
    size: Dimensions,
    geometry: Option<ScreenGeometry>,
}

fn build_global_multi_layout(frames: &[ReadableFrame]) -> Option<MultiLayout> {
    let mut slots = BTreeMap::<u32, SlotSpec>::new();
    for frame in frames {
        let ReadableFrame::Multi { readable_images } = frame else {
            continue;
        };
        for image in readable_images {
            let Some(screen_index) = image.image.screen_index else {
                continue;
            };
            let geometry = image.image.geometry;
            let size = geometry
                .map(|geometry| Dimensions {
                    width: geometry.width,
                    height: geometry.height,
                })
                .unwrap_or(image.source_size);
            slots
                .entry(screen_index)
                .and_modify(|slot| {
                    slot.size.width = slot.size.width.max(size.width);
                    slot.size.height = slot.size.height.max(size.height);
                    if slot.geometry.is_none() {
                        slot.geometry = geometry;
                    }
                })
                .or_insert(SlotSpec {
                    screen_index,
                    size,
                    geometry,
                });
        }
    }

    if slots.is_empty() {
        return None;
    }

    let slots = slots.into_values().collect::<Vec<_>>();
    let layout = if slots.iter().all(|slot| slot.geometry.is_some()) {
        geometry_layout(&slots)
    } else {
        fallback_grid_layout(&slots)
    };
    Some(scale_multi_layout(
        layout,
        MAX_MULTI_WIDTH,
        MAX_MULTI_HEIGHT,
    ))
}

fn geometry_layout(slots: &[SlotSpec]) -> MultiLayout {
    let min_x = slots
        .iter()
        .filter_map(|slot| slot.geometry.map(|geometry| geometry.x))
        .min()
        .unwrap_or(0);
    let min_y = slots
        .iter()
        .filter_map(|slot| slot.geometry.map(|geometry| geometry.y))
        .min()
        .unwrap_or(0);
    let mut rects = BTreeMap::new();
    let mut width = 0_u32;
    let mut height = 0_u32;

    for slot in slots {
        let geometry = slot.geometry.expect("geometry checked by caller");
        let x = (geometry.x - min_x).max(0) as u32;
        let y = (geometry.y - min_y).max(0) as u32;
        let rect = Rect {
            x,
            y,
            width: geometry.width,
            height: geometry.height,
        };
        width = width.max(rect.x + rect.width);
        height = height.max(rect.y + rect.height);
        rects.insert(slot.screen_index, rect);
    }

    MultiLayout {
        slots: rects,
        canvas: even_dimensions(Dimensions { width, height }),
    }
}

fn fallback_grid_layout(slots: &[SlotSpec]) -> MultiLayout {
    let mut slots = slots.to_vec();
    slots.sort_by_key(|slot| slot.screen_index);
    let count = slots.len();
    let columns = fallback_columns(count);
    let rows = count.div_ceil(columns);
    let mut column_widths = vec![0_u32; columns];
    let mut row_heights = vec![0_u32; rows];

    for (index, slot) in slots.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        column_widths[column] = column_widths[column].max(slot.size.width);
        row_heights[row] = row_heights[row].max(slot.size.height);
    }

    let mut x_offsets = vec![0_u32; columns];
    let mut y_offsets = vec![0_u32; rows];
    for column in 1..columns {
        x_offsets[column] = x_offsets[column - 1] + column_widths[column - 1];
    }
    for row in 1..rows {
        y_offsets[row] = y_offsets[row - 1] + row_heights[row - 1];
    }

    let mut rects = BTreeMap::new();
    for (index, slot) in slots.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        rects.insert(
            slot.screen_index,
            Rect {
                x: x_offsets[column],
                y: y_offsets[row],
                width: column_widths[column],
                height: row_heights[row],
            },
        );
    }

    MultiLayout {
        slots: rects,
        canvas: even_dimensions(Dimensions {
            width: column_widths.into_iter().sum(),
            height: row_heights.into_iter().sum(),
        }),
    }
}

fn fallback_columns(count: usize) -> usize {
    match count {
        0 | 1 => 1,
        2 => 2,
        3 | 4 => 2,
        _ => (count as f64).sqrt().ceil() as usize,
    }
}

fn scale_multi_layout(layout: MultiLayout, max_width: u32, max_height: u32) -> MultiLayout {
    if layout.canvas.width <= max_width && layout.canvas.height <= max_height {
        return layout;
    }

    let scale = (max_width as f64 / layout.canvas.width as f64)
        .min(max_height as f64 / layout.canvas.height as f64);
    let slots = layout
        .slots
        .into_iter()
        .map(|(screen_index, rect)| {
            (
                screen_index,
                Rect {
                    x: scale_u32(rect.x, scale),
                    y: scale_u32(rect.y, scale),
                    width: scale_u32(rect.width, scale).max(1),
                    height: scale_u32(rect.height, scale).max(1),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    MultiLayout {
        slots,
        canvas: even_dimensions(Dimensions {
            width: scale_u32(layout.canvas.width, scale),
            height: scale_u32(layout.canvas.height, scale),
        }),
    }
}

fn scale_u32(value: u32, scale: f64) -> u32 {
    ((value as f64 * scale).round() as u32).max(1)
}

fn even_dimensions(size: Dimensions) -> Dimensions {
    Dimensions {
        width: even_dimension(size.width),
        height: even_dimension(size.height),
    }
}

fn even_dimension(value: u32) -> u32 {
    let value = value.max(2);
    if value.is_multiple_of(2) {
        value
    } else {
        value + 1
    }
}

fn frame_bounds(frame: &PlannedFrame) -> Dimensions {
    frame.placements.iter().fold(
        Dimensions {
            width: 0,
            height: 0,
        },
        |bounds, placement| Dimensions {
            width: bounds.width.max(placement.slot.x + placement.slot.width),
            height: bounds.height.max(placement.slot.y + placement.slot.height),
        },
    )
}

fn render_placement(placement: &FramePlacement) -> AppResult<(RgbImage, u32, u32)> {
    let image = image::open(&placement.path)?;
    let (target_size, offset_x, offset_y) = fit_within_slot(placement.source_size, placement.slot);
    let image = image.resize_exact(target_size.width, target_size.height, FilterType::Lanczos3);
    let image = rgba_to_rgb_on_background(image);
    Ok((image, offset_x, offset_y))
}

fn fit_within_slot(source: Dimensions, slot: Rect) -> (Dimensions, u32, u32) {
    let scale = (slot.width as f64 / source.width as f64)
        .min(slot.height as f64 / source.height as f64)
        .min(1.0);
    let size = Dimensions {
        width: scale_u32(source.width, scale).min(slot.width),
        height: scale_u32(source.height, scale).min(slot.height),
    };
    let x = slot.x + (slot.width.saturating_sub(size.width)) / 2;
    let y = slot.y + (slot.height.saturating_sub(size.height)) / 2;
    (size, x, y)
}

fn rgba_to_rgb_on_background(image: DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, pixel) in rgba.enumerate_pixels() {
        let alpha = pixel[3] as u32;
        let inverse = 255 - alpha;
        let red = blend_channel(pixel[0], BACKGROUND[0], alpha, inverse);
        let green = blend_channel(pixel[1], BACKGROUND[1], alpha, inverse);
        let blue = blend_channel(pixel[2], BACKGROUND[2], alpha, inverse);
        rgb.put_pixel(x, y, Rgb([red, green, blue]));
    }
    rgb
}

fn blend_channel(source: u8, background: u8, alpha: u32, inverse: u32) -> u8 {
    (((source as u32 * alpha) + (background as u32 * inverse) + 127) / 255) as u8
}

fn paste_rgb(canvas: &mut RgbImage, image: &RgbImage, x: u32, y: u32) {
    for (pixel_x, pixel_y, pixel) in image.enumerate_pixels() {
        let target_x = x + pixel_x;
        let target_y = y + pixel_y;
        if target_x < canvas.width() && target_y < canvas.height() {
            canvas.put_pixel(target_x, target_y, *pixel);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use image::{ImageBuffer, Rgba};

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-video-test-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f")
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn slot(screen_index: u32, width: u32, height: u32) -> SlotSpec {
        SlotSpec {
            screen_index,
            size: Dimensions { width, height },
            geometry: None,
        }
    }

    fn create_png(path: &Path, width: u32, height: u32, color: [u8; 4]) {
        let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_pixel(width, height, Rgba(color));
        image.save(path).expect("save png");
    }

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
    fn choose_video_frame_groups_keeps_legacy_single_screen_as_single_frame() {
        let groups = choose_video_frame_groups(vec![PathBuf::from("14-03-22.184-000512.png")])
            .expect("choose frame groups");

        assert_eq!(
            group_paths(&groups),
            vec![vec![PathBuf::from("14-03-22.184-000512.png")]]
        );
    }

    #[test]
    fn choose_video_frame_groups_groups_multi_screen_batch() {
        let groups = choose_video_frame_groups(vec![
            PathBuf::from("14-03-22.184-screen-02-000512.png"),
            PathBuf::from("14-03-22.184-screen-01-000512.png"),
        ])
        .expect("choose frame groups");

        assert_eq!(
            group_paths(&groups),
            vec![vec![
                PathBuf::from("14-03-22.184-screen-01-000512.png"),
                PathBuf::from("14-03-22.184-screen-02-000512.png"),
            ]]
        );
    }

    #[test]
    fn choose_video_frame_groups_does_not_merge_different_sequences() {
        let groups = choose_video_frame_groups(vec![
            PathBuf::from("14-03-22.184-screen-01-000512.png"),
            PathBuf::from("14-03-22.184-screen-01-000513.png"),
        ])
        .expect("choose frame groups");

        assert_eq!(
            group_paths(&groups),
            vec![
                vec![PathBuf::from("14-03-22.184-screen-01-000512.png")],
                vec![PathBuf::from("14-03-22.184-screen-01-000513.png")]
            ]
        );
    }

    #[test]
    fn choose_video_frame_groups_prefers_sidecar_order() {
        let dir = test_dir();
        let screen_1 = dir.join("14-03-22.184-screen-01-000512.png");
        let screen_2 = dir.join("14-03-22.184-screen-02-000512.png");
        let metadata = dir.join("14-03-22.184-000512.screens.json");
        fs::write(
            metadata,
            r#"{
              "captured_at": "2026-05-30T14:03:22.184+08:00",
              "sequence": 512,
              "screens": [
                {
                  "screen_index": 2,
                  "file": "14-03-22.184-screen-02-000512.png",
                  "x": 1920,
                  "y": 0,
                  "width": 1920,
                  "height": 1080
                },
                {
                  "screen_index": 1,
                  "file": "14-03-22.184-screen-01-000512.png",
                  "x": 0,
                  "y": 0,
                  "width": 1920,
                  "height": 1080
                }
              ]
            }"#,
        )
        .expect("write metadata");

        let groups =
            choose_video_frame_groups(vec![screen_1.clone(), screen_2.clone()]).expect("groups");

        assert_eq!(group_paths(&groups), vec![vec![screen_2, screen_1]]);
        assert_eq!(
            groups[0].images[0].geometry,
            Some(ScreenGeometry {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080
            })
        );
    }

    #[test]
    fn choose_video_frame_groups_keeps_external_names_as_independent_frames() {
        let groups = choose_video_frame_groups(vec![
            PathBuf::from("external-b.png"),
            PathBuf::from("external-a.jpg"),
        ])
        .expect("choose frame groups");

        assert_eq!(
            group_paths(&groups),
            vec![
                vec![PathBuf::from("external-a.jpg")],
                vec![PathBuf::from("external-b.png")]
            ]
        );
    }

    #[test]
    fn fallback_layout_handles_two_2k_screens() {
        let layout = fallback_grid_layout(&[slot(1, 2560, 1440), slot(2, 2560, 1440)]);

        assert_eq!(
            layout.canvas,
            Dimensions {
                width: 5120,
                height: 1440
            }
        );
    }

    #[test]
    fn fallback_layout_handles_four_2k_screens() {
        let layout = fallback_grid_layout(&[
            slot(1, 2560, 1440),
            slot(2, 2560, 1440),
            slot(3, 2560, 1440),
            slot(4, 2560, 1440),
        ]);

        assert_eq!(
            layout.canvas,
            Dimensions {
                width: 5120,
                height: 2880
            }
        );
    }

    #[test]
    fn fallback_layout_handles_two_4k_screens() {
        let layout = fallback_grid_layout(&[slot(1, 3840, 2160), slot(2, 3840, 2160)]);

        assert_eq!(
            layout.canvas,
            Dimensions {
                width: 7680,
                height: 2160
            }
        );
    }

    #[test]
    fn fallback_layout_handles_four_4k_screens() {
        let layout = fallback_grid_layout(&[
            slot(1, 3840, 2160),
            slot(2, 3840, 2160),
            slot(3, 3840, 2160),
            slot(4, 3840, 2160),
        ]);

        assert_eq!(
            layout.canvas,
            Dimensions {
                width: 7680,
                height: 4320
            }
        );
    }

    #[test]
    fn multi_layout_scales_two_8k_screens_to_8k_bounds() {
        let layout = fallback_grid_layout(&[slot(1, 7680, 4320), slot(2, 7680, 4320)]);
        let layout = scale_multi_layout(layout, MAX_MULTI_WIDTH, MAX_MULTI_HEIGHT);

        assert!(layout.canvas.width <= MAX_MULTI_WIDTH);
        assert!(layout.canvas.height <= MAX_MULTI_HEIGHT);
        assert_eq!(
            layout.canvas,
            Dimensions {
                width: 7680,
                height: 2160
            }
        );
    }

    #[test]
    fn geometry_layout_uses_real_screen_coordinates() {
        let layout = geometry_layout(&[
            SlotSpec {
                screen_index: 1,
                size: Dimensions {
                    width: 2560,
                    height: 1440,
                },
                geometry: Some(ScreenGeometry {
                    x: 0,
                    y: 0,
                    width: 2560,
                    height: 1440,
                }),
            },
            SlotSpec {
                screen_index: 2,
                size: Dimensions {
                    width: 1920,
                    height: 1080,
                },
                geometry: Some(ScreenGeometry {
                    x: 2560,
                    y: 360,
                    width: 1920,
                    height: 1080,
                }),
            },
        ]);

        assert_eq!(
            layout.canvas,
            Dimensions {
                width: 4480,
                height: 1440
            }
        );
        assert_eq!(
            layout.slots[&2],
            Rect {
                x: 2560,
                y: 360,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn single_frame_plan_keeps_2k_size() {
        let dir = test_dir();
        let image = dir.join("screen.png");
        create_png(&image, 2560, 1440, [255, 0, 0, 255]);
        let groups = vec![VideoFrameGroup {
            images: vec![FrameImage {
                path: image,
                screen_index: None,
                geometry: None,
            }],
        }];
        let logger = test_logger(&dir);

        let plan = plan_frame_sequence(&groups, &logger).expect("plan frames");

        assert_eq!(
            plan.output_size,
            Dimensions {
                width: 2560,
                height: 1440
            }
        );
    }

    #[test]
    fn encoded_pipe_codec_accepts_same_size_single_screen_pngs() {
        let dir = test_dir();
        let first = dir.join("first.png");
        let second = dir.join("second.png");
        create_png(&first, 4, 2, [255, 0, 0, 255]);
        create_png(&second, 4, 2, [0, 255, 0, 255]);
        let groups = vec![
            VideoFrameGroup {
                images: vec![FrameImage {
                    path: first,
                    screen_index: None,
                    geometry: None,
                }],
            },
            VideoFrameGroup {
                images: vec![FrameImage {
                    path: second,
                    screen_index: None,
                    geometry: None,
                }],
            },
        ];
        let logger = test_logger(&dir);

        let plan = plan_frame_sequence(&groups, &logger).expect("plan frames");

        assert_eq!(
            encoded_pipe_codec_for_plan(&plan),
            Some(EncodedPipeCodec::Png)
        );
    }

    #[test]
    fn encoded_pipe_codec_rejects_frames_that_need_canvas_composition() {
        let dir = test_dir();
        let image = dir.join("screen.png");
        create_png(&image, 4, 2, [255, 0, 0, 255]);
        let groups = vec![VideoFrameGroup {
            images: vec![FrameImage {
                path: image,
                screen_index: None,
                geometry: None,
            }],
        }];
        let logger = test_logger(&dir);
        let mut plan = plan_frame_sequence(&groups, &logger).expect("plan frames");
        plan.output_size.width += 2;

        assert_eq!(encoded_pipe_codec_for_plan(&plan), None);
    }

    #[test]
    fn stream_frame_sequence_skips_bad_images_without_writing_temp_frames() {
        let dir = test_dir();
        let good = dir.join("good.png");
        let bad = dir.join("bad.png");
        create_png(&good, 2, 2, [255, 0, 0, 255]);
        fs::write(&bad, b"not an image").expect("write bad image");
        let groups = vec![
            VideoFrameGroup {
                images: vec![FrameImage {
                    path: bad.clone(),
                    screen_index: None,
                    geometry: None,
                }],
            },
            VideoFrameGroup {
                images: vec![FrameImage {
                    path: good,
                    screen_index: None,
                    geometry: None,
                }],
            },
        ];
        let logger = test_logger(&dir);

        let plan = plan_frame_sequence(&groups, &logger).expect("plan");
        let mut bytes = Vec::new();
        let progress = std::cell::RefCell::new(Vec::new());
        let cancel_token = VideoGenerationCancelToken::new();
        let report = stream_frame_sequence_to_writer(
            plan,
            &mut bytes,
            &logger,
            &|event| progress.borrow_mut().push(event),
            &cancel_token,
        )
        .expect("frames");

        assert_eq!(report.frame_count, 1);
        assert_eq!(report.skipped_images, vec![bad]);
        assert_eq!(bytes.len(), 2 * 2 * 3);
        assert!(!dir.join("frame_000000.png").exists());
        assert_eq!(
            progress.borrow().as_slice(),
            &[VideoGenerationProgress::PreparingFrames {
                current: 1,
                total: 1
            }]
        );
    }

    #[test]
    fn transparent_png_is_composited_to_background() {
        let transparent = DynamicImage::ImageRgba8(ImageBuffer::<Rgba<u8>, Vec<u8>>::from_pixel(
            1,
            1,
            Rgba([255, 0, 0, 0]),
        ));

        let rgb = rgba_to_rgb_on_background(transparent);

        assert_eq!(*rgb.get_pixel(0, 0), BACKGROUND);
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

    fn group_paths(groups: &[VideoFrameGroup]) -> Vec<Vec<PathBuf>> {
        groups
            .iter()
            .map(|group| {
                group
                    .images
                    .iter()
                    .map(|image| image.path.clone())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn test_logger(root: &Path) -> Logger {
        let paths = AppPaths {
            root: root.to_path_buf(),
            config: root.join("config.json"),
            screenshots: root.join("screenshots"),
            videos: root.join("videos"),
        };
        Logger::new(&paths).expect("create logger")
    }
}
