//! 可复用的游戏设置表单。
//!
//! 页面只编辑一份 [`LaunchConfig`]，所有写入都交给 [`GameOptionsTarget`]；全局分类和以后
//! 的实例设置页共用这个组件。控件改动先更新模型，400ms 防抖后按字段合并成一次写回。

use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use dmcl_launch::{auto_max_memory, JavaDetails};
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::os::Arch;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::prelude::*;

use crate::config::{AccountConfig, JavaChoice, JavaConfig, LaunchConfig};
use crate::gui::gtk4::app::ConfigRef;
use crate::gui::gtk4::pages::settings::common::JavaProbeCache;

use super::choice::{ChoiceRow, ChoiceRowInit, ChoiceRowMsg, ChoiceRowOut};
use super::{
    account_for_index, account_index_for, account_label, java_choice_for_index, java_detail,
    java_index_for_choice, java_label, java_title, window_preset, GameOptionField,
    GameOptionsTarget, GameOptionsUpdate, JavaEntry, WINDOW_PRESETS,
};

/// 连续改动合并成一次写回的间隔。
const FLUSH_DELAY: Duration = Duration::from_millis(400);

/// 「全局游戏选项」表单的初始化参数。
pub(crate) struct GameOptionsInit {
    /// 改动写到哪里：全局配置或实例覆盖层。
    pub target: Rc<dyn GameOptionsTarget>,
    /// 只读的全局配置：账户表和 Java 表是全局资源，不属于编辑目标。
    pub config: ConfigRef,
    /// 与 Java 管理页共享的探测结果缓存。
    pub cache: Rc<JavaProbeCache>,
}

/// 游戏设置表单收到的消息。
#[derive(Debug)]
pub(crate) enum GameOptionsMsg {
    /// 页面显示出来了：重新读取目标并探测还没缓存的 Java。
    Shown,
    /// 页面隐藏了：立即提交待写改动，停止发起探测。
    Hidden,
    /// 共享配置或实例覆盖变了：重新读取目标。
    Reload,
    /// 默认 Java 选择变了。
    JavaSelected(u32),
    /// 默认账户选择变了。
    AccountSelected(u32),
    WindowPresetSelected(u32),
    WindowWidthChanged(u32),
    WindowHeightChanged(u32),
    /// 内存模式：0 自动、1 手动。
    MemoryModeSelected(u32),
    MemoryChanged {
        min: u32,
        max: u32,
    },
    CustomInfoChanged(String),
    AdditionalJvmChanged(String),
    AdditionalGameChanged(String),
    /// 某个 Java 探测结束。
    Probed {
        path: PathBuf,
        result: Result<JavaDetails, String>,
    },
    /// 防抖到点，写回改动。
    Flush,
    /// 定时刷新自动分配的内存值。
    RefreshAutoMemory,
}

pub(crate) struct GameOptionsPage {
    target: Rc<dyn GameOptionsTarget>,
    config: ConfigRef,
    cache: Rc<JavaProbeCache>,
    launch: LaunchConfig,
    accounts: Vec<(String, AccountConfig)>,
    javas: Vec<JavaEntry>,
    /// Java 选择的当前下标；0 是「自动选择」。
    java_index: u32,
    /// 账户选择的当前下标；0 是「未选择」。
    account_index: u32,
    /// 待写回的字段。
    dirty: BTreeSet<GameOptionField>,
    /// 防抖计时器。
    flush: Option<glib::SourceId>,
    /// 每秒刷新自动分配内存值的计时器。
    _auto_memory_source: glib::SourceId,
    /// 正在探测的 Java 路径，避免重复执行。
    probing: HashSet<PathBuf>,
    /// 页面可见时才发起新的探测。
    visible: bool,
    /// 同步控件时临时屏蔽信号。
    syncing: Rc<Cell<bool>>,

