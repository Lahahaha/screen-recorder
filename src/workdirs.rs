use crate::{
    app::AppResult,
    config::{load_config, Language},
    i18n::Text,
    logging::Logger,
    paths::AppPaths,
    platform,
    single_instance::{InstanceGuard, InstanceKind},
};
use chrono::Local;
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

const STATE_FILE_NAME: &str = "workdirs.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct WorkdirState {
    selected_root: Option<PathBuf>,
    #[serde(default)]
    entries: Vec<WorkdirEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WorkdirEntry {
    root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deletion: Option<DeletionMarker>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeletionMarker {
    Trashed,
    Deleted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkdirAvailability {
    Available,
    Trashed,
    Deleted,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AddWorkdirResult {
    Added,
    AlreadyExists,
}

#[derive(Clone)]
enum WorkdirAction {
    Open(PathBuf),
    Switch(PathBuf),
    Delete(PathBuf),
    Remove(PathBuf),
}

pub(crate) fn startup_root(control: &Path, default_root: &Path) -> AppResult<PathBuf> {
    let mut state = load_state(control)?;
    let default_root = normalize_existing_dir(default_root)?;
    let selected_root = state.selected_root.clone();
    let selected_root = selected_root
        .as_deref()
        .and_then(|path| normalize_existing_dir(path).ok());

    let root = selected_root.unwrap_or(default_root);
    let mut changed = ensure_entry(&mut state, &root)?;
    if state
        .selected_root
        .as_ref()
        .is_none_or(|selected| !same_path(selected, &root))
    {
        state.selected_root = Some(root.clone());
        changed = true;
    }
    changed |= normalize_state(&mut state)?;
    if changed {
        save_state(control, &state)?;
    }
    Ok(root)
}

pub(crate) fn record_startup_root(control: &Path, root: &Path) -> AppResult<()> {
    let mut state = load_state(control)?;
    let mut changed = ensure_entry(&mut state, root)?;
    if state.selected_root.is_none() {
        state.selected_root = Some(normalize_existing_dir(root)?);
        changed = true;
    }
    changed |= normalize_state(&mut state)?;
    if changed {
        save_state(control, &state)?;
    }
    Ok(())
}

pub(crate) fn run(workdir: Option<PathBuf>) -> AppResult<()> {
    let paths = match workdir {
        Some(root) => AppPaths::from_root(root)?,
        None => AppPaths::new()?,
    };
    let logger = Logger::new(&paths)?;
    let Some(_instance_guard) = InstanceGuard::acquire(&paths, InstanceKind::Workdirs)? else {
        logger.info("工作目录窗口已在运行，忽略本次启动");
        return Ok(());
    };
    let config = load_config(&paths, &logger)?;
    let language = config.language;
    let title = Text::new(language).workdirs_window_title().to_string();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([820.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::System);
            install_workdirs_fonts(&cc.egui_ctx, &logger);
            Ok(Box::new(WorkdirsApp::new(paths, language, logger)))
        }),
    )
    .map_err(|error| io::Error::other(error.to_string()).into())
}

fn state_path(control: &Path) -> PathBuf {
    control.join(STATE_FILE_NAME)
}

fn load_state(control: &Path) -> AppResult<WorkdirState> {
    let path = state_path(control);
    if !path.exists() {
        return Ok(WorkdirState::default());
    }

    let content = fs::read_to_string(&path)?;
    let mut state: WorkdirState = match serde_json::from_str(&content) {
        Ok(state) => state,
        Err(error) => {
            backup_corrupted_state(control, &path)?;
            let _ = error;
            WorkdirState::default()
        }
    };
    normalize_state(&mut state)?;
    Ok(state)
}

fn save_state(control: &Path, state: &WorkdirState) -> AppResult<()> {
    fs::create_dir_all(control)?;
    let content = serde_json::to_string_pretty(state)?;
    let temp_path = control.join(format!(
        ".{STATE_FILE_NAME}.{}.{}.tmp",
        std::process::id(),
        Local::now().format("%Y%m%d%H%M%S%.3f")
    ));
    {
        let mut file = fs::File::create(&temp_path)?;
        use std::io::Write;
        file.write_all(format!("{content}\n").as_bytes())?;
        file.sync_all()?;
    }
    platform::replace_file(&temp_path, &state_path(control))?;
    Ok(())
}

fn backup_corrupted_state(control: &Path, path: &Path) -> AppResult<()> {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S%.3f");
    let backup = control.join(format!("{STATE_FILE_NAME}.corrupt.{timestamp}"));
    fs::rename(path, backup)?;
    Ok(())
}

fn normalize_state(state: &mut WorkdirState) -> AppResult<bool> {
    let mut changed = false;
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for mut entry in state.entries.drain(..) {
        let normalized = normalize_stored_path(&entry.root);
        if normalized != entry.root {
            entry.root = normalized;
            changed = true;
        }
        if entry.root.is_dir() && entry.deletion.take().is_some() {
            changed = true;
        }
        let key = path_key(&entry.root);
        if seen.insert(key) {
            entries.push(entry);
        } else {
            changed = true;
        }
    }
    state.entries = entries;

    if let Some(selected_root) = state.selected_root.clone() {
        let normalized = normalize_stored_path(&selected_root);
        if normalized != selected_root {
            state.selected_root = Some(normalized);
            changed = true;
        }
        if let Some(selected_root) = state.selected_root.clone() {
            changed |= ensure_entry(state, &selected_root)?;
        }
    }

    Ok(changed)
}

fn ensure_entry(state: &mut WorkdirState, root: &Path) -> AppResult<bool> {
    let normalized = normalize_stored_path(root);
    let key = path_key(&normalized);
    if let Some(entry) = state
        .entries
        .iter_mut()
        .find(|entry| path_key(&entry.root) == key)
    {
        let mut changed = false;
        if entry.root != normalized {
            entry.root = normalized;
            changed = true;
        }
        if entry.root.is_dir() && entry.deletion.take().is_some() {
            changed = true;
        }
        return Ok(changed);
    }

    state.entries.push(WorkdirEntry {
        root: normalized,
        deletion: None,
    });
    Ok(true)
}

fn add_workdir_to_state(state: &mut WorkdirState, root: &Path) -> AppResult<AddWorkdirResult> {
    let root = normalize_existing_dir(root)?;
    AppPaths::ensure_dir(&root.join("screenshots"))?;
    AppPaths::ensure_dir(&root.join("videos"))?;
    let existed = state
        .entries
        .iter()
        .any(|entry| same_path(&entry.root, &root));
    ensure_entry(state, &root)?;
    if existed {
        Ok(AddWorkdirResult::AlreadyExists)
    } else {
        Ok(AddWorkdirResult::Added)
    }
}

fn select_workdir(state: &mut WorkdirState, root: &Path) -> AppResult<()> {
    let root = normalize_existing_dir(root)?;
    ensure_entry(state, &root)?;
    state.selected_root = Some(root);
    Ok(())
}

fn mark_deleted(state: &mut WorkdirState, root: &Path, marker: DeletionMarker) -> AppResult<()> {
    ensure_entry(state, root)?;
    if let Some(entry) = state
        .entries
        .iter_mut()
        .find(|entry| same_path(&entry.root, root))
    {
        entry.deletion = Some(marker);
    }
    Ok(())
}

fn remove_from_history(state: &mut WorkdirState, root: &Path) {
    state.entries.retain(|entry| !same_path(&entry.root, root));
}

fn availability(entry: &WorkdirEntry) -> WorkdirAvailability {
    if entry.root.is_dir() {
        return WorkdirAvailability::Available;
    }
    match entry.deletion {
        Some(DeletionMarker::Trashed) => WorkdirAvailability::Trashed,
        Some(DeletionMarker::Deleted) => WorkdirAvailability::Deleted,
        None => WorkdirAvailability::Missing,
    }
}

fn normalize_existing_dir(path: &Path) -> AppResult<PathBuf> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("目录不存在: {}", path.display()),
        )
        .into());
    }
    if !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("路径不是目录: {}", path.display()),
        )
        .into());
    }
    path.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("规范化目录失败 {}: {error}", path.display()),
        )
        .into()
    })
}

