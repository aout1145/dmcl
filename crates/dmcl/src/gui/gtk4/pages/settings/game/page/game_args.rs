//! 游戏参数分组：追加到游戏命令行的参数。

use std::cell::Cell;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::LaunchConfig;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, FixedRow, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct GameArgsGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    entry: adw::EntryRow,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,
}

impl GameArgsGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let syncing = Rc::new(Cell::new(false));
        let expander = adw::ExpanderRow::builder()
            .title("游戏参数")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::new("使用全局配置");
        inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::GameArgs))
        });
        if allow_inherit {
            expander.add_row(&inherit.row);
        }
        let entry = adw::EntryRow::builder().title("附加参数").build();
        entry.connect_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            move |entry| {
                if syncing.get() {
                    return;
                }
                notify(
                    &sender,
                    GameOptionsMsg::GameArgsChanged(entry.text().to_string()),
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
pub(super) fn sync(group: &GameArgsGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    group
        .expander
        .set_subtitle(&view.subtitle(game_args_label(view.launch)));
    group.inherit.set_subtitle(&game_args_label(view.global));

    group.syncing.set(true);
    if group.entry.text() != view.launch.additional_game.as_str() {
        group.entry.set_text(&view.launch.additional_game);
    }
    group.syncing.set(false);
}

/// 游戏参数文本变了：标成覆盖并安排写回。
pub(super) fn changed(
    page: &mut GameOptionsPage,
    value: String,
    sender: &ComponentSender<GameOptionsPage>,
) {
    if page.launch.additional_game == value {
        return;
    }
    page.launch.additional_game = value;
    page.mark_override([GameOptionField::AdditionalGame]);
    sync(&page.game_args, page.view(GameOptionGroup::GameArgs));
    page.schedule_flush(sender);
}

fn game_args_label(launch: &LaunchConfig) -> String {
    let args = launch.additional_game.trim();
    if args.is_empty() {
        "未设置".to_string()
    } else {
        args.to_string()
    }
}
