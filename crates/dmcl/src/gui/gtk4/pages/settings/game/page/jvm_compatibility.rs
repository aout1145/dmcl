//! JVM 兼容性分组：启动时如何检查所选 Java 的版本与架构。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::JvmCompatibility;
use crate::gui::gtk4::pages::settings::common::FixedRow;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct JvmCompatibilityGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    ignore: FixedRow,
    check: FixedRow,
    complete: FixedRow,
}

impl JvmCompatibilityGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let expander = adw::ExpanderRow::builder()
            .title("JVM 兼容性")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| {
                notify(
                    &sender,
                    GameOptionsMsg::Inherit(GameOptionGroup::JvmCompatibility),
                )
            }
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }

        let ignore = FixedRow::builder("忽略").build();
        ignore.set_subtitle("不检查 Java 版本与架构");
        ignore.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::JvmCompatibilitySelected(1))
        });
        let check = FixedRow::builder("仅检查").build();
        check.set_subtitle("不匹配时提示，但不自动更换 Java");
        check.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::JvmCompatibilitySelected(2))
        });
        let complete = FixedRow::builder("检查并补全").build();
        complete.set_subtitle("自动选择且没有合适的 Java 时下载；手动选择时只检查");
        complete.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::JvmCompatibilitySelected(3))
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
pub(super) fn sync(group: &JvmCompatibilityGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    let compatibility = view.launch.jvm_compatibility;
    group
        .ignore
        .set_selected(!view.inherited && compatibility == JvmCompatibility::Ignore);
    group
        .check
        .set_selected(!view.inherited && compatibility == JvmCompatibility::Check);
    group
        .complete
        .set_selected(!view.inherited && compatibility == JvmCompatibility::Complete);
    group
        .expander
        .set_subtitle(&view.subtitle(compatibility_label(compatibility)));
    if view.allow_inherit {
        group
            .inherit
            .set_subtitle(&compatibility_label(view.global.jvm_compatibility));
    }
}

/// 选中兼容性检查方式；0 表示恢复继承。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    match index {
        0 => page.inherit_group(GameOptionGroup::JvmCompatibility, sender),
        1..=3 => {
            let compatibility = match index {
                1 => JvmCompatibility::Ignore,
                2 => JvmCompatibility::Check,
                _ => JvmCompatibility::Complete,
            };
            if compatibility == page.launch.jvm_compatibility
                && !page.inherited(GameOptionGroup::JvmCompatibility)
            {
                return;
            }
            page.launch.jvm_compatibility = compatibility;
            page.mark_override([GameOptionField::JvmCompatibility]);
            page.sync_ui();
            page.schedule_flush(sender);
        }
        _ => {}
    }
}

fn compatibility_label(compatibility: JvmCompatibility) -> String {
    match compatibility {
        JvmCompatibility::Ignore => "忽略",
        JvmCompatibility::Check => "仅检查",
        JvmCompatibility::Complete => "检查并补全",
    }
    .to_string()
}