fn normalize_stored_path(path: &Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

fn path_key(path: &Path) -> String {
    let value = normalize_stored_path(path).to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    {
        value.to_ascii_lowercase()
    }
    #[cfg(not(target_os = "windows"))]
    {
        value
    }
}

fn can_delete_files(root: &Path, current_root: &Path) -> bool {
    if !root.is_dir() {
        return false;
    }
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let Ok(current_root) = current_root.canonicalize() else {
        return false;
    };
    !current_root.starts_with(&root)
}

struct WorkdirsApp {
    paths: AppPaths,
    language: Language,
    logger: Logger,
    state: WorkdirState,
    status_message: String,
    pending_delete: Option<PathBuf>,
}

impl WorkdirsApp {
    fn new(paths: AppPaths, language: Language, logger: Logger) -> Self {
        let mut app = Self {
            state: WorkdirState::default(),
            paths,
            language,
            logger,
            status_message: String::new(),
            pending_delete: None,
        };
        app.reload_state();
        app
    }

    fn text(&self) -> Text {
        Text::new(self.language)
    }

    fn reload_state(&mut self) {
        match load_state(&self.paths.control) {
            Ok(mut state) => {
                let mut changed = false;
                match ensure_entry(&mut state, &self.paths.root) {
                    Ok(entry_changed) => changed |= entry_changed,
                    Err(error) => self.logger.warn(format!("记录当前工作目录失败: {error}")),
                }
                if state.selected_root.is_none() {
                    state.selected_root = Some(self.paths.root.clone());
                    changed = true;
                }
                match normalize_state(&mut state) {
                    Ok(state_changed) => changed |= state_changed,
                    Err(error) => self.logger.warn(format!("整理工作目录列表失败: {error}")),
                }
                if changed {
                    if let Err(error) = save_state(&self.paths.control, &state) {
                        self.logger.warn(format!("保存工作目录列表失败: {error}"));
                    }
                }
                self.state = state;
            }
            Err(error) => {
                self.status_message = error.to_string();
                self.state = WorkdirState {
                    selected_root: Some(self.paths.root.clone()),
                    entries: vec![WorkdirEntry {
                        root: self.paths.root.clone(),
                        deletion: None,
                    }],
                };
            }
        }
    }

    fn save_state(&mut self) -> bool {
        match save_state(&self.paths.control, &self.state) {
            Ok(()) => true,
            Err(error) => {
                self.status_message = error.to_string();
                false
            }
        }
    }

    fn add_folder(&mut self, folder: PathBuf) {
        let text = self.text();
        match add_workdir_to_state(&mut self.state, &folder) {
            Ok(AddWorkdirResult::Added) => {
                if self.save_state() {
                    self.status_message = text.workdir_added(&folder);
                }
            }
            Ok(AddWorkdirResult::AlreadyExists) => {
                if self.save_state() {
                    self.status_message = text.workdir_already_exists(&folder);
                }
            }
            Err(error) => {
                self.status_message = text.workdir_operation_failed(&error);
            }
        }
    }

    fn handle_action(&mut self, action: WorkdirAction) {
        match action {
            WorkdirAction::Open(root) => {
                if let Err(error) = platform::open_path(&root) {
                    self.status_message = self.text().workdir_operation_failed(&error);
                }
            }
            WorkdirAction::Switch(root) => {
                let text = self.text();
                match select_workdir(&mut self.state, &root) {
                    Ok(()) if self.save_state() => {
                        self.status_message = text.workdir_switch_saved(&root);
                    }
                    Ok(()) => {}
                    Err(error) => {
                        self.status_message = text.workdir_operation_failed(&error);
                    }
                }
            }
            WorkdirAction::Delete(root) => {
                self.pending_delete = Some(root);
            }
            WorkdirAction::Remove(root) => {
                remove_from_history(&mut self.state, &root);
                if self.save_state() {
                    self.status_message = self.text().workdir_removed(&root);
                }
            }
        }
    }

    fn delete_to_trash(&mut self, root: &Path) {
        if !can_delete_files(root, &self.paths.root) {
            self.status_message = self.text().workdir_cannot_delete_current();
            return;
        }
        match trash::delete(root) {
            Ok(()) => {
                let _ = mark_deleted(&mut self.state, root, DeletionMarker::Trashed);
                self.fallback_selected_if_deleted(root);
                if self.save_state() {
                    self.status_message = self.text().workdir_moved_to_trash(root);
                }
            }
            Err(error) => {
                self.status_message = self.text().workdir_operation_failed(&error);
            }
        }
    }

    fn permanently_delete(&mut self, root: &Path) {
        if !can_delete_files(root, &self.paths.root) {
            self.status_message = self.text().workdir_cannot_delete_current();
            return;
        }
        match fs::remove_dir_all(root) {
            Ok(()) => {
                let _ = mark_deleted(&mut self.state, root, DeletionMarker::Deleted);
                self.fallback_selected_if_deleted(root);
                if self.save_state() {
                    self.status_message = self.text().workdir_permanently_deleted(root);
                }
            }
            Err(error) => {
                self.status_message = self.text().workdir_operation_failed(&error);
            }
        }
    }

    fn fallback_selected_if_deleted(&mut self, root: &Path) {
        if self
            .state
            .selected_root
            .as_ref()
            .is_some_and(|selected| same_path(selected, root))
        {
            self.state.selected_root = Some(self.paths.root.clone());
            let _ = ensure_entry(&mut self.state, &self.paths.root);
        }
    }

    fn render_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(root) = self.pending_delete.clone() else {
            return;
        };
        let text = self.text();
        egui::Window::new(text.workdir_delete_title())
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(text.workdir_delete_prompt(&root));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(text.move_to_trash()).clicked() {
                        self.delete_to_trash(&root);
                        self.pending_delete = None;
                    }
                    if ui.button(text.permanently_delete()).clicked() {
                        self.permanently_delete(&root);
                        self.pending_delete = None;
                    }
                    if ui.button(text.cancel()).clicked() {
                        self.pending_delete = None;
                    }
                });
            });
    }
}

