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

use super::ToastService;
use task_card::{CardOut, TaskCard};

/// 任务窗口发给 root 的消息。
#[derive(Debug)]
pub(crate) enum TaskCenterOut {
    /// 当前运行中（含取消中）的任务数，用来更新标题栏角标。
    RunningCount(usize),
    /// 可以退出主窗口了（没有任务，或用户确认取消任务）。
    Quit,
}

pub(crate) struct TaskCenter {
    /// 组件本身就是那个模态窗口。
    dialog: adw::Window,
    /// 任务卡片列表；空列表时 `GtkStack` 切到空状态。
    cards: FactoryVecDeque<TaskCard>,
    cancels: HashMap<u64, CancelHandle>,
    /// 弹起止提示用。
    toasts: ToastService,
    /// 主窗口：任务窗口关着时，确认框挂它。
    window: adw::ApplicationWindow,
    /// 上次上报给标题栏的未结束任务数，只在变化时发消息。
    reported_running: usize,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for TaskCenter {
    type Init = (adw::ApplicationWindow, ToastService);
    type Input = TaskMsg;
    type Output = TaskCenterOut;

    view! {
        adw::Window {
            set_title: Some("任务"),
            set_default_width: 640,
            set_default_height: 720,
            set_modal: true,
            set_destroy_with_parent: true,

            // 关闭后隐藏，保留窗口本身（尺寸与内容），下次打开沿用；任务继续运行。
            // 取消任务由关闭主窗口时的确认流程负责。
            connect_close_request[sender] => move |_| {
                sender.input(TaskMsg::CloseRequested);
                gtk::glib::Propagation::Stop
            },

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {},

                #[wrap(Some)]
                set_content = &gtk::Stack {
                    add_named[Some("list")] = &gtk::ScrolledWindow {
                        set_vexpand: true,

                        #[local_ref]
                        cards_box -> gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 6,
                            set_margin_top: 6,
                            set_margin_bottom: 6,
                            set_margin_start: 6,
                            set_margin_end: 6,
                        },
                    },

                    add_named[Some("empty")] = &adw::StatusPage {
                        set_icon_name: Some("view-list-symbolic"),
                        set_title: "暂无任务",
                        set_description: Some("启动或安装时，任务会出现在这里"),
                    },

                    #[watch]
                    set_visible_child_name: if model.cards.is_empty() { "empty" } else { "list" },
                },
            },
        }
    }

    fn init(
        (window, toasts): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cards = FactoryVecDeque::builder()
            .launch(gtk::Box::new(gtk::Orientation::Vertical, 6))
            .forward(sender.input_sender(), |out| match out {
                CardOut::Close(index) => TaskMsg::CloseCard(index),
            });

        root.set_transient_for(Some(&window));

        let model = TaskCenter {
            dialog: root.clone(),
            cards,
            cancels: HashMap::new(),
            toasts,
            window,
            reported_running: 0,
        };

        let cards_box = model.cards.widget();
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            TaskMsg::Open => self.dialog.present(),
            TaskMsg::Start { id, name, cancel } => {
                self.cards.guard().push_front((id, name.clone()));
                self.cancels.insert(id, cancel);
                self.toasts.global_toast(&format!("任务「{name}」开始"));
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
                        .iter()
                        .find(|card| card.id == id)
                        .map(|card| card.name().to_string())
                        .unwrap_or_else(|| "确认".to_string());
                    confirm::ask(parent, &title, message, decision.clone());
                }
                let index = self.cards.iter().position(|card| card.id == id);
                if let Some(index) = index {
                    let mut guard = self.cards.guard();
                    if let Some(card) = guard.get_mut(index) {
                        card.apply(&event);
                    }
                }
            }
            TaskMsg::Finished {
                id,
                ok,
                cancelled,
                error,
            } => {
                let index = self.cards.iter().position(|card| card.id == id);
                let name = index
                    .and_then(|index| self.cards.get(index))
                    .map(|card| card.name().to_string())
                    .unwrap_or_default();

                if let Some(index) = index {
                    let mut guard = self.cards.guard();
                    if let Some(card) = guard.get_mut(index) {
                        card.finish(ok, cancelled, error);
                    }
                }
                self.cancels.remove(&id);
                self.toasts.global_toast(&if cancelled {
                    format!("任务「{name}」已取消")
                } else if ok {
                    format!("任务「{name}」完成")
                } else {
                    format!("任务「{name}」失败")
                });
            }
            TaskMsg::CloseCard(index) => {
                let index = index.current_index();
                if let Some(card) = self.cards.get(index) {
                    let id = card.id;

                    if card.is_finished() {
                        // 已结束：清除这条记录。
                        self.cards.guard().remove(index);
                        self.cancels.remove(&id);
                    } else {
                        // 运行中：取消，并立刻进入「取消中」。
                        if let Some(cancel) = self.cancels.get(&id) {
                            cancel.cancel();
                        }
                        let mut guard = self.cards.guard();
                        if let Some(card) = guard.get_mut(index) {
                            card.request_cancel();
                        }
                    }
                }
            }
            TaskMsg::CloseRequested => self.hide(),
            TaskMsg::CloseMain => {
                if self.cards.iter().any(|card| !card.is_finished()) {
                    self.confirm_quit(&sender);
                } else {
                    let _ = sender.output(TaskCenterOut::Quit);
                }
            }
            TaskMsg::CancelAllAndQuit => {
                self.cancel_unfinished();
                let _ = sender.output(TaskCenterOut::Quit);
            }
        }

        let running = self.cards.iter().filter(|card| !card.is_finished()).count();
        if running != self.reported_running {
            self.reported_running = running;
            let _ = sender.output(TaskCenterOut::RunningCount(running));
        }
    }
}

