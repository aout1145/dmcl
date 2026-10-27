//! 顶层组件：窗口 + 标题栏 + 标签页；只做组装。
//!
//! 每个 root 的主页从启动起就常驻：关闭标签页只影响用户看到的标签，不销毁
//! `HomePage`。运行状态、watcher、选中项都由常驻的主页保留。
//!
//! 页面与任务对话框（`tasks`）都是独立组件、各带自己的消息。一次性页面（实现
//! [`Page`](page_host::Page)）交给 [`PageHost`](page_host::PageHost) 懒创建；常驻页面由
//! [`PageSlot`](page_host::PageSlot) 直接持有。共享能力（跑任务、弹提示、打开任务对话框）
//! 在 [`Shell`] 里。这里没有实例业务逻辑。

mod config_warning;
mod header;
pub(crate) mod page_host;
pub(crate) mod shell;
mod tasks;
mod toast;

pub(crate) use shell::Shell;
pub(crate) use toast::{PageToast, ToastService};

use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;
use relm4::Sender;

use crate::config::{Config, LoadedConfig};
use crate::gui::gtk4::pages::download::common;
use crate::gui::gtk4::pages::download::custom::CustomPage;
use crate::gui::gtk4::pages::home::{HomeMsg, HomeOut, HomePage};
use crate::gui::gtk4::pages::settings::{GameOptionsPage, LauncherOptionsPage};
use header::{Header, HeaderMsg, HeaderOut};
use page_host::{PageHost, PageSlot};
use tasks::{TaskCenter, TaskCenterOut, TaskMsg};

/// 根组件收到的消息。
#[derive(Debug)]
enum AppMsg {
    Header(HeaderOut),
    Tasks(TaskCenterOut),
    /// 某个主页发出的动作；`root` 是创建该主页时捕获的 root，不需要跟踪当前选中的主页。
    Home {
        root: String,
        out: HomeOut,
    },
    /// 打开或聚焦启动器选项（应用级快捷键）。
    OpenLauncherOptions,
    /// 打开或聚焦全局游戏选项（应用级快捷键）。
    OpenGameOptions,
    /// 打开「关于」对话框（主菜单的应用级 action）。
    About,
    /// 某个标签已经被移除；清理对应页面的标签句柄。
    TabClosed(adw::TabPage),
    /// 主窗口收到关闭请求。
    CloseRequested,
}

