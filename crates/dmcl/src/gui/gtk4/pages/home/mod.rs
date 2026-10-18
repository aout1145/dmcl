//! 主页：已安装实例的网格（组件）。
//!
//! 实例列表与「运行中」状态都归它；启动/停止的游戏进程也由它持有——启动链交给
//! [`Shell`](crate::gui::gtk4::app::Shell) 跑，但它等结果并把 monitor 交给一个
//! supervisor 任务异步等退出（**不轮询**）。

mod instance_card;
pub(crate) mod message;

use std::collections::HashMap;
use std::rc::Rc;

use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use dmcl_launch::GameMonitor;
use dmcl_mcmeta::layout::installed_versions;
use dmcl_task::CancelHandle;

use crate::config::Config;
use crate::gui::gtk4::app::Shell;

pub(crate) use message::HomeMsg;

/// 主页的全局样式：底部胶囊操作栏、选中磁贴的高亮。
pub(crate) const CSS: &str = "\
.dmcl-action-bar { border-radius: 9999px; padding: 4px; box-shadow: 0 2px 8px 0 rgba(0, 0, 0, 0.18); }
.dmcl-instance-selected { background: color-mix(in srgb, currentColor 12%, transparent); border-radius: 12px; }";

/// 实例的运行状态。界面文案、按钮动作、磁贴绿点都从它派生，避免多个布尔量打架。
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

/// 一个实例在界面上的视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstanceView {
    pub name: String,
    pub state: InstanceState,
}

impl InstanceView {
    /// 磁贴绿点只表示游戏进程真的在跑，启动链阶段不算。
    pub(crate) fn is_running(&self) -> bool {
        self.state == InstanceState::Running
    }
}

pub(crate) struct HomePage {
    shell: Rc<Shell>,
    config: Config,
    /// 网格 / 空状态用 `GtkStack` 切换。
    stack: gtk::Stack,
    grid: adw::WrapBox,
    /// 底部胶囊操作栏（只在选中实例时显示）。
    bar: gtk::Box,
    /// 当前选中的实例名。
    selected: Option<String>,
    /// 磁贴（实例名 → 按钮），用于切换选中样式。
    cards: HashMap<String, gtk::Button>,
    instances: Vec<InstanceView>,
    /// 启动链的取消句柄：启动中（还没有游戏进程）用它取消。
    cancels: HashMap<String, CancelHandle>,
    /// 每个已启动实例的「停止信号」：supervisor 任务收到就 kill。
    ///
    /// 启动中也先建好：取消和启动完成撞车时，信号会缓冲到 supervisor 开始等它。
    stops: HashMap<String, tokio::sync::oneshot::Sender<()>>,
}

impl SimpleComponent for HomePage {
    type Init = (Rc<Shell>, Config);
    type Input = HomeMsg;
    type Output = ();
    type Widgets = ();
    type Root = gtk::Overlay;

    fn init_root() -> Self::Root {
        gtk::Overlay::new()
    }

    fn init(
        (shell, config): Self::Init,
        overlay: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let grid = adw::WrapBox::builder()
            .child_spacing(12)
            .line_spacing(12)
            .margin_top(18)
            .margin_bottom(80)
            .margin_start(18)
            .margin_end(18)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&grid)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        {
            // 点空白处取消选中（点磁贴时按钮会 claim 掉点击，不会冒泡到这里）。
            let sender = sender.clone();
            let background = gtk::GestureClick::new();
            background.connect_pressed(move |_, _, _, _| sender.input(HomeMsg::Deselect));
            scrolled.add_controller(background);
        }

