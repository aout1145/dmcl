//! 可复用的游戏设置表单。
//!
//! 页面只编辑一份 [`LaunchConfig`]，所有写入都交给 [`GameOptionsTarget`]；全局分类和实例
//! 设置页共用这个组件。实例页多一层「使用全局配置」：选中时把字段从目标里删掉，表单里
//! 的值和输入控件的可用状态都跟着全局配置走。
//!
//! 控件改动先更新模型，400ms 防抖后按字段合并成一次写回。每个 [`GameOptionGroup`] 一个
//! 子模块，持有该组自己的选择状态与控件，并负责同步和消息处理；本模块只保留跨组共享的
//! 启动配置、继承集合与写回。

mod account;
mod game_args;
mod java;
mod jvm_args;
mod jvm_compatibility;
mod memory;
mod priority;
mod quick_play;
mod version_isolation;
mod window;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use dmcl_java::JavaDetails;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk::glib;
use relm4::prelude::*;

use crate::config::LaunchConfig;
use crate::gui::gtk4::app::ConfigRef;
use crate::gui::gtk4::pages::settings::common::JavaProbeCache;

use super::{
    copy_fields, group_overridden, GameOptionField, GameOptionGroup, GameOptionsState,
    GameOptionsTarget, GameOptionsUpdate,
};

use account::AccountGroup;
use game_args::GameArgsGroup;
use java::JavaGroup;
use jvm_args::JvmArgsGroup;
use jvm_compatibility::JvmCompatibilityGroup;
use memory::MemoryGroup;
use priority::PriorityGroup;
use quick_play::QuickPlayGroup;
use version_isolation::VersionIsolationGroup;
use window::WindowGroup;

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
    /// 窗口化尺寸预设的下标。
    WindowPresetSelected(u32),
    /// 窗口类型：0 使用全局、1 窗口化、2 全屏。
    WindowModeSelected(u32),
    /// 快速游玩目标：0 无、1 单人游戏、2 多人联机、3 Realms。
    QuickPlaySelected(u32),
    /// 某个快速游玩目标的值变了（目标下标，文本）。
    QuickPlayValueChanged(u32, String),
    /// 游戏参数文本变了。
    GameArgsChanged(String),
    /// 进程优先级：0 使用全局、1 低、2 正常、3 高。
    ProcessPrioritySelected(u32),
    /// 内存模式：0 自动、1 手动。
    MemoryModeSelected(u32),
    MemoryChanged {
        min: u32,
        max: u32,
    },
    /// JVM 参数模式：0 默认、1 自定义。
    JvmArgsModeSelected(u32),
    /// 自定义 JVM 参数文本变了。
    JvmArgsChanged(String),
    /// JVM 兼容性：0 使用全局、1 忽略、2 仅检查、3 检查并补全。
    JvmCompatibilitySelected(u32),
    /// 版本隔离展开行：0 使用全局、1 开启、2 关闭。
    VersionIsolationSelected(u32),
    /// 恢复某一组字段的全局值。
    Inherit(GameOptionGroup),
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

/// 分组同步需要读取的共享状态。
pub(super) struct GroupView<'a> {
    /// 该组是否正在继承全局。
    pub inherited: bool,
    pub allow_inherit: bool,
    pub launch: &'a LaunchConfig,
    pub global: &'a LaunchConfig,
    pub config: &'a ConfigRef,
}

impl GroupView<'_> {
    /// 展开行收起时显示的小字：继承全局时明确写出来，否则显示当前值。
    fn subtitle(&self, value: String) -> String {
        if self.allow_inherit && self.inherited {
            "使用全局配置".to_string()
        } else {
            value
        }
    }
}

pub(crate) struct GameOptionsPage {
    target: Rc<dyn GameOptionsTarget>,
    config: ConfigRef,
    allow_inherit: bool,
    /// 全局启动配置，用来显示继承项的小字。
    global: LaunchConfig,
    launch: LaunchConfig,
    overrides: BTreeSet<GameOptionField>,
    /// 待写入目标的字段。
    dirty: BTreeSet<GameOptionField>,
    /// 待从目标删除、恢复继承的字段。
    inherit: BTreeSet<GameOptionField>,
    /// 防抖计时器。
    flush: Option<glib::SourceId>,

