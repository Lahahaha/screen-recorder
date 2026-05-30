use crate::{
    app::AppResult,
    config::{load_config, Config, Language},
    i18n::Text,
    logging::Logger,
    paths::AppPaths,
    platform,
    screenshot_naming::{parse_screenshot_file_name, screenshot_format_for_path, ScreenshotName},
    video::{
        generate_video_from_dir_with_mode_and_control, VideoGenerationCancelToken,
        VideoGenerationMode, VideoGenerationOptions, VideoGenerationProgress,
        VideoGenerationReport,
    },
};
use chrono::Local;
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::{
    collections::BTreeSet,
    fs, io,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

pub(crate) fn run() -> AppResult<()> {
    let paths = AppPaths::new()?;
    let logger = Logger::new(&paths)?;
    let config = load_config(&paths, &logger)?;
    let language = config.language;
    let title = Text::new(language).history_videos();
    let options = eframe::NativeOptions::default();

    eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            install_history_fonts(&cc.egui_ctx, &logger);
            Ok(Box::new(HistoryApp::new(paths, config, logger)))
        }),
    )
    .map_err(|error| io::Error::other(error.to_string()).into())
}

fn install_history_fonts(ctx: &egui::Context, logger: &Logger) {
    let Some(font_path) = cjk_font_candidates().into_iter().find(|path| path.exists()) else {
        logger.warn("未找到可用的系统 CJK 字体，历史窗口将使用 egui 默认字体");
        return;
    };

    let font_data = match fs::read(&font_path) {
        Ok(data) => data,
        Err(error) => {
            logger.warn(format!(
                "读取系统 CJK 字体失败 {}: {error}",
                font_path.display()
            ));
            return;
        }
    };

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("system_cjk".to_string(), FontData::from_owned(font_data));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "system_cjk".to_string());
    }
    ctx.set_fonts(fonts);
    logger.info(format!(
        "历史窗口已加载系统 CJK 字体: {}",
        font_path.display()
    ));
}

fn cjk_font_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(target_os = "macos")]
    {
        candidates.extend([
            PathBuf::from("/System/Library/Fonts/STHeiti Medium.ttc"),
            PathBuf::from("/Library/Fonts/Arial Unicode.ttf"),
            PathBuf::from("/System/Library/Fonts/Supplemental/Songti.ttc"),
        ]);
    }
    #[cfg(target_os = "windows")]
    {
        candidates.extend([
            PathBuf::from(r"C:\Windows\Fonts\msyh.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\simhei.ttf"),
            PathBuf::from(r"C:\Windows\Fonts\simsun.ttc"),
        ]);
    }
    candidates
}

struct HistoryApp {
    paths: AppPaths,
    config: Config,
    logger: Logger,
    sources: Vec<VideoSource>,
    generation_mode: VideoGenerationMode,
    next_id: u64,
    generating: bool,
    cancel_token: Option<VideoGenerationCancelToken>,
    cancel_requested: bool,
    receiver: Option<mpsc::Receiver<GenerateEvent>>,
    status_message: String,
}

#[derive(Clone)]
struct VideoSource {
    id: u64,
    label: String,
    input_dir: PathBuf,
    output_path: PathBuf,
    image_count: usize,
    single_compatible_count: usize,
    screen_indices: BTreeSet<u32>,
    selected: bool,
    external: bool,
    status: SourceStatus,
}

#[derive(Clone)]
enum SourceStatus {
    Ready,
    Unavailable,
    ModeUnavailable,
    Generating {
        progress: Option<VideoGenerationProgress>,
    },
    Done {
        frame_count: usize,
        skipped: usize,
    },
    Failed(String),
}

struct VideoJob {
    id: u64,
    input_dir: PathBuf,
    output_path: PathBuf,
    mode: VideoGenerationMode,
}

enum GenerateEvent {
    Started(u64),
    Progress(u64, VideoGenerationProgress),
    Finished(u64, Result<VideoGenerationReport, String>),
    AllDone,
}

impl HistoryApp {
    fn new(paths: AppPaths, config: Config, logger: Logger) -> Self {
        let mut app = Self {
            paths,
            config,
            logger,
            sources: Vec::new(),
            generation_mode: VideoGenerationMode::MultiScreen,
            next_id: 1,
            generating: false,
            cancel_token: None,
            cancel_requested: false,
            receiver: None,
            status_message: String::new(),
        };
        app.refresh_sources();
        app
    }

