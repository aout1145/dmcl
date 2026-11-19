//! 可复用的游戏设置表单。
//!
//! 页面只编辑一份 [`LaunchConfig`]，所有写入都交给 [`GameOptionsTarget`]；全局分类和实例
//! 设置页共用这个组件。实例页多一层「使用全局配置」：选中时把字段从目标里删掉，表单里
//! 的值和输入控件的可用状态都跟着全局配置走。
//!
//! 控件改动先更新模型，400ms 防抖后按字段合并成一次写回。

use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use dmcl_java::JavaDetails;
use dmcl_launch::auto_max_memory;
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::os::Arch;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::prelude::*;

use crate::config::{
    configured_java, same_java_path, AccountConfig, JavaChoice, JavaConfig, LaunchConfig,
};
use crate::gui::gtk4::app::ConfigRef;
use crate::gui::gtk4::pages::settings::common::{
    account_avatar_text, account_type_label, account_username, JavaProbeCache,
};

use super::choice::{ChoiceRow, ChoiceRowInit, ChoiceRowMsg, ChoiceRowOut};
use super::{
    account_for_index, account_index_for, account_label, copy_fields, group_overridden,
    java_choice_for_index, java_detail, java_index_for_choice, java_label, java_title,
    window_preset, GameOptionField, GameOptionGroup, GameOptionsState, GameOptionsTarget,
    GameOptionsUpdate, JavaEntry, ACCOUNT_NONE_INDEX, JAVA_AUTO_INDEX, WINDOW_PRESETS,
};

/// 连续改动合并成一次写回的间隔。
const FLUSH_DELAY: Duration = Duration::from_millis(400);

/// 游戏设置表单的初始化参数。
pub(crate) struct GameOptionsInit {
    /// 改动写到哪里：全局配置或实例覆盖层。
    pub target: Rc<dyn GameOptionsTarget>,
    /// 只读的全局配置：账户表和 Java 表是全局资源，不属于编辑目标。
    pub config: ConfigRef,
    /// 与 Java 管理页共享的探测结果缓存。
    pub cache: Rc<JavaProbeCache>,
    /// 实例页：每个选项都可以「使用全局配置」。
    pub allow_inherit: bool,
    /// 实例版本需要的 Java 大版本；只有实例页用来显示「自动选择」的实际结果。
    pub required_java_major: Option<u32>,
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
    /// 实例页点了「自定义」窗口大小。
    WindowCustomSelected,
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
    VersionIsolationChanged(bool),
    /// 实例页的版本隔离展开行：0 使用全局、1 开启、2 关闭。
    VersionIsolationSelected(u32),
    /// 恢复某一组字段的全局值。
    Inherit(GameOptionGroup),
    /// 把文本项切换成自定义，但还不改动内容。
    Custom(GameOptionGroup),
    /// 实例版本需要的 Java 大版本；`None` 表示读不出来。
    RequiredJavaMajor(Option<u32>),
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

/// 展开行里的一行固定选项：主标题 + 小字 + 右侧对勾。
struct FixedRow {
    row: adw::ActionRow,
    mark: gtk::Image,
}

impl FixedRow {
    fn new(title: &str) -> Self {
        let row = adw::ActionRow::builder()
            .title(title)
            .activatable(true)
            .build();
        let mark = gtk::Image::from_icon_name("object-select-symbolic");
        mark.add_css_class("accent");
        mark.set_valign(gtk::Align::Center);
        row.add_suffix(&mark);
        Self { row, mark }
    }

    fn set_selected(&self, selected: bool) {
        self.mark.set_visible(selected);
    }

    fn set_subtitle(&self, subtitle: &str) {
        self.row.set_subtitle(subtitle);
    }
}

/// 一个可以继承全局的文本项。
struct TextOption {
    expander: adw::ExpanderRow,
    inherit: FixedRow,
    custom: FixedRow,
    entry: adw::EntryRow,
}

pub(crate) struct GameOptionsPage {
    target: Rc<dyn GameOptionsTarget>,
    config: ConfigRef,
    cache: Rc<JavaProbeCache>,
    allow_inherit: bool,
    required_java_major: Option<u32>,
    /// 是否已经拿到实例版本的 Java 要求；没拿到之前不显示失败文案。
    java_major_resolved: bool,
    /// 全局启动配置，用来显示继承项的小字。
    global: LaunchConfig,
    launch: LaunchConfig,
    overrides: BTreeSet<GameOptionField>,
    accounts: Vec<(String, AccountConfig)>,
    javas: Vec<JavaEntry>,
    /// Java 选择的当前下标；0 是「自动选择」。
    java_index: u32,
    /// 账户选择的当前下标；账户从 1 开始，0 表示还没有选中账户。
    account_index: u32,
    /// 待写入目标的字段。
    dirty: BTreeSet<GameOptionField>,
    /// 待从目标删除、恢复继承的字段。
    inherit: BTreeSet<GameOptionField>,
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
    account_inherit: FixedRow,
    java_inherit: FixedRow,
    window_inherit: FixedRow,
    memory_inherit: FixedRow,
    account_rows: FactoryVecDeque<ChoiceRow>,
    java_rows: FactoryVecDeque<ChoiceRow>,
    window_rows: FactoryVecDeque<ChoiceRow>,
    custom_size_row: adw::ActionRow,
    custom_mark: gtk::Image,
    window_choice: WindowChoice,
    width_spin: gtk::SpinButton,
    height_spin: gtk::SpinButton,
    auto_memory_row: adw::ActionRow,
    auto_memory_mark: gtk::Image,
    manual_memory_mark: gtk::Image,
    min_memory_spin: gtk::SpinButton,
    max_memory_spin: gtk::SpinButton,
    version_isolation_row: adw::SwitchRow,
    version_isolation_expander: adw::ExpanderRow,
    version_inherit: FixedRow,
    version_on: FixedRow,
    version_off: FixedRow,
    custom_info: TextOption,
    additional_jvm: TextOption,
    additional_game: TextOption,
}

/// 窗口大小选中的是某个预设，还是手动输入的宽高。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowChoice {
    Preset(usize),
    Custom,
}