        let empty = adw::StatusPage::builder()
            .icon_name("view-grid-symbolic")
            .title("还没有安装任何版本")
            .description("用 dmcl install 安装一个版本后再回来")
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&scrolled, Some("grid"));
        stack.add_named(&empty, Some("empty"));
        stack.set_visible_child_name("empty");

        // 底部胶囊操作栏：浮在网格上，只在选中实例时出现。
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(18)
            .build();
        bar.add_css_class("card");
        bar.add_css_class("dmcl-action-bar");
        bar.set_visible(false);

        overlay.set_child(Some(&stack));
        overlay.add_overlay(&bar);

        let model = HomePage {
            shell,
            config,
            stack,
            grid,
            bar,
            selected: None,
            cards: HashMap::new(),
            instances: Vec::new(),
            cancels: HashMap::new(),
            stops: HashMap::new(),
        };
        sender.input(HomeMsg::Reload);
        ComponentParts { model, widgets: () }
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
                self.instances = versions
                    .into_iter()
                    .map(|version| InstanceView {
                        name: version.name,
                        state: InstanceState::Idle,
                    })
                    .collect();
                // 选中的实例如果没了，就取消选中。
                let selected_present = self.selected.as_ref().is_some_and(|selected| {
                    self.instances.iter().any(|item| &item.name == selected)
                });
                if !selected_present {
                    self.selected = None;
                }
                self.render(&sender);
            }
            HomeMsg::Loaded(Err(err)) => {
                self.shell.toast(&format!("读取已安装版本失败：{err}"));
            }

            HomeMsg::Select(name) => {
                self.selected = Some(name);
                // 只改样式，不重建网格——重建会把正在双击的按钮换掉。
                self.apply_selection();
                self.render_bar(&sender);
            }
            HomeMsg::Deselect => {
                self.selected = None;
                self.apply_selection();
                self.render_bar(&sender);
            }

            HomeMsg::Launch(name) => {
                let Some(state) = self.state_of(&name) else {
                    return;
                };
                match state {
                    InstanceState::Starting => {
                        self.shell.toast(&format!("{name} 正在启动"));
                        return;
                    }
                    InstanceState::Running => {
                        self.shell.toast(&format!("{name} 已在运行"));
                        return;
                    }
                    InstanceState::Stopping => {
                        self.shell.toast(&format!("{name} 正在停止"));
                        return;
                    }
                    InstanceState::Idle => {}
                }
                self.set_state(&name, InstanceState::Starting);
                // 立刻把按钮切成「取消」，不等启动链返回。
                self.render_bar(&sender);
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
            HomeMsg::GameStarted(name) => {
                self.cancels.remove(&name);
                // 取消和启动完成撞车：停止信号已经发出，supervisor 一开始等就会
                // 收到它。这里保持「停止中」，别闪一下「停止」。
                if self.state_of(&name) == Some(InstanceState::Stopping) {
                    return;
                }
                self.set_state(&name, InstanceState::Running);
                self.render(&sender);
            }
            HomeMsg::GameExited { name, abnormal } => {
                self.stops.remove(&name);
                self.cancels.remove(&name);
                self.set_state(&name, InstanceState::Idle);
                self.render(&sender);
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
                self.render_bar(&sender);
            }
            HomeMsg::Stop(name) => {
                if let Some(InstanceState::Starting | InstanceState::Running) = self.state_of(&name)
                {
                    // 启动中：取消启动链；运行中：让 supervisor kill 进程。
                    // 两个都发，覆盖「链刚好跑完、进程已经起来」的竞态。
                    if let Some(cancel) = self.cancels.remove(&name) {
                        cancel.cancel();
                    }
                    if let Some(stop) = self.stops.remove(&name) {
                        let _ = stop.send(());
                    }
                    self.set_state(&name, InstanceState::Stopping);
                    // 只重绘操作栏：磁贴绿点保持点击前的样子，等 GameExited 再收。
                    self.render_bar(&sender);
                }
            }
            HomeMsg::Manage(name) => {
                self.shell
                    .toast(&format!("打开 {name} 的管理页（尚未实现）"));
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

    /// 查某个实例当前的状态；实例已经不在列表里就返回 `None`。
    fn state_of(&self, name: &str) -> Option<InstanceState> {
        self.instances
            .iter()
            .find(|item| item.name == name)
            .map(|item| item.state)
    }

    fn set_state(&mut self, name: &str, state: InstanceState) {
        if let Some(instance) = self.instances.iter_mut().find(|item| item.name == name) {
            instance.state = state;
        }
    }

    /// 按模型重建网格与底部操作栏。
    fn render(&mut self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.grid.first_child() {
            self.grid.remove(&child);
        }
        self.cards.clear();
        if self.instances.is_empty() {
            self.stack.set_visible_child_name("empty");
            self.bar.set_visible(false);
            return;
        }
        for instance in &self.instances {
            let selected = self.selected.as_deref() == Some(instance.name.as_str());
            let card = instance_card::build(instance, selected, sender);
            self.grid.append(&card);
            self.cards.insert(instance.name.clone(), card);
        }
        self.stack.set_visible_child_name("grid");
        self.render_bar(sender);
    }

    /// 只更新磁贴的选中样式，不重建网格（否则双击会被打断）。
    fn apply_selection(&self) {
        for (name, card) in &self.cards {
            if self.selected.as_deref() == Some(name.as_str()) {
                card.add_css_class("dmcl-instance-selected");
            } else {
                card.remove_css_class("dmcl-instance-selected");
            }
        }
    }

    /// 重建底部胶囊操作栏的内容（只显示选中的实例）。
    fn render_bar(&self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.bar.first_child() {
            self.bar.remove(&child);
        }
        let Some(name) = self.selected.as_deref() else {
            self.bar.set_visible(false);
            return;
        };
        let Some(state) = self.state_of(name) else {
            self.bar.set_visible(false);
            return;
        };

        let label = gtk::Label::new(Some(name));
        label.add_css_class("heading");
        label.set_margin_start(10);
        label.set_margin_end(4);
        self.bar.append(&label);

        // 文案、图标、点击动作全部由同一个状态决定；`None` 表示按钮不可点。
        let (text, icon, action) = match state {
            InstanceState::Idle => (
                "启动",
                "media-playback-start-symbolic",
                Some(HomeMsg::Launch(name.to_string())),
            ),
            InstanceState::Starting => (
                "取消",
                "process-stop-symbolic",
                Some(HomeMsg::Stop(name.to_string())),
            ),
            InstanceState::Running => (
                "停止",
                "media-playback-stop-symbolic",
                Some(HomeMsg::Stop(name.to_string())),
            ),
            InstanceState::Stopping => ("停止中…", "content-loading-symbolic", None),
        };
        let toggle = action_button(text, icon);
        toggle.set_sensitive(action.is_some());
        if let Some(action) = action {
            let sender = sender.clone();
            toggle.connect_clicked(move |_| sender.input(action.clone()));
        }
        self.bar.append(&toggle);

        let manage = action_button("管理", "document-properties-symbolic");
        {
            let sender = sender.clone();
            let name = name.to_string();
            manage.connect_clicked(move |_| sender.input(HomeMsg::Manage(name.clone())));
        }
        self.bar.append(&manage);

        let close = icon_button("window-close-symbolic");
        {
            let sender = sender.clone();
            close.connect_clicked(move |_| sender.input(HomeMsg::Deselect));
        }
        self.bar.append(&close);
        self.bar.set_visible(true);
    }
}

/// 操作栏里的图标 + 文字按钮。
///
/// `GtkButton` 只有一个 child，`set_label` / `set_icon_name` 会互相覆盖，
/// 所以图标和文字要交给 `AdwButtonContent` 一起放进 child。
fn action_button(text: &str, icon: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder()
        .icon_name(icon)
        .label(text)
        .build();
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class("flat");
    button
}

/// 操作栏里的纯图标按钮。
fn icon_button(icon: &str) -> gtk::Button {
    let button = gtk::Button::builder().icon_name(icon).build();
    button.add_css_class("flat");
    button
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