    fn text(&self) -> Text {
        Text::new(self.config.language)
    }

    fn refresh_sources(&mut self) {
        self.sources.clear();
        let mut dirs = fs::read_dir(&self.paths.screenshots)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                    .filter(|path| path.is_dir())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        dirs.sort();

        for dir in dirs {
            let label = dir
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("screenshots")
                .to_string();
            let output_path = self.paths.video_path_for_date(&label);
            self.push_source(label, dir, output_path, false);
        }
        self.ensure_generation_mode_available();
        self.apply_generation_mode();
    }

    fn push_source(
        &mut self,
        label: String,
        input_dir: PathBuf,
        output_path: PathBuf,
        external: bool,
    ) {
        let stats = inspect_video_source(&input_dir).unwrap_or_default();
        self.sources.push(VideoSource {
            id: self.next_id,
            label,
            input_dir,
            output_path,
            image_count: stats.image_count,
            single_compatible_count: stats.single_compatible_count,
            screen_indices: stats.screen_indices,
            selected: false,
            external,
            status: SourceStatus::Ready,
        });
        let index = self.sources.len() - 1;
        self.sources[index].status = self.sources[index].status_for_mode(self.generation_mode);
        self.next_id += 1;
    }

    fn add_external_folder(&mut self, folder: PathBuf) {
        let label = folder
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Imported Folder")
            .to_string();
        let output_path = unique_external_video_path(&self.paths.videos, &label);
        self.push_source(label, folder, output_path, true);
    }

    fn selected_jobs(&self) -> Vec<VideoJob> {
        let mode = self.generation_mode;
        self.sources
            .iter()
            .filter(|source| source.selected && source.can_generate_for_mode(mode))
            .map(|source| VideoJob {
                id: source.id,
                input_dir: source.input_dir.clone(),
                output_path: source.output_path_for_mode(mode),
                mode,
            })
            .collect()
    }

    fn start_generation(&mut self, ctx: egui::Context) {
        let jobs = self.selected_jobs();
        if jobs.is_empty() {
            self.status_message = self.text().no_selection().to_string();
            return;
        }
        self.status_message = self.text().generating().to_string();
        for source in &mut self.sources {
            if jobs.iter().any(|job| job.id == source.id) {
                source.status = SourceStatus::Generating { progress: None };
            }
        }

        let (sender, receiver) = mpsc::channel();
        let config = self.config.clone();
        let logger = self.logger.clone();
        let cancel_token = VideoGenerationCancelToken::new();
        let worker_cancel_token = cancel_token.clone();
        thread::spawn(move || {
            for job in jobs {
                if worker_cancel_token.is_cancelled() {
                    break;
                }
                send_generate_event(&sender, &ctx, GenerateEvent::Started(job.id));
                let progress_sender = sender.clone();
                let progress_ctx = ctx.clone();
                let progress_cancel_token = worker_cancel_token.clone();
                let result = panic::catch_unwind(AssertUnwindSafe(|| {
                    generate_video_from_dir_with_mode_and_control(
                        &job.input_dir,
                        &job.output_path,
                        VideoGenerationOptions {
                            fps: config.fps,
                            video_codec: config.video_codec,
                            mode: job.mode,
                        },
                        &logger,
                        |progress| {
                            send_generate_event(
                                &progress_sender,
                                &progress_ctx,
                                GenerateEvent::Progress(job.id, progress),
                            );
                        },
                        &progress_cancel_token,
                    )
                }))
                .map_err(panic_payload_message)
                .and_then(|result| result.map_err(|error| error.to_string()));
                send_generate_event(&sender, &ctx, GenerateEvent::Finished(job.id, result));
                if worker_cancel_token.is_cancelled() {
                    break;
                }
            }
            send_generate_event(&sender, &ctx, GenerateEvent::AllDone);
        });

        self.generating = true;
        self.cancel_requested = false;
        self.cancel_token = Some(cancel_token);
        self.receiver = Some(receiver);
    }

    fn cancel_generation(&mut self) {
        if let Some(cancel_token) = &self.cancel_token {
            cancel_token.cancel();
            self.cancel_requested = true;
            self.status_message = self.text().cancelling().to_string();
        }
    }

