//! 单个任务卡片（工厂组件）：任务名、进度（运行中）或结果（已完成 / 失败）、可展开日志。
//!
//! 卡片的状态全在 [`TaskCard`] 里，视图用 `view!` 声明、`#[watch]` 刷新；父组件直接改模型
//! 并让工厂重新渲染。日志用 `GtkTextView` 只追加新行，展开时滚到最后一行；不确定进度条
//! 需要定时 `pulse()`，二者都在 `post_view` 里同步。

use std::cell::{Cell, RefCell};
use std::time::Duration;

use relm4::adw;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use dmcl_task::TaskEvent;

use super::task_view::{TaskState, TaskStatus};

/// 任务卡片日志区的样式（全局 CSS）：边框 + 比正文更小的日志字体。
pub(crate) const CSS: &str = "\
.dmcl-task-log { border: 1px solid color-mix(in srgb, currentColor 18%, transparent); border-radius: 6px; }
.dmcl-task-log-text { font-size: 0.8em; }";

/// 卡片发给任务窗口的消息。
#[derive(Debug)]
pub(crate) enum CardOut {
    /// 点右上角的叉（运行中 = 取消，已结束 = 清除）。
    Close(DynamicIndex),
}

/// 卡片自己的消息（只来自界面）。
#[derive(Debug)]
pub(crate) enum CardMsg {
    /// 日志区的展开状态变了。
    LogsToggled(bool),
}

pub(crate) struct TaskCard {
    /// 任务 id（任务窗口按它分发事件）。
    pub(crate) id: u64,
    state: TaskState,
    /// 已经写进 `GtkTextView` 的日志条数，用来只追加新日志。
    logs_rendered: Cell<usize>,
    /// 展开日志区后，等布局完成再滚到最后一行。
    scroll_to_end_pending: Cell<bool>,
    /// 不确定进度条的定时器。
    pulse: RefCell<Option<glib::SourceId>>,
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for TaskCard {
    type Init = (u64, String);
    type Input = CardMsg;
    type Output = CardOut;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;
    type Index = DynamicIndex;

    view! {
        root = gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            add_css_class: "card",
            set_margin_top: 6,
            set_margin_bottom: 6,

            gtk::Overlay {
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                    set_margin_top: 8,
                    set_margin_bottom: 12,
                    set_margin_start: 16,
                    set_margin_end: 16,

                    // 状态图标 + 任务名（右上角留给 overlay 的叉）。
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,

                        adw::Spinner {
                            #[watch]
                            set_visible: self.spinner_visible(),
                        },

                        gtk::Image {
                            set_icon_name: Some("object-select-symbolic"),
                            #[watch]
                            set_visible: self.status_icon_visible(),
                            #[watch]
                            set_icon_name: Some(self.status_icon()),
                        },

                        gtk::Label {
                            set_xalign: 0.0,
                            set_hexpand: true,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                            set_margin_end: 24,
                            add_css_class: "heading",
                            #[watch]
                            set_label: &self.state.name,
                        },
                    },

                    // 「步骤 a/b」和步骤名放在同一行。
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 6,

                        gtk::Label {
                            set_xalign: 0.0,
                            #[watch]
                            set_label: &self.overall_text(),
                            #[watch]
                            set_visible: self.overall_visible(),
                            #[watch]
                            set_css_classes: &self.overall_classes(),
                        },

                        gtk::Label {
                            set_xalign: 0.0,
                            set_hexpand: true,
                            set_ellipsize: gtk::pango::EllipsizeMode::Middle,
                            add_css_class: "caption",
                            #[watch]
                            set_label: &self.step_text(),
                            #[watch]
                            set_visible: self.step_visible(),
                        },
                    },

                    // 进度条上方的说明：百分比 / 字节 / 文件数等。
                    gtk::Label {
                        set_xalign: 0.0,
                        add_css_class: "caption",
                        add_css_class: "dim-label",
                        #[watch]
                        set_label: &self.state.progress_text(),
                        #[watch]
                        set_visible: self.progress_info_visible(),
                    },

                    gtk::Label {
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_wrap_mode: gtk::pango::WrapMode::WordChar,
                        set_selectable: true,
                        add_css_class: "caption",
                        add_css_class: "error",
                        #[watch]
                        set_label: self.state.error.as_deref().unwrap_or("失败"),
                        #[watch]
                        set_visible: self.state.status == TaskStatus::Failed,
                    },

                    #[name(progress)]
                    gtk::ProgressBar {
                        set_show_text: false,
                        #[watch]
                        set_visible: self.progress_visible(),
                        #[watch]
                        set_fraction: self.fraction(),
                    },

                    gtk::Expander {
                        set_tooltip_text: Some("日志"),
                        connect_expanded_notify[sender] => move |expander| {
                            sender.input(CardMsg::LogsToggled(expander.is_expanded()));
                        },

                        #[name(log_scroll)]
                        gtk::ScrolledWindow {
                            set_min_content_height: 96,
                            add_css_class: "dmcl-task-log",

                            #[name(log_view)]
                            gtk::TextView {
                                set_editable: false,
                                set_cursor_visible: false,
                                set_monospace: true,
                                set_wrap_mode: gtk::WrapMode::Char,
                                set_top_margin: 6,
                                set_bottom_margin: 6,
                                set_left_margin: 6,
                                set_right_margin: 6,
                                add_css_class: "dmcl-task-log-text",
                            },
                        },
                    },
                },

