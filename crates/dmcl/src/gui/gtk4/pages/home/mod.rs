//! 主页：一个 Minecraft 根目录下的已安装实例。
//!
//! 每个主页固定绑定一个根目录，并从程序启动起常驻。关闭标签只影响显示，
//! 不会结束游戏进程，也不会丢失运行状态；运行状态由本页持有的 [`GameMonitor`] 监视。
//!
//! 版本目录用 `notify` watch，出现变更事件后再重新读取列表，不做固定轮询。

mod category_section;
mod create_instance;
mod instance_card;
mod instance_warning;
pub(crate) mod message;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecursiveMode, Watcher};
use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use dmcl_launch::GameMonitor;
use dmcl_mcmeta::layout::installed_versions;
use dmcl_task::CancelHandle;

use crate::config::{read_instance_category, LaunchConfig};
use crate::gui::gtk4::app::{config_store, ConfigRef, PageToast, Shell};

use category_section::{group_instances, CategorySection};
use instance_card::CardOut;
pub(crate) use message::{HomeMsg, InstanceEntry};

/// 主页的全局样式：底部胶囊操作栏、选中磁贴的高亮、右下角新建按钮。
pub(crate) const CSS: &str = "\
.dmcl-action-bar { border-radius: 9999px; padding: 4px; box-shadow: 0 2px 8px 0 rgba(0, 0, 0, 0.18); }
.dmcl-instance-selected { background: color-mix(in srgb, currentColor 12%, transparent); border-radius: 12px; }
.dmcl-add-instance { min-width: 48px; min-height: 48px; border-radius: 9999px; box-shadow: 0 2px 8px 0 rgba(0, 0, 0, 0.24); }";

/// 主页发给 App 的动作。App 用创建主页时捕获的 root 处理，不需要跟踪当前选中的主页。
#[derive(Debug)]
pub(crate) enum HomeOut {
    /// 打开「新建实例」页；`categories` 是当前主页已有分类的拷贝，用于输入建议。
    CreateCustom { categories: Vec<String> },
}

/// 实例的运行状态。界面文案、按钮动作、磁贴绿点都从它派生。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstanceState {
    /// 没在启动，也没在运行。
    Idle,
    /// 启动链还在跑，游戏进程尚未起来。
    Starting,
    /// 游戏进程已经起来。
    Running,
    /// 已经请求停止/取消，正在等启动链或游戏进程收尾。
    Stopping,
}

/// 主页里的活动实例。Idle 不进入 map。
struct InstanceRuntime {
    state: InstanceState,
    source: RuntimeSource,
}

enum RuntimeSource {
    /// 本页启动链还在跑。
    Starting { cancel: Option<CancelHandle> },
    /// 本页拿到的 GameMonitor 正在监视。
    Monitor {
        /// 先于 stop 声明：drop 时先 abort monitor task，再丢掉 stop sender，
        /// 避免 oneshot 变成 Err 后误触发停止分支。
        _task: TaskGuard,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
    },
}