impl WindowChoice {
    /// 配置里只保存宽高，加载时按值反推选中的预设。
    fn of(launch: &LaunchConfig) -> Self {
        window_preset(launch.window_width, launch.window_height).map_or(Self::Custom, Self::Preset)
    }
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
                version_isolation_row -> adw::SwitchRow {},
                #[local_ref]
                version_isolation_expander -> adw::ExpanderRow {},
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
            allow_inherit,
            required_java_major,
        } = init;
        let (state, accounts, javas) = load_choices(&target, &config, &cache);
        let GameOptionsState { launch, overrides } = state;
        let global = config.read().launch.clone();
        let java_index = java_index_for_choice(&javas, &launch.java);
        let account_index = account_index_for(&accounts, &launch.account);

        let syncing = Rc::new(Cell::new(false));

        let account_expander = adw::ExpanderRow::builder()
            .title("游戏账户")
            .show_enable_switch(false)
            .build();
        let account_inherit = FixedRow::new("使用全局配置");
        account_inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::Account))
        });
        if allow_inherit {
            account_expander.add_row(&account_inherit.row);
        }
        let account_rows = FactoryVecDeque::builder()
            .launch(account_expander.clone())
            .forward(sender.input_sender(), |out| match out {
                ChoiceRowOut::Selected(index) => GameOptionsMsg::AccountSelected(index),
            });

        let java_expander = adw::ExpanderRow::builder()
            .title("游戏 Java")
            .show_enable_switch(false)
            .build();
        let java_inherit = FixedRow::new("使用全局配置");
        java_inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::Java))
        });
        if allow_inherit {
            java_expander.add_row(&java_inherit.row);
        }
        let java_rows = FactoryVecDeque::builder()
            .launch(java_expander.clone())
            .forward(sender.input_sender(), |out| match out {
                ChoiceRowOut::Selected(index) => GameOptionsMsg::JavaSelected(index),
            });

        let window_expander = adw::ExpanderRow::builder()
            .title("游戏窗口大小")
            .show_enable_switch(false)
            .build();
        let window_inherit = FixedRow::new("使用全局配置");
        window_inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| {
                notify(
                    &sender,
                    GameOptionsMsg::Inherit(GameOptionGroup::WindowSize),
                )
            }
        });
        if allow_inherit {
            window_expander.add_row(&window_inherit.row);
        }
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
                notify(
                    &sender,
                    GameOptionsMsg::WindowWidthChanged(spin.value() as u32),
                );
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
                notify(
                    &sender,
                    GameOptionsMsg::WindowHeightChanged(spin.value() as u32),
                );
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
        custom_size_row.add_suffix(&size_box);
        if allow_inherit {
            custom_size_row.set_activatable(true);
            custom_size_row.connect_activated({
                let sender = sender.clone();
                move |_| notify(&sender, GameOptionsMsg::WindowCustomSelected)
            });
        } else {
            custom_size_row.set_activatable(false);
        }
        window_expander.add_row(&custom_size_row);

        let memory_expander = adw::ExpanderRow::builder()
            .title("游戏内存")
            .show_enable_switch(false)
            .build();
        let memory_inherit = FixedRow::new("使用全局配置");
        memory_inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::Memory))
        });
        if allow_inherit {
            memory_expander.add_row(&memory_inherit.row);
        }

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
                notify(&sender, GameOptionsMsg::MemoryChanged { min, max });
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
            move |_| notify(&sender, GameOptionsMsg::MemoryModeSelected(0))
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
            move |_| notify(&sender, GameOptionsMsg::MemoryModeSelected(1))
        });

        memory_expander.add_row(&auto_memory_row);
        memory_expander.add_row(&manual_memory_row);

        let version_isolation_row = adw::SwitchRow::builder()
            .title("版本隔离")
            .subtitle("每个版本使用独立的游戏目录；关闭后共用 Minecraft 根目录")
            .active(launch.version_isolation)
            .build();
        version_isolation_row.connect_active_notify({
            let sender = sender.clone();
            let syncing = Rc::clone(&syncing);
            move |row| {
                if syncing.get() {
                    return;
                }
                notify(
                    &sender,
                    GameOptionsMsg::VersionIsolationChanged(row.is_active()),
                );
            }
        });

        let version_isolation_expander = adw::ExpanderRow::builder()
            .title("版本隔离")
            .show_enable_switch(false)
            .build();
        let version_inherit = FixedRow::new("使用全局配置");
        version_inherit.row.connect_activated({
            let sender = sender.clone();
            move |_| {
                notify(
                    &sender,
                    GameOptionsMsg::Inherit(GameOptionGroup::VersionIsolation),
                )
            }
        });
        let version_on = FixedRow::new("开启");
        version_on.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::VersionIsolationSelected(1))
        });
        let version_off = FixedRow::new("关闭");
        version_off.row.connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::VersionIsolationSelected(2))
        });
        version_isolation_expander.add_row(&version_inherit.row);
        version_isolation_expander.add_row(&version_on.row);
        version_isolation_expander.add_row(&version_off.row);
        version_isolation_row.set_visible(!allow_inherit);
        version_isolation_expander.set_visible(allow_inherit);

        let advanced_expander = adw::ExpanderRow::builder()
            .title("高级")
            .subtitle("启动器信息、JVM 参数与游戏参数")
            .build();

        let custom_info_entry = adw::EntryRow::builder()
            .title(if allow_inherit {
                "自定义内容"
            } else {
                "启动器信息"
            })
            .text(&launch.custom_info)
            .build();
        connect_entry(
            &custom_info_entry,
            &syncing,
            &sender,
            GameOptionsMsg::CustomInfoChanged,
        );
        let custom_info = text_option(
            "启动器信息",
            custom_info_entry,
            GameOptionGroup::CustomInfo,
            &sender,
            allow_inherit,
        );

        let additional_jvm_entry = adw::EntryRow::builder()
            .title(if allow_inherit {
                "自定义内容"
            } else {
                "额外 JVM 参数"
            })
            .text(&launch.additional_jvm)
            .build();
        connect_entry(
            &additional_jvm_entry,
            &syncing,
            &sender,
            GameOptionsMsg::AdditionalJvmChanged,
        );
        let additional_jvm = text_option(
            "额外 JVM 参数",
            additional_jvm_entry,
            GameOptionGroup::AdditionalJvm,
            &sender,
            allow_inherit,
        );

        let additional_game_entry = adw::EntryRow::builder()
            .title(if allow_inherit {
                "自定义内容"
            } else {
                "额外游戏参数"
            })
            .text(&launch.additional_game)
            .build();
        connect_entry(
            &additional_game_entry,
            &syncing,
            &sender,
            GameOptionsMsg::AdditionalGameChanged,
        );
        let additional_game = text_option(
            "额外游戏参数",
            additional_game_entry,
            GameOptionGroup::AdditionalGame,
            &sender,
            allow_inherit,
        );

        if allow_inherit {
            advanced_expander.add_row(&custom_info.expander);
            advanced_expander.add_row(&additional_jvm.expander);
            advanced_expander.add_row(&additional_game.expander);
        } else {
            advanced_expander.add_row(&custom_info.entry);
            advanced_expander.add_row(&additional_jvm.entry);
            advanced_expander.add_row(&additional_game.entry);
        }

        root_widget.connect_map({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Shown)
        });
        root_widget.connect_unmap({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Hidden)
        });

        // 系统可用内存会随其他进程变化，设置页可见时每秒刷新一次自动值。
        let auto_memory_source = glib::timeout_add_seconds_local(1, {
            move || {
                notify(&sender, GameOptionsMsg::RefreshAutoMemory);
                glib::ControlFlow::Continue
            }
        });

        let window_choice = WindowChoice::of(&launch);
        let mut model = GameOptionsPage {
            target,
            config,
            cache,
            allow_inherit,
            required_java_major,
            java_major_resolved: !allow_inherit || required_java_major.is_some(),
            global,
            launch,
            overrides,
            accounts,
            javas,
            java_index,
            account_index,
            dirty: BTreeSet::new(),
            inherit: BTreeSet::new(),
            flush: None,
            _auto_memory_source: auto_memory_source,
            probing: HashSet::new(),
            visible: false,
            syncing,
            account_expander: account_expander.clone(),
            java_expander: java_expander.clone(),
            window_expander: window_expander.clone(),
            memory_expander: memory_expander.clone(),
            account_inherit,
            java_inherit,
            window_inherit,
            memory_inherit,
            account_rows,
            java_rows,
            window_rows,
            custom_size_row,
            custom_mark,
            window_choice,
            width_spin,
            height_spin,
            auto_memory_row,
            auto_memory_mark,
            manual_memory_mark,
            min_memory_spin,
            max_memory_spin,
            version_isolation_row: version_isolation_row.clone(),
            version_isolation_expander: version_isolation_expander.clone(),
            version_inherit,
            version_on,
            version_off,
            custom_info,
            additional_jvm,
            additional_game,
        };
        let widgets = view_output!();
        model.rebuild_choices();
        model.sync_ui();
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
            GameOptionsMsg::Reload if self.dirty.is_empty() && self.inherit.is_empty() => {
                self.reload(&sender)
            }
            GameOptionsMsg::Reload => {}
            GameOptionsMsg::JavaSelected(index) => {
                if index as usize > self.javas.len() {
                    return;
                }
                let inherited = self.inherited(GameOptionGroup::Java);
                if index != self.java_index || inherited {
                    self.java_index = index;
                    self.launch.java = java_choice_for_index(&self.javas, index);
                    self.mark_override([GameOptionField::Java]);
                    self.schedule_flush(&sender);
                }
                self.sync_ui();
                // 自动内存的 32 位上限取决于选中的 Java。
                self.java_expander.set_expanded(false);
            }
            GameOptionsMsg::AccountSelected(index) => {
                if index == ACCOUNT_NONE_INDEX || index as usize > self.accounts.len() {
                    return;
                }
                let inherited = self.inherited(GameOptionGroup::Account);
                if index != self.account_index || inherited {
                    self.account_index = index;
                    self.launch.account = account_for_index(&self.accounts, index);
                    self.mark_override([GameOptionField::Account]);
                    self.schedule_flush(&sender);
                }
                self.sync_ui();
                self.account_expander.set_expanded(false);
            }
            GameOptionsMsg::WindowPresetSelected(index) => {
                let Some(preset) = index
                    .checked_sub(1)
                    .and_then(|index| WINDOW_PRESETS.get(index as usize))
                else {
                    return;
                };
                let inherited = self.inherited(GameOptionGroup::WindowSize);
                self.window_choice = WindowChoice::Preset(index as usize - 1);
                if !inherited
                    && preset.width == self.launch.window_width
                    && preset.height == self.launch.window_height
                {
                    self.sync_ui();
                    self.window_expander.set_expanded(false);
                    return;
                }
                self.launch.window_width = preset.width;
                self.launch.window_height = preset.height;
                self.mark_override([GameOptionField::WindowWidth, GameOptionField::WindowHeight]);
                self.sync_ui();
                self.window_expander.set_expanded(false);
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::WindowCustomSelected => {
                self.window_choice = WindowChoice::Custom;
                if self.inherited(GameOptionGroup::WindowSize) {
                    self.mark_override([
                        GameOptionField::WindowWidth,
                        GameOptionField::WindowHeight,
                    ]);
                    self.schedule_flush(&sender);
                }
                self.sync_ui();
            }
            GameOptionsMsg::WindowWidthChanged(value) => {
                if value == self.launch.window_width && !self.inherited(GameOptionGroup::WindowSize)
                {
                    return;
                }
                self.launch.window_width = value;
                self.window_choice = WindowChoice::Custom;
                self.mark_override([GameOptionField::WindowWidth]);
                self.sync_ui();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::WindowHeightChanged(value) => {
                if value == self.launch.window_height
                    && !self.inherited(GameOptionGroup::WindowSize)
                {
                    return;
                }
                self.launch.window_height = value;
                self.window_choice = WindowChoice::Custom;
                self.mark_override([GameOptionField::WindowHeight]);
                self.sync_ui();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::MemoryModeSelected(index) => match index {
                0 => {
                    let inherited = self.inherited(GameOptionGroup::Memory);
                    if inherited || !memory_is_auto(&self.launch) {
                        self.launch.min_memory = None;
                        self.launch.max_memory = None;
                        self.mark_override([
                            GameOptionField::MinMemory,
                            GameOptionField::MaxMemory,
                        ]);
                        self.schedule_flush(&sender);
                    }
                    self.sync_ui();
                    self.memory_expander.set_expanded(false);
                }
                1 => {
                    let inherited = self.inherited(GameOptionGroup::Memory);
                    if inherited
                        || self.launch.min_memory.is_none()
                        || self.launch.max_memory.is_none()
                    {
                        let (min, max) = manual_memory_values(&self.launch, &self.javas);
                        self.launch.min_memory = Some(min);
                        self.launch.max_memory = Some(max);
                        self.mark_override([
                            GameOptionField::MinMemory,
                            GameOptionField::MaxMemory,
                        ]);
                        self.schedule_flush(&sender);
                    }
                    self.sync_ui();
                }
                _ => {}
            },
            GameOptionsMsg::MemoryChanged { min, max } => {
                if self.launch.min_memory == Some(min) && self.launch.max_memory == Some(max) {
                    return;
                }
                self.launch.min_memory = Some(min);
                self.launch.max_memory = Some(max);
                self.mark_override([GameOptionField::MinMemory, GameOptionField::MaxMemory]);
                self.sync_ui();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::CustomInfoChanged(text) => {
                if text == self.launch.custom_info && !self.inherited(GameOptionGroup::CustomInfo) {
                    return;
                }
                self.launch.custom_info = text;
                self.mark_override([GameOptionField::CustomInfo]);
                self.sync_ui();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::AdditionalJvmChanged(text) => {
                if text == self.launch.additional_jvm
                    && !self.inherited(GameOptionGroup::AdditionalJvm)
                {
                    return;
                }
                self.launch.additional_jvm = text;
                self.mark_override([GameOptionField::AdditionalJvm]);
                self.sync_ui();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::AdditionalGameChanged(text) => {
                if text == self.launch.additional_game
                    && !self.inherited(GameOptionGroup::AdditionalGame)
                {
                    return;
                }
                self.launch.additional_game = text;
                self.mark_override([GameOptionField::AdditionalGame]);
                self.sync_ui();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::VersionIsolationChanged(value) => {
                if value == self.launch.version_isolation
                    && !self.inherited(GameOptionGroup::VersionIsolation)
                {
                    return;
                }
                self.launch.version_isolation = value;
                self.mark_override([GameOptionField::VersionIsolation]);
                self.sync_ui();
                self.schedule_flush(&sender);
            }
            GameOptionsMsg::VersionIsolationSelected(index) => match index {
                0 => self.inherit_group(GameOptionGroup::VersionIsolation, &sender),
                1 | 2 => {
                    self.version_isolation_expander.set_expanded(false);
                    notify(&sender, GameOptionsMsg::VersionIsolationChanged(index == 1));
                }
                _ => {}
            },
            GameOptionsMsg::Inherit(group) => self.inherit_group(group, &sender),
            GameOptionsMsg::Custom(group) => {
                if self.inherited(group) {
                    self.mark_override(group.fields().iter().copied());
                    self.schedule_flush(&sender);
                }
                self.sync_ui();
                match group {
                    GameOptionGroup::CustomInfo => {
                        self.custom_info.entry.grab_focus();
                    }
                    GameOptionGroup::AdditionalJvm => {
                        self.additional_jvm.entry.grab_focus();
                    }
                    GameOptionGroup::AdditionalGame => {
                        self.additional_game.entry.grab_focus();
                    }
                    _ => {}
                }
            }
            GameOptionsMsg::RequiredJavaMajor(required) => {
                self.required_java_major = required;
                self.java_major_resolved = true;
                self.sync_subtitles();
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
                self.sync_subtitles();
            }
            GameOptionsMsg::Flush => {
                self.flush = None;
                self.flush_pending();
            }
            GameOptionsMsg::RefreshAutoMemory => {
                if self.visible {
                    self.sync_memory();
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
        self.inherit.clear();

        let (state, accounts, javas) = load_choices(&self.target, &self.config, &self.cache);
        self.launch = state.launch;
        self.overrides = state.overrides;
        self.window_choice = WindowChoice::of(&self.launch);
        self.global = self.config.read().launch.clone();
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
        self.sync_ui();

        if self.visible {
            self.probe_missing(sender);
        }
    }

    /// 重建账户、Java 和窗口大小的展开选择列表。
    fn rebuild_choices(&mut self) {
        {
            let mut guard = self.account_rows.guard();
            guard.clear();
            for (offset, (_, account)) in self.accounts.iter().enumerate() {
                let index = offset as u32 + 1;
                let username = account_username(account);
                guard.push_back(ChoiceRowInit {
                    index,
                    title: username.to_string(),
                    subtitle: account_type_label(account).to_string(),
                    detail: String::new(),
                    avatar_text: Some(account_avatar_text(username)),
                    selected: false,
                });
            }
        }

        {
            let mut guard = self.java_rows.guard();
            guard.clear();
            guard.push_back(ChoiceRowInit {
                index: JAVA_AUTO_INDEX,
                title: "自动选择".to_string(),
                subtitle: String::new(),
                detail: String::new(),
                avatar_text: None,
                selected: false,
            });
            for (offset, entry) in self.javas.iter().enumerate() {
                let index = offset as u32 + 1;
                guard.push_back(ChoiceRowInit {
                    index,
                    title: java_title(entry),
                    subtitle: java_path(entry),
                    detail: java_detail(entry),
                    avatar_text: None,
                    selected: false,
                });
            }
        }

        self.rebuild_window_choices();
    }

    /// 重建窗口预设列表，并把「自定义」固定在最后。
    fn rebuild_window_choices(&mut self) {
        {
            let mut guard = self.window_rows.guard();
            guard.clear();
            for (index, preset) in WINDOW_PRESETS.iter().enumerate() {
                guard.push_back(ChoiceRowInit {
                    index: index as u32 + 1,
                    title: preset.label.to_string(),
                    subtitle: format!("{} × {}", preset.width, preset.height),
                    detail: String::new(),
                    avatar_text: None,
                    selected: false,
                });
            }
        }
        self.window_expander.remove(&self.custom_size_row);
        self.window_expander.add_row(&self.custom_size_row);
    }

    /// 整组是否正在继承全局。
    fn inherited(&self, group: GameOptionGroup) -> bool {
        self.allow_inherit && !group_overridden(&self.overrides, group)
    }

    /// 把一组字段标成覆盖，并取消对应的待继承操作。
    fn mark_override(&mut self, fields: impl IntoIterator<Item = GameOptionField>) {
        for field in fields {
            self.overrides.insert(field);
            self.inherit.remove(&field);
            self.dirty.insert(field);
        }
    }

    /// 把一组字段恢复成全局值，并安排从目标删除。
    fn inherit_group(&mut self, group: GameOptionGroup, sender: &ComponentSender<Self>) {
        let overridden = group_overridden(&self.overrides, group);
        copy_fields(&mut self.launch, &self.global, group.fields().iter());
        for field in group.fields() {
            self.dirty.remove(field);
            self.overrides.remove(field);
            if overridden {
                self.inherit.insert(*field);
            }
        }
        if overridden {
            self.schedule_flush(sender);
        }
        self.collapse_group(group);
        self.sync_ui();
    }

    /// 选定一组后收起对应的展开行，和选中账户、Java、预设后的行为一致。
    fn collapse_group(&self, group: GameOptionGroup) {
        match group {
            GameOptionGroup::Account => self.account_expander.set_expanded(false),
            GameOptionGroup::Java => self.java_expander.set_expanded(false),
            GameOptionGroup::WindowSize => self.window_expander.set_expanded(false),
            GameOptionGroup::Memory => self.memory_expander.set_expanded(false),
            GameOptionGroup::VersionIsolation => {
                self.version_isolation_expander.set_expanded(false);
            }
            GameOptionGroup::CustomInfo => self.custom_info.expander.set_expanded(false),
            GameOptionGroup::AdditionalJvm => self.additional_jvm.expander.set_expanded(false),
            GameOptionGroup::AdditionalGame => self.additional_game.expander.set_expanded(false),
        }
    }

    /// 把模型同步到所有控件；期间屏蔽信号。
    fn sync_ui(&self) {
        self.sync_choice_marks();
        self.sync_values();
        self.sync_subtitles();
        self.sync_memory();
    }

    /// 更新各选择器的对勾与继承状态。
    fn sync_choice_marks(&self) {
        let account_inherited = self.inherited(GameOptionGroup::Account);
        for index in 0..self.account_rows.len() {
            self.account_rows.send(
                index,
                ChoiceRowMsg::SetSelected(
                    !account_inherited && index as u32 + 1 == self.account_index,
                ),
            );
        }
        self.account_inherit.set_selected(account_inherited);

        let java_inherited = self.inherited(GameOptionGroup::Java);
        for index in 0..self.java_rows.len() {
            self.java_rows.send(
                index,
                ChoiceRowMsg::SetSelected(!java_inherited && index as u32 == self.java_index),
            );
        }
        self.java_inherit.set_selected(java_inherited);

        let window_inherited = self.inherited(GameOptionGroup::WindowSize);
        for index in 0..self.window_rows.len() {
            self.window_rows.send(
                index,
                ChoiceRowMsg::SetSelected(
                    !window_inherited && self.window_choice == WindowChoice::Preset(index),
                ),
            );
        }
        self.custom_mark
            .set_visible(!window_inherited && self.window_choice == WindowChoice::Custom);
        self.window_inherit.set_selected(window_inherited);

        let version_inherited = self.inherited(GameOptionGroup::VersionIsolation);
        self.version_inherit.set_selected(version_inherited);
        self.version_on
            .set_selected(!version_inherited && self.launch.version_isolation);
        self.version_off
            .set_selected(!version_inherited && !self.launch.version_isolation);

        self.sync_text_marks(&self.custom_info, GameOptionGroup::CustomInfo);
        self.sync_text_marks(&self.additional_jvm, GameOptionGroup::AdditionalJvm);
        self.sync_text_marks(&self.additional_game, GameOptionGroup::AdditionalGame);
    }

    fn sync_text_marks(&self, option: &TextOption, group: GameOptionGroup) {
        let inherited = self.inherited(group);
        option.inherit.set_selected(inherited);
        option.custom.set_selected(!inherited);
        option.entry.set_sensitive(!inherited);
    }

    /// 把模型同步到数字、文本和开关控件；期间屏蔽信号。
    fn sync_values(&self) {
        self.syncing.set(true);
        self.set_spin_button_value(&self.width_spin, self.launch.window_width);
        self.set_spin_button_value(&self.height_spin, self.launch.window_height);
        let window_inherited = self.inherited(GameOptionGroup::WindowSize);
        self.width_spin.set_sensitive(!window_inherited);
        self.height_spin.set_sensitive(!window_inherited);
        set_entry_text(&self.custom_info.entry, &self.launch.custom_info);
        set_entry_text(&self.additional_jvm.entry, &self.launch.additional_jvm);
        set_entry_text(&self.additional_game.entry, &self.launch.additional_game);
        if self.version_isolation_row.is_active() != self.launch.version_isolation {
            self.version_isolation_row
                .set_active(self.launch.version_isolation);
        }
        self.syncing.set(false);
    }

    /// 更新各展开行的小字。
    fn sync_subtitles(&self) {
        self.account_expander.set_subtitle(&self.inherited_subtitle(
            GameOptionGroup::Account,
            account_label_of(&self.accounts, &self.launch.account),
        ));
        self.account_inherit
            .set_subtitle(&account_label_of(&self.accounts, &self.global.account));
        self.java_expander.set_subtitle(&self.inherited_subtitle(
            GameOptionGroup::Java,
            java_label_of(&self.javas, &self.launch.java),
        ));
        self.java_inherit
            .set_subtitle(&java_label_of(&self.javas, &self.global.java));
        self.java_rows.send(
            JAVA_AUTO_INDEX as usize,
            ChoiceRowMsg::SetInfo {
                title: "自动选择".to_string(),
                subtitle: self.auto_java_subtitle(),
                detail: String::new(),
            },
        );
        self.window_expander.set_subtitle(
            &self.inherited_subtitle(GameOptionGroup::WindowSize, size_label(&self.launch)),
        );
        self.window_inherit.set_subtitle(&size_label(&self.global));
        if self.allow_inherit {
            self.version_isolation_expander
                .set_subtitle(&self.inherited_subtitle(
                    GameOptionGroup::VersionIsolation,
                    isolation_label(self.launch.version_isolation),
                ));
            self.version_inherit
                .set_subtitle(&isolation_label(self.global.version_isolation));
        }
        self.custom_info
            .expander
            .set_subtitle(&self.inherited_subtitle(
                GameOptionGroup::CustomInfo,
                text_label(&self.launch.custom_info),
            ));
        self.custom_info
            .inherit
            .set_subtitle(&text_label(&self.global.custom_info));
        self.additional_jvm
            .expander
            .set_subtitle(&self.inherited_subtitle(
                GameOptionGroup::AdditionalJvm,
                text_label(&self.launch.additional_jvm),
            ));
        self.additional_jvm
            .inherit
            .set_subtitle(&text_label(&self.global.additional_jvm));
        self.additional_game
            .expander
            .set_subtitle(&self.inherited_subtitle(
                GameOptionGroup::AdditionalGame,
                text_label(&self.launch.additional_game),
            ));
        self.additional_game
            .inherit
            .set_subtitle(&text_label(&self.global.additional_game));
    }

    /// 展开行收起时显示的小字：继承全局时明确写出来，否则显示当前值。
    fn inherited_subtitle(&self, group: GameOptionGroup, value: String) -> String {
        if self.allow_inherit && self.inherited(group) {
            "使用全局配置".to_string()
        } else {
            value
        }
    }

    /// 更新内存组的对勾、小字和输入框。
    fn sync_memory(&self) {
        let inherited = self.inherited(GameOptionGroup::Memory);
        let auto = memory_is_auto(&self.launch);
        let arch = current_java_arch(&self.launch, &self.javas);
        let auto_memory = auto_max_memory(arch);
        let auto_detail = if arch == Some(Arch::X86) {
            format!("当前自动分配：{auto_memory} MiB（32 位 Java 上限 1536 MiB）")
        } else {
            format!("当前自动分配：{auto_memory} MiB")
        };
        self.auto_memory_row.set_subtitle(&auto_detail);
        self.auto_memory_mark.set_visible(!inherited && auto);
        self.manual_memory_mark.set_visible(!inherited && !auto);
        self.memory_inherit.set_selected(inherited);
        self.memory_inherit
            .set_subtitle(&memory_summary(&self.global, &self.javas));
        self.memory_expander.set_subtitle(&self.inherited_subtitle(
            GameOptionGroup::Memory,
            memory_summary(&self.launch, &self.javas),
        ));

        let (min, max) = if auto {
            (auto_memory, auto_memory)
        } else {
            manual_memory_values(&self.launch, &self.javas)
        };
        self.syncing.set(true);
        self.set_spin_button_value(&self.min_memory_spin, min);
        self.set_spin_button_value(&self.max_memory_spin, max);
        self.syncing.set(false);
        self.min_memory_spin.set_sensitive(!inherited);
        self.max_memory_spin.set_sensitive(!inherited);
    }

    /// 「自动选择」行的小字：实例实际会选中的 Java。
    fn auto_java_subtitle(&self) -> String {
        if !self.allow_inherit || !self.java_major_resolved {
            return String::new();
        }
        let Some(required_major) = self.required_java_major else {
            return "无法确定所需 Java 版本".to_string();
        };
        let configs = self.config.read().javas.clone();
        let Some(java) = configured_java(required_major, &configs) else {
            return "配置中没有可用的 Java".to_string();
        };
        self.javas
            .iter()
            .find(|entry| same_java_path(&entry.java.path, &java.path))
            .map(java_label)
            .unwrap_or_else(|| format!("Java {}（{}）", java.major, java.arch.name()))
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
                notify(&sender, GameOptionsMsg::Probed { path, result });
            });
        }
    }

    fn schedule_flush(&mut self, sender: &ComponentSender<Self>) {
        self.cancel_flush();
        let sender = sender.clone();
        self.flush = Some(glib::timeout_add_local_once(FLUSH_DELAY, move || {
            notify(&sender, GameOptionsMsg::Flush);
        }));
    }

    fn cancel_flush(&mut self) {
        if let Some(source) = self.flush.take() {
            source.remove();
        }
    }

    fn flush_pending(&mut self) {
        self.cancel_flush();
        if self.dirty.is_empty() && self.inherit.is_empty() {
            return;
        }
        let update = GameOptionsUpdate {
            launch: self.launch.clone(),
            fields: std::mem::take(&mut self.dirty),
            inherit: std::mem::take(&mut self.inherit),
        };
        self.target.save(update);
    }
}

/// 读取当前启动配置（含覆盖集合），以及全局账户 / Java 表。
fn load_choices(
    target: &Rc<dyn GameOptionsTarget>,
    config: &ConfigRef,
    cache: &Rc<JavaProbeCache>,
) -> (
    GameOptionsState,
    Vec<(String, AccountConfig)>,
    Vec<JavaEntry>,
) {
    let state = target.load();
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
    (state, accounts, javas)
}

/// 回调可能在组件关闭后才触发（计时器、异步探测、控件信号），用 `send` 静默丢弃，
/// 不能用运行期已结束时会 panic 的 `ComponentSender::input`。
fn notify(sender: &ComponentSender<GameOptionsPage>, message: GameOptionsMsg) {
    let _ = sender.input_sender().send(message);
}

/// 一个带继承选择的文本项：继承行、自定义行和输入行。
///
/// 全局页没有继承，输入行由调用方直接放进「高级」，这里只保留控件本身。
fn text_option(
    title: &str,
    entry: adw::EntryRow,
    group: GameOptionGroup,
    sender: &ComponentSender<GameOptionsPage>,
    allow_inherit: bool,
) -> TextOption {
    let expander = adw::ExpanderRow::builder()
        .title(title)
        .show_enable_switch(false)
        .build();
    let inherit = FixedRow::new("使用全局配置");
    inherit.row.connect_activated({
        let sender = sender.clone();
        move |_| notify(&sender, GameOptionsMsg::Inherit(group))
    });
    let custom = FixedRow::new("自定义");
    custom.row.connect_activated({
        let sender = sender.clone();
        move |_| notify(&sender, GameOptionsMsg::Custom(group))
    });
    if allow_inherit {
        expander.add_row(&inherit.row);
        expander.add_row(&custom.row);
        expander.add_row(&entry);
    }
    TextOption {
        expander,
        inherit,
        custom,
        entry,
    }
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

fn account_label_of(accounts: &[(String, AccountConfig)], account: &Option<String>) -> String {
    match account_index_for(accounts, account) {
        ACCOUNT_NONE_INDEX => "未选择".to_string(),
        index => accounts
            .get((index - 1) as usize)
            .map(|(name, account)| account_label(name, account))
            .unwrap_or_else(|| "未选择".to_string()),
    }
}

fn java_label_of(javas: &[JavaEntry], choice: &JavaChoice) -> String {
    match java_index_for_choice(javas, choice) {
        JAVA_AUTO_INDEX => "自动选择".to_string(),
        index => javas
            .get((index - 1) as usize)
            .map(java_label)
            .unwrap_or_else(|| "自动选择".to_string()),
    }
}

fn size_label(launch: &LaunchConfig) -> String {
    format!("{} × {}", launch.window_width, launch.window_height)
}

fn isolation_label(enabled: bool) -> String {
    if enabled {
        "开启".to_string()
    } else {
        "关闭".to_string()
    }
}

fn text_label(text: &str) -> String {
    if text.trim().is_empty() {
        "空".to_string()
    } else {
        text.to_string()
    }
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
        notify(&sender, message(row.text().into()));
    });
}

/// 只在文本确实变化时改写输入框，避免同步时把光标移到末尾。
fn set_entry_text(row: &adw::EntryRow, text: &str) {
    if row.text() != text {
        row.set_text(text);
    }
}