    account_expander: adw::ExpanderRow,
    java_expander: adw::ExpanderRow,
    window_expander: adw::ExpanderRow,
    memory_expander: adw::ExpanderRow,
    account_rows: FactoryVecDeque<ChoiceRow>,
    java_rows: FactoryVecDeque<ChoiceRow>,
    window_rows: FactoryVecDeque<ChoiceRow>,
    custom_size_row: adw::ActionRow,
    custom_mark: gtk::Image,
    width_spin: gtk::SpinButton,
    height_spin: gtk::SpinButton,
    auto_memory_row: adw::ActionRow,
    auto_memory_mark: gtk::Image,
    manual_memory_mark: gtk::Image,
    min_memory_spin: gtk::SpinButton,
    max_memory_spin: gtk::SpinButton,
    custom_info_row: adw::EntryRow,
    additional_jvm_row: adw::EntryRow,
    additional_game_row: adw::EntryRow,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for GameOptionsPage {
    type Init = GameOptionsInit;
    type Input = GameOptionsMsg;
    type Output = ();

    view! {
        adw::PreferencesPage {
            add = &adw::PreferencesGroup {
                set_title: "游戏",
                #[local_ref]
                account_expander -> adw::ExpanderRow {},
                #[local_ref]
                java_expander -> adw::ExpanderRow {},
                #[local_ref]
                window_expander -> adw::ExpanderRow {},
                #[local_ref]
                memory_expander -> adw::ExpanderRow {},
                #[local_ref]
                advanced_expander -> adw::ExpanderRow {},
            },
        }
    }

    fn init(
        init: Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let GameOptionsInit {
            target,
            config,
            cache,
        } = init;
        let (launch, accounts, javas) = load_choices(&target, &config, &cache);
        let java_index = java_index_for_choice(&javas, &launch.java);
        let account_index = account_index_for(&accounts, &launch.account);

        let syncing = Rc::new(Cell::new(false));

        let account_expander = adw::ExpanderRow::builder()
            .title("游戏账户")
            .show_enable_switch(false)
            .build();
        let account_rows = FactoryVecDeque::builder()
            .launch(account_expander.clone())
            .forward(sender.input_sender(), |out| match out {
                ChoiceRowOut::Selected(index) => GameOptionsMsg::AccountSelected(index),
            });

        let java_expander = adw::ExpanderRow::builder()
            .title("游戏 Java")
            .show_enable_switch(false)
            .build();
        let java_rows = FactoryVecDeque::builder()
            .launch(java_expander.clone())
            .forward(sender.input_sender(), |out| match out {
                ChoiceRowOut::Selected(index) => GameOptionsMsg::JavaSelected(index),
            });

        let window_expander = adw::ExpanderRow::builder()
            .title("游戏窗口大小")
            .show_enable_switch(false)
            .build();
        let window_rows = FactoryVecDeque::builder()
            .launch(window_expander.clone())
            .forward(sender.input_sender(), |out| match out {
                ChoiceRowOut::Selected(index) => GameOptionsMsg::WindowPresetSelected(index),
            });

        let width_spin = spin_button(launch.window_width);
        width_spin.connect_value_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            move |spin| {
                if syncing.get() {
                    return;
                }
                sender.input(GameOptionsMsg::WindowWidthChanged(spin.value() as u32));
            }
        });