/// 组件里的后台任务；drop 时主动中止。
struct TaskGuard(relm4::JoinHandle<()>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) struct HomePage {
    shell: Rc<Shell>,
    /// 共享配置的只读句柄；启动 / 安装时现读，始终是最新值。
    config: ConfigRef,
    root_path: PathBuf,
    /// 主窗口：实例配置警告框以它为父窗口。
    window: adw::ApplicationWindow,
    /// 页面级提示：只有主页可见时才显示。
    page_toast: PageToast,
    /// 按分类分组的实例磁贴。
    sections: FactoryVecDeque<CategorySection>,
    /// 当前选中的版本名。重建卡片时按名字恢复。
    selected: Option<String>,
    /// `versions/` 目录里的版本，保持 `installed_versions` 的顺序。
    installed: Vec<String>,
    /// 已读取的实例分类；活动但暂时不在目录里的实例保留原分类。
    categories: HashMap<String, Option<String>>,
    /// 当前活动实例，key 是版本名。
    runtimes: HashMap<String, InstanceRuntime>,
    watch_task: Option<TaskGuard>,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for HomePage {
    type Init = (Rc<Shell>, PathBuf, adw::ApplicationWindow);
    type Input = HomeMsg;
    type Output = HomeOut;

    view! {
        gtk::Overlay {
            #[wrap(Some)]
            set_child = &gtk::Stack {
                add_named[Some("grid")] = &gtk::ScrolledWindow {
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_vexpand: true,
                    // 点空白处取消选中（点磁贴时按钮会 claim 掉点击，不会冒泡到这里）。
                    add_controller = gtk::GestureClick {
                        connect_pressed[sender] => move |_, _, _, _| sender.input(HomeMsg::Deselect),
                    },

                    #[local_ref]
                    sections_widget -> gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 24,
                        set_margin_top: 18,
                        set_margin_bottom: 80,
                        set_margin_start: 18,
                        set_margin_end: 18,
                    },
                },

                add_named[Some("empty")] = &adw::StatusPage {
                    set_icon_name: Some("view-grid-symbolic"),
                    set_title: "还没有安装任何版本",
                    set_description: Some("点击右下角按钮添加新的 Minecraft 实例"),
                },

                // 放在 add_named 之后：初始化时两个页面已经存在。
                #[watch]
                set_visible_child_name: if model.sections.is_empty() { "empty" } else { "grid" },
            },

            // 底部胶囊操作栏：浮在网格上，只在选中实例时出现。
            add_overlay = &gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::End,
                set_margin_bottom: 18,
                add_css_class: "card",
                add_css_class: "dmcl-action-bar",
                #[watch]
                set_visible: model.selected.is_some(),

                gtk::Label {
                    add_css_class: "heading",
                    set_margin_start: 10,
                    set_margin_end: 4,
                    #[watch]
                    set_label: model.selected_name(),
                },

                gtk::Button {
                    add_css_class: "flat",
                    #[watch]
                    set_sensitive: model.action_enabled(),
                    connect_clicked => HomeMsg::Toggle,
                    #[wrap(Some)]
                    set_child = &adw::ButtonContent {
                        #[watch]
                        set_label: model.action_text(),
                        #[watch]
                        set_icon_name: model.action_icon(),
                    },
                },

                gtk::Button {
                    add_css_class: "flat",
                    connect_clicked => HomeMsg::Manage,
                    #[wrap(Some)]
                    set_child = &adw::ButtonContent {
                        set_icon_name: "document-properties-symbolic",
                        set_label: "管理",
                    },
                },

                gtk::Button {
                    set_icon_name: "window-close-symbolic",
                    add_css_class: "flat",
                    connect_clicked => HomeMsg::Deselect,
                },
            },

            // 右下角悬浮的新建实例按钮。它属于这个主页，所以目标 root 是确定的。
            add_overlay = &gtk::Button {
                set_icon_name: "list-add-symbolic",
                set_tooltip_text: Some("新建实例"),
                set_halign: gtk::Align::End,
                set_valign: gtk::Align::End,
                set_margin_end: 24,
                set_margin_bottom: 24,
                add_css_class: "circular",
                add_css_class: "suggested-action",
                add_css_class: "dmcl-add-instance",
                connect_clicked => HomeMsg::CreateInstance,
            },
        }
    }

    fn init(
        (shell, root_path, window): Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = config_store::store().config();
        let sections = FactoryVecDeque::builder()
            .launch(gtk::Box::new(gtk::Orientation::Vertical, 24))
            .forward(sender.input_sender(), |msg| match msg {
                CardOut::Select(name) => HomeMsg::Select(name),
                CardOut::Launch(name) => HomeMsg::Launch(name),
            });
        let mut model = HomePage {
            shell,
            config,
            root_path,
            window,
            page_toast: PageToast::new(&root_widget),
            sections,
            selected: None,
            installed: Vec::new(),
            categories: HashMap::new(),
            runtimes: HashMap::new(),
            watch_task: None,
        };

        let sections_widget = model.sections.widget();
        let widgets = view_output!();

        // 安装 / 删除版本后自动刷新：watch 版本目录，变化后再重新扫描。
        model.watch_task = Some(TaskGuard(relm4::spawn(watch_versions(
            model.root_path.clone(),
            sender.clone(),
        ))));
        sender.input(HomeMsg::Reload);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            HomeMsg::Reload => {
                let root = self.root_path.clone();
                let sender = sender.clone();
                relm4::spawn(async move {
                    let result = load_instances(&root)
                        .await
                        .map_err(|err| format!("{err:#}"));
                    sender.input(HomeMsg::Loaded(result));
                });
            }
            HomeMsg::Loaded(Ok(instances)) => {
                self.installed = instances
                    .iter()
                    .map(|instance| instance.name.clone())
                    .collect();
                for instance in instances {
                    self.categories.insert(instance.name, instance.category);
                }
                self.sync_sections();
            }
            HomeMsg::Loaded(Err(err)) => {
                self.shell
                    .page_toast(&self.page_toast, &format!("读取已安装版本失败：{err}"));
            }

            HomeMsg::Select(name) => self.select(name),
            HomeMsg::Deselect => self.deselect(),
            HomeMsg::Launch(name) => match self.state_of(&name) {
                InstanceState::Idle => self.request_start(name, &sender),
                InstanceState::Starting => {
                    self.shell
                        .page_toast(&self.page_toast, &format!("{name} 正在启动"));
                }
                InstanceState::Running => {
                    self.shell
                        .page_toast(&self.page_toast, &format!("{name} 已在运行"));
                }
                InstanceState::Stopping => {
                    self.shell
                        .page_toast(&self.page_toast, &format!("{name} 正在停止"));
                }
            },
            HomeMsg::Toggle => {
                let Some(name) = self.selected.clone() else {
                    return;
                };
                match self.state_of(&name) {
                    InstanceState::Idle => self.request_start(name, &sender),
                    InstanceState::Starting | InstanceState::Running => self.stop(&name),
                    InstanceState::Stopping => {}
                }
            }
            HomeMsg::LaunchConfirmed { name, launch } => {
                self.begin_start(name, launch, &sender);
            }
            HomeMsg::Launched { name, monitor } => {
                self.on_launched(name, monitor, &sender);
            }
            HomeMsg::LaunchFailed { name } => {
                if self
                    .runtimes
                    .get(&name)
                    .is_some_and(|runtime| matches!(runtime.source, RuntimeSource::Starting { .. }))
                {
                    self.runtimes.remove(&name);
                    self.sync_sections();
                }
            }
            HomeMsg::MonitorExited {
                name,
                stopped,
                abnormal,
            } => {
                let accept = self.runtimes.get(&name).is_some_and(|runtime| {
                    matches!(runtime.source, RuntimeSource::Monitor { .. })
                        && matches!(
                            runtime.state,
                            InstanceState::Running | InstanceState::Stopping
                        )
                });
                if !accept {
                    return;
                }
                self.runtimes.remove(&name);
                self.sync_sections();
                self.shell.global_toast(&if stopped {
                    format!("{name} 已停止")
                } else if abnormal {
                    format!("{name} 异常退出")
                } else {
                    format!("{name} 已退出")
                });
            }
            HomeMsg::WatchFailed(err) => {
                self.shell.global_toast(&format!("监听版本目录失败：{err}"));
            }
            HomeMsg::Manage => {
                let name = self.selected_name();
                if !name.is_empty() {
                    self.shell.page_toast(
                        &self.page_toast,
                        &format!("打开 {name} 的管理页（尚未实现）"),
                    );
                }
            }
            HomeMsg::CreateInstance => {
                create_instance::show(&self.window, &sender, self.category_names())
            }
            // 句柄指向的就是共享配置本身，不需要保存新值；消息只用来触发一次重画。
            HomeMsg::ConfigChanged => {}
        }
    }
}

