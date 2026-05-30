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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

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

    fn available_generation_modes_for_selection(&self) -> Vec<VideoGenerationMode> {
        let mut selected = self.sources.iter().filter(|source| source.selected);
        let Some(first) = selected.next() else {
            return Vec::new();
        };

        let mut modes = first.available_generation_modes();
        for source in selected {
            let source_modes = source.available_generation_modes();
            modes.retain(|mode| source_modes.contains(mode));
        }
        modes
    }

    fn reconcile_generation_mode_with_selection(&mut self) {
        let modes = self.available_generation_modes_for_selection();
        if let Some(mode) = modes.first().copied() {
            if !modes.contains(&self.generation_mode) {
                self.generation_mode = mode;
            }
        }
        self.apply_generation_mode();
    }

    fn set_generation_mode(&mut self, mode: VideoGenerationMode) {
        if self.generation_mode == mode {
            return;
        }
        self.generation_mode = mode;
        self.apply_generation_mode();
    }

    fn ensure_generation_mode_available(&mut self) {
        self.reconcile_generation_mode_with_selection();
    }

    fn apply_generation_mode(&mut self) {
        let mode = self.generation_mode;
        for source in &mut self.sources {
            if !matches!(source.status, SourceStatus::Generating { .. }) {
                source.status = source.status_for_mode(mode);
            }
        }
    }
}

