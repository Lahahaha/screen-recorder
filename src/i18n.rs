use crate::config::Language;
use std::path::Path;

pub(crate) const APP_NAME: &str = "Screen Recorder";

pub(crate) struct Text {
    bundle: &'static TextBundle,
}

pub(crate) struct TextBundle {
    capture_now: &'static str,
    start: &'static str,
    pause: &'static str,
    interval_menu_prefix: &'static str,
    capture_source_menu: &'static str,
    capture_source_auto: &'static str,
    refresh_capture_sources: &'static str,
    screen_label_prefix: &'static str,
    primary_screen_suffix: &'static str,
    generate_today_video: &'static str,
    open_output_dir: &'static str,
    language_menu: &'static str,
    about: &'static str,
    about_window_title: &'static str,
    about_description: &'static str,
    about_failed_title: &'static str,
    version_label: &'static str,
    author_label: &'static str,
    license_label: &'static str,
    copyright_label: &'static str,
    homepage_label: &'static str,
    save_folder_label: &'static str,
    config_file_label: &'static str,
    quit: &'static str,
    status_running: &'static str,
    status_paused: &'static str,
    no_screenshots: &'static str,
    tooltip_status_label: &'static str,
    tooltip_interval_label: &'static str,
    tooltip_count_label: &'static str,
    tooltip_latest_label: &'static str,
    saved_capture_prefix: &'static str,
    failed_capture_prefix: &'static str,
    saved_to_prefix: &'static str,
    saved_screenshots_prefix: &'static str,
    saved_screenshots_suffix: &'static str,
    partial_capture_failed_prefix: &'static str,
    partial_capture_failed_suffix: &'static str,
    skipped_duplicate_capture: &'static str,
    capture_success_title: &'static str,
    capture_failed_title: &'static str,
    output_dir_failed_title: &'static str,
    history_videos: &'static str,
    workdirs_menu: &'static str,
    workdirs_window_title: &'static str,
    current_workdir_label: &'static str,
    next_workdir_label: &'static str,
    add_workdir: &'static str,
    open_workdir: &'static str,
    switch_workdir: &'static str,
    delete_workdir: &'static str,
    remove_workdir_record: &'static str,
    workdir_status_label: &'static str,
    workdir_path_label: &'static str,
    workdir_current_label: &'static str,
    workdir_next_label: &'static str,
    workdir_actions_label: &'static str,
    workdir_available_status: &'static str,
    workdir_missing_status: &'static str,
    workdir_trashed_status: &'static str,
    workdir_deleted_status: &'static str,
    workdir_added_prefix: &'static str,
    workdir_already_exists_prefix: &'static str,
    workdir_switch_saved_prefix: &'static str,
    workdir_removed_prefix: &'static str,
    workdir_moved_to_trash_prefix: &'static str,
    workdir_permanently_deleted_prefix: &'static str,
    workdir_operation_failed_prefix: &'static str,
    workdir_cannot_delete_current: &'static str,
    workdir_delete_title: &'static str,
    workdir_delete_prompt_prefix: &'static str,
    workdir_delete_prompt_suffix: &'static str,
    move_to_trash: &'static str,
    permanently_delete: &'static str,
    yes: &'static str,
    refresh: &'static str,
    add_folder: &'static str,
    generate_selected: &'static str,
    generation_mode: &'static str,
    multi_screen_generation_mode: &'static str,
    current_mode_unavailable: &'static str,
    cancel: &'static str,
    cancelling: &'static str,
    open_videos_folder: &'static str,
    date_or_folder: &'static str,
    images: &'static str,
    video: &'static str,
    status: &'static str,
    ready: &'static str,
    unavailable: &'static str,
    failed: &'static str,
    no_available_images: &'static str,
    exists: &'static str,
    missing: &'static str,
    generating: &'static str,
    done: &'static str,
    scanning: &'static str,
    processing_frames_prefix: &'static str,
    encoding: &'static str,
    finishing: &'static str,
    selected_count_prefix: &'static str,
    selected_count_suffix: &'static str,
    frames_label: &'static str,
    skipped_label: &'static str,
    output: &'static str,
    selected_folders: &'static str,
    no_selection: &'static str,
    video_generating_title: &'static str,
    video_generating_body: &'static str,
    video_success_title: &'static str,
    video_failed_title: &'static str,
    config_read_failed_prefix: &'static str,
    background_task_failed_title: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    screen_capture_permission_title: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    screen_capture_permission_body: &'static str,
}