const WORKDIR_MARGIN_X: i8 = 24;
const WORKDIR_CONTENT_TOP_GAP: i8 = 12;
const WORKDIR_STATUS_TOP_GAP: i8 = 10;
const WORKDIR_PANEL_BOTTOM_GAP: i8 = 12;
const WORKDIR_SUMMARY_HEIGHT: f32 = 96.0;
const WORKDIR_SUMMARY_GAP: f32 = 20.0;
const WORKDIR_LIST_TITLE_HEIGHT: f32 = 70.0;
const WORKDIR_HEADER_HEIGHT: f32 = 40.0;
const WORKDIR_ROW_HEIGHT: f32 = 68.0;
const WORKDIR_ROW_GAP: f32 = 10.0;
const WORKDIR_STATUS_HEIGHT: f32 = 38.0;

#[derive(Clone, Copy)]
struct WorkdirsPalette {
    bg: egui::Color32,
    card: egui::Color32,
    card_selected: egui::Color32,
    border: egui::Color32,
    primary: egui::Color32,
    primary_soft: egui::Color32,
    success: egui::Color32,
    success_soft: egui::Color32,
    warning: egui::Color32,
    warning_soft: egui::Color32,
    danger: egui::Color32,
    danger_soft: egui::Color32,
    text: egui::Color32,
    muted: egui::Color32,
    disabled_text: egui::Color32,
    header: egui::Color32,
    inset: egui::Color32,
    footer: egui::Color32,
    button: egui::Color32,
    button_disabled: egui::Color32,
}

