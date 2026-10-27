//! 顶层组件：窗口 + 标题栏 + 标签页；只做组装。
//!
//! 每个 root 的主页从启动起就常驻：关闭标签页只影响用户看到的标签，不销毁
//! `HomePage`。运行状态、watcher、选中项都由常驻的主页保留。
//!
//! 页面与任务对话框（`tasks`）都是独立组件、各带自己的消息。一次性页面（实现
//! [`Page`](page_host::Page)）交给 [`PageHost`](page_host::PageHost) 懒创建；常驻页面由
//! [`PageSlot`](page_host::PageSlot) 直接持有。共享能力（跑任务、弹提示、打开任务对话框）
//! 在 [`Shell`] 里；共享配置是 [`config_store`] 里的全局单例，根组件负责初始化和订阅。
//! 这里没有实例业务逻辑。

pub(crate) mod config_store;
mod config_warning;
mod header;
pub(crate) mod page_host;
pub(crate) mod shell;
mod tasks;
mod toast;

pub(crate) use config_store::ConfigRef;
pub(crate) use shell::Shell;
pub(crate) use toast::{PageToast, ToastService};

use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;
use relm4::Sender;

use crate::config::{ConfigError, LoadedConfig};
use crate::gui::gtk4::pages::download::common;
use crate::gui::gtk4::pages::download::custom::CustomPage;
use crate::gui::gtk4::pages::home::{HomeMsg, HomeOut, HomePage};
use crate::gui::gtk4::pages::settings::{PreferencesMsg, PreferencesPage};
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
    /// 共享配置改过了：补建新增 root 的主页，并刷新标题栏菜单。
    ConfigChanged,
    /// 打开或聚焦首选项（应用级快捷键）。
    OpenPreferences,
    /// 打开「关于」对话框（主菜单的应用级 action）。
    About,
    /// 某个标签已经被移除；清理对应页面的标签句柄。
    TabClosed(adw::TabPage),
    /// 主窗口收到关闭请求。
    CloseRequested,
}