        let height_spin = spin_button(launch.window_height);
        height_spin.connect_value_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            move |spin| {
                if syncing.get() {
                    return;
                }
                sender.input(GameOptionsMsg::WindowHeightChanged(spin.value() as u32));
            }
        });

        let size_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        size_box.set_valign(gtk::Align::Center);
        let times = gtk::Label::new(Some("×"));
        times.add_css_class("dim-label");
        size_box.append(&width_spin);
        size_box.append(&times);
        size_box.append(&height_spin);

        let custom_mark = gtk::Image::from_icon_name("object-select-symbolic");
        custom_mark.add_css_class("accent");
        custom_mark.set_valign(gtk::Align::Center);
        size_box.append(&custom_mark);

        let custom_size_row = adw::ActionRow::builder()
            .title("自定义")
            .subtitle("手动输入宽高")
            .build();
        custom_size_row.set_activatable(false);
        custom_size_row.add_suffix(&size_box);
        window_expander.add_row(&custom_size_row);

        let memory_expander = adw::ExpanderRow::builder()
            .title("游戏内存")
            .show_enable_switch(false)
            .build();

        let auto_memory = auto_memory_mib(&launch, &javas);
        let (initial_min, initial_max) = manual_memory_values(&launch, &javas);
        let memory_limit = match system_memory_mib() {
            Some(total) => total
                .max(initial_min)
                .max(initial_max)
                .max(auto_memory)
                .max(1),
            // 读不到系统总内存时不限制输入上限。
            None => u32::MAX,
        };

        let min_memory_spin = spin_button_with_upper(initial_min, memory_limit);
        let max_memory_spin = spin_button_with_upper(initial_max, memory_limit);
        min_memory_spin.connect_value_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            let max_memory_spin = max_memory_spin.clone();
            move |spin| {
                if syncing.get() {
                    return;
                }
                let min = spin.value() as u32;
                let max = (max_memory_spin.value() as u32).max(min);
                if max_memory_spin.value() != f64::from(max) {
                    syncing.set(true);
                    max_memory_spin.set_value(f64::from(max));
                    syncing.set(false);
                }
                sender.input(GameOptionsMsg::MemoryChanged { min, max });
            }
        });
        max_memory_spin.connect_value_changed({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            let min_memory_spin = min_memory_spin.clone();
            move |spin| {
                if syncing.get() {
                    return;
                }
                let max = spin.value() as u32;
                let min = (min_memory_spin.value() as u32).min(max);
                if min_memory_spin.value() != f64::from(min) {
                    syncing.set(true);
                    min_memory_spin.set_value(f64::from(min));
                    syncing.set(false);
                }
                sender.input(GameOptionsMsg::MemoryChanged { min, max });
            }
        });

        let memory_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        memory_box.set_valign(gtk::Align::Center);
        let xms = gtk::Label::new(Some("Xms"));
        xms.add_css_class("dim-label");
        let xmx = gtk::Label::new(Some("Xmx"));
        xmx.add_css_class("dim-label");
        memory_box.append(&xms);
        memory_box.append(&min_memory_spin);
        memory_box.append(&xmx);
        memory_box.append(&max_memory_spin);

        let auto_memory_row = adw::ActionRow::builder().title("自动分配内存").build();
        auto_memory_row.set_activatable(true);
        let auto_memory_mark = gtk::Image::from_icon_name("object-select-symbolic");
        auto_memory_mark.add_css_class("accent");
        auto_memory_mark.set_valign(gtk::Align::Center);
        auto_memory_row.add_suffix(&auto_memory_mark);
        auto_memory_row.connect_activated({
            let sender = sender.clone();
            move |_| sender.input(GameOptionsMsg::MemoryModeSelected(0))
        });

        let manual_memory_row = adw::ActionRow::builder().title("手动分配内存").build();
        manual_memory_row.set_activatable(true);
        manual_memory_row.add_suffix(&memory_box);
        let manual_memory_mark = gtk::Image::from_icon_name("object-select-symbolic");
        manual_memory_mark.add_css_class("accent");
        manual_memory_mark.set_valign(gtk::Align::Center);
        manual_memory_row.add_suffix(&manual_memory_mark);
        manual_memory_row.connect_activated({
            let sender = sender.clone();
            move |_| sender.input(GameOptionsMsg::MemoryModeSelected(1))
        });

        memory_expander.add_row(&auto_memory_row);
        memory_expander.add_row(&manual_memory_row);

        let advanced_expander = adw::ExpanderRow::builder()
            .title("高级")
            .subtitle("启动器信息、JVM 参数与游戏参数")
            .build();

        let custom_info_row = adw::EntryRow::builder()
            .title("启动器信息")
            .text(&launch.custom_info)
            .build();
        connect_entry(
            &custom_info_row,
            &syncing,
            &sender,
            GameOptionsMsg::CustomInfoChanged,
        );
        let additional_jvm_row = adw::EntryRow::builder()
            .title("额外 JVM 参数")
            .text(&launch.additional_jvm)
            .build();
        connect_entry(
            &additional_jvm_row,
            &syncing,
            &sender,
            GameOptionsMsg::AdditionalJvmChanged,
        );
        let additional_game_row = adw::EntryRow::builder()
            .title("额外游戏参数")
            .text(&launch.additional_game)
            .build();
        connect_entry(
            &additional_game_row,
            &syncing,
            &sender,
            GameOptionsMsg::AdditionalGameChanged,
        );

        advanced_expander.add_row(&custom_info_row);
        advanced_expander.add_row(&additional_jvm_row);
        advanced_expander.add_row(&additional_game_row);

        root_widget.connect_map({
            let sender = sender.clone();
            move |_| sender.input(GameOptionsMsg::Shown)
        });
        root_widget.connect_unmap({
            let sender = sender.clone();
            move |_| sender.input(GameOptionsMsg::Hidden)
        });

        // 系统可用内存会随其他进程变化，设置页可见时每秒刷新一次自动值。
        let auto_memory_source = glib::timeout_add_seconds_local(1, {
            let sender = sender.clone();
            move || {
                sender.input(GameOptionsMsg::RefreshAutoMemory);
                glib::ControlFlow::Continue
            }
        });

        let mut model = GameOptionsPage {
            target,
            config,
            cache,
            launch,
            accounts,
            javas,
            java_index,
            account_index,
            dirty: BTreeSet::new(),
            flush: None,
            _auto_memory_source: auto_memory_source,
            probing: HashSet::new(),
            visible: false,
            syncing,
            account_expander: account_expander.clone(),
            java_expander: java_expander.clone(),
            window_expander: window_expander.clone(),
            memory_expander: memory_expander.clone(),
            account_rows,
            java_rows,
            window_rows,
            custom_size_row: custom_size_row.clone(),
            custom_mark: custom_mark.clone(),
            width_spin: width_spin.clone(),
            height_spin: height_spin.clone(),
            auto_memory_row: auto_memory_row.clone(),
            auto_memory_mark: auto_memory_mark.clone(),
            manual_memory_mark: manual_memory_mark.clone(),
            min_memory_spin: min_memory_spin.clone(),
            max_memory_spin: max_memory_spin.clone(),
            custom_info_row: custom_info_row.clone(),
            additional_jvm_row: additional_jvm_row.clone(),
            additional_game_row: additional_game_row.clone(),
        };
        let widgets = view_output!();
        model.rebuild_choices();
        model.sync_widgets();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            GameOptionsMsg::Shown => {
                self.visible = true;
                self.reload(&sender);
            }
            GameOptionsMsg::Hidden => {
                self.visible = false;
                self.flush_pending();
            }
            // 还有改动没写回时先不动本地模型；这次保存会再触发一次 ConfigChanged。
            GameOptionsMsg::Reload if self.dirty.is_empty() => self.reload(&sender),
            GameOptionsMsg::Reload => {}
            GameOptionsMsg::JavaSelected(index) => {
                if index as usize > self.javas.len() {
                    return;
                }
                if index != self.java_index {
                    self.java_index = index;
                    self.launch.java = java_choice_for_index(&self.javas, index);
                    self.dirty.insert(GameOptionField::Java);
                    self.schedule_flush(&sender);
                }
                self.sync_java_selection();
                // 自动内存的 32 位上限取决于选中的 Java。
                self.sync_memory_selection();
            }
            GameOptionsMsg::AccountSelected(index) => {
                if index as usize > self.accounts.len() {
                    return;
                }
                if index != self.account_index {
                    self.account_index = index;
                    self.launch.account = account_for_index(&self.accounts, index);
                    self.dirty.insert(GameOptionField::Account);
                    self.schedule_flush(&sender);
                }
                self.sync_account_selection();
            }
            GameOptionsMsg::WindowPresetSelected(index) => {
                let Some(preset) = index
                    .checked_sub(1)
                    .and_then(|index| WINDOW_PRESETS.get(index as usize))
                else {
                    return;
                };
                if preset.width == self.launch.window_width
                    && preset.height == self.launch.window_height
                {
                    self.window_expander.set_expanded(false);
                    return;
                }
                self.launch.window_width = preset.width;
                self.launch.window_height = preset.height;
                self.dirty.insert(GameOptionField::WindowWidth);
                self.dirty.insert(GameOptionField::WindowHeight);
                self.syncing.set(true);
                self.set_spin_button_value(&self.width_spin, preset.width);
                self.set_spin_button_value(&self.height_spin, preset.height);
                self.syncing.set(false);
                self.sync_window_selection();
                self.window_expander.set_expanded(false);
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::WindowWidthChanged(value) => {
                if value == self.launch.window_width {
                    return;
                }
                self.launch.window_width = value;
                self.dirty.insert(GameOptionField::WindowWidth);
                self.sync_window_selection();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::WindowHeightChanged(value) => {
                if value == self.launch.window_height {
                    return;
                }
                self.launch.window_height = value;
                self.dirty.insert(GameOptionField::WindowHeight);
                self.sync_window_selection();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::MemoryModeSelected(index) => match index {
                0 => {
                    if !memory_is_auto(&self.launch) {
                        self.launch.min_memory = None;
                        self.launch.max_memory = None;
                        self.dirty.insert(GameOptionField::MinMemory);
                        self.dirty.insert(GameOptionField::MaxMemory);
                        self.schedule_flush(&sender);
                    }
                    self.sync_memory_selection();
                    self.memory_expander.set_expanded(false);
                }
                1 => {
                    if self.launch.min_memory.is_none() || self.launch.max_memory.is_none() {
                        let (min, max) = manual_memory_values(&self.launch, &self.javas);
                        self.launch.min_memory = Some(min);
                        self.launch.max_memory = Some(max);
                        self.dirty.insert(GameOptionField::MinMemory);
                        self.dirty.insert(GameOptionField::MaxMemory);
                        self.syncing.set(true);
                        self.set_spin_button_value(&self.min_memory_spin, min);
                        self.set_spin_button_value(&self.max_memory_spin, max);
                        self.syncing.set(false);
                        self.schedule_flush(&sender);
                    }
                    self.sync_memory_selection();
                }
                _ => {}
            },
            GameOptionsMsg::MemoryChanged { min, max } => {
                if self.launch.min_memory == Some(min) && self.launch.max_memory == Some(max) {
                    return;
                }
                self.launch.min_memory = Some(min);
                self.launch.max_memory = Some(max);
                self.dirty.insert(GameOptionField::MinMemory);
                self.dirty.insert(GameOptionField::MaxMemory);
                self.sync_memory_selection();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::CustomInfoChanged(text) => {
                if text == self.launch.custom_info {
                    return;
                }
                self.launch.custom_info = text;
                self.dirty.insert(GameOptionField::CustomInfo);
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::AdditionalJvmChanged(text) => {
                if text == self.launch.additional_jvm {
                    return;
                }
                self.launch.additional_jvm = text;
                self.dirty.insert(GameOptionField::AdditionalJvm);
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::AdditionalGameChanged(text) => {
                if text == self.launch.additional_game {
                    return;
                }
                self.launch.additional_game = text;
                self.dirty.insert(GameOptionField::AdditionalGame);
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::Probed { path, result } => {
                self.probing.remove(&path);
                let Ok(details) = result else {
                    return;
                };
                self.cache.insert(path.clone(), details.clone());
                let Some(position) = self
                    .javas
                    .iter()
                    .position(|entry| absolute(&entry.java.path) == path)
                else {
                    return;
                };
                self.javas[position].details = Some(details);
                let entry = &self.javas[position];
                self.java_rows.send(
                    position + 1,
                    ChoiceRowMsg::SetInfo {
                        title: java_title(entry),
                        subtitle: java_path(entry),
                        detail: java_detail(entry),
                    },
                );
                if self.java_index == position as u32 + 1 {
                    self.java_expander.set_subtitle(&self.current_java_label());
                }
            }
            GameOptionsMsg::Flush => {
                self.flush = None;
                self.flush_pending();
            }
            GameOptionsMsg::RefreshAutoMemory => {
                if self.visible {
                    self.sync_memory_selection();
                }
            }
        }
    }
}

impl GameOptionsPage {
    /// 重新读取目标；会丢弃防抖队列里还没写回的改动。
    fn reload(&mut self, sender: &ComponentSender<Self>) {
        self.cancel_flush();
        self.dirty.clear();

        let (launch, accounts, javas) = load_choices(&self.target, &self.config, &self.cache);
        self.launch = launch;
        self.accounts = accounts;
        self.javas = javas;

        let paths: Vec<PathBuf> = self
            .javas
            .iter()
            .map(|entry| entry.java.path.clone())
            .collect();
        self.cache.retain(&paths);
        self.probing
            .retain(|path| paths.iter().any(|candidate| absolute(candidate) == *path));

        self.java_index = java_index_for_choice(&self.javas, &self.launch.java);
        self.account_index = account_index_for(&self.accounts, &self.launch.account);
        self.rebuild_choices();
        self.sync_widgets();

        if self.visible {
            self.probe_missing(sender);
        }
    }

    /// 重建账户、Java、窗口大小和内存模式的展开选择列表。
    fn rebuild_choices(&mut self) {
        {
            let mut guard = self.account_rows.guard();
            guard.clear();
            guard.push_back(ChoiceRowInit {
                index: 0,
                title: "未选择".to_string(),
                subtitle: String::new(),
                detail: String::new(),
                selected: self.account_index == 0,
            });
            for (offset, (name, account)) in self.accounts.iter().enumerate() {
                let index = offset as u32 + 1;
                guard.push_back(ChoiceRowInit {
                    index,
                    title: account_label(name, account),
                    subtitle: String::new(),
                    detail: String::new(),
                    selected: self.account_index == index,
                });
            }
        }

        {
            let mut guard = self.java_rows.guard();
            guard.clear();
            guard.push_back(ChoiceRowInit {
                index: 0,
                title: "自动选择".to_string(),
                subtitle: String::new(),
                detail: String::new(),
                selected: self.java_index == 0,
            });
            for (offset, entry) in self.javas.iter().enumerate() {
                let index = offset as u32 + 1;
                guard.push_back(ChoiceRowInit {
                    index,
                    title: java_title(entry),
                    subtitle: java_path(entry),
                    detail: java_detail(entry),
                    selected: self.java_index == index,
                });
            }
        }

        self.rebuild_window_choices();
        self.sync_memory_selection();
        self.update_choice_subtitles();
    }

    /// 重建窗口预设列表，并把「自定义」固定在最后。
    fn rebuild_window_choices(&mut self) {
        let selected = window_preset(self.launch.window_width, self.launch.window_height);
        {
            let mut guard = self.window_rows.guard();
            guard.clear();
            for (index, preset) in WINDOW_PRESETS.iter().enumerate() {
                guard.push_back(ChoiceRowInit {
                    index: index as u32 + 1,
                    title: preset.label.to_string(),
                    subtitle: format!("{} × {}", preset.width, preset.height),
                    detail: String::new(),
                    selected: selected == Some(index),
                });
            }
        }
        self.window_expander.remove(&self.custom_size_row);
        self.window_expander.add_row(&self.custom_size_row);
        self.sync_window_selection();
    }

    /// 更新窗口展开行的当前值、预设对勾与「自定义」状态。
    fn sync_window_selection(&self) {
        self.window_expander.set_subtitle(&format!(
            "{} × {}",
            self.launch.window_width, self.launch.window_height
        ));
        let selected = window_preset(self.launch.window_width, self.launch.window_height);
        self.custom_mark.set_visible(selected.is_none());
        for index in 0..self.window_rows.len() {
            self.window_rows
                .send(index, ChoiceRowMsg::SetSelected(selected == Some(index)));
        }
    }

    /// 更新内存展开行的当前值、模式对勾与自动分配小字。
    fn sync_memory_selection(&self) {
        let auto = memory_is_auto(&self.launch);
        let arch = current_java_arch(&self.launch, &self.javas);
        let auto_memory = auto_max_memory(arch);
        let auto_detail = if arch == Some(Arch::X86) {
            format!("当前自动分配：{auto_memory} MiB（32 位 Java 上限 1536 MiB）")
        } else {
            format!("当前自动分配：{auto_memory} MiB")
        };
        self.auto_memory_row.set_subtitle(&auto_detail);
        self.auto_memory_mark.set_visible(auto);
        self.manual_memory_mark.set_visible(!auto);

        let summary = if auto {
            format!("自动分配（{auto_memory} MiB）")
        } else {
            let (min, max) = manual_memory_values(&self.launch, &self.javas);
            format!("手动分配（Xms {min} MiB，Xmx {max} MiB）")
        };
        self.memory_expander.set_subtitle(&summary);

        if auto {
            self.syncing.set(true);
            self.set_spin_button_value(&self.min_memory_spin, auto_memory);
            self.set_spin_button_value(&self.max_memory_spin, auto_memory);
            self.syncing.set(false);
        }
    }

    /// 把模型同步到数字和文本控件；期间屏蔽信号。
    fn sync_widgets(&self) {
        self.syncing.set(true);
        self.set_spin_button_value(&self.width_spin, self.launch.window_width);
        self.set_spin_button_value(&self.height_spin, self.launch.window_height);
        let (min_memory, max_memory) = manual_memory_values(&self.launch, &self.javas);
        self.set_spin_button_value(&self.min_memory_spin, min_memory);
        self.set_spin_button_value(&self.max_memory_spin, max_memory);
        set_entry_text(&self.custom_info_row, &self.launch.custom_info);
        set_entry_text(&self.additional_jvm_row, &self.launch.additional_jvm);
        set_entry_text(&self.additional_game_row, &self.launch.additional_game);
        self.syncing.set(false);
    }

    fn sync_account_selection(&self) {
        for index in 0..self.account_rows.len() {
            self.account_rows.send(
                index,
                ChoiceRowMsg::SetSelected(index as u32 == self.account_index),
            );
        }
        self.update_choice_subtitles();
        self.account_expander.set_expanded(false);
    }

    fn sync_java_selection(&self) {
        for index in 0..self.java_rows.len() {
            self.java_rows.send(
                index,
                ChoiceRowMsg::SetSelected(index as u32 == self.java_index),
            );
        }
        self.update_choice_subtitles();
        self.java_expander.set_expanded(false);
    }

    fn update_choice_subtitles(&self) {
        self.account_expander
            .set_subtitle(&self.current_account_label());
        self.java_expander.set_subtitle(&self.current_java_label());
    }

    fn current_account_label(&self) -> String {
        if self.account_index == 0 {
            return "未选择".to_string();
        }
        self.accounts
            .get((self.account_index - 1) as usize)
            .map(|(name, account)| account_label(name, account))
            .unwrap_or_else(|| "未选择".to_string())
    }

    fn current_java_label(&self) -> String {
        if self.java_index == 0 {
            return "自动选择".to_string();
        }
        self.javas
            .get((self.java_index - 1) as usize)
            .map(java_label)
            .unwrap_or_else(|| "自动选择".to_string())
    }

    fn set_spin_button_value(&self, spin: &gtk::SpinButton, value: u32) {
        if spin.value() != f64::from(value) {
            spin.set_value(f64::from(value));
        }
    }

    /// 探测还没缓存、也还没在探测中的 Java；只在页面可见时调用。
    fn probe_missing(&mut self, sender: &ComponentSender<Self>) {
        if !self.visible {
            return;
        }
        let missing: Vec<PathBuf> = self
            .javas
            .iter()
            .filter(|entry| entry.details.is_none())
            .map(|entry| absolute(&entry.java.path))
            .filter(|path| !self.probing.contains(path))
            .collect();

        for path in missing {
            self.probing.insert(path.clone());
            let (tx, rx) = tokio::sync::oneshot::channel();
            relm4::spawn({
                let path = path.clone();
                async move {
                    let _ = tx.send(JavaDetails::probe(path).await);
                }
            });
            let sender = sender.clone();
            relm4::spawn_local(async move {
                let result = match rx.await {
                    Ok(result) => result.map_err(|err| format!("{err:#}")),
                    Err(_) => Err("Java 探测任务意外结束".to_string()),
                };
                sender.input(GameOptionsMsg::Probed { path, result });
            });
        }
    }

    fn schedule_flush(&mut self, sender: &ComponentSender<Self>) {
        self.cancel_flush();
        let sender = sender.clone();
        self.flush = Some(glib::timeout_add_local_once(FLUSH_DELAY, move || {
            sender.input(GameOptionsMsg::Flush);
        }));
    }

    fn cancel_flush(&mut self) {
        if let Some(source) = self.flush.take() {
            source.remove();
        }
    }

    fn flush_pending(&mut self) {
        self.cancel_flush();
        if self.dirty.is_empty() {
            return;
        }
        let update = GameOptionsUpdate {
            launch: self.launch.clone(),
            fields: std::mem::take(&mut self.dirty),
        };
        self.target.save(update);
    }
}

/// 读取当前启动配置，以及全局账户 / Java 表。
fn load_choices(
    target: &Rc<dyn GameOptionsTarget>,
    config: &ConfigRef,
    cache: &Rc<JavaProbeCache>,
) -> (LaunchConfig, Vec<(String, AccountConfig)>, Vec<JavaEntry>) {
    let launch = target.load();
    let (accounts, javas) = {
        let config = config.read();
        (
            config
                .accounts
                .iter()
                .map(|(name, account)| (name.clone(), account.clone()))
                .collect::<Vec<(String, AccountConfig)>>(),
            config
                .javas
                .iter()
                .map(|(name, java)| (name.clone(), java.clone()))
                .collect::<Vec<(String, JavaConfig)>>(),
        )
    };
    let javas = javas
        .into_iter()
        .map(|(name, java)| {
            let details = cache.get(&java.path);
            JavaEntry {
                name,
                java,
                details,
            }
        })
        .collect();
    (launch, accounts, javas)
}

fn spin_button(value: u32) -> gtk::SpinButton {
    let adjustment =
        gtk::Adjustment::new(f64::from(value), 0.0, f64::from(u32::MAX), 1.0, 100.0, 0.0);
    gtk::SpinButton::builder()
        .adjustment(&adjustment)
        .climb_rate(1.0)
        .digits(0)
        .numeric(true)
        .width_chars(6)
        .valign(gtk::Align::Center)
        .build()
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

/// 系统总内存（MiB）；读不到时返回 `None`。
fn system_memory_mib() -> Option<u32> {
    let mut system = sysinfo::System::new().ok()?;
    system.refresh_memory();
    let mib = system.total_memory() / 1024 / 1024;
    u32::try_from(mib).ok().filter(|mib| *mib > 0)
}

/// Java 可执行文件的绝对路径，显示在选择项下方。
fn java_path(entry: &JavaEntry) -> String {
    absolute(&entry.java.path).display().to_string()
}

fn connect_entry(
    row: &adw::EntryRow,
    syncing: &Rc<Cell<bool>>,
    sender: &ComponentSender<GameOptionsPage>,
    message: impl Fn(String) -> GameOptionsMsg + 'static,
) {
    let sender = sender.clone();
    let syncing = Rc::clone(syncing);
    row.connect_changed(move |row| {
        if syncing.get() {
            return;
        }
        sender.input(message(row.text().into()));
    });
}

/// 只在文本确实变化时改写输入框，避免同步时把光标移到末尾。
fn set_entry_text(row: &adw::EntryRow, text: &str) {
    if row.text() != text {
        row.set_text(text);
    }
}