struct AppModel {
    // 子组件靠持有 Controller 存活。
    _tasks: Controller<TaskCenter>,
    header: Controller<Header>,
    /// 任务入口：标题栏的「任务」按钮转发给任务组件。
    tasks: Sender<TaskMsg>,
    // 一次性页面的宿主：打开的 CustomPage 由它保活。
    page_host: Rc<PageHost>,
    /// 所有 root 的主页；关闭标签只清空槽位的标签句柄，组件继续存活。
    homes: Vec<(String, PageSlot<HomePage>)>,
    /// 启动器选项：常驻页面，最多一个标签。
    launcher_options: PageSlot<LauncherOptionsPage>,
    /// 全局游戏选项：常驻页面，最多一个标签。
    game_options: PageSlot<GameOptionsPage>,
    /// 打开「新实例」页时要用。
    shell: Rc<Shell>,
    config: Config,
    tab_view: adw::TabView,
    /// 主窗口：任务组件确认可以退出后销毁它。
    window: adw::ApplicationWindow,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for AppModel {
    type Init = (LoadedConfig, String);
    type Input = AppMsg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_application: Some(&relm4::main_adw_application()),
            set_title: Some("DMCL"),
            set_default_width: 960,
            set_default_height: 640,

            // 交给任务组件判断：没任务直接放行，有任务先确认。游戏进程是独立的，
            // 关闭窗口不会结束正在运行的游戏。
            connect_close_request[sender] => move |_| {
                sender.input(AppMsg::CloseRequested);
                gtk::glib::Propagation::Stop
            },

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                // 标题栏（一行：根目录 / 标签页栏 / 任务主菜单 / 窗口按钮）
                add_top_bar: &header_widget,

                #[wrap(Some)]
                set_content = &gtk::Overlay {
                    #[local_ref]
                    tab_view_widget -> adw::TabView {},

                    // 主界面提示层：自制的堆叠 toast（AdwToastOverlay 一次只能显示一条）。
                    #[name(toast_layer)]
                    add_overlay = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 6,
                        set_halign: gtk::Align::Start,
                        set_valign: gtk::Align::End,
                        set_margin_start: 18,
                        set_margin_bottom: 18,
                    },
                },
            },
        }
    }

    fn init(
        (loaded, startup_root_name): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let LoadedConfig { config, error } = loaded;
        let tab_view = adw::TabView::new();

        // 页面宿主：只负责 CustomPage 这类关闭即销毁的一次性页面。
        let page_host = Rc::new(PageHost::new(tab_view.clone()));

        // 应用级 action：主菜单项和应用快捷键共用同一组入口，避免菜单点击走另一条路径。
        register_menu_actions(&sender);

        // 标题栏是独立组件；需要页面宿主 / 任务组件的动作通过 HeaderOut 回来。
        let roots = config.roots();
        let header = Header::builder()
            .launch((tab_view.clone(), root.clone(), roots))
            .forward(sender.input_sender(), AppMsg::Header);
        let header_widget = header.widget().clone();

        let tab_view_widget = &tab_view;
        let widgets = view_output!();

        // 配置加载有问题时，等主窗口显示后再弹汇总警告；关掉后按修复后的配置继续。
        if let Some(error) = error {
            let window = root.clone();
            let config = config.clone();
            gtk::glib::idle_add_local_once(move || config_warning::show(&window, &config, &error));
        }

        // 任务对话框：组件本身就是那个模态窗口，自己管卡片/确认/提示/关闭清理。
        let toasts = ToastService::new(widgets.toast_layer.clone());
        let tasks = TaskCenter::builder()
            .launch((root.clone(), toasts.clone()))
            .forward(sender.input_sender(), AppMsg::Tasks);

        // 共享服务：任务入口 + 提示层
        let shell = Rc::new(Shell::new(tasks.sender().clone(), toasts));
        let tasks_sender = tasks.sender().clone();

        // 所有 root 的主页从启动起就常驻；启动时只给默认 root 建标签页。
        let mut homes = Vec::new();
        for (name, path) in config.roots() {
            let controller = HomePage::builder()
                .launch((shell.clone(), config.clone(), path, root.clone()))
                .forward(sender.input_sender(), {
                    let name = name.clone();
                    move |out| AppMsg::Home {
                        root: name.clone(),
                        out,
                    }
                });
            homes.push((
                name,
                PageSlot {
                    controller,
                    tab: None,
                },
            ));
        }

        // 设置页同样是常驻页面；启动时创建，之后重复触发只聚焦已有标签。
        let launcher_options = PageSlot {
            controller: LauncherOptionsPage::builder().launch(()).detach(),
            tab: None,
        };
        let game_options = PageSlot {
            controller: GameOptionsPage::builder().launch(()).detach(),
            tab: None,
        };

        // 标签被移除时只清掉各页面的标签句柄；常驻页面继续存活，运行状态不会丢。
        // 关闭主窗口时，`AdwTabView` 会在组件 runtime 关闭之后才 detach 标签页；此时
        // 清理已经没有意义，用 `send` 静默丢弃，避免 `input()` 因 receiver 已关闭而 panic。
        {
            let input = sender.input_sender().clone();
            tab_view.connect_page_detached(move |_, page, _| {
                let _ = input.send(AppMsg::TabClosed(page.clone()));
            });
        }

        let mut model = AppModel {
            _tasks: tasks,
            header,
            tasks: tasks_sender,
            page_host,
            homes,
            launcher_options,
            game_options,
            shell,
            config,
            tab_view,
            window: root.clone(),
        };
        model.open_home(&startup_root_name);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            AppMsg::Header(HeaderOut::Tasks) => {
                let _ = self.tasks.send(TaskMsg::Open);
            }
            AppMsg::Header(HeaderOut::Home(name)) => self.open_home(&name),
            AppMsg::OpenLauncherOptions => self.open_launcher_options(),
            AppMsg::OpenGameOptions => self.open_game_options(),
            AppMsg::About => {
                let _ = self.header.sender().send(HeaderMsg::About);
            }
            AppMsg::Home {
                root,
                out: HomeOut::CreateCustom,
            } => self.open_custom(&root),
            AppMsg::Tasks(TaskCenterOut::RunningCount(count)) => {
                let _ = self.header.sender().send(HeaderMsg::SetRunningCount(count));
            }
            AppMsg::Tasks(TaskCenterOut::Quit) => self.window.destroy(),
            AppMsg::TabClosed(page) => {
                // 一次性页面：关闭即销毁。
                self.page_host.close(&page);
                // 常驻页面：只清空标签句柄，组件继续存活。
                for (_, slot) in &mut self.homes {
                    slot.close(&page);
                }
                self.launcher_options.close(&page);
                self.game_options.close(&page);
                self.sync_header_homes();
            }
            AppMsg::CloseRequested => {
                let _ = self.tasks.send(TaskMsg::CloseMain);
            }
        }
    }
}