impl HomePage {
    /// 拼出本 root 的启动链。
    fn launch_chain(
        &self,
        name: &str,
        launch: &LaunchConfig,
    ) -> anyhow::Result<dmcl_task::TaskChain<(), GameMonitor>> {
        let config = self.config.read();
        let option = config.launch_option(launch, name, &self.root_path)?;
        let install = std::sync::Arc::new(config.install_options(
            config.download_source(),
            launch,
            &self.root_path,
        )?);
        Ok(dmcl_launch::launch_chain(option, install))
    }

    /// 读取实例配置；有警告先弹确认框，否则直接开始。
    fn request_start(&mut self, name: String, sender: &ComponentSender<Self>) {
        let loaded = self
            .config
            .read()
            .load_instance_sync(&self.root_path, &name);
        if let Some(error) = loaded.error {
            let launch = loaded.launch;
            let sender = sender.clone();
            let msg_name = name.clone();
            instance_warning::show(&self.window, &error, move || {
                sender.input(HomeMsg::LaunchConfirmed {
                    name: msg_name.clone(),
                    launch: launch.clone(),
                });
            });
        } else {
            self.begin_start(name, loaded.launch, sender);
        }
    }

    fn begin_start(&mut self, name: String, launch: LaunchConfig, sender: &ComponentSender<Self>) {
        let chain = match self.launch_chain(&name, &launch) {
            Ok(chain) => chain,
            Err(err) => {
                self.shell
                    .global_toast(&format!("启动 {name} 失败：{err:#}"));
                return;
            }
        };
        let cancel = chain.cancel_handle();
        self.runtimes.insert(
            name.clone(),
            InstanceRuntime {
                state: InstanceState::Starting,
                source: RuntimeSource::Starting {
                    cancel: Some(cancel),
                },
            },
        );
        self.sync_sections();

        let handle = self.shell.run(format!("启动 {name}"), chain, ());
        let sender = sender.clone();
        let task_name = name.clone();
        // 这个任务不挂在 HomePage 的 TaskGuard 上：关闭标签后启动链继续跑，结果仍会
        // 回到常驻的组件。只有程序退出时发送失败，GameMonitor 随消息一起丢弃，
        // 游戏进程继续独立运行。
        relm4::spawn(async move {
            match handle.finished().await {
                Ok(monitor) => {
                    let _ = sender.input_sender().send(HomeMsg::Launched {
                        name: task_name,
                        monitor,
                    });
                }
                Err(_) => {
                    let _ = sender
                        .input_sender()
                        .send(HomeMsg::LaunchFailed { name: task_name });
                }
            }
        });
    }

