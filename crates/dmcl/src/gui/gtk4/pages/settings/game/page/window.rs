//! 窗口分组：窗口化尺寸预设与全屏。

use std::cell::Cell;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use crate::config::LaunchConfig;

use super::super::{window_preset, GameOptionField, GameOptionGroup, WINDOW_PRESETS};
use super::{notify, FixedRow, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct WindowGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    preset_dropdown: gtk::DropDown,
    windowed: FixedRow,
    fullscreen: FixedRow,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,
}

impl WindowGroup {
    pub(super) fn new(allow_inherit: bool, sender: &ComponentSender<GameOptionsPage>) -> Self {
        let syncing = Rc::new(Cell::new(false));
        let expander = adw::ExpanderRow::builder()
            .title("窗口类型")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::new("使用全局配置");
        inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::Window))
        });
        if allow_inherit {
            expander.add_row(&inherit.row);
        }

        let preset_labels: Vec<String> = WINDOW_PRESETS
            .iter()
            .map(|preset| format!("{} × {}", preset.width, preset.height))
            .collect();
        let preset_labels: Vec<&str> = preset_labels.iter().map(String::as_str).collect();
        let preset_dropdown = gtk::DropDown::from_strings(&preset_labels);
        preset_dropdown.set_enable_search(false);
        preset_dropdown.set_valign(gtk::Align::Center);
        preset_dropdown.connect_selected_notify({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            move |dropdown| {
                if syncing.get() {
                    return;
                }
                notify(
                    &sender,
                    GameOptionsMsg::WindowPresetSelected(dropdown.selected()),
                );
            }
        });

        let windowed_row = adw::ActionRow::builder()
            .title("窗口化")
            .activatable(true)
            .build();
        windowed_row.add_suffix(&preset_dropdown);
        let windowed_mark = gtk::Image::from_icon_name("object-select-symbolic");
        windowed_mark.add_css_class("accent");
        windowed_mark.set_valign(gtk::Align::Center);
        windowed_row.add_suffix(&windowed_mark);
        windowed_row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::WindowModeSelected(1))
        });
        let windowed = FixedRow {
            row: windowed_row,
            mark: windowed_mark,
        };

        let fullscreen = FixedRow::new("全屏");
        fullscreen.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::WindowModeSelected(2))
        });
        expander.add_row(&windowed.row);
        expander.add_row(&fullscreen.row);

        Self {
            expander,
            inherit,
            preset_dropdown,
            windowed,
            fullscreen,
            syncing,
        }
    }
}

/// 同步对勾、预设下拉框、继承状态与小字。
pub(super) fn sync(group: &WindowGroup, view: GroupView<'_>) {
    group.inherit.set_selected(view.inherited);
    group
        .windowed
        .set_selected(!view.inherited && !view.launch.fullscreen);
    group
        .fullscreen
        .set_selected(!view.inherited && view.launch.fullscreen);
    group
        .expander
        .set_subtitle(&view.subtitle(window_label(view.launch)));
    group.inherit.set_subtitle(&window_label(view.global));

    let index =
        window_preset(view.launch.window_width, view.launch.window_height).unwrap_or(0) as u32;
    group.syncing.set(true);
    if group.preset_dropdown.selected() != index {
        group.preset_dropdown.set_selected(index);
    }
    group
        .preset_dropdown
        .set_sensitive(!view.inherited && !view.launch.fullscreen);
    group.syncing.set(false);
}

/// 选中窗口化尺寸：写回宽高并切到窗口化。
pub(super) fn preset_selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    let Some(preset) = WINDOW_PRESETS.get(index as usize) else {
        return;
    };
    let inherited = page.inherited(GameOptionGroup::Window);
    if !inherited
        && !page.launch.fullscreen
        && preset.width == page.launch.window_width
        && preset.height == page.launch.window_height
    {
        return;
    }
    page.launch.fullscreen = false;
    page.launch.window_width = preset.width;
    page.launch.window_height = preset.height;
    page.mark_override([
        GameOptionField::Fullscreen,
        GameOptionField::WindowWidth,
        GameOptionField::WindowHeight,
    ]);
    page.sync_ui();
    page.schedule_flush(sender);
}

/// 选中窗口化 / 全屏；0 表示恢复继承。
pub(super) fn mode_selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    match index {
        0 => page.inherit_group(GameOptionGroup::Window, sender),
        1 | 2 => {
            page.window.expander.set_expanded(false);
            let fullscreen = index == 2;
            if fullscreen == page.launch.fullscreen && !page.inherited(GameOptionGroup::Window) {
                return;
            }
            page.launch.fullscreen = fullscreen;
            page.mark_override([GameOptionField::Fullscreen]);
            page.sync_ui();
            page.schedule_flush(sender);
        }
        _ => {}
    }
}

fn window_label(launch: &LaunchConfig) -> String {
    if launch.fullscreen {
        return "全屏".to_string();
    }
    let index = window_preset(launch.window_width, launch.window_height).unwrap_or(0);
    let preset = &WINDOW_PRESETS[index];
    format!("窗口化（{} × {}）", preset.width, preset.height)
}