impl TaskCenter {
    /// 隐藏窗口，并清掉成功的任务卡片。
    fn hide(&mut self) {
        self.dialog.set_visible(false);
        self.clear_succeeded();
    }

    /// 关闭主窗口时还有任务在跑：确认后取消任务并退出。
    fn confirm_quit(&self, sender: &ComponentSender<Self>) {
        let dialog = adw::AlertDialog::new(
            Some("任务仍在运行"),
            Some("关闭主窗口会取消正在运行的任务。确定退出吗？"),
        );
        dialog.add_response("cancel", "取消");
        dialog.add_response("quit", "取消任务并退出");
        dialog.set_response_appearance("quit", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "quit" {
                sender.input(TaskMsg::CancelAllAndQuit);
            }
        });

        // 任务窗口开着时挂它，否则会被模态的任务窗口挡住。
        let parent: &gtk::Window = if self.dialog.is_visible() {
            self.dialog.upcast_ref()
        } else {
            self.window.upcast_ref()
        };
        dialog.present(Some(parent));
    }

    /// 给所有还没结束的卡片发取消请求；已经结束的不动。
    fn cancel_unfinished(&mut self) {
        let unfinished: Vec<(usize, u64)> = self
            .cards
            .iter()
            .enumerate()
            .filter(|(_, card)| !card.is_finished())
            .map(|(index, card)| (index, card.id))
            .collect();

        for (index, id) in unfinished {
            if let Some(cancel) = self.cancels.get(&id) {
                cancel.cancel();
            }
            let mut guard = self.cards.guard();
            if let Some(card) = guard.get_mut(index) {
                card.request_cancel();
            }
        }
    }

    /// 去掉所有成功的卡片（关闭窗口时用），失败、已取消和运行中的保留。
    fn clear_succeeded(&mut self) {
        let mut removed = Vec::new();
        {
            let mut guard = self.cards.guard();
            for index in (0..guard.len()).rev() {
                if guard.get(index).is_some_and(TaskCard::is_succeeded) {
                    if let Some(card) = guard.remove(index) {
                        removed.push(card.id);
                    }
                }
            }
        }
        for id in removed {
            self.cancels.remove(&id);
        }
    }
}