                // 叉用 overlay 贴住卡片右上角，不受内容内边距影响。
                add_overlay = &gtk::Button {
                    set_icon_name: "window-close-symbolic",
                    set_halign: gtk::Align::End,
                    set_valign: gtk::Align::Start,
                    set_margin_top: 4,
                    set_margin_end: 4,
                    add_css_class: "flat",
                    #[watch]
                    set_tooltip_text: Some(self.close_tooltip()),
                    #[watch]
                    set_sensitive: self.state.status != TaskStatus::Canceling,
                    connect_clicked[sender, index] => move |_| {
                        let _ = sender.output(CardOut::Close(index.clone()));
                    },
                },
            },
        }
    }

    fn init_model(
        (id, name): Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        Self {
            id,
            state: TaskState::new(name),
            logs_rendered: Cell::new(0),
            scroll_to_end_pending: Cell::new(false),
            pulse: RefCell::new(None),
        }
    }

    fn update(&mut self, message: Self::Input, _sender: FactorySender<Self>) {
        match message {
            CardMsg::LogsToggled(expanded) => {
                if expanded {
                    self.scroll_to_end_pending.set(true);
                }
            }
        }
    }

    /// `update_view` 之后同步日志文本和 pulse 定时器。
    fn post_view() {
        // 只把新增日志追加到缓冲区。
        let buffer = widgets.log_view.buffer();
        let rendered = self.logs_rendered.get();
        if rendered < self.state.logs.len() {
            if rendered == 0 {
                buffer.set_text(&self.state.logs_text());
            } else {
                let appended = self.state.logs[rendered..].join("\n");
                let mut end = buffer.end_iter();
                buffer.insert(&mut end, "\n");
                buffer.insert(&mut end, &appended);
            }
            self.logs_rendered.set(self.state.logs.len());
        }

        // 展开时等布局完成再滚到最后一行。
        if self.scroll_to_end_pending.replace(false) {
            let adjustment = widgets.log_scroll.vadjustment().clone();
            glib::idle_add_local_once(move || {
                adjustment.set_value(
                    (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
                );
            });
        }

        // 只有「运行中 + 有步骤 + 没百分比」才需要 pulse 定时器。
        let mut pulse = self.pulse.borrow_mut();
        if self.state.wants_pulse() {
            if pulse.is_none() {
                let progress = widgets.progress.clone();
                let id = glib::timeout_add_local(Duration::from_millis(120), move || {
                    progress.pulse();
                    glib::ControlFlow::Continue
                });
                *pulse = Some(id);
            }
        } else if let Some(id) = pulse.take() {
            id.remove();
        }
    }
}