const ZH_CN_TEXT: TextBundle = TextBundle {
    capture_now: "截一张",
    start: "开始",
    pause: "暂停",
    interval_menu_prefix: "间隔 当前：",
    capture_source_menu: "截屏范围",
    capture_source_auto: "自动",
    refresh_capture_sources: "刷新屏幕列表",
    screen_label_prefix: "屏幕",
    primary_screen_suffix: "主屏",
    generate_today_video: "生成今日视频",
    open_output_dir: "打开保存目录",
    language_menu: "语言",
    about: "关于",
    about_window_title: "关于 Screen Recorder",
    about_description: "轻量级屏幕记录工具，定时保存截图并生成延时视频。",
    about_failed_title: "打开关于窗口失败",
    version_label: "版本",
    author_label: "作者",
    license_label: "许可证",
    copyright_label: "版权",
    homepage_label: "项目主页",
    save_folder_label: "保存目录",
    config_file_label: "配置文件",
    quit: "退出",
    status_running: "运行中",
    status_paused: "已暂停",
    no_screenshots: "暂无截图",
    tooltip_status_label: "状态",
    tooltip_interval_label: "间隔",
    tooltip_count_label: "本次截图",
    tooltip_latest_label: "最近",
    saved_capture_prefix: "已保存",
    failed_capture_prefix: "失败",
    saved_to_prefix: "已保存到",
    saved_screenshots_prefix: "已保存 ",
    saved_screenshots_suffix: " 张截图",
    partial_capture_failed_prefix: "失败屏幕 ",
    partial_capture_failed_suffix: " 块",
    skipped_duplicate_capture: "跳过重复截图",
    capture_success_title: "截图成功",
    capture_failed_title: "截图失败",
    output_dir_failed_title: "打开保存目录失败",
    history_videos: "历史视频",
    workdirs_menu: "工作目录...",
    workdirs_window_title: "工作目录",
    current_workdir_label: "当前使用",
    next_workdir_label: "下次启动使用",
    add_workdir: "新增工作目录",
    open_workdir: "打开",
    switch_workdir: "切换",
    delete_workdir: "删除文件夹",
    remove_workdir_record: "移出列表",
    workdir_status_label: "状态",
    workdir_path_label: "路径",
    workdir_current_label: "当前",
    workdir_next_label: "下次",
    workdir_actions_label: "操作",
    workdir_available_status: "可用",
    workdir_missing_status: "缺失",
    workdir_trashed_status: "已移到回收站",
    workdir_deleted_status: "已删除",
    workdir_added_prefix: "已添加工作目录",
    workdir_already_exists_prefix: "该工作目录已在列表中",
    workdir_switch_saved_prefix: "已设为下次启动使用",
    workdir_removed_prefix: "已从列表移除",
    workdir_moved_to_trash_prefix: "已移到回收站",
    workdir_permanently_deleted_prefix: "已永久删除",
    workdir_operation_failed_prefix: "工作目录操作失败",
    workdir_cannot_delete_current: "不能删除当前正在使用的工作目录或其父目录。",
    workdir_delete_title: "删除工作目录",
    workdir_delete_prompt_prefix: "请选择如何处理工作目录：",
    workdir_delete_prompt_suffix: "文件操作成功后仍会保留历史记录，便于恢复识别。",
    move_to_trash: "移到回收站",
    permanently_delete: "永久删除",
    yes: "是",
    refresh: "刷新",
    add_folder: "添加文件夹",
    generate_selected: "生成选中项",
    generation_mode: "生成模式",
    multi_screen_generation_mode: "多屏合成",
    current_mode_unavailable: "当前模式不可生成",
    cancel: "取消",
    cancelling: "正在取消...",
    open_videos_folder: "打开视频目录",
    date_or_folder: "日期 / 文件夹",
    images: "图片",
    video: "视频",
    status: "状态",
    ready: "就绪",
    unavailable: "不可用",
    failed: "失败",
    no_available_images: "无可用图片",
    exists: "已存在",
    missing: "未生成",
    generating: "生成中",
    done: "完成",
    scanning: "扫描图片",
    processing_frames_prefix: "处理帧",
    encoding: "编码视频",
    finishing: "写入视频",
    selected_count_prefix: "已选 ",
    selected_count_suffix: " 项",
    frames_label: "帧",
    skipped_label: "跳过",
    output: "输出",
    selected_folders: "选中文件夹",
    no_selection: "未选择",
    video_generating_title: "视频生成中",
    video_generating_body: "已有视频生成任务正在运行。",
    video_success_title: "视频生成成功",
    video_failed_title: "视频生成失败",
    config_read_failed_prefix: "读取配置失败",
    background_task_failed_title: "后台任务异常退出",
    screen_capture_permission_title: "截屏权限不足",
    screen_capture_permission_body: "无法读取屏幕内容，请在系统设置中允许屏幕录制权限。",
};

