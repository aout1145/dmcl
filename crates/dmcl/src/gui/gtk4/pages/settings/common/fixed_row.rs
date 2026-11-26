//! 展开行里的固定选项行：主标题、小字和右侧对勾。
//!
//! 默认在激活后收起所在的展开行；带输入框等内联控件的行用
//! [`FixedRowBuilder::keep_open`] 保持展开。展开行的查找交给
//! [`collapse_expander`](super::collapse_expander)，行不在展开行里时不会出错。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use super::collapse_expander;

/// 一行固定选项。
pub(crate) struct FixedRow {
    row: adw::ActionRow,
    mark: gtk::Image,
}

/// [`FixedRow`] 的构造器。
#[must_use]
pub(crate) struct FixedRowBuilder {
    title: String,
    suffix: Option<gtk::Widget>,
    collapse: bool,
}

impl FixedRow {
    pub(crate) fn builder(title: &str) -> FixedRowBuilder {
        FixedRowBuilder {
            title: title.to_string(),
            suffix: None,
            collapse: true,
        }
    }

    pub(crate) fn row(&self) -> &adw::ActionRow {
        &self.row
    }

    pub(crate) fn set_selected(&self, selected: bool) {
        self.mark.set_visible(selected);
    }

    pub(crate) fn set_subtitle(&self, subtitle: &str) {
        self.row.set_subtitle(subtitle);
    }
}

impl FixedRowBuilder {
    /// 在标题与对勾之间放一个控件。
    pub(crate) fn suffix<T: IsA<gtk::Widget> + Clone>(mut self, suffix: &T) -> Self {
        self.suffix = Some(suffix.clone().upcast());
        self
    }

    /// 选中后保持展开，给带内联输入的行用。
    pub(crate) fn keep_open(mut self) -> Self {
        self.collapse = false;
        self
    }

    pub(crate) fn build(self) -> FixedRow {
        let row = adw::ActionRow::builder()
            .title(&self.title)
            .activatable(true)
            .build();
        if let Some(suffix) = &self.suffix {
            row.add_suffix(suffix);
        }
        let mark = gtk::Image::from_icon_name("object-select-symbolic");
        mark.add_css_class("accent");
        mark.set_valign(gtk::Align::Center);
        row.add_suffix(&mark);
        if self.collapse {
            row.connect_activated(collapse_expander);
        }
        FixedRow { row, mark }
    }
}