    account: AccountGroup,
    java: JavaGroup,
    window: WindowGroup,
    quick_play: QuickPlayGroup,
    game_args: GameArgsGroup,
    priority: PriorityGroup,
    memory: MemoryGroup,
    jvm_args: JvmArgsGroup,
    jvm_compatibility: JvmCompatibilityGroup,
    version_isolation: VersionIsolationGroup,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for GameOptionsPage {
    type Init = GameOptionsInit;
    type Input = GameOptionsMsg;
    type Output = ();

    view! {
        adw::PreferencesPage {
            add = &adw::PreferencesGroup {
                set_title: "通用",
                set_description: Some("设置启动游戏时使用的账户、Java、窗口与内存。"),
                #[local_ref]
                account_expander -> adw::ExpanderRow {},
                #[local_ref]
                java_expander -> adw::ExpanderRow {},
                #[local_ref]
                version_isolation_expander -> adw::ExpanderRow {},
                #[local_ref]
                window_expander -> adw::ExpanderRow {},
                #[local_ref]
                memory_expander -> adw::ExpanderRow {},
            },
            add = &adw::PreferencesGroup {
                set_title: "Java 虚拟机",
                set_description: Some("设置附加 JVM 参数与对所选 Java 的兼容性检查。"),
                #[local_ref]
                jvm_args_expander -> adw::ExpanderRow {},
                #[local_ref]
                jvm_compatibility_expander -> adw::ExpanderRow {},
            },
            add = &adw::PreferencesGroup {
                set_title: "游戏",
                set_description: Some("设置快速游玩、游戏参数和进程优先级。"),
                #[local_ref]
                quick_play_expander -> adw::ExpanderRow {},
                #[local_ref]
                game_args_expander -> adw::ExpanderRow {},
                #[local_ref]
                priority_expander -> adw::ExpanderRow {},
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
        let GameOptionsState { launch, overrides } = target.load();
        let global = config.read().launch.clone();

        let mut account = AccountGroup::new(allow_inherit, &sender);
        let mut java = JavaGroup::new(cache, required_java_major, allow_inherit, &sender);
        {
            let config = config.read();
            account::reload(&mut account, &config, &launch);
            java::reload(&mut java, &config, &launch);
        }
        let window = WindowGroup::new(allow_inherit, &sender);
        let quick_play = QuickPlayGroup::new(allow_inherit, &sender);
        let game_args = GameArgsGroup::new(allow_inherit, &sender);
        let priority = PriorityGroup::new(allow_inherit, &sender);
        let memory = MemoryGroup::new(&launch, &java.javas, allow_inherit, &sender);
        let jvm_args = JvmArgsGroup::new(allow_inherit, &sender);
        let jvm_compatibility = JvmCompatibilityGroup::new(allow_inherit, &sender);
        let version_isolation = VersionIsolationGroup::new(allow_inherit, &sender);

        let account_expander = account.expander.clone();
        let java_expander = java.expander.clone();
        let window_expander = window.expander.clone();
        let memory_expander = memory.expander.clone();
        let jvm_args_expander = jvm_args.expander.clone();
        let jvm_compatibility_expander = jvm_compatibility.expander.clone();
        let quick_play_expander = quick_play.expander.clone();
        let game_args_expander = game_args.expander.clone();
        let priority_expander = priority.expander.clone();
        let version_isolation_expander = version_isolation.expander.clone();

        root_widget.connect_map({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Shown)
        });
        root_widget.connect_unmap(move |_| notify(&sender, GameOptionsMsg::Hidden));

        let model = GameOptionsPage {
            target,
            config,
            allow_inherit,
            global,
            launch,
            overrides,
            dirty: BTreeSet::new(),
            inherit: BTreeSet::new(),
            flush: None,
            account,
            java,
            window,
            quick_play,
            game_args,
            priority,
            memory,
            jvm_args,
            jvm_compatibility,
            version_isolation,
        };
        let widgets = view_output!();
        model.sync_ui();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            GameOptionsMsg::Shown => {
                self.java.set_visible(true);
                self.memory.set_visible(true);
                self.reload(&sender);
            }
            GameOptionsMsg::Hidden => {
                self.java.set_visible(false);
                self.memory.set_visible(false);
                self.flush_pending();
            }
            // 还有改动没写回时先不动本地模型；这次保存会再触发一次 ConfigChanged。
            GameOptionsMsg::Reload if self.dirty.is_empty() && self.inherit.is_empty() => {
                self.reload(&sender)
            }
            GameOptionsMsg::Reload => {}
            GameOptionsMsg::JavaSelected(index) => java::selected(self, index, &sender),
            GameOptionsMsg::AccountSelected(index) => account::selected(self, index, &sender),
            GameOptionsMsg::WindowPresetSelected(index) => {
                window::preset_selected(self, index, &sender)
            }
            GameOptionsMsg::WindowModeSelected(index) => {
                window::mode_selected(self, index, &sender)
            }
            GameOptionsMsg::QuickPlaySelected(index) => quick_play::selected(self, index, &sender),
            GameOptionsMsg::QuickPlayValueChanged(index, value) => {
                quick_play::value_changed(self, index, value, &sender)
            }
            GameOptionsMsg::GameArgsChanged(value) => game_args::changed(self, value, &sender),
            GameOptionsMsg::ProcessPrioritySelected(index) => {
                priority::selected(self, index, &sender)
            }
            GameOptionsMsg::MemoryModeSelected(index) => {
                memory::mode_selected(self, index, &sender)
            }
            GameOptionsMsg::MemoryChanged { min, max } => memory::changed(self, min, max, &sender),
            GameOptionsMsg::JvmArgsModeSelected(index) => {
                jvm_args::mode_selected(self, index, &sender)
            }
            GameOptionsMsg::JvmArgsChanged(value) => jvm_args::changed(self, value, &sender),
            GameOptionsMsg::JvmCompatibilitySelected(index) => {
                jvm_compatibility::selected(self, index, &sender)
            }
            GameOptionsMsg::VersionIsolationSelected(index) => {
                version_isolation::selected(self, index, &sender)
            }
            GameOptionsMsg::Inherit(group) => self.inherit_group(group, &sender),
            GameOptionsMsg::RequiredJavaMajor(required) => java::required_major(self, required),
            GameOptionsMsg::Probed { path, result } => java::probed(self, path, result),
            GameOptionsMsg::Flush => {
                self.flush = None;
                self.flush_pending();
            }
            GameOptionsMsg::RefreshAutoMemory => {
                memory::refresh(
                    &self.memory,
                    &self.java.javas,
                    self.view(GameOptionGroup::Memory),
                );
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

        let GameOptionsState { launch, overrides } = self.target.load();
        self.launch = launch;
        self.overrides = overrides;
        {
            let config = self.config.read();
            self.global = config.launch.clone();
            account::reload(&mut self.account, &config, &self.launch);
            java::reload(&mut self.java, &config, &self.launch);
        }
        self.sync_ui();

        java::probe_missing(&mut self.java, sender);
    }

    /// 分组同步需要读取的共享状态。
    fn view(&self, group: GameOptionGroup) -> GroupView<'_> {
        GroupView {
            inherited: self.inherited(group),
            allow_inherit: self.allow_inherit,
            launch: &self.launch,
            global: &self.global,
            config: &self.config,
        }
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
        self.sync_ui();
    }

    /// 把模型同步到所有控件；期间屏蔽信号。
    fn sync_ui(&self) {
        account::sync(&self.account, self.view(GameOptionGroup::Account));
        java::sync(&self.java, self.view(GameOptionGroup::Java));
        window::sync(&self.window, self.view(GameOptionGroup::Window));
        quick_play::sync(&self.quick_play, self.view(GameOptionGroup::QuickPlay));
        game_args::sync(&self.game_args, self.view(GameOptionGroup::GameArgs));
        priority::sync(&self.priority, self.view(GameOptionGroup::ProcessPriority));
        memory::sync(
            &self.memory,
            &self.java.javas,
            self.view(GameOptionGroup::Memory),
        );
        jvm_args::sync(&self.jvm_args, self.view(GameOptionGroup::JvmArgs));
        jvm_compatibility::sync(
            &self.jvm_compatibility,
            self.view(GameOptionGroup::JvmCompatibility),
        );
        version_isolation::sync(
            &self.version_isolation,
            self.view(GameOptionGroup::VersionIsolation),
        );
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

/// 回调可能在组件关闭后才触发（计时器、异步探测、控件信号），用 `send` 静默丢弃，
/// 不能用运行期已结束时会 panic 的 `ComponentSender::input`。
fn notify(sender: &ComponentSender<GameOptionsPage>, message: GameOptionsMsg) {
    let _ = sender.input_sender().send(message);
}
