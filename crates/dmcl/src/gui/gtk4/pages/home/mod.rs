//! 主页：已安装实例的网格（组件）。
//!
//! 实例列表和运行状态都在 [`HomePage::instances`] 的工厂模型里；视图由 `view!`
//! 声明，`#[watch]` 负责在状态变化后刷新文案 / 图标 / 小绿点。启动与停止的游戏进程
//! 也由它持有——启动链交给 [`Shell`] 跑，但它等结果并把 monitor 交给一个 supervisor
//! 任务异步等退出（**不轮询**）。
//!
//! 实例列表会自动跟随 `versions` 目录的变化（轻量目录轮询）：安装 / 删除版本后无需
//! 手动刷新。

mod instance_card;
pub(crate) mod message;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use dmcl_launch::GameMonitor;
use dmcl_mcmeta::layout::installed_versions;
use dmcl_task::CancelHandle;

use crate::config::Config;
use crate::gui::gtk4::app::Shell;

use instance_card::{CardOut, InstanceCard};
pub(crate) use message::HomeMsg;

/// 主页的全局样式：底部胶囊操作栏、选中磁贴的高亮。
pub(crate) const CSS: &str = "\
.dmcl-action-bar { border-radius: 9999px; padding: 4px; box-shadow: 0 2px 8px 0 rgba(0, 0, 0, 0.18); }
.dmcl-instance-selected { background: color-mix(in srgb, currentColor 12%, transparent); border-radius: 12px; }";

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

pub(crate) struct HomePage {
    shell: Rc<Shell>,
    config: Config,
    /// 实例磁贴：模型即状态源，工厂负责同步控件。
    instances: FactoryVecDeque<InstanceCard>,
    /// 当前选中的磁贴。
    selected: Option<DynamicIndex>,
    /// 启动链的取消句柄：启动中（还没有游戏进程）用它取消。
    cancels: HashMap<String, CancelHandle>,
    /// 每个已启动实例的「停止信号」：supervisor 任务收到就 kill。
    ///
    /// 启动中也先建好：取消和启动完成撞车时，信号会缓冲到 supervisor 开始等它。
    stops: HashMap<String, tokio::sync::oneshot::Sender<()>>,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for HomePage {
    type Init = (Rc<Shell>, Config);
    type Input = HomeMsg;
    type Output = ();

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
                    set_description: Some("用 dmcl install 安装一个版本后再回来"),
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
        }
    }

    fn init(
        (shell, config): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let instances = FactoryVecDeque::builder()
            .launch(adw::WrapBox::new())
            .forward(sender.input_sender(), |msg| match msg {
                CardOut::Select(index) => HomeMsg::Select(index),
                CardOut::Launch(index) => HomeMsg::Launch(index),
            });
        let model = HomePage {
            shell,
            config,
            instances,
            selected: None,
            cancels: HashMap::new(),
            stops: HashMap::new(),
        };

        let grid = model.instances.widget();
        let widgets = view_output!();

        // 安装 / 删除版本后自动刷新：主页是常驻页面，不能只靠切标签触发。
        // 先同步拍一份快照再触发首次 Reload，避免「Reload 读完到监听开始」之间漏掉变化。
        let versions_dir = model.config.launch.minecraft_root_path.join("versions");
        let snapshot = versions_snapshot(&versions_dir);
        relm4::spawn(watch_versions(versions_dir, snapshot, sender.clone()));
        sender.input(HomeMsg::Reload);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            HomeMsg::Reload => {
                let root_path = self.config.launch.minecraft_root_path.clone();
                let sender = sender.clone();
                relm4::spawn(async move {
                    let result = installed_versions(&root_path)
                        .await
                        .map_err(|err| format!("{err:#}"));
                    sender.input(HomeMsg::Loaded(result));
                });
            }
            HomeMsg::Loaded(Ok(versions)) => {
                {
                    let mut guard = self.instances.guard();
                    guard.clear();
                    for version in versions {
                        guard.push_back(version.name);
                    }
                }
                self.selected = None;
            }
            HomeMsg::Loaded(Err(err)) => {
                self.shell.toast(&format!("读取已安装版本失败：{err}"));
            }

            HomeMsg::Select(index) => self.select(index),
            HomeMsg::Deselect => self.deselect(),
            HomeMsg::Launch(index) => {
                let Some(name) = self.name_at(&index) else {
                    return;
                };
                match self.state_at(&index) {
                    Some(InstanceState::Idle) => self.start(name, &sender),
                    Some(InstanceState::Starting) => {
                        self.shell.toast(&format!("{name} 正在启动"));
                    }
                    Some(InstanceState::Running) => {
                        self.shell.toast(&format!("{name} 已在运行"));
                    }
                    Some(InstanceState::Stopping) => {
                        self.shell.toast(&format!("{name} 正在停止"));
                    }
                    None => {}
                }
            }
            HomeMsg::Toggle => {
                let Some(index) = self.selected.clone() else {
                    return;
                };
                let Some(name) = self.name_at(&index) else {
                    return;
                };
                match self.state_at(&index) {
                    Some(InstanceState::Idle) => self.start(name, &sender),
                    Some(InstanceState::Starting | InstanceState::Running) => self.stop(&name),
                    _ => {}
                }
            }

            HomeMsg::GameStarted(name) => {
                self.cancels.remove(&name);
                // 取消和启动完成撞车：停止信号已经发出，supervisor 一开始等就会
                // 收到它。这里保持「停止中」，别闪一下「停止」。
                if self.state_of(&name) == Some(InstanceState::Stopping) {
                    return;
                }
                self.set_state(&name, InstanceState::Running);
            }
            HomeMsg::GameExited { name, abnormal } => {
                self.stops.remove(&name);
                self.cancels.remove(&name);
                self.set_state(&name, InstanceState::Idle);
                self.shell.toast(&if abnormal {
                    format!("{name} 异常退出")
                } else {
                    format!("{name} 已退出")
                });
            }
            HomeMsg::LaunchFailed(name) => {
                self.stops.remove(&name);
                self.cancels.remove(&name);
                self.set_state(&name, InstanceState::Idle);
            }
            HomeMsg::Manage => {
                let name = self.selected_name();
                if !name.is_empty() {
                    self.shell
                        .toast(&format!("打开 {name} 的管理页（尚未实现）"));
                }
            }
        }
    }
}