impl eframe::App for HistoryApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        if !self.generating {
            self.reconcile_generation_mode_with_selection();
        }
        if self.generating {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let text = self.text();
        let selected_count = self.sources.iter().filter(|source| source.selected).count();
        let available_modes = self.available_generation_modes_for_selection();
        let can_generate = !self.generating
            && !available_modes.is_empty()
            && self.sources.iter().any(|source| {
                source.selected && source.can_generate_for_mode(self.generation_mode)
            });

        egui::TopBottomPanel::top("history_toolbar")
            .frame(
                egui::Frame::none()
                    .fill(history_bg())
                    .inner_margin(egui::Margin::symmetric(24.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading(
                            egui::RichText::new(text.history_videos())
                                .size(24.0)
                                .color(history_text()),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new(history_subtitle(self.config.language))
                                .color(history_muted()),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        if ui
                            .add_sized([136.0, 42.0], egui::Button::new(text.open_videos_folder()))
                            .clicked()
                        {
                            if let Err(error) = platform::open_path(&self.paths.videos) {
                                self.status_message = error.to_string();
                            }
                        }
                        if add_enabled_sized(
                            ui,
                            !self.generating,
                            [126.0, 42.0],
                            egui::Button::new(text.add_folder()),
                        )
                        .clicked()
                        {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.add_external_folder(folder);
                            }
                        }
                        if add_enabled_sized(
                            ui,
                            !self.generating,
                            [96.0, 42.0],
                            egui::Button::new(text.refresh()),
                        )
                        .clicked()
                        {
                            self.refresh_sources();
                        }
                    });
                });

                ui.add_space(14.0);
                egui::Frame::none()
                    .fill(history_card())
                    .stroke(egui::Stroke::new(1.0, history_border()))
                    .rounding(egui::Rounding::same(12.0))
                    .inner_margin(egui::Margin::symmetric(18.0, 14.0))
                    .show(ui, |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), HISTORY_MODE_ROW_HEIGHT),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.add_sized(
                                    [92.0, HISTORY_MODE_ROW_HEIGHT],
                                    egui::Label::new(
                                        egui::RichText::new(text.generation_mode())
                                            .strong()
                                            .color(history_muted()),
                                    )
                                    .halign(egui::Align::Min),
                                );
                                ui.add_space(14.0);
                                if available_modes.is_empty() {
                                    let label = if selected_count == 0 {
                                        text.no_selection()
                                    } else {
                                        text.current_mode_unavailable()
                                    };
                                    ui.add_sized(
                                        [112.0, HISTORY_MODE_ROW_HEIGHT],
                                        egui::Label::new(
                                            egui::RichText::new(label).color(history_muted()),
                                        )
                                        .halign(egui::Align::Center),
                                    );
                                } else {
                                    for mode in &available_modes {
                                        if mode_button(
                                            ui,
                                            &generation_mode_label(&text, *mode),
                                            *mode == self.generation_mode,
                                            !self.generating,
                                        )
                                        .clicked()
                                        {
                                            self.set_generation_mode(*mode);
                                        }
                                    }
                                }

                                ui.add_space(18.0);
                                show_history_mode_summary(
                                    ui,
                                    text.selected_count(selected_count),
                                    total_sources_label(self.config.language, self.sources.len()),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if add_enabled_sized(
                                            ui,
                                            can_generate,
                                            [160.0, HISTORY_MODE_ROW_HEIGHT],
                                            egui::Button::new(
                                                egui::RichText::new(text.generate_selected())
                                                    .strong()
                                                    .color(egui::Color32::WHITE),
                                            )
                                            .fill(history_blue()),
                                        )
                                        .clicked()
                                        {
                                            self.start_generation(ctx.clone());
                                        }
                                    },
                                );
                            },
                        );
                    });
                ui.add_space(12.0);
            });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(ready_label(self.config.language)).color(history_muted()),
                );
                if !self.status_message.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.status_message).color(history_muted()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(text.selected_count(selected_count))
                            .color(history_muted()),
                    );
                });
            });
        });

        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(340.0)
            .frame(egui::Frame::none().fill(history_bg()))
            .show(ctx, |ui| {
                ui.add_space(8.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Frame::none()
                        .fill(history_card())
                        .stroke(egui::Stroke::new(1.0, history_border()))
                        .rounding(egui::Rounding::same(12.0))
                        .inner_margin(egui::Margin::same(18.0))
                        .show(ui, |ui| {
                            show_details_panel(ui, self, &text, selected_count);
                        });
                    ui.add_space(32.0);
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            let mut selection_changed = false;
            egui::Frame::none()
                .fill(history_card())
                .stroke(egui::Stroke::new(1.0, history_border()))
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(egui::Margin::same(16.0))
                .show(ui, |ui| {
                    ui.heading(
                        egui::RichText::new(folder_list_title(self.config.language))
                            .size(22.0)
                            .color(history_text()),
                    );
                    ui.label(
                        egui::RichText::new(folder_list_hint(self.config.language))
                            .color(history_muted()),
                    );
                    ui.add_space(16.0);
                    let source_scroll_style = source_scroll_style();
                    let table_layout = SourceTableLayout::new(
                        (ui.available_width() - source_scroll_style.allocated_width()).max(1.0),
                    );
                    show_source_header(ui, &text, table_layout);
                    ui.add_space(SOURCE_ROW_GAP);
                    ui.scope(|ui| {
                        ui.style_mut().spacing.scroll = source_scroll_style;
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            for source in &mut self.sources {
                                let changed = show_source_card(
                                    ui,
                                    source,
                                    table_layout,
                                    self.generation_mode,
                                    self.config.language,
                                    !self.generating,
                                    &text,
                                );
                                selection_changed |= changed;
                                ui.add_space(SOURCE_ROW_GAP);
                            }
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(loaded_sources_label(
                                    self.config.language,
                                    self.sources.len(),
                                ))
                                .color(history_muted()),
                            );
                            ui.add_space(28.0);
                        });
                    });
                });
            if selection_changed {
                self.reconcile_generation_mode_with_selection();
                ctx.request_repaint();
            }
        });

        self.show_generation_dialog(ctx);
    }
}

fn history_bg() -> egui::Color32 {
    egui::Color32::from_rgb(247, 248, 250)
}

fn history_card() -> egui::Color32 {
    egui::Color32::WHITE
}

fn history_card_selected() -> egui::Color32 {
    egui::Color32::from_rgb(244, 248, 255)
}

fn history_border() -> egui::Color32 {
    egui::Color32::from_rgb(218, 222, 228)
}

fn history_blue() -> egui::Color32 {
    egui::Color32::from_rgb(38, 111, 255)
}

fn history_blue_soft() -> egui::Color32 {
    egui::Color32::from_rgb(232, 240, 255)
}