impl TaskCard {
    /// 应用一条事件。日志的控件同步统一放在 `post_view` 里，只追加新行。
    pub fn apply(&mut self, event: &TaskEvent) {
        self.state.apply(event);
    }

    /// 任务结束。
    pub fn finish(&mut self, ok: bool, cancelled: bool, error: Option<String>) {
        self.state.finish(ok, cancelled, error);
    }

    /// 请求取消（运行中才生效）。
    pub fn request_cancel(&mut self) {
        self.state.request_cancel();
    }

    /// 任务名。
    pub fn name(&self) -> &str {
        &self.state.name
    }

    /// 是否已经结束（成功、失败或被取消）。
    pub fn is_finished(&self) -> bool {
        self.state.is_finished()
    }

    fn overall_text(&self) -> String {
        let (done, total) = self.state.overall();
        match self.state.status {
            TaskStatus::Canceling => {
                if total == 0 {
                    "取消中…".to_string()
                } else {
                    format!("取消中…（步骤 {done}/{total}）")
                }
            }
            TaskStatus::Done => "已完成".to_string(),
            TaskStatus::Cancelled => "已取消".to_string(),
            TaskStatus::Failed => String::new(),
            TaskStatus::Running if total == 0 => "准备中…".to_string(),
            TaskStatus::Running => format!("步骤 {done}/{total}"),
        }
    }

    fn overall_visible(&self) -> bool {
        self.state.status != TaskStatus::Failed
    }

    fn overall_classes(&self) -> Vec<&'static str> {
        match self.state.status {
            TaskStatus::Running => vec!["caption", "dim-label"],
            TaskStatus::Canceling => vec!["caption", "warning"],
            TaskStatus::Done => vec!["caption", "success"],
            TaskStatus::Cancelled | TaskStatus::Failed => vec!["caption", "dim-label"],
        }
    }

    fn step_text(&self) -> String {
        self.state.current_step().unwrap_or_default().to_string()
    }

    fn step_visible(&self) -> bool {
        matches!(
            self.state.status,
            TaskStatus::Running | TaskStatus::Canceling
        ) && self.state.current_step().is_some()
    }

    fn spinner_visible(&self) -> bool {
        matches!(
            self.state.status,
            TaskStatus::Running | TaskStatus::Canceling
        )
    }

    fn status_icon_visible(&self) -> bool {
        self.state.is_finished()
    }

    fn status_icon(&self) -> &'static str {
        match self.state.status {
            TaskStatus::Done => "object-select-symbolic",
            TaskStatus::Cancelled => "process-stop-symbolic",
            TaskStatus::Failed => "dialog-error-symbolic",
            TaskStatus::Running | TaskStatus::Canceling => "object-select-symbolic",
        }
    }

    fn progress_visible(&self) -> bool {
        matches!(
            self.state.status,
            TaskStatus::Running | TaskStatus::Canceling
        )
    }

    fn progress_info_visible(&self) -> bool {
        self.progress_visible() && !self.state.progress_text().is_empty()
    }

    fn fraction(&self) -> f64 {
        self.state
            .current_percent()
            .map(|percent| (percent / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.0)
    }

    fn close_tooltip(&self) -> &'static str {
        match self.state.status {
            TaskStatus::Running => "取消任务",
            TaskStatus::Canceling => "正在取消…",
            TaskStatus::Done | TaskStatus::Cancelled | TaskStatus::Failed => "清除这条记录",
        }
    }
}

impl Drop for TaskCard {
    fn drop(&mut self) {
        if let Some(id) = self.pulse.get_mut().take() {
            id.remove();
        }
    }
}
