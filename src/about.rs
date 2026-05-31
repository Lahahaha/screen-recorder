use crate::{
    app::AppResult,
    config::load_config,
    i18n::{Text, APP_NAME},
    logging::Logger,
    paths::{display_path, AppPaths},
    platform,
};
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::{fs, io, path::PathBuf, sync::Arc};

const APP_HOMEPAGE: &str = "https://github.com/Lahahaha/screen-recorder";
const APP_AUTHOR: &str = "Lahahaha";
const APP_LICENSE: &str = "MIT License";
const APP_COPYRIGHT: &str = "Copyright (c) 2026 Lahahaha";

pub(crate) fn run(workdir: Option<PathBuf>) -> AppResult<()> {
    let paths = match workdir {
        Some(root) => AppPaths::from_root(root)?,
        None => AppPaths::new()?,
    };
    let logger = Logger::new(&paths)?;
    let config = load_config(&paths, &logger)?;
    let text = Text::new(config.language);
    let title = text.about_window_title();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([520.0, 360.0])
            .with_min_inner_size([440.0, 320.0])
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        title,
        options,
        Box::new(move |cc| {
            install_about_fonts(&cc.egui_ctx, &logger);
            Ok(Box::new(AboutApp::new(paths, config.language)))
        }),
    )
    .map_err(|error| io::Error::other(error.to_string()).into())
}

fn install_about_fonts(ctx: &egui::Context, logger: &Logger) {
    let Some(font_path) = cjk_font_candidates().into_iter().find(|path| path.exists()) else {
        logger.warn("未找到可用的系统 CJK 字体，关于窗口将使用 egui 默认字体");
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
    fonts.font_data.insert(
        "system_cjk".to_string(),
        Arc::new(FontData::from_owned(font_data)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "system_cjk".to_string());
    }
    ctx.set_fonts(fonts);
    logger.info(format!(
        "关于窗口已加载系统 CJK 字体: {}",
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

struct AboutApp {
    paths: AppPaths,
    language: crate::config::Language,
    status_message: String,
}

impl AboutApp {
    fn new(paths: AppPaths, language: crate::config::Language) -> Self {
        Self {
            paths,
            language,
            status_message: String::new(),
        }
    }

    fn text(&self) -> Text {
        Text::new(self.language)
    }
}

impl eframe::App for AboutApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let text = self.text();

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.heading(APP_NAME);
                ui.label(text.about_description());
            });

            ui.add_space(16.0);
            egui::Grid::new("about_info")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .striped(true)
                .show(ui, |ui| {
                    about_row(ui, text.version_label(), env!("CARGO_PKG_VERSION"));
                    about_row(ui, text.author_label(), APP_AUTHOR);
                    about_row(ui, text.license_label(), APP_LICENSE);
                    about_row(ui, text.copyright_label(), APP_COPYRIGHT);
                    ui.strong(text.homepage_label());
                    ui.hyperlink_to(APP_HOMEPAGE, APP_HOMEPAGE);
                    ui.end_row();
                    about_row(ui, text.save_folder_label(), display_path(&self.paths.root));
                    about_row(
                        ui,
                        text.config_file_label(),
                        display_path(&self.paths.config),
                    );
                });

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.button(text.open_output_dir()).clicked() {
                    match platform::open_path(&self.paths.root) {
                        Ok(()) => self.status_message.clear(),
                        Err(error) => self.status_message = error.to_string(),
                    }
                }
                if !self.status_message.is_empty() {
                    ui.label(&self.status_message);
                }
            });
        });
    }
}

fn about_row(ui: &mut egui::Ui, label: &str, value: impl std::fmt::Display) {
    ui.strong(label);
    ui.label(value.to_string());
    ui.end_row();
}
