//! 任务卡片里的单个 Step 行：点击标题展开，查看该 Step 下的每个任务。
//!
//! 标题是一个扁平按钮，右侧的箭头指示展开状态；展开区用 `GtkRevealer` 播放动画。
//! 展开状态放在行模型里，任务计数变化时只更新模型、不重建控件，因此不会把用户点开的
//! 行收起。任务明细用嵌套的 `FactoryVecDeque<TaskRow>` 同步：只有展开时才创建行，列表
//! 上限由 [`super::task_view`] 控制，超出部分用一行「还有 N 个任务未显示」概括；展开区
//! 限高，任务多时内部滚动。

use std::cell::RefCell;

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::task_row::TaskRow;
use super::task_view::{StepView, TaskItemView};

/// 一个 Step 的显示行：状态点 + 名称 + 计数 + 右侧展开箭头，展开后是任务明细。
pub(crate) struct StepRow {
    view: StepView,
    /// 由用户点击标题切换。
    expanded: bool,
    /// 任务明细列表（懒创建；容器在 `view!` 里）。
    tasks: RefCell<Option<FactoryVecDeque<TaskRow>>>,
    /// 上次同步到任务行的快照。
    task_snapshot: RefCell<Vec<TaskItemView>>,
}

impl StepRow {
    pub(crate) fn new(view: StepView) -> Self {
        Self {
            view,
            expanded: false,
            tasks: RefCell::new(None),
            task_snapshot: RefCell::new(Vec::new()),
        }
    }

    fn arrow_icon(&self) -> &'static str {
        if self.expanded {
            "pan-down-symbolic"
        } else {
            "pan-start-symbolic"
        }
    }

    fn toggle_tooltip(&self) -> &'static str {
        if self.expanded {
            "收起任务列表"
        } else {
            "展开查看具体任务"
        }
    }

    fn hidden_text(&self) -> String {
        if self.view.hidden_tasks > 0 {
            format!("还有 {} 个任务未显示", self.view.hidden_tasks)
        } else {
            String::new()
        }
    }
}

/// 行自己的消息：用户切换展开状态，或父组件带来新的 Step 快照。
#[derive(Debug)]
pub(crate) enum StepRowMsg {
    Toggle,
    View(StepView),
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for StepRow {
    type Init = StepView;
    type Input = StepRowMsg;
    type Output = ();
    type CommandOutput = ();
    type ParentWidget = gtk::Box;
    type Index = DynamicIndex;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,

            gtk::Button {
                set_hexpand: true,
                add_css_class: "flat",
                add_css_class: "dmcl-step-header",
                #[watch]
                set_tooltip_text: Some(self.toggle_tooltip()),
                connect_clicked[sender] => move |_| {
                    sender.input(StepRowMsg::Toggle);
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_hexpand: true,
                    set_margin_top: 2,
                    set_margin_bottom: 2,

                    gtk::Image {
                        set_pixel_size: 16,
                        #[watch]
                        set_icon_name: Some(self.view.icon_name()),
                        #[watch]
                        set_css_classes: &self.view.icon_classes(),
                    },

                    gtk::Label {
                        set_xalign: 0.0,
                        set_hexpand: true,
                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                        #[watch]
                        set_label: &self.view.title(),
                        #[watch]
                        set_css_classes: &self.view.name_classes(),
                    },

                    gtk::Label {
                        set_xalign: 1.0,
                        #[watch]
                        set_label: &self.view.counts_text(),
                        #[watch]
                        set_css_classes: &self.view.counts_classes(),
                    },

                    gtk::Image {
                        set_pixel_size: 16,
                        add_css_class: "dim-label",
                        #[watch]
                        set_icon_name: Some(self.arrow_icon()),
                    },
                },
            },

            gtk::Revealer {
                set_transition_type: gtk::RevealerTransitionType::SlideDown,
                #[watch]
                set_reveal_child: self.expanded,

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::ScrolledWindow {
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_vscrollbar_policy: gtk::PolicyType::Automatic,
                        set_propagate_natural_height: true,
                        set_max_content_height: 240,
                        set_margin_top: 4,
                        set_margin_start: 28,
                        set_margin_end: 4,
                        set_margin_bottom: 4,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,

                            #[name(tasks_box)]
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 2,
                            },

                            // 放在滚动内容的末尾：滚到最底下才看得到。
                            gtk::Label {
                                set_xalign: 0.0,
                                set_margin_top: 4,
                                add_css_class: "caption",
                                add_css_class: "dim-label",
                                #[watch]
                                set_label: &self.hidden_text(),
                                #[watch]
                                set_visible: self.view.hidden_tasks > 0,
                            },
                        },
                    },
                },
            },
        }
    }

    fn init_model(view: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self::new(view)
    }

    fn update(&mut self, message: Self::Input, _sender: FactorySender<Self>) {
        match message {
            StepRowMsg::Toggle => self.expanded = !self.expanded,
            StepRowMsg::View(view) => self.view = view,
        }
    }

    fn post_view() {
        // 折叠时不创建任务行；展开后再按快照补齐。
        if !self.expanded {
            return;
        }
        if self.tasks.borrow().is_none() {
            let factory = FactoryVecDeque::builder()
                .launch(widgets.tasks_box.clone())
                .detach();
            *self.tasks.borrow_mut() = Some(factory);
        }
        let tasks = self.view.tasks.clone();
        if *self.task_snapshot.borrow() != tasks {
            if let Some(factory) = self.tasks.borrow_mut().as_mut() {
                let mut guard = factory.guard();
                while guard.len() > tasks.len() {
                    guard.pop_back();
                }
                for (index, task) in tasks.iter().cloned().enumerate() {
                    if let Some(row) = guard.get_mut(index) {
                        *row = TaskRow::new(task);
                    } else {
                        guard.push_back(task);
                    }
                }
            }
            *self.task_snapshot.borrow_mut() = tasks;
        }
    }
}
