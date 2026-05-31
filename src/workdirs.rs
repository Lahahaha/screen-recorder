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
            .with_inner_size([860.0, 560.0])
            .with_min_inner_size([720.0, 420.0]),
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

impl eframe::App for WorkdirsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let text = self.text();
        let selected_root = self.state.selected_root.clone();
        let entries = self.state.entries.clone();
        let mut action = None;

        egui::Panel::top("workdirs_toolbar").show_inside(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(text.workdirs_window_title());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(text.add_workdir()).clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            self.add_folder(folder);
                        }
                    }
                    if ui.button(text.refresh()).clicked() {
                        self.reload_state();
                    }
                });
            });
            ui.label(format!(
                "{}: {}",
                text.current_workdir_label(),
                self.paths.root.display()
            ));
            let next_root = selected_root
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_string());
            ui.label(format!("{}: {next_root}", text.next_workdir_label()));
            ui.add_space(8.0);
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("workdirs_grid")
                    .num_columns(5)
                    .striped(true)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong(text.workdir_status_label());
                        ui.strong(text.workdir_path_label());
                        ui.strong(text.workdir_current_label());
                        ui.strong(text.workdir_next_label());
                        ui.strong(text.workdir_actions_label());
                        ui.end_row();

                        for entry in &entries {
                            let is_current = same_path(&entry.root, &self.paths.root);
                            let is_next = selected_root
                                .as_ref()
                                .is_some_and(|selected| same_path(selected, &entry.root));
                            let available = availability(entry);
                            ui.label(workdir_availability_label(&text, available));
                            ui.label(entry.root.display().to_string());
                            ui.label(if is_current { text.yes() } else { "" });
                            ui.label(if is_next { text.yes() } else { "" });
                            ui.horizontal(|ui| {
                                let can_open = available == WorkdirAvailability::Available;
                                if ui
                                    .add_enabled(can_open, egui::Button::new(text.open_workdir()))
                                    .clicked()
                                {
                                    action = Some(WorkdirAction::Open(entry.root.clone()));
                                }
                                let can_switch = can_open && !is_next;
                                if ui
                                    .add_enabled(
                                        can_switch,
                                        egui::Button::new(text.switch_workdir()),
                                    )
                                    .clicked()
                                {
                                    action = Some(WorkdirAction::Switch(entry.root.clone()));
                                }
                                let can_delete = can_delete_files(&entry.root, &self.paths.root);
                                if ui
                                    .add_enabled(
                                        can_delete,
                                        egui::Button::new(text.delete_workdir()),
                                    )
                                    .clicked()
                                {
                                    action = Some(WorkdirAction::Delete(entry.root.clone()));
                                }
                                let can_remove = !is_current && !is_next;
                                if ui
                                    .add_enabled(
                                        can_remove,
                                        egui::Button::new(text.remove_workdir_record()),
                                    )
                                    .clicked()
                                {
                                    action = Some(WorkdirAction::Remove(entry.root.clone()));
                                }
                            });
                            ui.end_row();
                        }
                    });
            });

            if !self.status_message.is_empty() {
                ui.separator();
                ui.label(&self.status_message);
            }
        });

        if let Some(action) = action {
            self.handle_action(action);
        }
        self.render_delete_dialog(&ctx);
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
