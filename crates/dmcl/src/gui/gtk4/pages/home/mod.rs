//! 主页：一个 Minecraft 根目录下的已安装实例。
//!
//! 每个主页固定绑定一个根目录，并从程序启动起常驻。关闭标签只影响显示，
//! 不会结束游戏进程，也不会丢失运行状态；运行状态由本页持有的 [`GameMonitor`] 监视。
//!
//! 版本目录用 `notify` watch，出现变更事件后再重新读取列表，不做固定轮询。

mod create_instance;
mod instance_card;
mod instance_warning;
pub(crate) mod message;

use std::collections::HashMap;
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

use crate::config::{Config, LaunchConfig};
use crate::gui::gtk4::app::{PageToast, Shell};

use instance_card::{CardOut, InstanceCard};
pub(crate) use message::HomeMsg;

/// 主页的全局样式：底部胶囊操作栏、选中磁贴的高亮、右下角新建按钮。
pub(crate) const CSS: &str = "\
.dmcl-action-bar { border-radius: 9999px; padding: 4px; box-shadow: 0 2px 8px 0 rgba(0, 0, 0, 0.18); }
.dmcl-instance-selected { background: color-mix(in srgb, currentColor 12%, transparent); border-radius: 12px; }
.dmcl-add-instance { min-width: 48px; min-height: 48px; border-radius: 9999px; box-shadow: 0 2px 8px 0 rgba(0, 0, 0, 0.24); }";

/// 主页发给 App 的动作。App 用创建主页时捕获的 root 处理，不需要跟踪当前选中的主页。
#[derive(Debug)]
pub(crate) enum HomeOut {
    /// 打开「新建实例」页。
    CreateCustom,
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
    config: Config,
    root_path: PathBuf,
    /// 主窗口：实例配置警告框以它为父窗口。
    window: adw::ApplicationWindow,
    /// 页面级提示：只有主页可见时才显示。
    page_toast: PageToast,
    instances: FactoryVecDeque<InstanceCard>,
    /// 当前选中的版本名。重建卡片时按名字恢复。
    selected: Option<String>,
    /// `versions/` 目录里的版本，保持 `installed_versions` 的顺序。
    installed: Vec<String>,
    /// 当前活动实例，key 是版本名。
    runtimes: HashMap<String, InstanceRuntime>,
    watch_task: Option<TaskGuard>,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for HomePage {
    type Init = (Rc<Shell>, Config, PathBuf, adw::ApplicationWindow);
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
                    grid -> adw::WrapBox {
                        set_child_spacing: 12,
                        set_line_spacing: 12,
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
                set_visible_child_name: if model.instances.is_empty() { "empty" } else { "grid" },
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
        (shell, config, root_path, window): Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let instances = FactoryVecDeque::builder()
            .launch(adw::WrapBox::new())
            .forward(sender.input_sender(), |msg| match msg {
                CardOut::Select(index) => HomeMsg::Select(index),
                CardOut::Launch(index) => HomeMsg::Launch(index),
            });
        let mut model = HomePage {
            shell,
            config,
            root_path,
            window,
            page_toast: PageToast::new(&root_widget),
            instances,
            selected: None,
            installed: Vec::new(),
            runtimes: HashMap::new(),
            watch_task: None,
        };

        let grid = model.instances.widget();
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
                    let result = installed_versions(&root)
                        .await
                        .map_err(|err| format!("{err:#}"));
                    sender.input(HomeMsg::Loaded(result));
                });
            }
            HomeMsg::Loaded(Ok(versions)) => {
                self.installed = versions.into_iter().map(|version| version.name).collect();
                self.sync_cards();
            }
            HomeMsg::Loaded(Err(err)) => {
                self.shell
                    .page_toast(&self.page_toast, &format!("读取已安装版本失败：{err}"));
            }

            HomeMsg::Select(index) => self.select(index),
            HomeMsg::Deselect => self.deselect(),
            HomeMsg::Launch(index) => {
                let Some(name) = self.name_at(&index) else {
                    return;
                };
                match self.state_of(&name) {
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
                }
            }
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
                    self.sync_cards();
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
                self.sync_cards();
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
            HomeMsg::CreateInstance => create_instance::show(&self.window, &sender),
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
        let option = self.config.launch_option(launch, name, &self.root_path)?;
        let install = std::sync::Arc::new(self.config.install_options(
            self.config.download_source(),
            launch,
            &self.root_path,
        )?);
        Ok(dmcl_launch::launch_chain(option, install))
    }

    /// 读取实例配置；有警告先弹确认框，否则直接开始。
    fn request_start(&mut self, name: String, sender: &ComponentSender<Self>) {
        let loaded = self.config.load_instance_sync(&self.root_path, &name);
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
        self.sync_cards();

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
        self.sync_cards();
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
        self.sync_cards();
    }

    /// 从 installed + runtimes 重建卡片并恢复选中项。
    fn sync_cards(&mut self) {
        let active: Vec<String> = self.runtimes.keys().cloned().collect();
        let names = visible_versions(&self.installed, active);

        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !names.contains(selected))
        {
            self.selected = None;
        }

        {
            let mut guard = self.instances.guard();
            guard.clear();
            for name in &names {
                let state = self
                    .runtimes
                    .get(name)
                    .map(|runtime| runtime.state)
                    .unwrap_or(InstanceState::Idle);
                guard.push_back((name.clone(), state));
            }
        }
        self.apply_selection();
    }

    fn apply_selection(&mut self) {
        let selected = self.selected.clone();
        let mut guard = self.instances.guard();
        for card in guard.iter_mut() {
            card.selected = selected.as_deref() == Some(card.name.as_str());
        }
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

    fn name_at(&self, index: &DynamicIndex) -> Option<String> {
        self.instances
            .get(index.current_index())
            .map(|card| card.name.clone())
    }

    fn state_of(&self, name: &str) -> InstanceState {
        self.runtimes
            .get(name)
            .map(|runtime| runtime.state)
            .unwrap_or(InstanceState::Idle)
    }

    fn select(&mut self, index: DynamicIndex) {
        let Some(name) = self.name_at(&index) else {
            return;
        };
        self.selected = Some(name);
        self.apply_selection();
    }

    fn deselect(&mut self) {
        self.selected = None;
        self.apply_selection();
    }
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

/// 只关心直接改变 `versions` 子项的事件：创建 / 删除 / 重命名。
///
/// 版本目录内部的文件写入、目录 metadata 变化、访问事件都不算列表变化。即使某个平台
/// 的 watcher 仍然把子目录里的事件送上来，这个过滤也能把它们挡掉。
fn event_changes_versions(event: &Event, watched: &Path) -> bool {
    let touches_watched = event
        .paths
        .iter()
        .any(|path| path == watched || path.parent() == Some(watched));
    if !touches_watched {
        return false;
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
    loop {
        let target = watch_target(&versions_dir);
        if target != watched {
            if let Some(old) = watched.take() {
                let _ = watcher.unwatch(&old);
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

        // 安装器一次会制造一串事件，合并成一次 Reload。
        tokio::time::sleep(Duration::from_millis(200)).await;
        while rx.try_recv().is_ok() {}
        if sender.input_sender().send(HomeMsg::Reload).is_err() {
            break;
        }
    }
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

        // 版本目录内部的文件写入不触发列表刷新。
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
