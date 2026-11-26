//! 快速游玩分组：无 / 单人游戏 / 多人联机 / Realms，每个目标带一个输入框。

use std::cell::Cell;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use crate::config::{LaunchConfig, QuickPlayConfig, QuickPlayTarget};
use crate::gui::gtk4::pages::settings::common::FixedRow;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct QuickPlayGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    none: FixedRow,
    single: FixedRow,
    multi: FixedRow,
    realms: FixedRow,
    single_entry: gtk::Entry,
    multi_entry: gtk::Entry,
    realms_entry: gtk::Entry,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,
}

impl QuickPlayGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let syncing = Rc::new(Cell::new(false));
        let expander = adw::ExpanderRow::builder()
            .title("快速游玩")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::QuickPlay))
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }

        let none = FixedRow::builder("无").build();
        none.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::QuickPlaySelected(0))
        });
        let (single, single_entry) =
            quick_play_row("单人游戏", "世界文件夹名", 1, sender, &syncing);
        let (multi, multi_entry) = quick_play_row("多人联机", "服务器地址", 2, sender, &syncing);
        let (realms, realms_entry) = quick_play_row("Realms", "Realm ID", 3, sender, &syncing);
        expander.add_row(none.row());
        expander.add_row(single.row());
        expander.add_row(multi.row());
        expander.add_row(realms.row());

        Self {
            expander,
            inherit,
            none,
            single,
            multi,
            realms,
            single_entry,
            multi_entry,
            realms_entry,
            syncing,
        }
    }

    /// 某个快速游玩目标对应的输入框。
    fn entry(&self, target: QuickPlayTarget) -> &gtk::Entry {
        match target {
            QuickPlayTarget::Singleplayer => &self.single_entry,
            QuickPlayTarget::Multiplayer => &self.multi_entry,
            QuickPlayTarget::Realms => &self.realms_entry,
        }
    }
}

/// 同步对勾、当前目标的输入框、继承状态与小字。
pub(super) fn sync(group: &QuickPlayGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    let target = view.launch.quick_play.as_ref().map(|quick| quick.target);
    group.none.set_selected(!view.inherited && target.is_none());
    group
        .single
        .set_selected(!view.inherited && target == Some(QuickPlayTarget::Singleplayer));
    group
        .multi
        .set_selected(!view.inherited && target == Some(QuickPlayTarget::Multiplayer));
    group
        .realms
        .set_selected(!view.inherited && target == Some(QuickPlayTarget::Realms));
    group
        .expander
        .set_subtitle(&view.subtitle(quick_play_label(view.launch)));
    group.inherit.set_subtitle(&quick_play_label(view.global));

    if let Some(quick) = &view.launch.quick_play {
        let entry = group.entry(quick.target);
        if entry.text() != quick.value.as_str() {
            group.syncing.set(true);
            entry.set_text(&quick.value);
            group.syncing.set(false);
        }
    }
}

/// 选中某个目标：读取该行输入框的值；选「无」时清空配置。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    let inherited = page.inherited(GameOptionGroup::QuickPlay);
    if index == 0 {
        if page.launch.quick_play.is_none() && !inherited {
            return;
        }
        page.launch.quick_play = None;
        page.mark_override([GameOptionField::QuickPlay]);
        page.sync_ui();
        page.schedule_flush(sender);
        return;
    }
    let Some(target) = quick_play_target(index) else {
        return;
    };
    if !inherited
        && page
            .launch
            .quick_play
            .as_ref()
            .is_some_and(|quick| quick.target == target)
    {
        return;
    }
    let value = page.quick_play.entry(target).text().to_string();
    page.launch.quick_play = Some(QuickPlayConfig { target, value });
    page.mark_override([GameOptionField::QuickPlay]);
    page.sync_ui();
    page.schedule_flush(sender);
}

/// 某个目标的输入框改了文本：自动选中该目标。
pub(super) fn value_changed(
    page: &mut GameOptionsPage,
    index: u32,
    value: String,
    sender: &ComponentSender<GameOptionsPage>,
) {
    let Some(target) = quick_play_target(index) else {
        return;
    };
    let current = page.launch.quick_play.as_ref();
    if current.is_some_and(|quick| quick.target == target && quick.value == value) {
        return;
    }
    page.launch.quick_play = Some(QuickPlayConfig { target, value });
    page.mark_override([GameOptionField::QuickPlay]);
    sync(&page.quick_play, page.view(GameOptionGroup::QuickPlay));
    page.schedule_flush(sender);
}

/// 快速游玩的一个目标行：右侧输入框 + 选中对勾。
fn quick_play_row(
    title: &str,
    placeholder: &str,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
    syncing: &Rc<Cell<bool>>,
) -> (FixedRow, gtk::Entry) {
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .width_chars(18)
        .valign(gtk::Align::Center)
        .build();
    entry.connect_changed({
        let sender = sender.clone();
        let syncing = Rc::clone(syncing);
        move |entry| {
            if syncing.get() {
                return;
            }
            notify(
                &sender,
                GameOptionsMsg::QuickPlayValueChanged(index, entry.text().to_string()),
            );
        }
    });

    let row = FixedRow::builder(title).suffix(&entry).keep_open().build();
    row.row().connect_activated({
        let sender = sender.clone();
        move |_| notify(&sender, GameOptionsMsg::QuickPlaySelected(index))
    });
    (row, entry)
}

/// 下拉下标 → 快速游玩目标；0 是无。
fn quick_play_target(index: u32) -> Option<QuickPlayTarget> {
    match index {
        1 => Some(QuickPlayTarget::Singleplayer),
        2 => Some(QuickPlayTarget::Multiplayer),
        3 => Some(QuickPlayTarget::Realms),
        _ => None,
    }
}

fn quick_play_target_label(target: QuickPlayTarget) -> &'static str {
    match target {
        QuickPlayTarget::Singleplayer => "单人游戏",
        QuickPlayTarget::Multiplayer => "多人联机",
        QuickPlayTarget::Realms => "Realms",
    }
}

fn quick_play_label(launch: &LaunchConfig) -> String {
    let Some(quick) = &launch.quick_play else {
        return "无".to_string();
    };
    let target = quick_play_target_label(quick.target);
    if quick.value.trim().is_empty() {
        format!("{target}（未填写）")
    } else {
        format!("{target}：{}", quick.value)
    }
}