const EN_TEXT: TextBundle = TextBundle {
    capture_now: "Capture Now",
    start: "Start",
    pause: "Pause",
    interval_menu_prefix: "Interval: ",
    capture_source_menu: "Capture Source",
    capture_source_auto: "Auto",
    refresh_capture_sources: "Refresh Screens",
    screen_label_prefix: "Screen",
    primary_screen_suffix: "Primary",
    generate_today_video: "Generate Today's Video",
    open_output_dir: "Open Save Folder",
    language_menu: "Language",
    about: "About",
    about_window_title: "About Screen Recorder",
    about_description:
        "A lightweight screen recorder that saves timed screenshots and creates time-lapse videos.",
    about_failed_title: "Failed to Open About Window",
    version_label: "Version",
    author_label: "Author",
    license_label: "License",
    copyright_label: "Copyright",
    homepage_label: "Homepage",
    save_folder_label: "Save Folder",
    config_file_label: "Config File",
    quit: "Quit",
    status_running: "Running",
    status_paused: "Paused",
    no_screenshots: "No screenshots yet",
    tooltip_status_label: "Status",
    tooltip_interval_label: "Interval",
    tooltip_count_label: "Screenshots this session",
    tooltip_latest_label: "Latest",
    saved_capture_prefix: "Saved",
    failed_capture_prefix: "Failed",
    saved_to_prefix: "Saved to",
    saved_screenshots_prefix: "Saved ",
    saved_screenshots_suffix: " screenshots",
    partial_capture_failed_prefix: "Failed screens: ",
    partial_capture_failed_suffix: "",
    skipped_duplicate_capture: "Skipped duplicate screenshot",
    capture_success_title: "Screenshot Saved",
    capture_failed_title: "Screenshot Failed",
    output_dir_failed_title: "Failed to Open Save Folder",
    history_videos: "History Videos",
    workdirs_menu: "Work Folders...",
    workdirs_window_title: "Work Folders",
    current_workdir_label: "Current",
    next_workdir_label: "Next Launch",
    add_workdir: "Add Work Folder",
    open_workdir: "Open",
    switch_workdir: "Switch",
    delete_workdir: "Delete Folder",
    remove_workdir_record: "Remove",
    workdir_status_label: "Status",
    workdir_path_label: "Path",
    workdir_current_label: "Current",
    workdir_next_label: "Next",
    workdir_actions_label: "Actions",
    workdir_available_status: "Available",
    workdir_missing_status: "Missing",
    workdir_trashed_status: "Moved to Trash",
    workdir_deleted_status: "Deleted",
    workdir_added_prefix: "Added work folder",
    workdir_already_exists_prefix: "This work folder is already in the list",
    workdir_switch_saved_prefix: "Will use on next launch",
    workdir_removed_prefix: "Removed from the list",
    workdir_moved_to_trash_prefix: "Moved to Trash",
    workdir_permanently_deleted_prefix: "Permanently deleted",
    workdir_operation_failed_prefix: "Work folder operation failed",
    workdir_cannot_delete_current:
        "Cannot delete the current work folder or one of its parent folders.",
    workdir_delete_title: "Delete Work Folder",
    workdir_delete_prompt_prefix: "Choose how to handle this work folder:",
    workdir_delete_prompt_suffix:
        "The history record will be kept after file deletion so restored folders can be recognized.",
    move_to_trash: "Move to Trash",
    permanently_delete: "Permanently Delete",
    yes: "Yes",
    refresh: "Refresh",
    add_folder: "Add Folder",
    generate_selected: "Generate Selected",
    generation_mode: "Generation Mode",
    multi_screen_generation_mode: "Multi-screen",
    current_mode_unavailable: "Not available for this mode",
    cancel: "Cancel",
    cancelling: "Cancelling...",
    open_videos_folder: "Open Videos Folder",
    date_or_folder: "Date / Folder",
    images: "Images",
    video: "Video",
    status: "Status",
    ready: "Ready",
    unavailable: "Unavailable",
    failed: "Failed",
    no_available_images: "No available images",
    exists: "Exists",
    missing: "Missing",
    generating: "Generating",
    done: "Done",
    scanning: "Scanning images",
    processing_frames_prefix: "Processing frames",
    encoding: "Encoding video",
    finishing: "Writing video",
    selected_count_prefix: "",
    selected_count_suffix: " selected",
    frames_label: "frames",
    skipped_label: "skipped",
    output: "Output",
    selected_folders: "Selected Folders",
    no_selection: "No selection",
    video_generating_title: "Video Generation Running",
    video_generating_body: "A video generation task is already running.",
    video_success_title: "Video Generated",
    video_failed_title: "Video Generation Failed",
    config_read_failed_prefix: "Failed to read config",
    background_task_failed_title: "Background Task Failed",
    screen_capture_permission_title: "Screen Recording Permission Required",
    screen_capture_permission_body:
        "Unable to read the screen. Please allow screen recording permission in System Settings.",
};