    fn on_launched(&mut self, name: String, monitor: GameMonitor, sender: &ComponentSender<Self>) {
        let Some(runtime) = self.runtimes.get_mut(&name) else {
            return;
        };
        if !matches!(runtime.source, RuntimeSource::Starting { .. }) {
            return;
        }
        let was_stopping = runtime.state == InstanceState::Stopping;
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let stop = if was_stopping {
            let _ = stop_tx.send(());
            None
        } else {
            runtime.state = InstanceState::Running;
            Some(stop_tx)
        };
        runtime.source = RuntimeSource::Monitor {
            _task: TaskGuard(spawn_monitor(
                monitor,
                stop_rx,
                name.clone(),
                sender.clone(),
            )),
            stop,
        };
        self.sync_sections();
    }

    fn stop(&mut self, name: &str) {
        let Some(runtime) = self.runtimes.get_mut(name) else {
            return;
        };
        match &mut runtime.source {
            RuntimeSource::Starting { cancel } => {
                if let Some(cancel) = cancel.take() {
                    cancel.cancel();
                }
                runtime.state = InstanceState::Stopping;
            }
            RuntimeSource::Monitor { stop, .. } => {
                if let Some(stop) = stop.take() {
                    let _ = stop.send(());
                }
                runtime.state = InstanceState::Stopping;
            }
        }
        self.sync_sections();
    }

    /// 从 installed + runtimes + categories 重建分组并恢复选中项。
    fn sync_sections(&mut self) {
        let active: Vec<String> = self.runtimes.keys().cloned().collect();
        let names = visible_versions(&self.installed, active);

        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !names.contains(selected))
        {
            self.selected = None;
        }

        let entries: Vec<(String, Option<String>)> = names
            .into_iter()
            .map(|name| {
                let category = self.categories.get(&name).cloned().flatten();
                (name, category)
            })
            .collect();
        let groups = group_instances(entries);

