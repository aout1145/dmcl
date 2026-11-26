//! 进程优先级分组：低 / 正常 / 高。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::ProcessPriority;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, FixedRow, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct PriorityGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    low: FixedRow,
    normal: FixedRow,
    high: FixedRow,
}

impl PriorityGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let expander = adw::ExpanderRow::builder()
            .title("进程优先级")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::new("使用全局配置");
        inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| {
                notify(
                    &sender,
                    GameOptionsMsg::Inherit(GameOptionGroup::ProcessPriority),
                )
            }
        });
        if allow_inherit {
            expander.add_row(&inherit.row);
        }
        let low = FixedRow::new("低");
        low.set_subtitle("低于正常优先级");
        low.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::ProcessPrioritySelected(1))
        });
        let normal = FixedRow::new("正常");
        normal.set_subtitle("系统默认");
        normal.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::ProcessPrioritySelected(2))
        });
        let high = FixedRow::new("高");
        high.set_subtitle("高于正常优先级");
        high.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::ProcessPrioritySelected(3))
        });
        expander.add_row(&low.row);
        expander.add_row(&normal.row);
        expander.add_row(&high.row);

        Self {
            expander,
            inherit,
            low,
            normal,
            high,
        }
    }
}

/// 同步对勾、继承状态与小字。
pub(super) fn sync(group: &PriorityGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    group
        .low
        .set_selected(!view.inherited && view.launch.process_priority == ProcessPriority::Low);
    group
        .normal
        .set_selected(!view.inherited && view.launch.process_priority == ProcessPriority::Normal);
    group
        .high
        .set_selected(!view.inherited && view.launch.process_priority == ProcessPriority::High);
    group
        .expander
        .set_subtitle(&view.subtitle(priority_label(view.launch.process_priority)));
    if view.allow_inherit {
        group
            .inherit
            .set_subtitle(&priority_label(view.global.process_priority));
    }
}

/// 选中优先级；0 表示恢复继承。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    match index {
        0 => page.inherit_group(GameOptionGroup::ProcessPriority, sender),
        1..=3 => {
            let priority = match index {
                1 => ProcessPriority::Low,
                2 => ProcessPriority::Normal,
                _ => ProcessPriority::High,
            };
            if priority == page.launch.process_priority
                && !page.inherited(GameOptionGroup::ProcessPriority)
            {
                return;
            }
            page.priority.expander.set_expanded(false);
            page.launch.process_priority = priority;
            page.mark_override([GameOptionField::ProcessPriority]);
            page.sync_ui();
            page.schedule_flush(sender);
        }
        _ => {}
    }
}

fn priority_label(priority: ProcessPriority) -> String {
    match priority {
        ProcessPriority::Low => "低",
        ProcessPriority::Normal => "正常",
        ProcessPriority::High => "高",
    }
    .to_string()
}