impl WorkdirsPalette {
    fn from_theme(theme: egui::Theme) -> Self {
        match theme {
            egui::Theme::Light => Self {
                bg: egui::Color32::from_rgb(247, 248, 250),
                card: egui::Color32::WHITE,
                card_selected: egui::Color32::from_rgb(244, 248, 255),
                border: egui::Color32::from_rgb(218, 222, 228),
                primary: egui::Color32::from_rgb(38, 111, 255),
                primary_soft: egui::Color32::from_rgb(232, 240, 255),
                success: egui::Color32::from_rgb(28, 155, 93),
                success_soft: egui::Color32::from_rgb(222, 246, 234),
                warning: egui::Color32::from_rgb(188, 125, 35),
                warning_soft: egui::Color32::from_rgb(255, 245, 220),
                danger: egui::Color32::from_rgb(206, 71, 71),
                danger_soft: egui::Color32::from_rgb(255, 235, 235),
                text: egui::Color32::from_rgb(32, 38, 46),
                muted: egui::Color32::from_rgb(101, 112, 126),
                disabled_text: egui::Color32::from_rgb(166, 174, 186),
                header: egui::Color32::from_rgb(248, 250, 252),
                inset: egui::Color32::from_rgb(250, 251, 253),
                footer: egui::Color32::from_rgb(245, 247, 250),
                button: egui::Color32::from_rgb(239, 242, 246),
                button_disabled: egui::Color32::from_rgb(245, 246, 248),
            },
            egui::Theme::Dark => Self {
                bg: egui::Color32::from_rgb(17, 19, 24),
                card: egui::Color32::from_rgb(26, 29, 36),
                card_selected: egui::Color32::from_rgb(28, 39, 61),
                border: egui::Color32::from_rgb(60, 66, 78),
                primary: egui::Color32::from_rgb(91, 146, 255),
                primary_soft: egui::Color32::from_rgb(30, 51, 85),
                success: egui::Color32::from_rgb(82, 204, 143),
                success_soft: egui::Color32::from_rgb(24, 67, 48),
                warning: egui::Color32::from_rgb(236, 181, 83),
                warning_soft: egui::Color32::from_rgb(73, 53, 27),
                danger: egui::Color32::from_rgb(245, 113, 113),
                danger_soft: egui::Color32::from_rgb(75, 34, 39),
                text: egui::Color32::from_rgb(235, 239, 245),
                muted: egui::Color32::from_rgb(157, 166, 179),
                disabled_text: egui::Color32::from_rgb(112, 121, 135),
                header: egui::Color32::from_rgb(34, 38, 47),
                inset: egui::Color32::from_rgb(32, 36, 44),
                footer: egui::Color32::from_rgb(29, 33, 41),
                button: egui::Color32::from_rgb(37, 42, 51),
                button_disabled: egui::Color32::from_rgb(31, 35, 43),
            },
        }
    }
}

#[derive(Clone, Copy)]
struct WorkdirTableLayout {
    row_width: f32,
    status: f32,
    path: f32,
    current: f32,
    next: f32,
    actions: f32,
}

#[derive(Clone, Copy)]
struct WorkdirTableColumns {
    status: egui::Rect,
    path: egui::Rect,
    current: egui::Rect,
    next: egui::Rect,
    actions: egui::Rect,
}

impl WorkdirTableLayout {
    fn new(row_width: f32) -> Self {
        let status = 104.0;
        let current = 54.0;
        let next = 54.0;
        let actions = 348.0;
        let path = (row_width - status - current - next - actions).max(96.0);
        Self {
            row_width: status + path + current + next + actions,
            status,
            path,
            current,
            next,
            actions,
        }
    }

    fn column_rects(&self, row_rect: egui::Rect) -> WorkdirTableColumns {
        let mut x = row_rect.left();
        let y = row_rect.top();
        let h = row_rect.height();
        let status = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.status, h));
        x += self.status;
        let path = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.path, h));
        x += self.path;
        let current = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.current, h));
        x += self.current;
        let next = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.next, h));
        x += self.next;
        let actions = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(self.actions, h));
        WorkdirTableColumns {
            status,
            path,
            current,
            next,
            actions,
        }
    }
}

impl eframe::App for WorkdirsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let text = self.text();
        let selected_root = self.state.selected_root.clone();
        let entries = self.state.entries.clone();
        let mut action = None;
        let palette = WorkdirsPalette::from_theme(ctx.theme());

        egui::Panel::top("workdirs_toolbar")
            .frame(
                egui::Frame::NONE
                    .fill(palette.bg)
                    .inner_margin(egui::Margin::symmetric(WORKDIR_MARGIN_X, 0)),
            )
            .show_inside(ui, |ui| {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading(
                            egui::RichText::new(text.workdirs_window_title())
                                .size(24.0)
                                .color(palette.text),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            egui::RichText::new(workdirs_subtitle(self.language))
                                .color(palette.muted),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        if ui
                            .add_sized(
                                [142.0, 38.0],
                                egui::Button::new(
                                    egui::RichText::new(text.add_workdir())
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(palette.primary)
                                .stroke(egui::Stroke::new(1.0, palette.primary)),
                            )
                            .clicked()
                        {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.add_folder(folder);
                            }
                        }
                        if ui
                            .add_sized([78.0, 38.0], egui::Button::new(text.refresh()))
                            .clicked()
                        {
                            self.reload_state();
                        }
                    });
                });
                ui.add_space(16.0);
            });

        egui::Panel::bottom("workdirs_status")
            .frame(
                egui::Frame::NONE
                    .fill(palette.bg)
                    .inner_margin(egui::Margin {
                        left: WORKDIR_MARGIN_X,
                        right: WORKDIR_MARGIN_X,
                        top: WORKDIR_STATUS_TOP_GAP,
                        bottom: WORKDIR_PANEL_BOTTOM_GAP,
                    }),
            )
            .show_inside(ui, |ui| {
                egui::Frame::NONE
                    .fill(palette.footer)
                    .stroke(egui::Stroke::new(1.0, palette.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.set_min_height(WORKDIR_STATUS_HEIGHT - 16.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(text.ready())
                                    .strong()
                                    .color(palette.muted),
                            );
                            ui.separator();
                            let detail = if self.status_message.is_empty() {
                                let same_next = selected_root
                                    .as_ref()
                                    .is_some_and(|root| same_path(root, &self.paths.root));
                                workdirs_ready_detail(self.language, same_next).to_string()
                            } else {
                                self.status_message.clone()
                            };
                            ui.label(egui::RichText::new(detail).color(palette.muted));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(workdirs_count_label(
                                            self.language,
                                            entries.len(),
                                        ))
                                        .color(palette.muted),
                                    );
                                },
                            );
                        });
                    });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(palette.bg)
                    .inner_margin(egui::Margin {
                        left: WORKDIR_MARGIN_X,
                        right: WORKDIR_MARGIN_X,
                        top: WORKDIR_CONTENT_TOP_GAP,
                        bottom: 10,
                    }),
            )
            .show_inside(ui, |ui| {
                let next_root = selected_root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string());
                let summary_width = ui.available_width();
                let summary_origin = ui.cursor().min;
                let card_width = ((summary_width - WORKDIR_SUMMARY_GAP) / 2.0).max(240.0);
                paint_workdir_summary_card(
                    ui,
                    egui::Rect::from_min_size(
                        summary_origin,
                        egui::vec2(card_width, WORKDIR_SUMMARY_HEIGHT),
                    ),
                    text.current_workdir_label(),
                    &self.paths.root.display().to_string(),
                    summary_badge(self.language, true),
                    true,
                    palette,
                );
                paint_workdir_summary_card(
                    ui,
                    egui::Rect::from_min_size(
                        summary_origin + egui::vec2(card_width + WORKDIR_SUMMARY_GAP, 0.0),
                        egui::vec2(card_width, WORKDIR_SUMMARY_HEIGHT),
                    ),
                    text.next_workdir_label(),
                    &next_root,
                    summary_badge(self.language, false),
                    false,
                    palette,
                );
                ui.allocate_space(egui::vec2(summary_width, WORKDIR_SUMMARY_HEIGHT));
                ui.add_space(16.0);

                show_workdir_list(
                    ui,
                    &text,
                    self.language,
                    &entries,
                    &self.paths.root,
                    selected_root.as_deref(),
                    palette,
                    &mut action,
                );
            });

        if let Some(action) = action {
            self.handle_action(action);
        }
        self.render_delete_dialog(&ctx);
    }
}