    fn poll_events(&mut self) {
        let Some(receiver) = self.receiver.take() else {
            return;
        };
        let text = self.text();
        let mut keep_receiver = true;
        for event in receiver.try_iter() {
            match event {
                GenerateEvent::Started(id) => {
                    let label = self.source_label(id);
                    if let Some(source) = self.source_mut(id) {
                        source.status = SourceStatus::Generating { progress: None };
                    }
                    self.status_message = format!("{}: {label}", text.generating());
                }
                GenerateEvent::Progress(id, progress) => {
                    let label = self.source_label(id);
                    let progress_message = progress_status_label(&text, &progress);
                    if let Some(source) = self.source_mut(id) {
                        source.status = SourceStatus::Generating {
                            progress: Some(progress),
                        };
                    }
                    self.status_message = format!("{label}: {progress_message}");
                }
                GenerateEvent::Finished(id, result) => {
                    let label = self.source_label(id);
                    let (status, status_message) = match result {
                        Ok(report) => {
                            let frame_count = report.frame_count;
                            let skipped = report.skipped_images.len();
                            (
                                SourceStatus::Done {
                                    frame_count,
                                    skipped,
                                },
                                format!(
                                    "{label}: {}",
                                    text.generation_done_status(frame_count, skipped)
                                ),
                            )
                        }
                        Err(error) => (
                            SourceStatus::Failed(error.clone()),
                            format!("{label}: {}: {error}", text.failed()),
                        ),
                    };
                    if let Some(source) = self.source_mut(id) {
                        source.status = status;
                    }
                    self.status_message = status_message;
                }
                GenerateEvent::AllDone => {
                    self.generating = false;
                    keep_receiver = false;
                    self.cancel_token = None;
                    self.cancel_requested = false;
                    self.status_message = self.batch_status_message();
                }
            }
        }
        if keep_receiver {
            self.receiver = Some(receiver);
        }
    }

    fn source_mut(&mut self, id: u64) -> Option<&mut VideoSource> {
        self.sources.iter_mut().find(|source| source.id == id)
    }

    fn source_label(&self, id: u64) -> String {
        self.sources
            .iter()
            .find(|source| source.id == id)
            .map(|source| source.label.clone())
            .unwrap_or_else(|| id.to_string())
    }

    fn batch_status_message(&self) -> String {
        let text = self.text();
        let done = self
            .sources
            .iter()
            .filter(|source| matches!(source.status, SourceStatus::Done { .. }))
            .count();
        let failed = self
            .sources
            .iter()
            .filter(|source| matches!(source.status, SourceStatus::Failed(_)))
            .count();
        if failed == 0 {
            return format!("{}: {done}", text.done_label());
        }
        format!("{}: {done}, {}: {failed}", text.done_label(), text.failed())
    }

    fn available_screen_indices(&self) -> Vec<u32> {
        let mut indices = BTreeSet::new();
        for source in &self.sources {
            indices.extend(source.screen_indices.iter().copied());
        }
        indices.into_iter().collect()
    }

    fn set_generation_mode(&mut self, mode: VideoGenerationMode) {
        if self.generation_mode == mode {
            return;
        }
        self.generation_mode = mode;
        self.apply_generation_mode();
    }

    fn ensure_generation_mode_available(&mut self) {
        let VideoGenerationMode::SingleScreen(index) = self.generation_mode else {
            return;
        };
        if !self
            .sources
            .iter()
            .any(|source| source.screen_indices.contains(&index))
        {
            self.generation_mode = VideoGenerationMode::MultiScreen;
        }
    }

    fn apply_generation_mode(&mut self) {
        let mode = self.generation_mode;
        for source in &mut self.sources {
            if !source.can_generate_for_mode(mode) {
                source.selected = false;
            }
            if !matches!(source.status, SourceStatus::Generating { .. }) {
                source.status = source.status_for_mode(mode);
            }
        }
    }
}