fn history_green() -> egui::Color32 {
    egui::Color32::from_rgb(28, 155, 93)
}

fn history_green_soft() -> egui::Color32 {
    egui::Color32::from_rgb(222, 246, 234)
}

fn history_text() -> egui::Color32 {
    egui::Color32::from_rgb(32, 38, 46)
}

fn history_muted() -> egui::Color32 {
    egui::Color32::from_rgb(101, 112, 126)
}

const HISTORY_MODE_ROW_HEIGHT: f32 = 44.0;
const HISTORY_MODE_SUMMARY_WIDTH: f32 = 150.0;
const HISTORY_MODE_SUMMARY_CONTENT_HEIGHT: f32 = 36.0;

fn show_history_mode_summary(ui: &mut egui::Ui, selected_label: String, total_label: String) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(HISTORY_MODE_SUMMARY_WIDTH, HISTORY_MODE_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    let top = rect.center().y - HISTORY_MODE_SUMMARY_CONTENT_HEIGHT / 2.0;
    let selected_rect =
        egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(rect.width(), 20.0));
    let total_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), top + 20.0),
        egui::vec2(rect.width(), 16.0),
    );

    ui.put(
        selected_rect,
        egui::Label::new(
            egui::RichText::new(&selected_label)
                .strong()
                .color(history_text()),
        )
        .truncate()
        .halign(egui::Align::Min),
    )
    .on_hover_text(&selected_label);
    ui.put(
        total_rect,
        egui::Label::new(
            egui::RichText::new(&total_label)
                .small()
                .color(history_muted()),
        )
        .truncate()
        .halign(egui::Align::Min),
    )
    .on_hover_text(total_label);
    ui.advance_cursor_after_rect(rect);
}

fn mode_button(ui: &mut egui::Ui, label: &str, selected: bool, enabled: bool) -> egui::Response {
    let fill = if selected {
        egui::Color32::WHITE
    } else {
        egui::Color32::from_rgb(239, 242, 246)
    };
    let stroke = if selected {
        egui::Stroke::new(1.5, history_blue())
    } else {
        egui::Stroke::NONE
    };
    let text_color = if selected {
        history_blue()
    } else {
        history_muted()
    };
    add_enabled_sized(
        ui,
        enabled,
        [112.0, HISTORY_MODE_ROW_HEIGHT],
        egui::Button::new(egui::RichText::new(label).strong().color(text_color))
            .fill(fill)
            .stroke(stroke),
    )
}

fn add_enabled_sized(
    ui: &mut egui::Ui,
    enabled: bool,
    size: [f32; 2],
    widget: impl egui::Widget,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| ui.add_sized(size, widget))
        .inner
}

fn show_details_panel(ui: &mut egui::Ui, app: &mut HistoryApp, text: &Text, selected_count: usize) {
    ui.heading(
        egui::RichText::new(text.selected_folders())
            .size(22.0)
            .color(history_text()),
    );
    ui.add_space(14.0);
    egui::Frame::none()
        .fill(egui::Color32::from_rgb(248, 250, 252))
        .stroke(egui::Stroke::new(1.0, history_border()))
        .rounding(egui::Rounding::same(12.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(selected_count.to_string())
                        .size(26.0)
                        .strong()
                        .color(history_blue()),
                );
                ui.label(
                    egui::RichText::new(selected_summary_label(app.config.language))
                        .strong()
                        .color(history_text()),
                );
            });
            ui.label(
                egui::RichText::new(selection_hint(app.config.language)).color(history_muted()),
            );
        });

    ui.add_space(18.0);
    ui.label(
        egui::RichText::new(text.selected_folders())
            .strong()
            .color(history_text()),
    );
    ui.add_space(8.0);
    let selected = app
        .sources
        .iter()
        .filter(|source| source.selected)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        ui.label(egui::RichText::new(text.no_selection()).color(history_muted()));
    } else {
        for source in selected.iter().take(4) {
            egui::Frame::none()
                .fill(egui::Color32::from_rgb(250, 251, 253))
                .stroke(egui::Stroke::new(1.0, history_border()))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&source.label).color(history_text()))
                            .truncate(),
                    )
                    .on_hover_text(&source.label);
                    let output = format!(
                        "{}: {}",
                        text.output(),
                        source.output_path_for_mode(app.generation_mode).display()
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&output).small().color(history_muted()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(output);
                });
            ui.add_space(6.0);
        }
        if selected.len() > 4 {
            ui.label(
                egui::RichText::new(more_selected_label(app.config.language, selected.len() - 4))
                    .color(history_muted()),
            );
        }
    }

    ui.add_space(18.0);
    ui.label(
        egui::RichText::new(generation_params_label(app.config.language))
            .strong()
            .color(history_text()),
    );
    ui.add_space(8.0);
    egui::Frame::none()
        .fill(egui::Color32::from_rgb(248, 250, 252))
        .stroke(egui::Stroke::new(1.0, history_border()))
        .rounding(egui::Rounding::same(12.0))
        .inner_margin(egui::Margin::same(14.0))
        .show(ui, |ui| {
            detail_row(
                ui,
                text.generation_mode(),
                &generation_mode_label(text, app.generation_mode),
            );
            detail_row(ui, "FPS", &app.config.fps.to_string());
            detail_row(ui, "Codec", app.config.video_codec.config_value());
        });

    ui.add_space(18.0);
    show_output_location_footer(ui, app);
    ui.add_space(14.0);
}

