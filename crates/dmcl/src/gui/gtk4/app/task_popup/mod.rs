//! 任务对话框：一个模态窗口 + 任务卡片列表（组件）。
//!
//! 任务相关的一切都在这里：对话框本身、卡片、取消、警告确认、起止提示、关闭时清理。
//! 它不知道任务是什么；事件由 [`crate::gui::gtk4::app::Shell`] 转发过来。

mod confirm;
pub(crate) mod message;
mod task_card;
mod task_view;

use std::collections::HashMap;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use dmcl_task::{CancelHandle, TaskEvent};

pub(crate) use message::TaskMsg;
/// 任务卡片的全局样式（日志区边框）。
pub(crate) use task_card::CSS;

use super::toast;
use task_card::TaskCard;

pub(crate) struct TaskCenter {
    /// 组件本身就是那个模态窗口。
    dialog: adw::Window,
    /// 任务卡片列表 / 空状态用 `GtkStack` 切换。
    stack: gtk::Stack,
    /// 任务卡片的容器（在对话框的滚动区里）。
    container: gtk::Box,
    cards: HashMap<u64, TaskCard>,
    cancels: HashMap<u64, CancelHandle>,
    /// 弹起止提示用。
    toast: gtk::Box,
    /// 主窗口：任务窗口关着时，确认框挂它。
    window: adw::ApplicationWindow,
}

impl SimpleComponent for TaskCenter {
    type Init = (adw::ApplicationWindow, gtk::Box);
    type Input = TaskMsg;
    type Output = ();
    type Widgets = ();
    type Root = adw::Window;

    fn init_root() -> Self::Root {
        // 用真正的窗口而不是 AdwDialog：AdwDialog 不能由用户拖动改尺寸。
        adw::Window::builder()
            .title("任务")
            .default_width(420)
            .default_height(520)
            .modal(true)
            .destroy_with_parent(true)
            .build()
    }

    fn init(
        (window, toast): Self::Init,
        dialog: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let container = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("view-list-symbolic")
            .title("暂无任务")
            .description("启动或安装时，任务会出现在这里")
            .build();

        let scrolled = gtk::ScrolledWindow::builder()
            .child(&container)
            .vexpand(true)
            .build();
        let stack = gtk::Stack::new();
        stack.add_named(&scrolled, Some("list"));
        stack.add_named(&empty, Some("empty"));
        stack.set_visible_child_name("empty");

        let dialog_header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&dialog_header);
        toolbar.set_content(Some(&stack));
        dialog.set_transient_for(Some(&window));
        dialog.set_content(Some(&toolbar));

        {
            let sender = sender.clone();
            let dialog = dialog.clone();
            // 关闭只是隐藏，保留窗口本身（尺寸与内容），下次打开沿用。
            dialog.connect_close_request(move |dialog| {
                sender.input(TaskMsg::ClearFinished);
                dialog.set_visible(false);
                gtk::glib::Propagation::Stop
            });
        }

        ComponentParts {
            model: TaskCenter {
                dialog,
                stack,
                container,
                cards: HashMap::new(),
                cancels: HashMap::new(),
                toast,
                window,
            },
            widgets: (),
        }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            TaskMsg::Open => self.dialog.present(),
            TaskMsg::Start { id, name, cancel } => {
                let card = TaskCard::new(id, &name, &sender);
                self.container.prepend(&card.root);
                self.cards.insert(id, card);
                self.cancels.insert(id, cancel);
                self.stack.set_visible_child_name("list");
                toast::show(&self.toast, &format!("任务「{name}」开始"));
            }
            TaskMsg::Event { id, event } => {
                // 需要用户拍板的警告：弹模态确认框（任务在 gate 上等着）。
                if let TaskEvent::Warning {
                    message, decision, ..
                } = &event
                {
                    // 确认框挂在当前最上层的窗口上：任务窗口开着就挂它（否则会被模态的
                    // 任务窗口挡住）；关着就挂主窗口，不要把任务列表强行弹出来。
                    let parent: &gtk::Window = if self.dialog.is_visible() {
                        self.dialog.upcast_ref()
                    } else {
                        self.window.upcast_ref()
                    };
                    // 确认框的标题用任务名（没有的话退回「确认」）。
                    let title = self
                        .cards
                        .get(&id)
                        .map(|card| card.name().to_string())
                        .unwrap_or_else(|| "确认".to_string());
                    confirm::ask(parent, &title, message, decision.clone());
                }
                if let Some(card) = self.cards.get_mut(&id) {
                    card.apply(&event);
                }
            }
            TaskMsg::Finished { id, ok, error } => {
                let name = self
                    .cards
                    .get(&id)
                    .map(|card| card.name().to_string())
                    .unwrap_or_default();
                if let Some(card) = self.cards.get_mut(&id) {
                    card.finish(ok, error);
                }
                self.cancels.remove(&id);
                toast::show(
                    &self.toast,
                    &if ok {
                        format!("任务「{name}」完成")
                    } else {
                        format!("任务「{name}」失败")
                    },
                );
            }
            TaskMsg::Close(id) => {
                let finished = self.cards.get(&id).is_some_and(|card| card.is_finished());
                if finished {
                    // 已结束：清除这条记录。
                    if let Some(card) = self.cards.remove(&id) {
                        self.container.remove(&card.root);
                    }
                    self.cancels.remove(&id);
                    self.stack.set_visible_child_name(if self.cards.is_empty() {
                        "empty"
                    } else {
                        "list"
                    });
                } else if let Some(cancel) = self.cancels.get(&id) {
                    // 运行中：取消。
                    cancel.cancel();
                }
            }
            TaskMsg::ClearFinished => {
                let finished: Vec<u64> = self
                    .cards
                    .iter()
                    .filter(|(_, card)| card.is_finished())
                    .map(|(id, _)| *id)
                    .collect();
                for id in finished {
                    if let Some(card) = self.cards.remove(&id) {
                        self.container.remove(&card.root);
                    }
                    self.cancels.remove(&id);
                }
                self.stack.set_visible_child_name(if self.cards.is_empty() {
                    "empty"
                } else {
                    "list"
                });
            }
        }
    }
}