impl Text {
    pub(crate) fn new(language: Language) -> Self {
        Self {
            bundle: bundle_for_language(language),
        }
    }

    pub(crate) fn capture_now(&self) -> &'static str {
        self.bundle.capture_now
    }

    pub(crate) fn start(&self) -> &'static str {
        self.bundle.start
    }

    pub(crate) fn pause(&self) -> &'static str {
        self.bundle.pause
    }

    pub(crate) fn interval_menu(&self, seconds: u64) -> String {
        format!("{}{seconds}s", self.bundle.interval_menu_prefix)
    }

    pub(crate) fn capture_source_menu(&self) -> &'static str {
        self.bundle.capture_source_menu
    }

    pub(crate) fn capture_source_auto(&self) -> &'static str {
        self.bundle.capture_source_auto
    }

    pub(crate) fn refresh_capture_sources(&self) -> &'static str {
        self.bundle.refresh_capture_sources
    }

    pub(crate) fn screen_label(
        &self,
        index: u32,
        is_primary: bool,
        width: u32,
        height: u32,
    ) -> String {
        let primary = if is_primary {
            format!(", {}", self.bundle.primary_screen_suffix)
        } else {
            String::new()
        };
        format!(
            "{} {index:02} ({width}x{height}{primary})",
            self.bundle.screen_label_prefix
        )
    }

    pub(crate) fn generate_today_video(&self) -> &'static str {
        self.bundle.generate_today_video
    }

    pub(crate) fn open_output_dir(&self) -> &'static str {
        self.bundle.open_output_dir
    }

    pub(crate) fn language_menu(&self) -> &'static str {
        self.bundle.language_menu
    }

    pub(crate) fn about(&self) -> &'static str {
        self.bundle.about
    }

    pub(crate) fn about_window_title(&self) -> &'static str {
        self.bundle.about_window_title
    }

    pub(crate) fn about_description(&self) -> &'static str {
        self.bundle.about_description
    }

    pub(crate) fn about_failed_title(&self) -> &'static str {
        self.bundle.about_failed_title
    }

    pub(crate) fn version_label(&self) -> &'static str {
        self.bundle.version_label
    }

    pub(crate) fn author_label(&self) -> &'static str {
        self.bundle.author_label
    }

    pub(crate) fn license_label(&self) -> &'static str {
        self.bundle.license_label
    }

    pub(crate) fn copyright_label(&self) -> &'static str {
        self.bundle.copyright_label
    }

    pub(crate) fn homepage_label(&self) -> &'static str {
        self.bundle.homepage_label
    }

    pub(crate) fn save_folder_label(&self) -> &'static str {
        self.bundle.save_folder_label
    }

    pub(crate) fn config_file_label(&self) -> &'static str {
        self.bundle.config_file_label
    }

    pub(crate) fn quit(&self) -> &'static str {
        self.bundle.quit
    }

    pub(crate) fn status_tooltip(
        &self,
        is_running: bool,
        interval: u64,
        screenshot_count: u64,
        last_capture: Option<&str>,
    ) -> String {
        let status = if is_running {
            self.bundle.status_running
        } else {
            self.bundle.status_paused
        };
        let last_capture = last_capture.unwrap_or(self.bundle.no_screenshots);
        format!(
            "{APP_NAME}\n{}: {status}\n{}: {interval}s\n{}: {screenshot_count}\n{}: {last_capture}",
            self.bundle.tooltip_status_label,
            self.bundle.tooltip_interval_label,
            self.bundle.tooltip_count_label,
            self.bundle.tooltip_latest_label
        )
    }

    pub(crate) fn saved_capture(&self, path: &Path) -> String {
        format!("{} {}", self.bundle.saved_capture_prefix, path.display())
    }

    pub(crate) fn saved_screenshots(&self, count: usize, dir: &Path) -> String {
        format!(
            "{}{count}{}: {}",
            self.bundle.saved_screenshots_prefix,
            self.bundle.saved_screenshots_suffix,
            dir.display()
        )
    }

    pub(crate) fn partial_capture_failed(&self, failed: usize, total: usize) -> String {
        format!(
            "{}{failed}/{total}{}",
            self.bundle.partial_capture_failed_prefix, self.bundle.partial_capture_failed_suffix
        )
    }

    pub(crate) fn skipped_duplicate_capture(&self) -> &'static str {
        self.bundle.skipped_duplicate_capture
    }

    pub(crate) fn failed_capture(&self, message: &str) -> String {
        format!("{}: {message}", self.bundle.failed_capture_prefix)
    }

    pub(crate) fn saved_to(&self, path: &Path) -> String {
        format!("{}: {}", self.bundle.saved_to_prefix, path.display())
    }

    pub(crate) fn capture_success_title(&self) -> &'static str {
        self.bundle.capture_success_title
    }

    pub(crate) fn capture_failed_title(&self) -> &'static str {
        self.bundle.capture_failed_title
    }

    pub(crate) fn output_dir_failed_title(&self) -> &'static str {
        self.bundle.output_dir_failed_title
    }

    pub(crate) fn history_videos(&self) -> &'static str {
        self.bundle.history_videos
    }

    pub(crate) fn workdirs_menu(&self) -> &'static str {
        self.bundle.workdirs_menu
    }

    pub(crate) fn workdirs_window_title(&self) -> &'static str {
        self.bundle.workdirs_window_title
    }

    pub(crate) fn current_workdir_label(&self) -> &'static str {
        self.bundle.current_workdir_label
    }

    pub(crate) fn next_workdir_label(&self) -> &'static str {
        self.bundle.next_workdir_label
    }

    pub(crate) fn add_workdir(&self) -> &'static str {
        self.bundle.add_workdir
    }

    pub(crate) fn open_workdir(&self) -> &'static str {
        self.bundle.open_workdir
    }

    pub(crate) fn switch_workdir(&self) -> &'static str {
        self.bundle.switch_workdir
    }

    pub(crate) fn delete_workdir(&self) -> &'static str {
        self.bundle.delete_workdir
    }

    pub(crate) fn remove_workdir_record(&self) -> &'static str {
        self.bundle.remove_workdir_record
    }

    pub(crate) fn workdir_status_label(&self) -> &'static str {
        self.bundle.workdir_status_label
    }

    pub(crate) fn workdir_path_label(&self) -> &'static str {
        self.bundle.workdir_path_label
    }

    pub(crate) fn workdir_current_label(&self) -> &'static str {
        self.bundle.workdir_current_label
    }

    pub(crate) fn workdir_next_label(&self) -> &'static str {
        self.bundle.workdir_next_label
    }

    pub(crate) fn workdir_actions_label(&self) -> &'static str {
        self.bundle.workdir_actions_label
    }

    pub(crate) fn workdir_available_status(&self) -> &'static str {
        self.bundle.workdir_available_status
    }

    pub(crate) fn workdir_missing_status(&self) -> &'static str {
        self.bundle.workdir_missing_status
    }

    pub(crate) fn workdir_trashed_status(&self) -> &'static str {
        self.bundle.workdir_trashed_status
    }

    pub(crate) fn workdir_deleted_status(&self) -> &'static str {
        self.bundle.workdir_deleted_status
    }

    pub(crate) fn workdir_added(&self, path: &Path) -> String {
        format!("{}: {}", self.bundle.workdir_added_prefix, path.display())
    }

    pub(crate) fn workdir_already_exists(&self, path: &Path) -> String {
        format!(
            "{}: {}",
            self.bundle.workdir_already_exists_prefix,
            path.display()
        )
    }

    pub(crate) fn workdir_switch_saved(&self, path: &Path) -> String {
        format!(
            "{}: {}",
            self.bundle.workdir_switch_saved_prefix,
            path.display()
        )
    }

    pub(crate) fn workdir_removed(&self, path: &Path) -> String {
        format!("{}: {}", self.bundle.workdir_removed_prefix, path.display())
    }

    pub(crate) fn workdir_moved_to_trash(&self, path: &Path) -> String {
        format!(
            "{}: {}",
            self.bundle.workdir_moved_to_trash_prefix,
            path.display()
        )
    }

    pub(crate) fn workdir_permanently_deleted(&self, path: &Path) -> String {
        format!(
            "{}: {}",
            self.bundle.workdir_permanently_deleted_prefix,
            path.display()
        )
    }

    pub(crate) fn workdir_operation_failed(&self, error: &dyn std::fmt::Display) -> String {
        format!("{}: {error}", self.bundle.workdir_operation_failed_prefix)
    }

    pub(crate) fn workdir_cannot_delete_current(&self) -> String {
        self.bundle.workdir_cannot_delete_current.to_string()
    }

    pub(crate) fn workdir_delete_title(&self) -> &'static str {
        self.bundle.workdir_delete_title
    }

    pub(crate) fn workdir_delete_prompt(&self, path: &Path) -> String {
        format!(
            "{}\n{}\n{}",
            self.bundle.workdir_delete_prompt_prefix,
            path.display(),
            self.bundle.workdir_delete_prompt_suffix
        )
    }

    pub(crate) fn move_to_trash(&self) -> &'static str {
        self.bundle.move_to_trash
    }

    pub(crate) fn permanently_delete(&self) -> &'static str {
        self.bundle.permanently_delete
    }

    pub(crate) fn yes(&self) -> &'static str {
        self.bundle.yes
    }

    pub(crate) fn refresh(&self) -> &'static str {
        self.bundle.refresh
    }

    pub(crate) fn add_folder(&self) -> &'static str {
        self.bundle.add_folder
    }

    pub(crate) fn generate_selected(&self) -> &'static str {
        self.bundle.generate_selected
    }

    pub(crate) fn generation_mode(&self) -> &'static str {
        self.bundle.generation_mode
    }

    pub(crate) fn multi_screen_generation_mode(&self) -> &'static str {
        self.bundle.multi_screen_generation_mode
    }

    pub(crate) fn generation_screen_mode(&self, index: u32) -> String {
        format!("{} {index:02}", self.bundle.screen_label_prefix)
    }

    pub(crate) fn current_mode_unavailable(&self) -> &'static str {
        self.bundle.current_mode_unavailable
    }

    pub(crate) fn cancel(&self) -> &'static str {
        self.bundle.cancel
    }

    pub(crate) fn cancelling(&self) -> &'static str {
        self.bundle.cancelling
    }

    pub(crate) fn open_videos_folder(&self) -> &'static str {
        self.bundle.open_videos_folder
    }

    pub(crate) fn date_or_folder(&self) -> &'static str {
        self.bundle.date_or_folder
    }

    pub(crate) fn images(&self) -> &'static str {
        self.bundle.images
    }

    pub(crate) fn video(&self) -> &'static str {
        self.bundle.video
    }

    pub(crate) fn status(&self) -> &'static str {
        self.bundle.status
    }

    pub(crate) fn ready(&self) -> &'static str {
        self.bundle.ready
    }

    pub(crate) fn unavailable(&self) -> &'static str {
        self.bundle.unavailable
    }

    pub(crate) fn failed(&self) -> &'static str {
        self.bundle.failed
    }

    pub(crate) fn no_available_images(&self) -> &'static str {
        self.bundle.no_available_images
    }

    pub(crate) fn exists(&self) -> &'static str {
        self.bundle.exists
    }

    pub(crate) fn missing(&self) -> &'static str {
        self.bundle.missing
    }

    pub(crate) fn generating(&self) -> &'static str {
        self.bundle.generating
    }

    pub(crate) fn done_label(&self) -> &'static str {
        self.bundle.done
    }

    pub(crate) fn scanning(&self) -> &'static str {
        self.bundle.scanning
    }

    pub(crate) fn preparing_frames(&self, current: usize, total: usize) -> String {
        format!("{} {current}/{total}", self.bundle.processing_frames_prefix)
    }

    pub(crate) fn encoding(&self) -> &'static str {
        self.bundle.encoding
    }

    pub(crate) fn finishing(&self) -> &'static str {
        self.bundle.finishing
    }

    pub(crate) fn selected_count(&self, count: usize) -> String {
        format!(
            "{}{count}{}",
            self.bundle.selected_count_prefix, self.bundle.selected_count_suffix
        )
    }

    pub(crate) fn generation_done_status(&self, frame_count: usize, skipped: usize) -> String {
        format!(
            "{}: {frame_count} {}, {} {skipped}",
            self.bundle.done, self.bundle.frames_label, self.bundle.skipped_label
        )
    }

    pub(crate) fn output(&self) -> &'static str {
        self.bundle.output
    }

    pub(crate) fn selected_folders(&self) -> &'static str {
        self.bundle.selected_folders
    }

    pub(crate) fn no_selection(&self) -> &'static str {
        self.bundle.no_selection
    }

    pub(crate) fn video_generating_title(&self) -> &'static str {
        self.bundle.video_generating_title
    }

    pub(crate) fn video_generating_body(&self) -> &'static str {
        self.bundle.video_generating_body
    }

    pub(crate) fn video_success_title(&self) -> &'static str {
        self.bundle.video_success_title
    }

    pub(crate) fn video_failed_title(&self) -> &'static str {
        self.bundle.video_failed_title
    }

    pub(crate) fn config_read_failed(&self, error: &dyn std::fmt::Display) -> String {
        format!("{}: {error}", self.bundle.config_read_failed_prefix)
    }

    pub(crate) fn background_task_failed_title(&self) -> &'static str {
        self.bundle.background_task_failed_title
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn screen_capture_permission_title(&self) -> &'static str {
        self.bundle.screen_capture_permission_title
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn screen_capture_permission_body(&self) -> &'static str {
        self.bundle.screen_capture_permission_body
    }
}