impl eframe::App for HistoryApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        if self.generating {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let text = self.text();

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(text.history_videos());
                ui.separator();
                if ui
                    .add_enabled(!self.generating, egui::Button::new(text.refresh()))
                    .clicked()
                {
                    self.refresh_sources();
                }
                if ui
                    .add_enabled(!self.generating, egui::Button::new(text.add_folder()))
                    .clicked()
                {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.add_external_folder(folder);
                    }
                }
                ui.separator();
                ui.label(text.generation_mode());
                let mut selected_mode = self.generation_mode;
                let available_screen_indices = self.available_screen_indices();
                ui.add_enabled_ui(!self.generating, |ui| {
                    egui::ComboBox::from_id_salt("history_generation_mode")
                        .selected_text(generation_mode_label(&text, self.generation_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut selected_mode,
                                VideoGenerationMode::MultiScreen,
                                text.multi_screen_generation_mode(),
                            );
                            for screen_index in available_screen_indices {
                                ui.selectable_value(
                                    &mut selected_mode,
                                    VideoGenerationMode::SingleScreen(screen_index),
                                    text.generation_screen_mode(screen_index),
                                );
                            }
                        });
                });
                if !self.generating && selected_mode != self.generation_mode {
                    self.set_generation_mode(selected_mode);
                }
                ui.separator();
                let can_generate = self.sources.iter().any(|source| {
                    source.selected && source.can_generate_for_mode(self.generation_mode)
                });
                if ui
                    .add_enabled(
                        !self.generating && can_generate,
                        egui::Button::new(text.generate_selected()),
                    )
                    .clicked()
                {
                    self.start_generation(ctx.clone());
                }
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let selected = self.sources.iter().filter(|source| source.selected).count();
                ui.label(text.selected_count(selected));
                if !self.status_message.is_empty() {
                    ui.separator();
                    ui.label(&self.status_message);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(text.open_videos_folder()).clicked() {
                        if let Err(error) = platform::open_path(&self.paths.videos) {
                            self.status_message = error.to_string();
                        }
                    }
                });
            });
        });

        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.heading(text.selected_folders());
                let selected = self
                    .sources
                    .iter()
                    .filter(|source| source.selected)
                    .collect::<Vec<_>>();
                if selected.is_empty() {
                    ui.label(text.no_selection());
                } else {
                    for source in selected {
                        ui.separator();
                        ui.label(&source.label);
                        ui.small(format!(
                            "{}: {}",
                            text.output(),
                            source.output_path_for_mode(self.generation_mode).display()
                        ));
                    }
                }
                ui.separator();
                ui.label(format!(
                    "{}: {}",
                    text.generation_mode(),
                    generation_mode_label(&text, self.generation_mode)
                ));
                ui.label(format!("FPS: {}", self.config.fps));
                ui.label(format!("Codec: {}", self.config.video_codec.config_value()));
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("sources")
                    .striped(true)
                    .min_col_width(80.0)
                    .show(ui, |ui| {
                        ui.label("");
                        ui.strong(text.date_or_folder());
                        ui.strong(text.images());
                        ui.strong(text.video());
                        ui.strong(text.status());
                        ui.end_row();

                        for source in &mut self.sources {
                            let enabled = !self.generating
                                && source.can_generate_for_mode(self.generation_mode);
                            ui.add_enabled(enabled, egui::Checkbox::new(&mut source.selected, ""));
                            ui.label(source.display_label());
                            ui.label(source.image_count.to_string());
                            ui.label(
                                if source.output_path_for_mode(self.generation_mode).exists() {
                                    text.exists()
                                } else {
                                    text.missing()
                                },
                            );
                            ui.label(source.status_label(self.config.language));
                            ui.end_row();
                        }
                    });
            });
        });

        self.show_generation_dialog(ctx);
    }
}

impl HistoryApp {
    fn show_generation_dialog(&mut self, ctx: &egui::Context) {
        if !self.generating {
            return;
        }

        let text = self.text();
        let current = self.current_generation_status();
        egui::Window::new(text.video_generating_title())
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(&self.status_message);
                if let Some((label, progress)) = current {
                    ui.separator();
                    ui.label(label);
                    match progress {
                        VideoGenerationProgress::PreparingFrames { current, total } => {
                            let fraction = if total == 0 {
                                0.0
                            } else {
                                current as f32 / total as f32
                            };
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .show_percentage()
                                    .text(text.preparing_frames(current, total)),
                            );
                        }
                        other => {
                            ui.add(
                                egui::ProgressBar::new(0.0)
                                    .animate(true)
                                    .text(progress_status_label(&text, &other)),
                            );
                        }
                    }
                }
                ui.separator();
                let button_text = if self.cancel_requested {
                    text.cancelling()
                } else {
                    text.cancel()
                };
                if ui
                    .add_enabled(!self.cancel_requested, egui::Button::new(button_text))
                    .clicked()
                {
                    self.cancel_generation();
                }
            });
    }

    fn current_generation_status(&self) -> Option<(String, VideoGenerationProgress)> {
        self.sources.iter().find_map(|source| {
            let SourceStatus::Generating {
                progress: Some(progress),
            } = &source.status
            else {
                return None;
            };
            Some((source.label.clone(), progress.clone()))
        })
    }
}

