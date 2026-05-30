use crate::{
    app::AppResult,
    config::{ScreenshotFormat, VideoCodec},
    logging::Logger,
    paths::AppPaths,
    screenshot_naming::{
        multi_screen_metadata_file_name, screenshot_file_name, MultiScreenCaptureMetadata,
        ScreenCaptureMetadata,
    },
    video::generate_video_from_dir,
};
use chrono::{Local, SecondsFormat};
use image::{Rgb, RgbImage, Rgba, RgbaImage};
use std::{fs, path::Path};

pub(crate) fn run() -> AppResult<()> {
    let paths = AppPaths::new()?;
    let logger = Logger::new(&paths)?;
    let label = format!(
        "_simulated-multiscreen-{}",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let input_dir = paths.screenshots.join(&label);
    fs::create_dir_all(&input_dir)?;

    create_virtual_multiscreen_captures(&input_dir)?;
    fs::write(input_dir.join("broken.png"), b"not a real image")?;

    let output = paths.videos.join(format!("{label}.mp4"));
    let report = generate_video_from_dir(
        &input_dir,
        &output,
        2,
        ScreenshotFormat::Png,
        VideoCodec::H264,
        &logger,
    )?;

    println!("模拟多屏截图目录: {}", input_dir.display());
    println!("模拟视频输出: {}", report.output.display());
    println!("生成帧数: {}", report.frame_count);
    println!("跳过坏图: {}", report.skipped_images.len());
    for path in report.skipped_images {
        println!("  - {}", path.display());
    }

    Ok(())
}

fn create_virtual_multiscreen_captures(input_dir: &Path) -> AppResult<()> {
    for sequence in 1..=6_u64 {
        let timestamp = format!("14-03-{:02}.{:03}", 20 + sequence, sequence * 10);
        let captured_at = Local::now().to_rfc3339_opts(SecondsFormat::Millis, false);
        let mut screens = Vec::new();

        let screen_1 = screenshot_file_name(&timestamp, Some(1), sequence, ScreenshotFormat::Png);
        create_rgb_screen(
            &input_dir.join(&screen_1),
            1280,
            720,
            [42, 113, 204],
            sequence,
            1,
        )?;
        screens.push(ScreenCaptureMetadata {
            screen_index: 1,
            file: screen_1,
            x: Some(0),
            y: Some(0),
            width: 1280,
            height: 720,
            scale_factor: Some(1.0),
        });

        if sequence != 4 {
            let screen_2 =
                screenshot_file_name(&timestamp, Some(2), sequence, ScreenshotFormat::Png);
            create_rgba_screen(
                &input_dir.join(&screen_2),
                900,
                600,
                [213, 83, 74],
                sequence,
                2,
            )?;
            screens.push(ScreenCaptureMetadata {
                screen_index: 2,
                file: screen_2,
                x: Some(1280),
                y: Some(60),
                width: 900,
                height: 600,
                scale_factor: Some(1.0),
            });
        }

        let metadata = MultiScreenCaptureMetadata {
            captured_at,
            sequence,
            screens,
        };
        let metadata_path = input_dir.join(multi_screen_metadata_file_name(&timestamp, sequence));
        fs::write(metadata_path, serde_json::to_vec_pretty(&metadata)?)?;
    }

    Ok(())
}

fn create_rgb_screen(
    path: &Path,
    width: u32,
    height: u32,
    color: [u8; 3],
    sequence: u64,
    screen_index: u32,
) -> AppResult<()> {
    let mut image = RgbImage::from_pixel(width, height, Rgb(color));
    paint_grid_rgb(&mut image, [255, 255, 255]);
    let offset = (sequence as u32 * 97) % width.saturating_sub(160).max(1);
    paint_rect_rgb(
        &mut image,
        offset,
        80 + screen_index * 30,
        160,
        120,
        [245, 202, 92],
    );
    image.save(path)?;
    Ok(())
}

fn create_rgba_screen(
    path: &Path,
    width: u32,
    height: u32,
    color: [u8; 3],
    sequence: u64,
    screen_index: u32,
) -> AppResult<()> {
    let mut image = RgbaImage::from_pixel(width, height, Rgba([color[0], color[1], color[2], 220]));
    paint_grid_rgba(&mut image, [255, 255, 255, 180]);
    let offset = (sequence as u32 * 71) % height.saturating_sub(140).max(1);
    paint_rect_rgba(
        &mut image,
        100 + screen_index * 20,
        offset,
        220,
        140,
        [86, 185, 132, 190],
    );
    image.save(path)?;
    Ok(())
}

fn paint_grid_rgb(image: &mut RgbImage, color: [u8; 3]) {
    for x in (0..image.width()).step_by(120) {
        for y in 0..image.height() {
            image.put_pixel(x, y, Rgb(color));
        }
    }
    for y in (0..image.height()).step_by(120) {
        for x in 0..image.width() {
            image.put_pixel(x, y, Rgb(color));
        }
    }
}

fn paint_grid_rgba(image: &mut RgbaImage, color: [u8; 4]) {
    for x in (0..image.width()).step_by(100) {
        for y in 0..image.height() {
            image.put_pixel(x, y, Rgba(color));
        }
    }
    for y in (0..image.height()).step_by(100) {
        for x in 0..image.width() {
            image.put_pixel(x, y, Rgba(color));
        }
    }
}

fn paint_rect_rgb(image: &mut RgbImage, x: u32, y: u32, width: u32, height: u32, color: [u8; 3]) {
    for target_y in y..(y + height).min(image.height()) {
        for target_x in x..(x + width).min(image.width()) {
            image.put_pixel(target_x, target_y, Rgb(color));
        }
    }
}

fn paint_rect_rgba(image: &mut RgbaImage, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
    for target_y in y..(y + height).min(image.height()) {
        for target_x in x..(x + width).min(image.width()) {
            image.put_pixel(target_x, target_y, Rgba(color));
        }
    }
}