fn show_output_location_footer(ui: &mut egui::Ui, app: &mut HistoryApp) {
    ui.label(
        egui::RichText::new(output_location_label(app.config.language))
            .strong()
            .color(history_text()),
    );
    ui.add_space(8.0);
    egui::Frame::none()
        .fill(egui::Color32::from_rgb(245, 247, 250))
        .stroke(egui::Stroke::new(1.0, history_border()))
        .rounding(egui::Rounding::same(12.0))
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let path = app.paths.videos.display().to_string();
                let button_width = 72.0;
                let path_width = (ui.available_width() - button_width - 12.0).max(48.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(path_width, 30.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&path).small().color(history_muted()),
                            )
                            .truncate(),
                        )
                        .on_hover_text(path);
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(button_width, 30.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button(open_label(app.config.language)).clicked() {
                            if let Err(error) = platform::open_path(&app.paths.videos) {
                                app.status_message = error.to_string();
                            }
                        }
                    },
                );
            });
        });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(history_muted()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).strong().color(history_text()));
        });
    });
}

const SOURCE_LEADING_WIDTH: f32 = 88.0;
const SOURCE_IMAGES_WIDTH: f32 = 72.0;
const SOURCE_VIDEO_WIDTH: f32 = 96.0;
const SOURCE_STATUS_WIDTH: f32 = 96.0;
const SOURCE_COLUMN_GAP: f32 = 10.0;
const SOURCE_ROW_PADDING_X: f32 = 12.0;
const SOURCE_HEADER_HEIGHT: f32 = 44.0;
const SOURCE_ROW_HEIGHT: f32 = 68.0;
const SOURCE_ROW_GAP: f32 = 8.0;
const SOURCE_SCROLLBAR_WIDTH: f32 = 8.0;
const SOURCE_SCROLLBAR_GAP: f32 = 12.0;

#[derive(Clone, Copy)]
struct SourceTableLayout {
    row_width: f32,
    leading: f32,
    name: f32,
    images: f32,
    video: f32,
    status: f32,
    gap: f32,
    padding_x: f32,
}

#[derive(Clone, Copy)]
struct SourceColumnRects {
    leading: egui::Rect,
    name: egui::Rect,
    images: egui::Rect,
    video: egui::Rect,
    status: egui::Rect,
}

fn source_scroll_style() -> egui::style::ScrollStyle {
    let mut style = egui::style::ScrollStyle::solid();
    style.bar_width = SOURCE_SCROLLBAR_WIDTH;
    style.bar_inner_margin = SOURCE_SCROLLBAR_GAP;
    style
}

