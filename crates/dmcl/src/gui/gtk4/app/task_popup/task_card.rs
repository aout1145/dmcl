//! 单个任务卡片：任务名、进度（运行中）或结果（已完成 / 失败）、可展开的日志。

use std::time::Duration;

use relm4::adw;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use dmcl_task::TaskEvent;

use super::task_view::{TaskState, TaskStatus};
use super::{TaskCenter, TaskMsg};

/// 任务卡片日志区的样式（全局 CSS）：给一个明确的边框。
pub(crate) const CSS: &str = "\
.dmcl-task-log { border: 1px solid color-mix(in srgb, currentColor 18%, transparent); border-radius: 6px; }";

pub(crate) struct TaskCard {
    pub root: gtk::Box,
    state: TaskState,
    /// 运行中：转圈的 `AdwSpinner`。
    spinner: adw::Spinner,
    /// 已结束：对勾 / 错误图标。
    status_icon: gtk::Image,
    name: gtk::Label,
    overall: gtk::Label,
    current: gtk::Label,
    reason: gtk::Label,
    progress: gtk::ProgressBar,
    logs: gtk::Label,
    close: gtk::Button,
    /// 不确定进度条用的定时器。
    pulse: Option<glib::SourceId>,
}

impl TaskCard {
    pub fn new(id: u64, name: &str, sender: &ComponentSender<TaskCenter>) -> Self {
        let spinner = adw::Spinner::new();
        let status_icon = gtk::Image::from_icon_name("object-select-symbolic");
        status_icon.set_visible(false);

        let name_label = gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        name_label.add_css_class("heading");
        // 右上角留给 overlay 的叉。
        name_label.set_margin_end(24);

        // 右上角：叉。运行中 = 取消；已结束 = 清除这条记录（由 TaskCenter 判断）。
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("取消任务")
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(4)
            .margin_end(4)
            .build();
        close.add_css_class("flat");
        {
            let sender = sender.clone();
            close.connect_clicked(move |_| sender.input(TaskMsg::Close(id)));
        }

        let top = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        top.append(&spinner);
        top.append(&status_icon);
        top.append(&name_label);

        let overall = gtk::Label::builder().xalign(0.0).build();
        overall.add_css_class("caption");
        overall.add_css_class("dim-label");

        let current = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        current.add_css_class("caption");

        let reason = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .selectable(true)
            .build();
        reason.add_css_class("caption");
        reason.add_css_class("error");
        reason.set_visible(false);

        let progress = gtk::ProgressBar::builder().show_text(false).build();

        let logs = gtk::Label::builder()
            .xalign(0.0)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .build();
        logs.set_margin_top(6);
        logs.set_margin_bottom(6);
        logs.set_margin_start(6);
        logs.set_margin_end(6);
        logs.add_css_class("monospace");
        logs.add_css_class("caption");
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&logs)
            .min_content_height(96)
            .build();
        scrolled.add_css_class("dmcl-task-log");
        let expander = gtk::Expander::builder()
            .label("日志")
            .child(&scrolled)
            .build();

        // 卡片内容留出内边距（上方不要太厚，左右给足）。
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(8)
            .margin_bottom(12)
            .margin_start(16)
            .margin_end(16)
            .build();
        inner.append(&top);
        inner.append(&overall);
        inner.append(&current);
        inner.append(&reason);
        inner.append(&progress);
        inner.append(&expander);

        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        root.add_css_class("card");
        root.set_margin_top(6);
        root.set_margin_bottom(6);