fn send_generate_event(
    sender: &mpsc::Sender<GenerateEvent>,
    ctx: &egui::Context,
    event: GenerateEvent,
) {
    let _ = sender.send(event);
    ctx.request_repaint();
}

fn panic_payload_message(error: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = error.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = error.downcast_ref::<String>() {
        return message.clone();
    }
    "历史视频生成线程异常退出".to_string()
}

fn progress_status_label(text: &Text, progress: &VideoGenerationProgress) -> String {
    match progress {
        VideoGenerationProgress::Scanning => text.scanning().to_string(),
        VideoGenerationProgress::PreparingFrames { current, total } => {
            text.preparing_frames(*current, *total)
        }
        VideoGenerationProgress::Encoding => text.encoding().to_string(),
        VideoGenerationProgress::Replacing => text.finishing().to_string(),
    }
}

impl VideoSource {
    fn can_generate_for_mode(&self, mode: VideoGenerationMode) -> bool {
        self.has_compatible_images(mode)
            && !matches!(
                self.status,
                SourceStatus::Unavailable | SourceStatus::ModeUnavailable
            )
    }

    fn has_compatible_images(&self, mode: VideoGenerationMode) -> bool {
        if self.image_count == 0 {
            return false;
        }
        match mode {
            VideoGenerationMode::MultiScreen => true,
            VideoGenerationMode::SingleScreen(index) => {
                self.single_compatible_count > 0 || self.screen_indices.contains(&index)
            }
        }
    }

    fn status_for_mode(&self, mode: VideoGenerationMode) -> SourceStatus {
        if self.image_count == 0 {
            SourceStatus::Unavailable
        } else if self.has_compatible_images(mode) {
            SourceStatus::Ready
        } else {
            SourceStatus::ModeUnavailable
        }
    }

    fn output_path_for_mode(&self, mode: VideoGenerationMode) -> PathBuf {
        output_path_for_mode(&self.output_path, mode)
    }

    fn display_label(&self) -> String {
        if self.external {
            format!("{} *", self.label)
        } else {
            self.label.clone()
        }
    }