pub(crate) fn bundle_for_language(language: Language) -> &'static TextBundle {
    match language {
        Language::ZhCn => &ZH_CN_TEXT,
        Language::En => &EN_TEXT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_language_has_metadata_and_bundle() {
        for language in Language::ALL {
            assert!(!language.config_value().is_empty());
            assert!(!language.menu_label().is_empty());
            assert!(!bundle_for_language(*language).capture_now.is_empty());
            assert!(!bundle_for_language(*language).about.is_empty());
            assert!(!bundle_for_language(*language).about_window_title.is_empty());
            assert!(!bundle_for_language(*language).about_description.is_empty());
            assert!(!bundle_for_language(*language)
                .screen_capture_permission_body
                .is_empty());
        }
    }

    #[test]
    fn language_config_values_are_unique() {
        let mut values = HashSet::new();

        for language in Language::ALL {
            assert!(values.insert(language.config_value()));
        }
    }

    #[test]
    fn status_tooltip_uses_chinese_text() {
        let tooltip =
            Text::new(Language::ZhCn).status_tooltip(true, 30, 2, Some("已保存 screen.png"));

        assert!(tooltip.contains("状态: 运行中"));
        assert!(tooltip.contains("间隔: 30s"));
        assert!(tooltip.contains("本次截图: 2"));
        assert!(tooltip.contains("最近: 已保存 screen.png"));
    }

    #[test]
    fn status_tooltip_uses_english_text() {
        let tooltip =
            Text::new(Language::En).status_tooltip(false, 60, 3, Some("Saved screen.png"));

        assert!(tooltip.contains("Status: Paused"));
        assert!(tooltip.contains("Interval: 60s"));
        assert!(tooltip.contains("Screenshots this session: 3"));
        assert!(tooltip.contains("Latest: Saved screen.png"));
    }
}