        let selected = self.selected.clone();
        let runtimes = &self.runtimes;
        let mut guard = self.sections.guard();
        guard.clear();
        for (title, names) in groups {
            let cards = names
                .into_iter()
                .map(|name| {
                    let state = runtimes
                        .get(&name)
                        .map(|runtime| runtime.state)
                        .unwrap_or(InstanceState::Idle);
                    let is_selected = selected.as_deref() == Some(name.as_str());
                    (name, state, is_selected)
                })
                .collect();
            guard.push_back((title, cards));
        }
    }

    fn apply_selection(&mut self) {
        let selected = self.selected.clone();
        let mut sections = self.sections.guard();
        for section in sections.iter_mut() {
            let mut cards = section.cards.guard();
            for card in cards.iter_mut() {
                card.selected = selected.as_deref() == Some(card.name.as_str());
            }
        }
    }

    /// 当前已安装实例用到的分类，去重排序后给新建实例页做输入建议。
    fn category_names(&self) -> Vec<String> {
        let mut categories: Vec<String> = self
            .installed
            .iter()
            .filter_map(|name| self.categories.get(name).cloned().flatten())
            .collect();
        categories.sort();
        categories.dedup();
        categories
    }

    /// 操作栏上的实例名；没有选中时为空串（操作栏此时隐藏）。
    fn selected_name(&self) -> &str {
        self.selected.as_deref().unwrap_or_default()
    }

    fn action_state(&self) -> Option<InstanceState> {
        self.selected.as_deref().map(|name| self.state_of(name))
    }

    fn action_text(&self) -> &'static str {
        match self.action_state() {
            Some(InstanceState::Starting) => "取消",
            Some(InstanceState::Running) => "停止",
            Some(InstanceState::Stopping) => "停止中…",
            _ => "启动",
        }
    }

    fn action_icon(&self) -> &'static str {
        match self.action_state() {
            Some(InstanceState::Starting) => "process-stop-symbolic",
            Some(InstanceState::Running) => "media-playback-stop-symbolic",
            Some(InstanceState::Stopping) => "content-loading-symbolic",
            _ => "media-playback-start-symbolic",
        }
    }

    fn action_enabled(&self) -> bool {
        !matches!(self.action_state(), Some(InstanceState::Stopping))
    }

    fn state_of(&self, name: &str) -> InstanceState {
        self.runtimes
            .get(name)
            .map(|runtime| runtime.state)
            .unwrap_or(InstanceState::Idle)
    }

    fn select(&mut self, name: String) {
        self.selected = Some(name);
        self.apply_selection();
    }

    fn deselect(&mut self) {
        self.selected = None;
        self.apply_selection();
    }
}

/// 读取已安装版本及各自的分类；分类读取失败按未分类处理，不影响列表。
async fn load_instances(root: &Path) -> anyhow::Result<Vec<InstanceEntry>> {
    let mut entries = Vec::new();
    for version in installed_versions(root).await? {
        let category = read_instance_category(root, &version.name).await;
        entries.push(InstanceEntry {
            name: version.name,
            category,
        });
    }
    Ok(entries)
}

