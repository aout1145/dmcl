//! 展开式选择行里的一项。
//!
//! 账户、Java 和窗口大小等选择器都用 `AdwExpanderRow`：收起时显示当前值，展开后列出全部
//! 选项。每一项是一行 `AdwActionRow`：主标题、下方小字和右侧信息都由调用方给出，
//! 最右侧用对勾表示当前选择。账户项可以在行首带一个占位头像。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::factory::FactoryView;
use relm4::gtk;
use relm4::prelude::*;

use super::collapse_expander;

/// 一行可选项。
pub(crate) struct ChoiceRow {
    index: u32,
    title: String,
    subtitle: String,
    detail: String,
    /// 行首占位头像的文字；`None` 表示不显示头像。
    avatar_text: Option<String>,
    selected: bool,
}

/// 新建一行可选项。
pub(crate) struct ChoiceRowInit {
    pub index: u32,
    pub title: String,
    pub subtitle: String,
    /// 右侧小字，例如 Java 的供应商与架构。
    pub detail: String,
    /// 行首占位头像的文字；`None` 表示不显示头像。
    pub avatar_text: Option<String>,
    pub selected: bool,
}

/// 选择行的内部消息。
#[derive(Debug)]
pub(crate) enum ChoiceRowMsg {
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
pub(crate) enum ChoiceRowOut {
    /// 用户选中了下标为 `index` 的项。
    Selected(u32),
}

#[relm4::factory(pub(crate))]
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

            connect_activated[sender] => move |row| {
                collapse_expander(row);
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
            avatar_text: init.avatar_text,
            selected: init.selected,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &Self::Index,
        root: Self::Root,
        _returned_widget: &<Self::ParentWidget as FactoryView>::ReturnedWidget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        // 只有账户行有头像。`AdwActionRow` 会给任何前缀留出缩进，空头像也会让整行
        // 比「使用全局配置」行更靠右，所以没有头像时干脆不挂前缀。
        if let Some(text) = &self.avatar_text {
            let avatar = adw::Avatar::new(32, Some(text.as_str()), true);
            avatar.set_valign(gtk::Align::Center);
            root.add_prefix(&avatar);
        }
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