fn paint_workdir_summary_card(
    ui: &egui::Ui,
    rect: egui::Rect,
    title: &str,
    path: &str,
    badge: &str,
    active: bool,
    palette: WorkdirsPalette,
) {
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(8),
        palette.card,
        egui::Stroke::new(1.0, palette.border),
        egui::StrokeKind::Middle,
    );

    let chip_width = chip_width(badge);
    let chip_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - chip_width - 18.0, rect.top() + 15.0),
        egui::vec2(chip_width, 26.0),
    );
    paint_chip(
        ui,
        chip_rect,
        badge,
        if active {
            WorkdirChipKind::Success
        } else {
            WorkdirChipKind::Info
        },
        palette,
    );

    let title_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(18.0, 14.0),
        egui::pos2(chip_rect.left() - 10.0, rect.top() + 40.0),
    );
    paint_clipped_text(
        ui,
        title_rect,
        egui::pos2(title_rect.left(), title_rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(14.0),
        palette.muted,
    );

    let path_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(18.0, 52.0),
        rect.max - egui::vec2(18.0, 18.0),
    );
    ui.painter().rect(
        path_rect,
        egui::CornerRadius::same(6),
        palette.inset,
        egui::Stroke::new(1.0, palette.border),
        egui::StrokeKind::Middle,
    );
    paint_clipped_text(
        ui,
        path_rect.shrink2(egui::vec2(12.0, 0.0)),
        egui::pos2(path_rect.left() + 12.0, path_rect.center().y),
        egui::Align2::LEFT_CENTER,
        path,
        egui::FontId::monospace(15.0),
        palette.text,
    );
}

#[allow(clippy::too_many_arguments)]
fn show_workdir_list(
    ui: &mut egui::Ui,
    text: &Text,
    language: Language,
    entries: &[WorkdirEntry],
    current_root: &Path,
    selected_root: Option<&Path>,
    palette: WorkdirsPalette,
    action: &mut Option<WorkdirAction>,
) {
    let list_height = ui.available_height().max(240.0);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), list_height),
        egui::Sense::hover(),
    );
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(8),
        palette.card,
        egui::Stroke::new(1.0, palette.border),
        egui::StrokeKind::Middle,
    );

    let title_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(18.0, 15.0),
        egui::pos2(rect.right() - 112.0, rect.top() + 41.0),
    );
    paint_clipped_text(
        ui,
        title_rect,
        egui::pos2(title_rect.left(), title_rect.center().y),
        egui::Align2::LEFT_CENTER,
        workdirs_list_title(language),
        egui::FontId::proportional(19.0),
        palette.text,
    );
    let hint_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(18.0, 44.0),
        egui::pos2(rect.right() - 18.0, rect.top() + 68.0),
    );
    paint_clipped_text(
        ui,
        hint_rect,
        egui::pos2(hint_rect.left(), hint_rect.center().y),
        egui::Align2::LEFT_CENTER,
        workdirs_list_hint(language),
        egui::FontId::proportional(14.0),
        palette.muted,
    );
    let count_label = short_count_label(language, entries.len());
    let count_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 98.0, rect.top() + 22.0),
        egui::vec2(78.0, 28.0),
    );
    paint_chip(
        ui,
        count_rect,
        &count_label,
        WorkdirChipKind::Neutral,
        palette,
    );

    let table_left = rect.left() + 18.0;
    let table_right = rect.right() - 18.0;
    let table_width = table_right - table_left;
    let layout = WorkdirTableLayout::new(table_width);
    let table_left = table_left + ((table_width - layout.row_width).max(0.0) / 2.0);
    let table_right = table_left + layout.row_width;
    let header_rect = egui::Rect::from_min_max(
        egui::pos2(table_left, rect.top() + WORKDIR_LIST_TITLE_HEIGHT),
        egui::pos2(
            table_right,
            rect.top() + WORKDIR_LIST_TITLE_HEIGHT + WORKDIR_HEADER_HEIGHT,
        ),
    );
    paint_workdir_header(ui, text, layout, header_rect, palette);

    let rows_rect = egui::Rect::from_min_max(
        egui::pos2(table_left, header_rect.bottom() + WORKDIR_ROW_GAP),
        egui::pos2(table_right, rect.bottom() - 56.0),
    );
    let mut rows_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rows_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    rows_ui.set_clip_rect(rows_rect);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(&mut rows_ui, |ui| {
            ui.set_min_width(layout.row_width);
            for entry in entries {
                show_workdir_row(
                    ui,
                    text,
                    language,
                    layout,
                    entry,
                    current_root,
                    selected_root,
                    palette,
                    action,
                );
                ui.add_space(WORKDIR_ROW_GAP);
            }
        });

    ui.painter().line_segment(
        [
            egui::pos2(table_left, rows_rect.bottom() + 20.0),
            egui::pos2(table_right, rows_rect.bottom() + 20.0),
        ],
        egui::Stroke::new(1.0, palette.border),
    );
    let footer_rect = egui::Rect::from_min_max(
        egui::pos2(table_left + 12.0, rows_rect.bottom() + 30.0),
        egui::pos2(table_right - 12.0, rect.bottom() - 12.0),
    );
    paint_clipped_text(
        ui,
        footer_rect,
        egui::pos2(footer_rect.left(), footer_rect.center().y),
        egui::Align2::LEFT_CENTER,
        workdirs_restore_hint(language),
        egui::FontId::proportional(14.0),
        palette.muted,
    );
}