/// 一个 root 的常驻主页。`name` 是创建时的 root 名字，用来和配置对齐。
struct HomeSlot {
    name: String,
    page: PageSlot<HomePage>,
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
    homes: Vec<HomeSlot>,
    /// 首选项：常驻页面，最多一个标签。
    preferences: PageSlot<PreferencesPage>,
    /// 打开「新实例」页时要用。
    shell: Rc<Shell>,
    /// 共享配置的只读句柄，始终指向当前那份配置。
    config: ConfigRef,
    tab_view: adw::TabView,
    /// 主窗口：任务组件确认可以退出后销毁它。
    window: adw::ApplicationWindow,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for AppModel {
    type Init = (Option<ConfigError>, String);
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
        (error, startup_root_name): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // 全局仓库已在 `run` 里初始化；配置变更只由根组件监听，再分发给各页面。
        let config = config_store::store().config();
        let mut changes = config_store::store().changes();
        let change_sender = sender.clone();
        relm4::spawn_local(async move {
            while changes.changed().await.is_ok() {
                if change_sender
                    .input_sender()
                    .send(AppMsg::ConfigChanged)
                    .is_err()
                {
                    break;
                }
            }
        });
        let tab_view = adw::TabView::new();

        // 页面宿主：只负责 CustomPage 这类关闭即销毁的一次性页面。
        let page_host = Rc::new(PageHost::new(tab_view.clone()));

        // 应用级 action：主菜单项和应用快捷键共用同一组入口，避免菜单点击走另一条路径。
        register_menu_actions(&sender);

        // 标题栏是独立组件；需要页面宿主 / 任务组件的动作通过 HeaderOut 回来。
        let roots = config.read().roots();
        let header = Header::builder()
            .launch((tab_view.clone(), root.clone(), roots))
            .forward(sender.input_sender(), AppMsg::Header);
        let header_widget = header.widget().clone();

        let tab_view_widget = &tab_view;
        let widgets = view_output!();

        // 配置加载有问题时，等主窗口显示后写回修复后的配置，并弹窗说明结果。
        if let Some(error) = error {
            let window = root.clone();
            gtk::glib::idle_add_local_once(move || config_warning::repair(&window, &error));
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
        let roots = config.read().roots();
        let mut homes = Vec::new();
        for (name, path) in roots {
            let controller = HomePage::builder()
                .launch((shell.clone(), path, root.clone()))
                .forward(sender.input_sender(), {
                    let name = name.clone();
                    move |out| AppMsg::Home {
                        root: name.clone(),
                        out,
                    }
                });
            homes.push(HomeSlot {
                name,
                page: PageSlot {
                    controller,
                    tab: None,
                },
            });
        }

        // 设置页同样是常驻页面；启动时创建，之后重复触发只聚焦已有标签。
        let preferences = PageSlot {
            controller: PreferencesPage::builder().launch(shell.clone()).detach(),
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
            preferences,
            shell,
            config,
            tab_view,
            window: root.clone(),
        };
        model.open_home(&startup_root_name);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            AppMsg::Header(HeaderOut::Tasks) => {
                let _ = self.tasks.send(TaskMsg::Open);
            }
            AppMsg::Header(HeaderOut::Home(name)) => self.open_home(&name),
            AppMsg::ConfigChanged => {
                // roots 只增不减：新 root 补建常驻主页，删掉的保留页面；菜单按配置刷新。
                let roots = self.config.read().roots();
                self.sync_roots(roots, &sender);
                for home in &self.homes {
                    let _ = home.page.controller.sender().send(HomeMsg::ConfigChanged);
                }
                let _ = self
                    .preferences
                    .controller
                    .sender()
                    .send(PreferencesMsg::ConfigChanged);
            }
            AppMsg::OpenPreferences => self.open_preferences(),
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
                for home in &mut self.homes {
                    home.page.close(&page);
                }
                self.preferences.close(&page);
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
        let Some(home) = self.homes.iter_mut().find(|home| home.name == name) else {
            self.shell
                .global_toast(&format!("配置里找不到主页：{name}"));
            return;
        };
        let reopen = home.page.tab.is_none();
        home.page.open(&self.tab_view, &home_tab_title(name));
        if reopen {
            // 隐藏期间目录可能已经变化；重新打开时补一次刷新。
            let _ = home.page.controller.sender().send(HomeMsg::Reload);
        }
        self.sync_header_homes();
    }

    /// 打开或聚焦首选项。
    fn open_preferences(&mut self) {
        self.preferences.open(&self.tab_view, "首选项");
    }

    /// 新建实例页绑定到发出请求的主页 root。
    fn open_custom(&mut self, root_name: &str) {
        let root_path = match self.config.read().root_path(root_name) {
            Ok(path) => path,
            Err(err) => {
                self.shell
                    .global_toast(&format!("无法确定安装目录：{err:#}"));
                return;
            }
        };
        self.page_host
            .open_page::<CustomPage>((self.shell.clone(), root_path));
    }

    /// 配置里的 root 有增减：给新 root 补建主页，再把菜单交给标题栏。
    ///
    /// 只在新增时建页面——同名的 root 改路径不重建（页面已经按旧路径挂了 watcher），
    /// 删除 root 也不销毁页面，已经打开的标签保留。
    fn sync_roots(&mut self, roots: Vec<(String, PathBuf)>, sender: &ComponentSender<Self>) {
        for (name, path) in &roots {
            if self.homes.iter().any(|home| &home.name == name) {
                continue;
            }
            let controller = HomePage::builder()
                .launch((self.shell.clone(), path.clone(), self.window.clone()))
                .forward(sender.input_sender(), {
                    let name = name.clone();
                    move |out| AppMsg::Home {
                        root: name.clone(),
                        out,
                    }
                });
            self.homes.push(HomeSlot {
                name: name.clone(),
                page: PageSlot {
                    controller,
                    tab: None,
                },
            });
        }
        let _ = self.header.sender().send(HeaderMsg::SetRoots(roots));
    }

    fn sync_header_homes(&self) {
        let mut open: Vec<String> = self
            .homes
            .iter()
            .filter(|home| home.page.tab.is_some())
            .map(|home| home.name.clone())
            .collect();
        open.sort();
        open.dedup();
        let _ = self.header.sender().send(HeaderMsg::SetOpenHomes(open));
    }
}

/// 注册主菜单项对应的应用级 action 与快捷键。
fn register_menu_actions(sender: &ComponentSender<AppModel>) {
    let app = relm4::main_adw_application();

    let action = gtk::gio::SimpleAction::new("preferences", None);
    let action_sender = sender.clone();
    action.connect_activate(move |_, _| action_sender.input(AppMsg::OpenPreferences));
    app.add_action(&action);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);

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
    // 配置只读一次，之后所有页面都从全局仓库取；加载警告留给根组件弹窗。
    let LoadedConfig { config, error } = loaded;
    config_store::init(config);
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
        crate::gui::gtk4::pages::settings::CSS,
        common::CSS,
        tasks::CSS,
        toast::CSS,
        config_warning::CSS,
    ] {
        relm4::set_global_css_with_priority(css, relm4::gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    app.run::<AppModel>((error, startup_root));
    Ok(())
}
