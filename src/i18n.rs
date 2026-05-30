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
    generate_today_video: &'static str,
    open_output_dir: &'static str,
    language_menu: &'static str,
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
    capture_success_title: &'static str,
    capture_failed_title: &'static str,
    output_dir_failed_title: &'static str,
    video_generating_title: &'static str,
    video_generating_body: &'static str,
    video_success_title: &'static str,
    video_failed_title: &'static str,
    config_read_failed_prefix: &'static str,
    background_task_failed_title: &'static str,
    screen_capture_permission_title: &'static str,
    screen_capture_permission_body: &'static str,
}

const ZH_CN_TEXT: TextBundle = TextBundle {
    capture_now: "📷 截一张",
    start: "▶ 开始",
    pause: "⏸ 暂停",
    interval_menu_prefix: "⏱ 间隔 当前：",
    generate_today_video: "🎬 生成今日视频",
    open_output_dir: "📁 打开保存目录",
    language_menu: "🌐 语言",
    quit: "❌ 退出",
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
    capture_success_title: "截图成功",
    capture_failed_title: "截图失败",
    output_dir_failed_title: "打开保存目录失败",
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
    capture_now: "📷 Capture Now",
    start: "▶ Start",
    pause: "⏸ Pause",
    interval_menu_prefix: "⏱ Interval: ",
    generate_today_video: "🎬 Generate Today's Video",
    open_output_dir: "📁 Open Save Folder",
    language_menu: "🌐 Language",
    quit: "❌ Quit",
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
    capture_success_title: "Screenshot Saved",
    capture_failed_title: "Screenshot Failed",
    output_dir_failed_title: "Failed to Open Save Folder",
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

    pub(crate) fn generate_today_video(&self) -> &'static str {
        self.bundle.generate_today_video
    }

    pub(crate) fn open_output_dir(&self) -> &'static str {
        self.bundle.open_output_dir
    }

    pub(crate) fn language_menu(&self) -> &'static str {
        self.bundle.language_menu
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

    pub(crate) fn screen_capture_permission_title(&self) -> &'static str {
        self.bundle.screen_capture_permission_title
    }

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
