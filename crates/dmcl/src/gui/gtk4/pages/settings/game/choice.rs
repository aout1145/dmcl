//! 展开式选择行里的一项。
//!
//! 账户和 Java 都用 `AdwExpanderRow` 作为选择器：收起时显示当前值，展开后列出全部
//! 选项。每一项是一行 `AdwActionRow`：主标题、下方小字和右侧信息都由调用方给出，
//! 最右侧用对勾表示当前选择。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::factory::FactoryView;
use relm4::gtk;
use relm4::prelude::*;

/// 一行可选项。
pub(super) struct ChoiceRow {
    index: u32,
    title: String,
    subtitle: String,
    detail: String,
    selected: bool,
}

/// 新建一行可选项。
pub(super) struct ChoiceRowInit {
    pub index: u32,
    pub title: String,
    pub subtitle: String,
    /// 右侧小字，例如 Java 的供应商与架构。
    pub detail: String,
    pub selected: bool,
}

/// 选择行的内部消息。
#[derive(Debug)]
pub(super) enum ChoiceRowMsg {
    /// 当前项是否是选中项。
    SetSelected(bool),
    /// 探测结果更新后改写文字。
    SetInfo {
        title: String,
        subtitle: String,
        detail: String,
    },
}

/// 选择行发出的动作。
#[derive(Debug)]
pub(super) enum ChoiceRowOut {
    /// 用户选中了下标为 `index` 的项。
    Selected(u32),
}

#[relm4::factory(pub(super))]
impl FactoryComponent for ChoiceRow {
    type ParentWidget = adw::ExpanderRow;
    type Init = ChoiceRowInit;
    type Input = ChoiceRowMsg;
    type Output = ChoiceRowOut;
    type CommandOutput = ();
    type Index = DynamicIndex;

    view! {
        adw::ActionRow {
            #[watch]
            set_title: &self.title,
            #[watch]
            set_subtitle: &self.subtitle,
            set_subtitle_lines: 1,
            set_subtitle_selectable: false,
            set_activatable: true,

            add_suffix = &gtk::Label {
                #[watch]
                set_label: &self.detail,
                #[watch]
                set_visible: !self.detail.is_empty(),
                add_css_class: "dim-label",
                add_css_class: "caption",
                set_valign: gtk::Align::Center,
            },

            add_suffix = &gtk::Image {
                set_icon_name: Some("object-select-symbolic"),
                #[watch]
                set_visible: self.selected,
                add_css_class: "accent",
                set_valign: gtk::Align::Center,
            },

            connect_activated[sender] => move |_| {
                let _ = sender.output(ChoiceRowOut::Selected(index));
            },
        }
    }

    fn init_model(init: Self::Init, _index: &Self::Index, _sender: FactorySender<Self>) -> Self {
        Self {
            index: init.index,
            title: init.title,
            subtitle: init.subtitle,
            detail: init.detail,
            selected: init.selected,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &Self::Index,
        _root: Self::Root,
        _returned_widget: &<Self::ParentWidget as FactoryView>::ReturnedWidget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let index = self.index;
        let widgets = view_output!();
        widgets
    }

    fn update(&mut self, message: Self::Input, _sender: FactorySender<Self>) {
        match message {
            ChoiceRowMsg::SetSelected(selected) => self.selected = selected,
            ChoiceRowMsg::SetInfo {
                title,
                subtitle,
                detail,
            } => {
                self.title = title;
                self.subtitle = subtitle;
                self.detail = detail;
            }
        }
    }
}
