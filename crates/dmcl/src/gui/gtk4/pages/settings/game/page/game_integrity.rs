//! 游戏完整性分组：启动时如何检查游戏文件。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::GameIntegrity;
use crate::gui::gtk4::pages::settings::common::FixedRow;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct GameIntegrityGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    ignore: FixedRow,
    check: FixedRow,
    complete: FixedRow,
}

impl GameIntegrityGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let expander = adw::ExpanderRow::builder()
            .title("游戏完整性")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| {
                notify(
                    &sender,
                    GameOptionsMsg::Inherit(GameOptionGroup::GameIntegrity),
                )
            }
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }

        let ignore = FixedRow::builder("忽略").build();
        ignore.set_subtitle("不检查游戏文件");
        ignore.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::GameIntegritySelected(1))
        });
        let check = FixedRow::builder("仅检查").build();
        check.set_subtitle("在不完整时提示");
        check.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::GameIntegritySelected(2))
        });
        let complete = FixedRow::builder("检查并补全").build();
        complete.set_subtitle("自动下载缺失或损坏的文件");
        complete.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::GameIntegritySelected(3))
        });
        expander.add_row(ignore.row());
        expander.add_row(check.row());
        expander.add_row(complete.row());

        Self {
            expander,
            inherit,
            ignore,
            check,
            complete,
        }
    }
}

/// 同步对勾、继承状态与小字。
pub(super) fn sync(group: &GameIntegrityGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    let integrity = view.launch.game_integrity;
    group
        .ignore
        .set_selected(!view.inherited && integrity == GameIntegrity::Ignore);
    group
        .check
        .set_selected(!view.inherited && integrity == GameIntegrity::Check);
    group
        .complete
        .set_selected(!view.inherited && integrity == GameIntegrity::Complete);
    group
        .expander
        .set_subtitle(&view.subtitle(integrity_label(integrity)));
    if view.allow_inherit {
        group
            .inherit
            .set_subtitle(&integrity_label(view.global.game_integrity));
    }
}

/// 选中完整性处理方式；0 表示恢复继承。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    match index {
        0 => page.inherit_group(GameOptionGroup::GameIntegrity, sender),
        1..=3 => {
            let integrity = match index {
                1 => GameIntegrity::Ignore,
                2 => GameIntegrity::Check,
                _ => GameIntegrity::Complete,
            };
            if integrity == page.launch.game_integrity
                && !page.inherited(GameOptionGroup::GameIntegrity)
            {
                return;
            }
            page.launch.game_integrity = integrity;
            page.mark_override([GameOptionField::GameIntegrity]);
            page.sync_ui();
            page.schedule_flush(sender);
        }
        _ => {}
    }
}

fn integrity_label(integrity: GameIntegrity) -> String {
    match integrity {
        GameIntegrity::Ignore => "忽略",
        GameIntegrity::Check => "仅检查",
        GameIntegrity::Complete => "检查并补全",
    }
    .to_string()
}
