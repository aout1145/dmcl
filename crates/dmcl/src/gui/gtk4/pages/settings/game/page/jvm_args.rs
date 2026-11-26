//! JVM 参数分组：使用默认参数，或用自定义参数取代。

use std::cell::Cell;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::LaunchConfig;
use crate::gui::gtk4::pages::settings::common::FixedRow;

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct JvmArgsGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    default: FixedRow,
    custom: FixedRow,
    entry: adw::EntryRow,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,
}

impl JvmArgsGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let syncing = Rc::new(Cell::new(false));
        let expander = adw::ExpanderRow::builder()
            .title("JVM 参数")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::JvmArgs))
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }

        let default = FixedRow::builder("默认").build();
        default.set_subtitle("使用默认 JVM 参数");
        default.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::JvmArgsModeSelected(0))
        });
        let custom = FixedRow::builder("自定义").keep_open().build();
        custom.set_subtitle("使用自定义的附加参数");
        custom.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::JvmArgsModeSelected(1))
        });
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
                    GameOptionsMsg::JvmArgsChanged(entry.text().to_string()),
                );
            }
        });
        expander.add_row(default.row());
        expander.add_row(custom.row());
        expander.add_row(&entry);

        Self {
            expander,
            inherit,
            default,
            custom,
            entry,
            syncing,
        }
    }
}

/// 同步模式对勾、输入框、继承状态与小字。
pub(super) fn sync(group: &JvmArgsGroup, view: GroupView<'_>) {
    let custom = view.launch.additional_jvm.is_some();
    group.inherit.set_selected(view.inherited);
    group.default.set_selected(!view.inherited && !custom);
    group.custom.set_selected(!view.inherited && custom);
    group
        .expander
        .set_subtitle(&view.subtitle(jvm_args_label(view.launch)));
    if view.allow_inherit {
        group.inherit.set_subtitle(&jvm_args_label(view.global));
    }

    group.entry.set_visible(custom);
    group.syncing.set(true);
    let text = view.launch.additional_jvm.as_deref().unwrap_or_default();
    if group.entry.text() != text {
        group.entry.set_text(text);
    }
    group.syncing.set(false);
}

/// 选中默认 / 自定义；0 表示默认，1 表示自定义。
pub(super) fn mode_selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    match index {
        0 => {
            if page.launch.additional_jvm.is_none() && !page.inherited(GameOptionGroup::JvmArgs) {
                return;
            }
            page.launch.additional_jvm = None;
            page.mark_override([GameOptionField::AdditionalJvm]);
            page.sync_ui();
            page.schedule_flush(sender);
        }
        1 => {
            if page.launch.additional_jvm.is_some() && !page.inherited(GameOptionGroup::JvmArgs) {
                page.jvm_args.entry.grab_focus();
                return;
            }
            let value = page.launch.additional_jvm.clone().unwrap_or_default();
            page.launch.additional_jvm = Some(value);
            page.mark_override([GameOptionField::AdditionalJvm]);
            page.sync_ui();
            page.jvm_args.entry.grab_focus();
            page.schedule_flush(sender);
        }
        _ => {}
    }
}

/// 自定义参数文本变了：写进配置并安排写回。
pub(super) fn changed(
    page: &mut GameOptionsPage,
    value: String,
    sender: &ComponentSender<GameOptionsPage>,
) {
    if page.launch.additional_jvm.as_deref() == Some(value.as_str()) {
        return;
    }
    page.launch.additional_jvm = Some(value);
    page.mark_override([GameOptionField::AdditionalJvm]);
    sync(&page.jvm_args, page.view(GameOptionGroup::JvmArgs));
    page.schedule_flush(sender);
}

fn jvm_args_label(launch: &LaunchConfig) -> String {
    match launch.additional_jvm.as_deref() {
        None => "默认".to_string(),
        Some(args) if args.trim().is_empty() => "自定义（未填写）".to_string(),
        Some(args) => args.to_string(),
    }
}