fn paint_workdir_header(
    ui: &mut egui::Ui,
    text: &Text,
    layout: WorkdirTableLayout,
    rect: egui::Rect,
    palette: WorkdirsPalette,
) {
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(6),
        palette.header,
        egui::Stroke::new(1.0, palette.border),
        egui::StrokeKind::Middle,
    );
    let columns = layout.column_rects(rect);
    header_label(
        ui,
        columns.status,
        text.workdir_status_label(),
        egui::Align2::LEFT_CENTER,
        palette,
    );
    header_label(
        ui,
        columns.path,
        text.workdir_path_label(),
        egui::Align2::LEFT_CENTER,
        palette,
    );
    header_label(
        ui,
        columns.current,
        text.workdir_current_label(),
        egui::Align2::CENTER_CENTER,
        palette,
    );
    header_label(
        ui,
        columns.next,
        text.workdir_next_label(),
        egui::Align2::CENTER_CENTER,
        palette,
    );
    header_label(
        ui,
        columns.actions,
        text.workdir_actions_label(),
        egui::Align2::LEFT_CENTER,
        palette,
    );
}

#[allow(clippy::too_many_arguments)]
fn show_workdir_row(
    ui: &mut egui::Ui,
    text: &Text,
    language: Language,
    layout: WorkdirTableLayout,
    entry: &WorkdirEntry,
    current_root: &Path,
    selected_root: Option<&Path>,
    palette: WorkdirsPalette,
    action: &mut Option<WorkdirAction>,
) {
    let (row_rect, _) = ui.allocate_exact_size(
        egui::vec2(layout.row_width, WORKDIR_ROW_HEIGHT),
        egui::Sense::hover(),
    );
    let is_current = same_path(&entry.root, current_root);
    let is_next = selected_root.is_some_and(|selected| same_path(selected, &entry.root));
    let available = availability(entry);
    let fill = if is_current {
        palette.card_selected
    } else {
        palette.card
    };
    let stroke = if is_current {
        egui::Stroke::new(1.3, palette.primary)
    } else {
        egui::Stroke::new(1.0, palette.border)
    };
    ui.painter().rect(
        row_rect,
        egui::CornerRadius::same(8),
        fill,
        stroke,
        egui::StrokeKind::Middle,
    );

    let columns = layout.column_rects(row_rect);
    for x in [
        columns.path.left(),
        columns.current.left(),
        columns.next.left(),
        columns.actions.left(),
    ] {
        ui.painter().line_segment(
            [
                egui::pos2(x, row_rect.top() + 12.0),
                egui::pos2(x, row_rect.bottom() - 12.0),
            ],
            egui::Stroke::new(1.0, palette.border),
        );
    }

    let availability_label = workdir_availability_label(text, available);
    let chip_rect = egui::Rect::from_center_size(
        columns.status.center(),
        egui::vec2((columns.status.width() - 24.0).max(72.0), 26.0),
    );
    paint_chip(
        ui,
        chip_rect,
        availability_label,
        availability_chip_kind(available),
        palette,
    );

    let path_top = egui::Rect::from_min_max(
        columns.path.min + egui::vec2(16.0, 11.0),
        egui::pos2(columns.path.right() - 14.0, row_rect.top() + 35.0),
    );
    paint_clipped_text(
        ui,
        path_top,
        egui::pos2(path_top.left(), path_top.center().y),
        egui::Align2::LEFT_CENTER,
        &entry.root.display().to_string(),
        egui::FontId::monospace(15.0),
        palette.text,
    );
    let path_bottom = egui::Rect::from_min_max(
        egui::pos2(columns.path.left() + 16.0, row_rect.top() + 38.0),
        egui::pos2(columns.path.right() - 14.0, row_rect.bottom() - 8.0),
    );
    paint_clipped_text(
        ui,
        path_bottom,
        egui::pos2(path_bottom.left(), path_bottom.center().y),
        egui::Align2::LEFT_CENTER,
        workdir_path_meta_label(language),
        egui::FontId::monospace(12.0),
        palette.disabled_text,
    );

    paint_yes_chip(
        ui,
        columns.current,
        if is_current { text.yes() } else { "-" },
        is_current,
        palette,
    );
    paint_yes_chip(
        ui,
        columns.next,
        if is_next { text.yes() } else { "-" },
        is_next,
        palette,
    );

    let can_open = available == WorkdirAvailability::Available;
    let can_switch = can_open && !is_next;
    let can_delete = can_delete_files(&entry.root, current_root);
    let can_remove = !is_current && !is_next;
    let button_y = row_rect.top() + 17.0;
    let mut button_x = columns.actions.left() + 14.0;
    for (label, width, enabled, primary, next_action) in [
        (
            text.open_workdir(),
            58.0,
            can_open,
            is_current,
            WorkdirAction::Open(entry.root.clone()),
        ),
        (
            text.switch_workdir(),
            62.0,
            can_switch,
            false,
            WorkdirAction::Switch(entry.root.clone()),
        ),
        (
            text.delete_workdir(),
            104.0,
            can_delete,
            false,
            WorkdirAction::Delete(entry.root.clone()),
        ),
        (
            text.remove_workdir_record(),
            88.0,
            can_remove,
            false,
            WorkdirAction::Remove(entry.root.clone()),
        ),
    ] {
        let button_rect =
            egui::Rect::from_min_size(egui::pos2(button_x, button_y), egui::vec2(width, 34.0));
        if workdir_button_at(ui, button_rect, label, enabled, primary, palette).clicked() && enabled
        {
            *action = Some(next_action);
        }
        button_x += width + 6.0;
    }
}

