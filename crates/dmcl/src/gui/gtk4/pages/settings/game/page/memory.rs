//! 内存分组：自动分配或手动指定 Xms / Xmx。

use std::cell::Cell;
use std::rc::Rc;

use dmcl_launch::auto_max_memory;
use dmcl_mcmeta::os::Arch;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::prelude::*;

use crate::config::{JavaChoice, LaunchConfig};
use crate::gui::gtk4::pages::settings::common::{FixedRow, JavaEntry};

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct MemoryGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    auto_row: FixedRow,
    manual_row: FixedRow,
    min_spin: gtk::SpinButton,
    max_spin: gtk::SpinButton,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,
    /// 页面可见时才刷新自动值。
    visible: bool,
    /// 每秒刷新自动分配内存值的计时器。
    _auto_memory_source: glib::SourceId,
}

impl MemoryGroup {
    pub(super) fn new(
        launch: &LaunchConfig,
        javas: &[JavaEntry],
        allow_inherit: bool,
        sender: &ComponentSender<GameOptionsPage>,
    ) -> Self {
        let syncing = Rc::new(Cell::new(false));
        let expander = adw::ExpanderRow::builder()
            .title("内存")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::Memory))
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }

        let auto_memory = auto_memory_mib(launch, javas);
        let (initial_min, initial_max) = manual_memory_values(launch, javas);
        let memory_limit = match system_memory_mib() {
            Some(total) => total
                .max(initial_min)
                .max(initial_max)
                .max(auto_memory)
                .max(1),
            // 读不到系统总内存时不限制输入上限。
            None => u32::MAX,
        };

        let min_spin = spin_button_with_upper(initial_min, memory_limit);
        let max_spin = spin_button_with_upper(initial_max, memory_limit);
        min_spin.connect_value_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            let max_spin = max_spin.clone();
            move |spin| {
                if syncing.get() {
                    return;
                }
                let min = spin.value() as u32;
                let max = (max_spin.value() as u32).max(min);
                if max_spin.value() != f64::from(max) {
                    syncing.set(true);
                    max_spin.set_value(f64::from(max));
                    syncing.set(false);
                }
                notify(&sender, GameOptionsMsg::MemoryChanged { min, max });
            }
        });
        max_spin.connect_value_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            let min_spin = min_spin.clone();
            move |spin| {
                if syncing.get() {
                    return;
                }
                let max = spin.value() as u32;
                let min = (min_spin.value() as u32).min(max);
                if min_spin.value() != f64::from(min) {
                    syncing.set(true);
                    min_spin.set_value(f64::from(min));
                    syncing.set(false);
                }
                notify(&sender, GameOptionsMsg::MemoryChanged { min, max });
            }
        });

        let memory_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        memory_box.set_valign(gtk::Align::Center);
        let xms = gtk::Label::new(Some("Xms"));
        xms.add_css_class("dim-label");
        let xmx = gtk::Label::new(Some("Xmx"));
        xmx.add_css_class("dim-label");
        memory_box.append(&xms);
        memory_box.append(&min_spin);
        memory_box.append(&xmx);
        memory_box.append(&max_spin);

        let auto_row = FixedRow::builder("自动分配内存").build();
        auto_row.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::MemoryModeSelected(0))
        });

        let manual_row = FixedRow::builder("手动分配内存")
            .suffix(&memory_box)
            .keep_open()
            .build();
        manual_row.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::MemoryModeSelected(1))
        });

        expander.add_row(auto_row.row());
        expander.add_row(manual_row.row());

        // 系统可用内存会随其他进程变化，设置页可见时每秒刷新一次自动值。
        let auto_memory_source = glib::timeout_add_seconds_local(1, {
            let sender = sender.clone();
            move || {
                notify(&sender, GameOptionsMsg::RefreshAutoMemory);
                glib::ControlFlow::Continue
            }
        });

        Self {
            expander,
            inherit,
            auto_row,
            manual_row,
            min_spin,
            max_spin,
            syncing,
            visible: false,
            _auto_memory_source: auto_memory_source,
        }
    }

    pub(super) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
}

/// 页面可见时刷新自动分配的内存值。
pub(super) fn refresh(group: &MemoryGroup, javas: &[JavaEntry], view: GroupView<'_>) {
    if group.visible {
        sync(group, javas, view);
    }
}

