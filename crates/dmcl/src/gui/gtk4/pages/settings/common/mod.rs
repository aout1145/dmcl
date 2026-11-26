//! 首选项各分类共用的小组件。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

pub(crate) mod account_display;
pub(crate) mod choice_row;
pub(crate) mod drag_list;
pub(crate) mod fixed_row;
pub(crate) mod java_cache;
pub(crate) mod java_display;

pub(crate) use account_display::{account_avatar_text, account_type_label, account_username};
pub(crate) use choice_row::{ChoiceRow, ChoiceRowInit, ChoiceRowMsg, ChoiceRowOut};
pub(crate) use drag_list::CSS;
pub(crate) use fixed_row::FixedRow;
pub(crate) use java_cache::JavaProbeCache;
pub(crate) use java_display::{details_arch_text, details_title, java_summary};

/// 收起控件所在的展开行；控件不在展开行里时什么也不做。
pub(crate) fn collapse_expander(widget: &impl IsA<gtk::Widget>) {
    let Some(expander) = widget.ancestor(adw::ExpanderRow::static_type()) else {
        return;
    };
    if let Ok(expander) = expander.downcast::<adw::ExpanderRow>() {
        expander.set_expanded(false);
    }
}