fn paint_clipped_text(
    ui: &egui::Ui,
    rect: egui::Rect,
    pos: egui::Pos2,
    align: egui::Align2,
    text: &str,
    font_id: egui::FontId,
    color: egui::Color32,
) {
    ui.painter()
        .with_clip_rect(rect)
        .text(pos, align, text, font_id, color);
}

fn header_label(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    label: &str,
    align: egui::Align2,
    palette: WorkdirsPalette,
) {
    let pos = match align {
        egui::Align2::LEFT_CENTER => egui::pos2(rect.left() + 14.0, rect.center().y),
        egui::Align2::CENTER_CENTER => rect.center(),
        _ => rect.center(),
    };
    ui.painter().text(
        pos,
        align,
        label,
        egui::FontId::proportional(14.0),
        palette.muted,
    );
}

fn paint_yes_chip(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    label: &str,
    active: bool,
    palette: WorkdirsPalette,
) {
    let chip_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(42.0, 26.0));
    paint_chip(
        ui,
        chip_rect,
        label,
        if active {
            WorkdirChipKind::Success
        } else {
            WorkdirChipKind::Neutral
        },
        palette,
    );
}

fn workdir_button_at(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    label: &str,
    enabled: bool,
    primary: bool,
    palette: WorkdirsPalette,
) -> egui::Response {
    let (fill, stroke, text_color) = if primary && enabled {
        (
            palette.primary,
            egui::Stroke::new(1.0, palette.primary),
            egui::Color32::WHITE,
        )
    } else if enabled {
        (
            palette.button,
            egui::Stroke::new(1.0, palette.border),
            palette.text,
        )
    } else {
        (
            palette.button_disabled,
            egui::Stroke::new(1.0, palette.border),
            palette.disabled_text,
        )
    };
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    ui.put(
        rect,
        egui::Button::new(egui::RichText::new(label).strong().color(text_color))
            .fill(fill)
            .stroke(stroke)
            .sense(sense),
    )
}

#[derive(Clone, Copy)]
enum WorkdirChipKind {
    Success,
    Warning,
    Danger,
    Info,
    Neutral,
}

fn paint_chip(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    kind: WorkdirChipKind,
    palette: WorkdirsPalette,
) {
    let (fill, stroke, text_color) = match kind {
        WorkdirChipKind::Success => (
            palette.success_soft,
            egui::Stroke::new(1.0, palette.success.linear_multiply(0.35)),
            palette.success,
        ),
        WorkdirChipKind::Warning => (
            palette.warning_soft,
            egui::Stroke::new(1.0, palette.warning.linear_multiply(0.35)),
            palette.warning,
        ),
        WorkdirChipKind::Danger => (
            palette.danger_soft,
            egui::Stroke::new(1.0, palette.danger.linear_multiply(0.35)),
            palette.danger,
        ),
        WorkdirChipKind::Info => (
            palette.primary_soft,
            egui::Stroke::new(1.0, palette.primary.linear_multiply(0.35)),
            palette.primary,
        ),
        WorkdirChipKind::Neutral => (
            palette.inset,
            egui::Stroke::new(1.0, palette.border),
            palette.muted,
        ),
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same((rect.height() / 2.0).round() as u8),
        fill,
        stroke,
        egui::StrokeKind::Middle,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(13.0),
        text_color,
    );
}

fn chip_width(label: &str) -> f32 {
    (label.chars().count() as f32 * 13.0 + 28.0).clamp(62.0, 142.0)
}

fn availability_chip_kind(availability: WorkdirAvailability) -> WorkdirChipKind {
    match availability {
        WorkdirAvailability::Available => WorkdirChipKind::Success,
        WorkdirAvailability::Missing | WorkdirAvailability::Trashed => WorkdirChipKind::Warning,
        WorkdirAvailability::Deleted => WorkdirChipKind::Danger,
    }
}

fn summary_badge(language: Language, current: bool) -> &'static str {
    match (language, current) {
        (Language::ZhCn, true) => "运行中",
        (Language::ZhCn, false) => "已保存",
        (Language::En, true) => "Active",
        (Language::En, false) => "Saved",
    }
}

