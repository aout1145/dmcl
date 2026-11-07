//! 任务卡片里的单个 Step 行。

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::task_view::StepView;

/// 一个 Step 的显示行：状态点 + 名称 + 计数。
pub(crate) struct StepRow {
    view: StepView,
}

impl StepRow {
    pub(crate) fn new(view: StepView) -> Self {
        Self { view }
    }
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for StepRow {
    type Init = StepView;
    type Input = ();
    type Output = ();
    type CommandOutput = ();
    type ParentWidget = gtk::Box;
    type Index = DynamicIndex;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 8,
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
                add_css_class: "caption",
                #[watch]
                set_label: &self.view.counts_text(),
                #[watch]
                set_css_classes: &self.view.icon_classes(),
            },
        }
    }

    fn init_model(view: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { view }
    }

    fn update(&mut self, _input: Self::Input, _sender: FactorySender<Self>) {}
}