impl HomePage {
    /// 拼出 `dmcl-launch` 的一站式启动链。
    fn launch_chain(&self, name: &str) -> dmcl_task::TaskChain<(), GameMonitor> {
        let option = self.config.launch_option(self.config.account(), name);
        let install =
            std::sync::Arc::new(self.config.install_options(self.config.download_source()));
        dmcl_launch::launch_chain(option, install)
    }

    fn selected_card(&self) -> Option<&InstanceCard> {
        self.instances.get(self.selected.as_ref()?.current_index())
    }

    /// 操作栏上的实例名；没有选中时为空串（操作栏此时隐藏）。
    fn selected_name(&self) -> &str {
        self.selected_card()
            .map(|card| card.name.as_str())
            .unwrap_or_default()
    }

    fn action_state(&self) -> Option<InstanceState> {
        self.selected_card().map(|card| card.state)
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

    fn state_at(&self, index: &DynamicIndex) -> Option<InstanceState> {
        self.instances
            .get(index.current_index())
            .map(|card| card.state)
    }

    fn state_of(&self, name: &str) -> Option<InstanceState> {
        self.instances
            .iter()
            .find(|card| card.name == name)
            .map(|card| card.state)
    }

    fn select(&mut self, index: DynamicIndex) {
        let current = index.current_index();
        if self.instances.get(current).is_none() {
            return;
        }
        {
            let mut guard = self.instances.guard();
            for (position, card) in guard.iter_mut().enumerate() {
                card.selected = position == current;
            }
        }
        self.selected = Some(index);
    }

    fn deselect(&mut self) {
        {
            let mut guard = self.instances.guard();
            for card in guard.iter_mut() {
                card.selected = false;
            }
        }
        self.selected = None;
    }

    fn set_state(&mut self, name: &str, state: InstanceState) {
        let Some(index) = self.instances.iter().position(|card| card.name == name) else {
            return;
        };
        let mut guard = self.instances.guard();
        if let Some(card) = guard.get_mut(index) {
            card.state = state;
        }
    }

    fn start(&mut self, name: String, sender: &ComponentSender<Self>) {
        self.set_state(&name, InstanceState::Starting);
        let chain = self.launch_chain(&name);
        self.cancels.insert(name.clone(), chain.cancel_handle());
        let handle = self.shell.run(format!("启动 {name}"), chain, ());

        // supervisor 任务：等游戏退出（或停止信号），完了汇报。
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        self.stops.insert(name.clone(), stop_tx);
        let sender = sender.clone();
        relm4::spawn(async move {
            match handle.finished().await {
                Ok(monitor) => {
                    sender.input(HomeMsg::GameStarted(name.clone()));
                    supervise(monitor, stop_rx, sender.clone(), name).await;
                }
                Err(_) => sender.input(HomeMsg::LaunchFailed(name)),
            }
        });
    }

    fn stop(&mut self, name: &str) {
        // 启动中：取消启动链；运行中：让 supervisor kill 进程。
        // 两个都发，覆盖「链刚好跑完、进程已经起来」的竞态。
        if let Some(cancel) = self.cancels.remove(name) {
            cancel.cancel();
        }
        if let Some(stop) = self.stops.remove(name) {
            let _ = stop.send(());
        }
        self.set_state(name, InstanceState::Stopping);
    }
}

/// 等游戏进程退出：要么它自己退出，要么收到「停止」信号后 kill。
///
/// 用 `select!` + 「先 break 再 kill」的写法，避免在 `select!` 里对 monitor 双重可变借用。
async fn supervise(
    mut monitor: GameMonitor,
    mut stop_rx: tokio::sync::oneshot::Receiver<()>,
    sender: ComponentSender<HomePage>,
    name: String,
) {
    // 等「进程自己退出」或「停止信号」；两者都只取一个结果，不留悬空借用。
    let abnormal = tokio::select! {
        status = monitor.wait() => !matches!(status, Ok(status) if status.success()),
        _ = &mut stop_rx => {
            let _ = monitor.kill().await;
            let _ = monitor.wait().await;
            false
        }
    };
    sender.input(HomeMsg::GameExited { name, abnormal });
}

/// `versions` 目录快照：顶层子目录名 + 目录自身 mtime。
///
/// 主页列表就来自这些子目录（见 `installed_versions`）；目录里增删文件会更新目录
/// mtime，足以发现安装 / 删除。按名字排序，便于直接比较两份快照。
fn versions_snapshot(dir: &Path) -> Vec<(String, Option<SystemTime>)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // 目录还不存在（全新 root）视作空快照。
        return Vec::new();
    };
    let mut snapshot: Vec<(String, Option<SystemTime>)> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| {
            let modified = entry.metadata().ok().and_then(|meta| meta.modified().ok());
            (entry.file_name().to_string_lossy().into_owned(), modified)
        })
        .collect();
    snapshot.sort();
    snapshot
}