/// 同步对勾、小字和输入框。
pub(super) fn sync(group: &MemoryGroup, javas: &[JavaEntry], view: GroupView<'_>) {
    let auto = memory_is_auto(view.launch);
    let arch = current_java_arch(view.launch, javas);
    let auto_memory = auto_max_memory(arch);
    let auto_detail = if arch == Some(Arch::X86) {
        format!("当前自动分配：{auto_memory} MiB（32 位 Java 上限 1536 MiB）")
    } else {
        format!("当前自动分配：{auto_memory} MiB")
    };
    group.auto_row.set_subtitle(&auto_detail);
    group.auto_row.set_selected(!view.inherited && auto);
    group.manual_row.set_selected(!view.inherited && !auto);
    group.inherit.set_selected(view.inherited);
    group
        .inherit
        .set_subtitle(&memory_summary(view.global, javas));
    group
        .expander
        .set_subtitle(&view.subtitle(memory_summary(view.launch, javas)));

    let (min, max) = if auto {
        (auto_memory, auto_memory)
    } else {
        manual_memory_values(view.launch, javas)
    };
    group.syncing.set(true);
    set_spin_value(&group.min_spin, min);
    set_spin_value(&group.max_spin, max);
    group.syncing.set(false);
    group.min_spin.set_sensitive(!view.inherited);
    group.max_spin.set_sensitive(!view.inherited);
}

/// 切换自动 / 手动分配；0 是自动，1 是手动。
pub(super) fn mode_selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    match index {
        0 => {
            let inherited = page.inherited(GameOptionGroup::Memory);
            if inherited || !memory_is_auto(&page.launch) {
                page.launch.min_memory = None;
                page.launch.max_memory = None;
                page.mark_override([GameOptionField::MinMemory, GameOptionField::MaxMemory]);
                page.schedule_flush(sender);
            }
            page.sync_ui();
        }
        1 => {
            let inherited = page.inherited(GameOptionGroup::Memory);
            if inherited || page.launch.min_memory.is_none() || page.launch.max_memory.is_none() {
                let (min, max) = manual_memory_values(&page.launch, page.java.javas());
                page.launch.min_memory = Some(min);
                page.launch.max_memory = Some(max);
                page.mark_override([GameOptionField::MinMemory, GameOptionField::MaxMemory]);
                page.schedule_flush(sender);
            }
            page.sync_ui();
        }
        _ => {}
    }
}

/// 手动分配的两个输入框变了。
pub(super) fn changed(
    page: &mut GameOptionsPage,
    min: u32,
    max: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    if page.launch.min_memory == Some(min) && page.launch.max_memory == Some(max) {
        return;
    }
    page.launch.min_memory = Some(min);
    page.launch.max_memory = Some(max);
    page.mark_override([GameOptionField::MinMemory, GameOptionField::MaxMemory]);
    page.sync_ui();
    page.schedule_flush(sender);
}

fn spin_button_with_upper(value: u32, upper: u32) -> gtk::SpinButton {
    let adjustment = gtk::Adjustment::new(
        f64::from(value),
        0.0,
        f64::from(upper.max(value).max(1)),
        1.0,
        128.0,
        0.0,
    );
    gtk::SpinButton::builder()
        .adjustment(&adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .width_chars(6)
        .valign(gtk::Align::Center)
        .build()
}

fn set_spin_value(spin: &gtk::SpinButton, value: u32) {
    if spin.value() != f64::from(value) {
        spin.set_value(f64::from(value));
    }
}

/// 两个内存值都是 `null` 时视为自动分配。
fn memory_is_auto(launch: &LaunchConfig) -> bool {
    launch.min_memory.is_none() && launch.max_memory.is_none()
}

/// 自动内存要按哪个 Java 架构估算：指定 Java 用它缓存的架构，自动选择时用平台架构。
fn current_java_arch(launch: &LaunchConfig, javas: &[JavaEntry]) -> Option<Arch> {
    match &launch.java {
        JavaChoice::Named(name) => javas
            .iter()
            .find(|entry| &entry.name == name)
            .map(|entry| entry.java.arch)
            .or_else(Arch::current),
        JavaChoice::Auto => Arch::current(),
    }
}

/// 当前自动分配的内存（MiB）。
fn auto_memory_mib(launch: &LaunchConfig, javas: &[JavaEntry]) -> u32 {
    auto_max_memory(current_java_arch(launch, javas))
}

/// 手动设置里两个输入框应显示的值；`null` 按自动分配值补齐。
fn manual_memory_values(launch: &LaunchConfig, javas: &[JavaEntry]) -> (u32, u32) {
    let auto = auto_memory_mib(launch, javas);
    let max = launch.max_memory.unwrap_or(auto);
    let min = launch.min_memory.unwrap_or(max);
    (min, max)
}

fn memory_summary(launch: &LaunchConfig, javas: &[JavaEntry]) -> String {
    if memory_is_auto(launch) {
        format!("自动分配（{} MiB）", auto_memory_mib(launch, javas))
    } else {
        let (min, max) = manual_memory_values(launch, javas);
        format!("手动分配（Xms {min} MiB，Xmx {max} MiB）")
    }
}

/// 系统总内存（MiB）；读不到时返回 `None`。
fn system_memory_mib() -> Option<u32> {
    let mut system = sysinfo::System::new().ok()?;
    system.refresh_memory();
    let mib = system.total_memory() / 1024 / 1024;
    u32::try_from(mib).ok().filter(|mib| *mib > 0)
}
