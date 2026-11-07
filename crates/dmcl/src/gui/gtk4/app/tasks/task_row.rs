//! 任务卡片里某个 Step 展开后的单个任务行。

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::task_view::TaskItemView;

/// 一个任务的显示行：名称 + 状态文字。
pub(crate) struct TaskRow {
    view: TaskItemView,
}

impl TaskRow {
    pub(crate) fn new(view: TaskItemView) -> Self {
        Self { view }
    }
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for TaskRow {
    type Init = TaskItemView;
    type Input = ();
    type Output = ();
    type CommandOutput = ();
    type ParentWidget = gtk::Box;
    type Index = DynamicIndex;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 6,
            set_margin_top: 1,
            set_margin_bottom: 1,

            gtk::Label {
                set_xalign: 0.0,
                set_hexpand: true,
                set_ellipsize: gtk::pango::EllipsizeMode::End,
                add_css_class: "caption",
                #[watch]
                set_label: &self.view.name,
            },

            gtk::Label {
                set_xalign: 1.0,
                #[watch]
                set_label: self.view.status_text(),
                #[watch]
                set_css_classes: &self.view.status_classes(),
            },
        }
    }

    fn init_model(view: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { view }
    }

    fn update(&mut self, _input: Self::Input, _sender: FactorySender<Self>) {}
}