/// 轮询 `versions` 目录；有变化就给主页发一次 `Reload`。
///
/// `previous` 是调用方在首次 `Reload` **之前**拍下的快照，这样「首次读取」和「监听
/// 开始」之间不会有空档。
///
/// 不用事件监听是刻意的：版本目录很小，1s 一次目录快照的成本可以忽略；这样跨平台，
/// 也不用处理「目录还不存在 / 被整体替换」等监听器边界，更不用引入新依赖。
/// 发送失败（页面已关）就退出，避免往已销毁的组件里发消息。
async fn watch_versions(
    dir: PathBuf,
    mut previous: Vec<(String, Option<SystemTime>)>,
    sender: ComponentSender<HomePage>,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let current = versions_snapshot(&dir);
        if current != previous {
            previous = current;
            if sender.input_sender().send(HomeMsg::Reload).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_snapshot_lists_only_directories_sorted() {
        let root = std::env::temp_dir().join(format!("dmcl-home-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("1.21.10")).unwrap();
        std::fs::create_dir_all(root.join("1.20.1-forge")).unwrap();
        std::fs::write(root.join("note.txt"), "x").unwrap();

        let names: Vec<String> = versions_snapshot(&root)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, vec!["1.20.1-forge", "1.21.10"]);

        // 目录不存在 = 空快照（全新 root 也能起监听）。
        assert!(versions_snapshot(&root.join("missing")).is_empty());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