impl SourceTableLayout {
    fn new(available_width: f32) -> Self {
        let row_width = available_width.max(1.0);
        let content_width = (row_width - SOURCE_ROW_PADDING_X * 2.0).max(1.0);
        let compact = content_width < 500.0;
        let (leading, images, video, status, gap) = if compact {
            (74.0, 52.0, 76.0, 76.0, 8.0)
        } else {
            (
                SOURCE_LEADING_WIDTH,
                SOURCE_IMAGES_WIDTH,
                SOURCE_VIDEO_WIDTH,
                SOURCE_STATUS_WIDTH,
                SOURCE_COLUMN_GAP,
            )
        };
        let fixed_width = leading + images + video + status + gap * 4.0;
        Self {
            row_width,
            leading,
            name: (content_width - fixed_width).max(0.0),
            images,
            video,
            status,
            gap,
            padding_x: SOURCE_ROW_PADDING_X,
        }
    }

    fn column_rects(&self, row_rect: egui::Rect) -> SourceColumnRects {
        let mut x = row_rect.left() + self.padding_x;
        let y = row_rect.top();
        let h = row_rect.height();
        let leading = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.leading, h));
        x = leading.right() + self.gap;
        let name = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.name, h));
        x = name.right() + self.gap;
        let images = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.images, h));
        x = images.right() + self.gap;
        let video = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.video, h));
        x = video.right() + self.gap;
        let status = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.status, h));
        SourceColumnRects {
            leading,
            name,
            images,
            video,
            status,
        }
    }
}