/// 可见版本：已安装版本保持原顺序，活动但不在目录里的版本按名字追加。
fn visible_versions(installed: &[String], mut active: Vec<String>) -> Vec<String> {
    active.sort();
    let mut names = installed.to_vec();
    for name in active {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// 等 GameMonitor 退出；用户停止时先 kill 再 wait。
fn spawn_monitor(
    mut monitor: GameMonitor,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
    name: String,
    sender: ComponentSender<HomePage>,
) -> relm4::JoinHandle<()> {
    relm4::spawn(async move {
        let (stopped, status) = tokio::select! {
            status = monitor.wait() => (false, status),
            signal = stop_rx => {
                if signal.is_ok() {
                    let _ = monitor.kill().await;
                    (true, monitor.wait().await)
                } else {
                    // stop sender 被丢弃但页面还在收尾：等游戏自然退出，不要误杀。
                    (false, monitor.wait().await)
                }
            }
        };
        let abnormal = !matches!(status, Ok(status) if status.success()) && !stopped;
        let _ = sender.input_sender().send(HomeMsg::MonitorExited {
            name,
            stopped,
            abnormal,
        });
    })
}

/// 要 watch 的路径：优先 `versions`；还不存在就 watch 最近的存在祖先，
/// 等它被创建后再重新挂到 `versions`。
fn watch_target(versions: &Path) -> Option<PathBuf> {
    if versions.exists() {
        return Some(versions.to_path_buf());
    }
    let mut current = versions;
    while !current.exists() {
        current = current.parent()?;
    }
    Some(current.to_path_buf())
}

/// 只关心直接改变 `versions` 子项的事件：创建 / 删除 / 重命名；此外实例自己的
/// `dmcl.json` 变化（分类可能被手改或安装完成后写入）也要刷新分类。
///
/// 版本目录内部的其它文件写入、目录 metadata 变化、访问事件都不算列表变化。即使某个平台
/// 的 watcher 仍然把子目录里的事件送上来，这个过滤也能把它们挡掉。
fn event_changes_versions(event: &Event, watched: &Path) -> bool {
    let touches_watched = event
        .paths
        .iter()
        .any(|path| path == watched || path.parent() == Some(watched));
    let touches_instance_config = event
        .paths
        .iter()
        .any(|path| is_instance_config(path, watched));
    if !touches_watched && !touches_instance_config {
        return false;
    }
    if touches_instance_config {
        return !matches!(event.kind, EventKind::Access(_));
    }
    matches!(
        event.kind,
        EventKind::Any
            | EventKind::Other
            | EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Any | ModifyKind::Name(_) | ModifyKind::Other)
    )
}

/// `versions/<name>/dmcl.json`：实例自己的配置，分类变化要反映到分组。
///
/// 写入时会先写同目录的 `.dmcl-<pid>.json.tmp` 再 rename；不同平台的 watcher 对 rename
/// 事件给出的路径不完全一致，所以临时文件的事件也算，debounce 之后读到的是最终文件。
fn is_instance_config(path: &Path, versions: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let is_config =
        name == "dmcl.json" || (name.starts_with(".dmcl-") && name.ends_with(".json.tmp"));
    is_config && path.parent().and_then(Path::parent) == Some(versions)
}

/// watch 版本目录；出现变更事件后重新扫描，不做固定轮询。
async fn watch_versions(root: PathBuf, sender: ComponentSender<HomePage>) {
    let versions_dir = root.join("versions");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let mut watcher = match notify::recommended_watcher(move |event: notify::Result<Event>| {
        if let Ok(event) = event {
            let _ = tx.send(event);
        }
    }) {
        Ok(watcher) => watcher,
        Err(err) => {
            let _ = sender
                .input_sender()
                .send(HomeMsg::WatchFailed(format!("{err}")));
            return;
        }
    };

    let mut watched: Option<PathBuf> = None;
    let mut watched_versions: BTreeSet<PathBuf> = BTreeSet::new();
    loop {
        let target = watch_target(&versions_dir);
        if target != watched {
            if let Some(old) = watched.take() {
                let _ = watcher.unwatch(&old);
            }
            // 换监听目标时旧版本目录的监听一并撤掉；当前目标是 `versions` 时再重建。
            for dir in std::mem::take(&mut watched_versions) {
                let _ = watcher.unwatch(&dir);
            }
            match target {
                Some(path) => match watcher.watch(&path, RecursiveMode::NonRecursive) {
                    Ok(()) => watched = Some(path),
                    Err(err) => {
                        let _ = sender
                            .input_sender()
                            .send(HomeMsg::WatchFailed(format!("{err}")));
                        return;
                    }
                },
                None => {
                    let _ = sender
                        .input_sender()
                        .send(HomeMsg::WatchFailed("找不到可监听的版本目录".to_string()));
                    return;
                }
            }
            if watched.as_deref() == Some(versions_dir.as_path()) {
                sync_version_watches(&mut watcher, &versions_dir, &mut watched_versions, &sender);
            }
        }

        let Some(event) = rx.recv().await else {
            break;
        };
        let Some(watched_path) = watched.as_deref() else {
            continue;
        };
        if !event_changes_versions(&event, watched_path) {
            continue;
        }

        // 新版本目录出现后要马上把它的 `dmcl.json` 也挂上，否则安装完成后写入的分类
        // 会漏掉。这里不等待 debounce，先同步监听集合。
        if touches_version_list(&event, watched_path) {
            sync_version_watches(&mut watcher, &versions_dir, &mut watched_versions, &sender);
        }

        // 安装器一次会制造一串事件，合并成一次 Reload。
        tokio::time::sleep(Duration::from_millis(200)).await;
        while rx.try_recv().is_ok() {}
        if sender.input_sender().send(HomeMsg::Reload).is_err() {
            break;
        }
    }
}

/// 事件是否直接发生在 `versions` 这一层（新建 / 删除 / 重命名版本目录）。
fn touches_version_list(event: &Event, versions: &Path) -> bool {
    event
        .paths
        .iter()
        .any(|path| path == versions || path.parent() == Some(versions))
}

/// 让每个版本目录也被非递归监听，用于接收 `versions/<name>/dmcl.json` 的变化。
fn sync_version_watches(
    watcher: &mut notify::RecommendedWatcher,
    versions: &Path,
    watched: &mut BTreeSet<PathBuf>,
    sender: &ComponentSender<HomePage>,
) {
    let current = version_dirs(versions);

    let stale: Vec<PathBuf> = watched.difference(&current).cloned().collect();
    for dir in stale {
        let _ = watcher.unwatch(&dir);
        watched.remove(&dir);
    }

    let added: Vec<PathBuf> = current.difference(watched).cloned().collect();
    for dir in added {
        match watcher.watch(&dir, RecursiveMode::NonRecursive) {
            Ok(()) => {
                watched.insert(dir);
            }
            // 扫描到这里之后目录可能又被删了；下一轮事件会再同步，不当作致命错误。
            Err(_) if !dir.exists() => {}
            Err(err) => {
                let _ = sender
                    .input_sender()
                    .send(HomeMsg::WatchFailed(format!("{err}")));
            }
        }
    }
}

/// `versions/` 下的版本目录集合；目录不存在或读不了时为空。
fn version_dirs(versions: &Path) -> BTreeSet<PathBuf> {
    let Ok(entries) = std::fs::read_dir(versions) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_type().ok()?.is_dir().then(|| entry.path()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_versions_keep_installed_order_and_append_ghosts() {
        let installed = vec!["1.20.1".to_string(), "1.21".to_string()];
        let active = vec!["1.21".to_string(), "forgotten".to_string()];
        assert_eq!(
            visible_versions(&installed, active),
            vec![
                "1.20.1".to_string(),
                "1.21".to_string(),
                "forgotten".to_string()
            ]
        );

        assert_eq!(
            visible_versions(&[], vec!["b".to_string(), "a".to_string()]),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn watch_target_uses_versions_or_the_nearest_existing_parent() {
        let root = std::env::temp_dir().join(format!("dmcl-home-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let versions = root.join("versions");

        // versions 还不存在：先 watch root，等 versions 被创建。
        assert_eq!(watch_target(&versions), Some(root.clone()));

        std::fs::create_dir_all(&versions).unwrap();
        assert_eq!(watch_target(&versions), Some(versions.clone()));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn watcher_only_cares_about_direct_version_entries() {
        use notify::event::{CreateKind, DataChange, MetadataKind, RenameMode};

        let versions = Path::new("/mc/versions");
        let version = versions.join("1.20.1");

        let created = Event::new(EventKind::Create(CreateKind::Folder)).add_path(version.clone());
        assert!(event_changes_versions(&created, versions));

        let renamed = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(version.clone());
        assert!(event_changes_versions(&renamed, versions));

        // 实例自己的 dmcl.json 写入（例如分类变化）要刷新分组，即使只是数据修改。
        let config = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
            .add_path(version.join("dmcl.json"));
        assert!(event_changes_versions(&config, versions));

        // 分类写入用的临时文件也接受：rename 事件在不同平台给出的路径不一样。
        let temp = Event::new(EventKind::Create(CreateKind::File))
            .add_path(version.join(".dmcl-1234.json.tmp"));
        assert!(event_changes_versions(&temp, versions));

        // 版本目录内部其它文件的写入不触发列表刷新。
        let inner = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
            .add_path(version.join("mods/example.jar"));
        assert!(!event_changes_versions(&inner, versions));

        // 目录 metadata 变化也不代表版本列表变化。
        let metadata = Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::WriteTime,
        )))
        .add_path(version);
        assert!(!event_changes_versions(&metadata, versions));
    }
}
