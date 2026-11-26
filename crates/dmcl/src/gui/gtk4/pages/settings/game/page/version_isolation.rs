//! 版本隔离分组：每个版本是否使用独立的游戏目录。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::gui::gtk4::pages::settings::common::FixedRow;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct VersionIsolationGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    on: FixedRow,
    off: FixedRow,
}

impl VersionIsolationGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let expander = adw::ExpanderRow::builder()
            .title("版本隔离")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| {
                notify(
                    &sender,
                    GameOptionsMsg::Inherit(GameOptionGroup::VersionIsolation),
                )
            }
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }
        let on = FixedRow::builder("开启").build();
        on.set_subtitle("每个版本使用独立的游戏目录");
        on.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::VersionIsolationSelected(1))
        });
        let off = FixedRow::builder("关闭").build();
        off.set_subtitle("共用 Minecraft 根目录");
        off.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::VersionIsolationSelected(2))
        });
        expander.add_row(on.row());
        expander.add_row(off.row());

        Self {
            expander,
            inherit,
            on,
            off,
        }
    }
}

/// 同步对勾、继承状态与小字。
pub(super) fn sync(group: &VersionIsolationGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    group
        .on
        .set_selected(!view.inherited && view.launch.version_isolation);
    group
        .off
        .set_selected(!view.inherited && !view.launch.version_isolation);
    group
        .expander
        .set_subtitle(&view.subtitle(isolation_label(view.launch.version_isolation)));
    if view.allow_inherit {
        group
            .inherit
            .set_subtitle(&isolation_label(view.global.version_isolation));
    }
}

/// 选中开启 / 关闭；0 表示恢复继承。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    match index {
        0 => page.inherit_group(GameOptionGroup::VersionIsolation, sender),
        1 | 2 => {
            let value = index == 1;
            if value == page.launch.version_isolation
                && !page.inherited(GameOptionGroup::VersionIsolation)
            {
                return;
            }
            page.launch.version_isolation = value;
            page.mark_override([GameOptionField::VersionIsolation]);
            page.sync_ui();
            page.schedule_flush(sender);
        }
        _ => {}
    }
}

fn isolation_label(enabled: bool) -> String {
    if enabled {
        "开启".to_string()
    } else {
        "关闭".to_string()
    }
}