fn show_source_header(ui: &mut egui::Ui, text: &Text, layout: SourceTableLayout) {
    let (row_rect, _) = ui.allocate_exact_size(
        egui::vec2(layout.row_width, SOURCE_HEADER_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(
        row_rect,
        egui::Rounding::same(0.0),
        egui::Color32::from_rgb(248, 250, 252),
    );
    let columns = layout.column_rects(row_rect);
    show_header_label(ui, columns.name, text.date_or_folder(), egui::Align::Min);
    show_header_label(ui, columns.images, text.images(), egui::Align::Center);
    show_header_label(ui, columns.video, text.video(), egui::Align::Center);
    show_header_label(ui, columns.status, text.status(), egui::Align::Center);
    ui.advance_cursor_after_rect(row_rect);
}

fn show_header_label(ui: &mut egui::Ui, rect: egui::Rect, label: &str, align: egui::Align) {
    let rect = rect.shrink2(egui::vec2(4.0, 0.0));
    ui.put(
        rect,
        egui::Label::new(egui::RichText::new(label).size(15.0).color(history_muted()))
            .truncate()
            .halign(align),
    )
    .on_hover_text(label);
}

fn show_source_card(
    ui: &mut egui::Ui,
    source: &mut VideoSource,
    layout: SourceTableLayout,
    mode: VideoGenerationMode,
    language: Language,
    enabled: bool,
    text: &Text,
) -> bool {
    let before = source.selected;
    let can_select = enabled && !source.available_generation_modes().is_empty();
    let fill = if source.selected {
        history_card_selected()
    } else {
        history_card()
    };
    let stroke = if source.selected {
        egui::Stroke::new(1.4, egui::Color32::from_rgb(172, 196, 255))
    } else {
        egui::Stroke::new(1.0, history_border())
    };
    let (row_rect, _) = ui.allocate_exact_size(
        egui::vec2(layout.row_width, SOURCE_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter()
        .rect(row_rect, egui::Rounding::same(12.0), fill, stroke);
    let columns = layout.column_rects(row_rect);

    let checkbox_rect = egui::Rect::from_center_size(
        egui::pos2(columns.leading.left() + 22.0, row_rect.center().y),
        egui::vec2(24.0, 24.0),
    );
    ui.add_enabled_ui(can_select, |ui| {
        ui.put(checkbox_rect, egui::Checkbox::new(&mut source.selected, ""));
    });
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(columns.leading.left() + 58.0, row_rect.center().y),
        egui::vec2(34.0, 28.0),
    );
    paint_folder_icon(ui, icon_rect);

    let name_rect = egui::Rect::from_min_max(
        egui::pos2(columns.name.left(), row_rect.top() + 10.0),
        egui::pos2(columns.name.right(), row_rect.top() + 34.0),
    );
    ui.put(
        name_rect,
        egui::Label::new(
            egui::RichText::new(&source.label)
                .strong()
                .color(history_text()),
        )
        .truncate(),
    )
    .on_hover_text(&source.label);
    let subtitle_rect = egui::Rect::from_min_max(
        egui::pos2(columns.name.left(), row_rect.top() + 36.0),
        egui::pos2(columns.name.right(), row_rect.top() + 56.0),
    );
    ui.put(
        subtitle_rect,
        egui::Label::new(
            egui::RichText::new(source_origin_label(language, source.external))
                .small()
                .color(history_muted()),
        )
        .truncate(),
    );

    paint_centered_text(ui, columns.images, &source.image_count.to_string());
    let video_label = if source.output_path_for_mode(mode).exists() {
        text.exists()
    } else {
        text.missing()
    };
    paint_chip(
        ui,
        columns.video,
        video_label,
        history_blue_soft(),
        history_blue(),
    );
    paint_chip(
        ui,
        columns.status,
        &source.status_label(language),
        history_green_soft(),
        history_green(),
    );

    let changed = source.selected != before;
    ui.advance_cursor_after_rect(row_rect);
    changed
}

fn paint_centered_text(ui: &egui::Ui, rect: egui::Rect, value: &str) {
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        value,
        egui::FontId::proportional(16.0),
        history_text(),
    );
}

fn paint_chip(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    label: &str,
    fill: egui::Color32,
    color: egui::Color32,
) {
    let chip_rect = egui::Rect::from_center_size(
        rect.center(),
        egui::vec2((rect.width() - 12.0).max(36.0), 34.0),
    );
    ui.painter()
        .rect_filled(chip_rect, egui::Rounding::same(12.0), fill);
    ui.put(
        chip_rect.shrink2(egui::vec2(8.0, 0.0)),
        egui::Label::new(egui::RichText::new(label).small().color(color)).truncate(),
    )
    .on_hover_text(label);
}

fn paint_folder_icon(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let tab = egui::Rect::from_min_size(rect.min + egui::vec2(3.0, 3.0), egui::vec2(17.0, 9.0));
    let body = egui::Rect::from_min_size(rect.min + egui::vec2(2.0, 9.0), egui::vec2(30.0, 18.0));
    painter.rect_filled(
        tab,
        egui::Rounding::same(4.0),
        egui::Color32::from_rgb(255, 221, 130),
    );
    painter.rect_filled(
        body,
        egui::Rounding::same(5.0),
        egui::Color32::from_rgb(255, 205, 96),
    );
}

fn history_subtitle(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "按日期和文件夹管理截图序列，选择后生成视频。",
        Language::En => {
            "Manage screenshot folders by date, then generate videos from the selection."
        }
    }
}

fn folder_list_title(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "文件夹",
        Language::En => "Folders",
    }
}

fn folder_list_hint(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "最近截图序列优先显示，可多选批量生成。",
        Language::En => "Recent screenshot folders appear first and can be selected in batches.",
    }
}

fn total_sources_label(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("共 {count} 个文件夹"),
        Language::En => format!("{count} folders total"),
    }
}

fn loaded_sources_label(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("状态：已加载 {count} 个文件夹"),
        Language::En => format!("Status: {count} folders loaded"),
    }
}

fn ready_label(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "就绪",
        Language::En => "Ready",
    }
}

fn selected_summary_label(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "个文件夹已选中",
        Language::En => "folders selected",
    }
}

fn selection_hint(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "将按当前生成逻辑输出到视频目录。",
        Language::En => "Videos will be written to the configured video folder.",
    }
}

fn generation_params_label(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "生成参数",
        Language::En => "Generation Settings",
    }
}

fn output_location_label(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "输出位置",
        Language::En => "Output Location",
    }
}

fn open_label(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "打开",
        Language::En => "Open",
    }
}

fn source_origin_label(language: Language, external: bool) -> &'static str {
    match (language, external) {
        (Language::ZhCn, true) => "外部文件夹",
        (Language::ZhCn, false) => "默认保存目录",
        (Language::En, true) => "External folder",
        (Language::En, false) => "Default folder",
    }
}