        // 叉用 overlay 贴住卡片右上角，不受内容内边距影响。
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&inner));
        overlay.add_overlay(&close);
        root.append(&overlay);

        let mut card = Self {
            root,
            state: TaskState::new(name),
            spinner,
            status_icon,
            name: name_label,
            overall,
            current,
            reason,
            progress,
            logs,
            close,
            pulse: None,
        };
        card.refresh();
        card
    }

    /// 应用一条事件并刷新界面。
    pub fn apply(&mut self, event: &TaskEvent) {
        self.state.apply(event);
        self.refresh();
    }

    /// 任务结束。
    pub fn finish(&mut self, ok: bool, error: Option<String>) {
        self.state.finish(ok, error);
        self.refresh();
    }

    /// 任务名。
    pub fn name(&self) -> &str {
        &self.state.name
    }

    /// 是否已经结束（成功或失败）。
    pub fn is_finished(&self) -> bool {
        self.state.status != TaskStatus::Running
    }

    fn refresh(&mut self) {
        self.name.set_label(&self.state.name);

        match self.state.status {
            TaskStatus::Running => {
                self.spinner.set_visible(true);
                self.status_icon.set_visible(false);
            }
            TaskStatus::Done => {
                self.spinner.set_visible(false);
                self.status_icon.set_visible(true);
                self.status_icon
                    .set_icon_name(Some("object-select-symbolic"));
            }
            TaskStatus::Failed => {
                self.spinner.set_visible(false);
                self.status_icon.set_visible(true);
                self.status_icon
                    .set_icon_name(Some("dialog-error-symbolic"));
            }
        }

        self.overall.remove_css_class("success");
        self.overall.remove_css_class("error");
        self.overall.remove_css_class("dim-label");

        self.close
            .set_tooltip_text(Some(if self.state.status == TaskStatus::Running {
                "取消任务"
            } else {
                "清除这条记录"
            }));

        match self.state.status {
            TaskStatus::Running => self.refresh_running(),
            TaskStatus::Done => {
                self.stop_pulse();
                self.progress.set_visible(false);
                self.current.set_visible(false);
                self.reason.set_visible(false);

                self.overall.set_visible(true);
                self.overall.add_css_class("success");
                self.overall.set_label("已完成");
            }
            TaskStatus::Failed => {
                self.stop_pulse();
                self.progress.set_visible(false);
                self.overall.set_visible(false);
                // 任务链的错误文本里已经带了出错的步骤，直接原样显示。
                self.current.set_visible(false);
                self.reason.set_visible(true);
                self.reason
                    .set_label(self.state.error.as_deref().unwrap_or("失败"));
            }
        }

        self.logs.set_label(&self.state.logs_text());
    }

    /// 运行中：总进度 + 当前步骤 + 进度条。
    fn refresh_running(&mut self) {
        self.reason.set_visible(false);
        self.progress.set_visible(true);

        let (done, total) = self.state.overall();
        self.overall.set_visible(true);
        self.overall.add_css_class("dim-label");
        self.overall.set_label(&if total == 0 {
            "准备中…".to_string()
        } else {
            format!("步骤 {done}/{total}")
        });

        let step = self.state.current_step().map(str::to_string);
        self.current.set_visible(true);
        self.current.set_label(step.as_deref().unwrap_or(""));

        match self.state.current_percent() {
            Some(percent) => {
                self.stop_pulse();
                self.progress.set_show_text(true);
                self.progress
                    .set_fraction((percent / 100.0).clamp(0.0, 1.0));
            }
            None => {
                self.progress.set_show_text(false);
                if step.is_some() {
                    self.start_pulse();
                } else {
                    self.stop_pulse();
                    self.progress.set_fraction(0.0);
                }
            }
        }
    }

    /// 没有百分比时用 pulse 动画当「不确定进度条」。
    fn start_pulse(&mut self) {
        if self.pulse.is_some() {
            return;
        }
        let progress = self.progress.clone();
        let id = glib::timeout_add_local(Duration::from_millis(120), move || {
            progress.pulse();
            glib::ControlFlow::Continue
        });
        self.pulse = Some(id);
    }

    fn stop_pulse(&mut self) {
        if let Some(id) = self.pulse.take() {
            id.remove();
        }
    }
}
