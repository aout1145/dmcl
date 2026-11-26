//! 自定义信息分组：传给 `--versionType`、显示在游戏主菜单里的文本。

use std::cell::Cell;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::LaunchConfig;
use crate::gui::gtk4::pages::settings::common::FixedRow;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct CustomInfoGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    entry: adw::EntryRow,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,
}

impl CustomInfoGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let syncing = Rc::new(Cell::new(false));
        let expander = adw::ExpanderRow::builder()
            .title("自定义信息")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| {
                notify(
                    &sender,
                    GameOptionsMsg::Inherit(GameOptionGroup::CustomInfo),
                )
            }
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }

        let entry = adw::EntryRow::builder().title("版本类型").build();
        entry.connect_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            move |entry| {
                if syncing.get() {
                    return;
                }
                notify(
                    &sender,
                    GameOptionsMsg::CustomInfoChanged(entry.text().to_string()),
                );
            }
        });
        expander.add_row(&entry);

        Self {
            expander,
            inherit,
            entry,
            syncing,
        }
    }
}

/// 同步输入框、继承状态与小字。
pub(super) fn sync(group: &CustomInfoGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    group
        .expander
        .set_subtitle(&view.subtitle(custom_info_label(view.launch)));
    group.inherit.set_subtitle(&custom_info_label(view.global));

    group.syncing.set(true);
    if group.entry.text() != view.launch.custom_info.as_str() {
        group.entry.set_text(&view.launch.custom_info);
    }
    group.syncing.set(false);
}

/// 自定义信息变了：标成覆盖并安排写回。
pub(super) fn changed(
    page: &mut GameOptionsPage,
    value: String,
    sender: &ComponentSender<GameOptionsPage>,
) {
    if page.launch.custom_info == value {
        return;
    }
    page.launch.custom_info = value;
    page.mark_override([GameOptionField::CustomInfo]);
    sync(&page.custom_info, page.view(GameOptionGroup::CustomInfo));
    page.schedule_flush(sender);
}

fn custom_info_label(launch: &LaunchConfig) -> String {
    if launch.custom_info.is_empty() {
        "版本类型".to_string()
    } else {
        launch.custom_info.clone()
    }
}