fn more_selected_label(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("另有 {count} 项已选中"),
        Language::En => format!("{count} more selected"),
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
    fn available_generation_modes(&self) -> Vec<VideoGenerationMode> {
        if self.image_count == 0 {
            return Vec::new();
        }

        let mut screen_indices = self.screen_indices.clone();
        if self.single_compatible_count > 0 {
            screen_indices.insert(1);
        }

        let mut modes = Vec::new();
        if self.screen_indices.len() > 1 {
            modes.push(VideoGenerationMode::MultiScreen);
        }
        modes.extend(
            screen_indices
                .into_iter()
                .map(VideoGenerationMode::SingleScreen),
        );
        modes
    }

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
        source_with_stats(id, screens, 0, selected)
    }

    fn source_with_stats(
        id: u64,
        screens: &[u32],
        single_compatible_count: usize,
        selected: bool,
    ) -> VideoSource {
        VideoSource {
            id,
            label: format!("source-{id}"),
            input_dir: PathBuf::from(format!("input-{id}")),
            output_path: PathBuf::from(format!("video-{id}.mp4")),
            image_count: screens.len() + single_compatible_count,
            single_compatible_count,
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
    fn single_folder_with_one_screen_returns_only_that_screen_mode() {
        let source = source_with_screens(1, &[1], true);

        assert_eq!(
            source.available_generation_modes(),
            vec![VideoGenerationMode::SingleScreen(1)]
        );
    }

    #[test]
    fn single_folder_with_multiple_screens_returns_multi_and_screen_modes() {
        let source = source_with_screens(1, &[1, 2], true);

        assert_eq!(
            source.available_generation_modes(),
            vec![
                VideoGenerationMode::MultiScreen,
                VideoGenerationMode::SingleScreen(1),
                VideoGenerationMode::SingleScreen(2)
            ]
        );
    }

    #[test]
    fn legacy_single_images_are_exposed_as_screen_one() {
        let source = source_with_stats(1, &[], 3, true);

        assert_eq!(
            source.available_generation_modes(),
            vec![VideoGenerationMode::SingleScreen(1)]
        );
    }

    #[test]
    fn multiple_selected_folders_return_common_generation_modes() {
        let root = test_dir();
        let paths = test_paths(&root);
        let logger = Logger::new(&paths).expect("create logger");
        let mut app = HistoryApp::new(paths, Config::default(), logger);
        app.sources = vec![
            source_with_screens(1, &[1, 2], true),
            source_with_screens(2, &[2, 3], true),
        ];

        assert_eq!(
            app.available_generation_modes_for_selection(),
            vec![
                VideoGenerationMode::MultiScreen,
                VideoGenerationMode::SingleScreen(2)
            ]
        );
    }

    #[test]
    fn reconcile_generation_mode_switches_invalid_mode_to_first_common_mode() {
        let root = test_dir();
        let paths = test_paths(&root);
        let logger = Logger::new(&paths).expect("create logger");
        let mut app = HistoryApp::new(paths, Config::default(), logger);
        app.generation_mode = VideoGenerationMode::MultiScreen;
        app.sources = vec![source_with_screens(1, &[1], true)];

        app.reconcile_generation_mode_with_selection();

        assert!(app.sources[0].selected);
        assert_eq!(app.generation_mode, VideoGenerationMode::SingleScreen(1));
    }

    #[test]
    fn incompatible_multi_selection_keeps_selection_and_returns_no_modes() {
        let root = test_dir();
        let paths = test_paths(&root);
        let logger = Logger::new(&paths).expect("create logger");
        let mut app = HistoryApp::new(paths, Config::default(), logger);
        app.sources = vec![
            source_with_screens(1, &[1], true),
            source_with_screens(2, &[2], true),
        ];

        assert!(app.available_generation_modes_for_selection().is_empty());
        app.reconcile_generation_mode_with_selection();

        assert!(app.sources[0].selected);
        assert!(app.sources[1].selected);
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