impl AppModel {
    /// 打开或聚焦某个 root 的主页标签。
    fn open_home(&mut self, name: &str) {
        let Some(index) = self
            .homes
            .iter()
            .position(|(home_root, _)| home_root == name)
        else {
            self.shell
                .global_toast(&format!("配置里找不到主页：{name}"));
            return;
        };
        let (_, slot) = &mut self.homes[index];
        let reopen = slot.tab.is_none();
        slot.open(&self.tab_view, &home_tab_title(name));
        if reopen {
            // 隐藏期间目录可能已经变化；重新打开时补一次刷新。
            let _ = slot.controller.sender().send(HomeMsg::Reload);
        }
        self.sync_header_homes();
    }

    /// 打开或聚焦启动器选项。
    fn open_launcher_options(&mut self) {
        self.launcher_options.open(&self.tab_view, "启动器选项");
    }

    /// 打开或聚焦全局游戏选项。
    fn open_game_options(&mut self) {
        self.game_options.open(&self.tab_view, "全局游戏选项");
    }

    /// 新建实例页绑定到发出请求的主页 root。
    fn open_custom(&mut self, root_name: &str) {
        let root_path = match self.config.root_path(root_name) {
            Ok(path) => path,
            Err(err) => {
                self.shell
                    .global_toast(&format!("无法确定安装目录：{err:#}"));
                return;
            }
        };
        self.page_host.open_page::<CustomPage>((
            self.shell.clone(),
            self.config.clone(),
            root_path,
        ));
    }

    fn sync_header_homes(&self) {
        let mut open: Vec<String> = self
            .homes
            .iter()
            .filter(|(_, slot)| slot.tab.is_some())
            .map(|(name, _)| name.clone())
            .collect();
        open.sort();
        open.dedup();
        let _ = self.header.sender().send(HeaderMsg::SetOpenHomes(open));
    }
}

/// 注册主菜单项对应的应用级 action 与快捷键。
fn register_menu_actions(sender: &ComponentSender<AppModel>) {
    let app = relm4::main_adw_application();

    let action = gtk::gio::SimpleAction::new("launcher-options", None);
    let action_sender = sender.clone();
    action.connect_activate(move |_, _| action_sender.input(AppMsg::OpenLauncherOptions));
    app.add_action(&action);
    app.set_accels_for_action("app.launcher-options", &["<Control>comma"]);

    let action = gtk::gio::SimpleAction::new("game-options", None);
    let action_sender = sender.clone();
    action.connect_activate(move |_, _| action_sender.input(AppMsg::OpenGameOptions));
    app.add_action(&action);
    // Shift 会把逗号键的 keyval 变成 `<`（less）；两个都注册，保证 `Ctrl+Shift+,`
    // 在不同键盘布局 / GDK 后端下都能触发。
    app.set_accels_for_action(
        "app.game-options",
        &["<Control><Shift>comma", "<Control><Shift>less"],
    );

    let action = gtk::gio::SimpleAction::new("about", None);
    let action_sender = sender.clone();
    action.connect_activate(move |_, _| action_sender.input(AppMsg::About));
    app.add_action(&action);
}

fn home_tab_title(root_name: &str) -> String {
    format!("主页 · {root_name}")
}

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub(crate) fn run(loaded: LoadedConfig, startup_root: String) -> anyhow::Result<()> {
    // 只把程序名传给 GApplication：`dmcl gui` 里的 `gui` 是给我们自己解析的，
    // 别让它被当成要打开的文件（否则 GLib 会警告 "can not open files"）。
    let mut args: Vec<String> = std::env::args().take(1).collect();
    if args.is_empty() {
        args.push("dmcl".to_string());
    }
    let app = RelmApp::new("io.github.aout1145.DMCL").with_args(args);
    // 各模块自带自己的全局 CSS，这里统一挂上。
    for css in [
        header::CSS,
        header::THEME_CSS,
        crate::gui::gtk4::pages::home::CSS,
        common::CSS,
        tasks::CSS,
        toast::CSS,
        config_warning::CSS,
    ] {
        relm4::set_global_css_with_priority(css, relm4::gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    app.run::<AppModel>((loaded, startup_root));
    Ok(())
}