fn workdirs_subtitle(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "管理截图、视频、配置和日志所在的根目录",
        Language::En => "Manage the root folder for screenshots, videos, config, and logs",
    }
}

fn workdirs_list_title(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "工作目录列表",
        Language::En => "Work Folder List",
    }
}

fn workdirs_list_hint(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "路径过长时会保留首尾；不可用动作保持置灰。",
        Language::En => {
            "Long paths keep their beginning and end; unavailable actions stay disabled."
        }
    }
}

fn workdirs_restore_hint(language: Language) -> &'static str {
    match language {
        Language::ZhCn => "恢复已删除路径后，状态会在刷新后自动回到可用。",
        Language::En => "After restoring a deleted path, refresh to mark it available again.",
    }
}

fn workdirs_ready_detail(language: Language, same_next: bool) -> &'static str {
    match (language, same_next) {
        (Language::ZhCn, true) => "当前工作目录和下次启动目录一致",
        (Language::ZhCn, false) => "已选择不同的下次启动目录",
        (Language::En, true) => "Current and next launch folders match",
        (Language::En, false) => "A different folder is selected for next launch",
    }
}

fn workdirs_count_label(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("共 {count} 个工作目录"),
        Language::En => format!("{count} work folders"),
    }
}

fn short_count_label(language: Language, count: usize) -> String {
    match language {
        Language::ZhCn => format!("{count} 项"),
        Language::En => format!("{count} items"),
    }
}

fn workdir_path_meta_label(language: Language) -> &'static str {
    match language {
        Language::ZhCn | Language::En => "screenshots / videos / config.json",
    }
}

fn install_workdirs_fonts(ctx: &egui::Context, logger: &Logger) {
    let Some(font_path) = cjk_font_candidates().into_iter().find(|path| path.exists()) else {
        logger.warn("未找到可用的系统 CJK 字体，工作目录窗口将使用 egui 默认字体");
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
        "工作目录窗口已加载系统 CJK 字体: {}",
        font_path.display()
    ));
}

fn workdir_availability_label(text: &Text, availability: WorkdirAvailability) -> &'static str {
    match availability {
        WorkdirAvailability::Available => text.workdir_available_status(),
        WorkdirAvailability::Trashed => text.workdir_trashed_status(),
        WorkdirAvailability::Deleted => text.workdir_deleted_status(),
        WorkdirAvailability::Missing => text.workdir_missing_status(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "screen-recorder-workdirs-test-{name}-{}-{}",
            std::process::id(),
            Local::now().format("%Y%m%d%H%M%S%.3f")
        ));
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn adding_same_directory_deduplicates_by_canonical_path() {
        let root = test_dir("dedup");
        let mut state = WorkdirState::default();

        assert_eq!(
            add_workdir_to_state(&mut state, &root).expect("add first"),
            AddWorkdirResult::Added
        );
        assert_eq!(
            add_workdir_to_state(&mut state, &root.join(".")).expect("add duplicate"),
            AddWorkdirResult::AlreadyExists
        );

        assert_eq!(state.entries.len(), 1);
        assert!(root.join("screenshots").is_dir());
        assert!(root.join("videos").is_dir());
        assert!(!root.join("config.json").exists());
    }

    #[test]
    fn restored_trashed_directory_is_available_again() {
        let root = test_dir("restored");
        let entry = WorkdirEntry {
            root,
            deletion: Some(DeletionMarker::Trashed),
        };

        assert_eq!(availability(&entry), WorkdirAvailability::Available);
    }

    #[test]
    fn current_directory_and_ancestors_cannot_be_deleted() {
        let current = test_dir("current");
        let child = current.join("child");
        fs::create_dir_all(&child).expect("create child");
        let sibling = test_dir("sibling");

        assert!(!can_delete_files(&current, &current));
        assert!(!can_delete_files(current.parent().unwrap(), &current));
        assert!(can_delete_files(&child, &current));
        assert!(can_delete_files(&sibling, &current));
    }

    #[test]
    fn corrupted_state_file_is_backed_up_and_rebuilt() {
        let control = test_dir("corrupt");
        fs::write(state_path(&control), "{not valid json").expect("write corrupt state");

        let state = load_state(&control).expect("load rebuilt state");

        assert!(state.entries.is_empty());
        assert!(fs::read_dir(&control)
            .expect("read control")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("workdirs.json.corrupt.")));
    }

    #[test]
    fn startup_root_uses_selected_directory_when_available() {
        let control = test_dir("startup-control");
        let default_root = test_dir("startup-default");
        let selected = test_dir("startup-selected");
        let state = WorkdirState {
            selected_root: Some(selected.clone()),
            entries: vec![WorkdirEntry {
                root: selected.clone(),
                deletion: None,
            }],
        };
        save_state(&control, &state).expect("save state");

        let root = startup_root(&control, &default_root).expect("startup root");

        assert!(same_path(&root, &selected));
    }

    #[test]
    fn startup_root_falls_back_when_selected_directory_is_missing() {
        let control = test_dir("fallback-control");
        let default_root = test_dir("fallback-default");
        let missing = control.join("missing");
        let state = WorkdirState {
            selected_root: Some(missing.clone()),
            entries: vec![WorkdirEntry {
                root: missing,
                deletion: Some(DeletionMarker::Trashed),
            }],
        };
        save_state(&control, &state).expect("save state");

        let root = startup_root(&control, &default_root).expect("startup root");

        assert!(same_path(&root, &default_root));
        let state = load_state(&control).expect("reload state");
        assert!(state
            .selected_root
            .as_ref()
            .is_some_and(|root| same_path(root, &default_root)));
        assert_eq!(state.entries.len(), 2);
    }
}