    fn status_label(&self, language: Language) -> String {
        let text = Text::new(language);
        match &self.status {
            SourceStatus::Ready => text.ready().to_string(),
            SourceStatus::Unavailable => {
                format!("{}: {}", text.unavailable(), text.no_available_images())
            }
            SourceStatus::ModeUnavailable => {
                format!(
                    "{}: {}",
                    text.unavailable(),
                    text.current_mode_unavailable()
                )
            }
            SourceStatus::Generating { progress } => {
                let Some(progress) = progress else {
                    return text.generating().to_string();
                };
                format!(
                    "{}: {}",
                    text.generating(),
                    progress_status_label(&text, progress)
                )
            }
            SourceStatus::Done {
                frame_count,
                skipped,
            } => text.generation_done_status(*frame_count, *skipped),
            SourceStatus::Failed(error) => format!("{}: {error}", text.failed()),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct VideoSourceStats {
    image_count: usize,
    single_compatible_count: usize,
    screen_indices: BTreeSet<u32>,
}

fn inspect_video_source(input_dir: &Path) -> AppResult<VideoSourceStats> {
    let mut stats = VideoSourceStats::default();
    for path in fs::read_dir(input_dir)?.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
        if screenshot_format_for_path(&path).is_none() {
            continue;
        }
        stats.image_count += 1;
        match parse_screenshot_file_name(&path) {
            Some(ScreenshotName::MultiScreen { screen_index, .. }) => {
                stats.screen_indices.insert(screen_index);
            }
            Some(ScreenshotName::Single { .. }) | None => {
                stats.single_compatible_count += 1;
            }
        }
    }
    Ok(stats)
}

fn generation_mode_label(text: &Text, mode: VideoGenerationMode) -> String {
    match mode {
        VideoGenerationMode::MultiScreen => text.multi_screen_generation_mode().to_string(),
        VideoGenerationMode::SingleScreen(index) => text.generation_screen_mode(index),
    }
}

fn output_path_for_mode(output_path: &Path, mode: VideoGenerationMode) -> PathBuf {
    let VideoGenerationMode::SingleScreen(index) = mode else {
        return output_path.to_path_buf();
    };
    let stem = output_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("video");
    let extension = output_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("mp4");
    let file_name = format!("{stem}-screen-{index:02}.{extension}");
    output_path
        .parent()
        .map(|parent| parent.join(&file_name))
        .unwrap_or_else(|| PathBuf::from(file_name))
}

fn unique_external_video_path(videos_dir: &Path, label: &str) -> PathBuf {
    let base = sanitize_file_stem(label);
    let candidate = videos_dir.join(format!("{base}.mp4"));
    if !candidate.exists() {
        return candidate;
    }

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    videos_dir.join(format!("{base}-{timestamp}.mp4"))
}

fn sanitize_file_stem(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if sanitized.is_empty() {
        "imported-folder".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let sequence = TEST_DIR_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-history-test-{}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f"),
            sequence
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn test_paths(root: &Path) -> AppPaths {
        let paths = AppPaths {
            root: root.to_path_buf(),
            config: root.join("config.json"),
            screenshots: root.join("screenshots"),
            videos: root.join("videos"),
        };
        fs::create_dir_all(&paths.screenshots).expect("create screenshots");
        fs::create_dir_all(&paths.videos).expect("create videos");
        paths
    }

    fn source_with_screens(id: u64, screens: &[u32], selected: bool) -> VideoSource {
        VideoSource {
            id,
            label: format!("source-{id}"),
            input_dir: PathBuf::from(format!("input-{id}")),
            output_path: PathBuf::from(format!("video-{id}.mp4")),
            image_count: screens.len(),
            single_compatible_count: 0,
            screen_indices: screens.iter().copied().collect(),
            selected,
            external: false,
            status: SourceStatus::Ready,
        }
    }

    #[test]
    fn sanitize_file_stem_replaces_path_unfriendly_characters() {
        assert_eq!(
            sanitize_file_stem("Project Demo Folder"),
            "Project_Demo_Folder"
        );
        assert_eq!(sanitize_file_stem("///"), "imported-folder");
    }

    #[test]
    fn inspect_video_source_finds_screen_indices_and_single_compatible_images() {
        let dir = test_dir();
        fs::write(dir.join("14-03-22.184-screen-02-000512.png"), b"").expect("write");
        fs::write(dir.join("14-03-22.184-screen-01-000512.png"), b"").expect("write");
        fs::write(dir.join("14-03-23.184-000513.png"), b"").expect("write");
        fs::write(dir.join("external.jpg"), b"").expect("write");
        fs::write(dir.join("notes.txt"), b"").expect("write");

        let stats = inspect_video_source(&dir).expect("inspect source");

        assert_eq!(stats.image_count, 4);
        assert_eq!(stats.single_compatible_count, 2);
        assert_eq!(stats.screen_indices, BTreeSet::from([1, 2]));
    }

    #[test]
    fn output_path_for_single_screen_mode_adds_screen_suffix() {
        let output = output_path_for_mode(
            Path::new("/tmp/2026-05-30.mp4"),
            VideoGenerationMode::SingleScreen(2),
        );

        assert_eq!(output, PathBuf::from("/tmp/2026-05-30-screen-02.mp4"));
        assert_eq!(
            output_path_for_mode(
                Path::new("/tmp/2026-05-30.mp4"),
                VideoGenerationMode::MultiScreen
            ),
            PathBuf::from("/tmp/2026-05-30.mp4")
        );
    }

    #[test]
    fn generation_mode_change_clears_incompatible_selected_sources() {
        let root = test_dir();
        let paths = test_paths(&root);
        let logger = Logger::new(&paths).expect("create logger");
        let mut app = HistoryApp::new(paths, Config::default(), logger);
        app.sources = vec![
            source_with_screens(1, &[1], true),
            source_with_screens(2, &[2], true),
        ];

        app.set_generation_mode(VideoGenerationMode::SingleScreen(1));

        assert!(app.sources[0].selected);
        assert!(app.sources[0].can_generate_for_mode(app.generation_mode));
        assert!(!app.sources[1].selected);
        assert!(matches!(
            app.sources[1].status,
            SourceStatus::ModeUnavailable
        ));
    }

    #[test]
    fn selected_jobs_use_current_generation_mode_and_output_path() {
        let root = test_dir();
        let paths = test_paths(&root);
        let logger = Logger::new(&paths).expect("create logger");
        let mut app = HistoryApp::new(paths, Config::default(), logger);
        app.generation_mode = VideoGenerationMode::SingleScreen(2);
        app.sources = vec![source_with_screens(7, &[2], true)];

        let jobs = app.selected_jobs();

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].mode, VideoGenerationMode::SingleScreen(2));
        assert_eq!(jobs[0].output_path, PathBuf::from("video-7-screen-02.mp4"));
    }
}
