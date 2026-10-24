//! 顶层组件：窗口 + 标题栏 + 标签页；只做组装。
//!
//! 每个 root 的主页是可关闭、可重新打开的独立标签。关闭时直接销毁 `HomePage`；
//! 游戏进程继续运行，但页面里的运行状态不会跨页面保留。
//!
//! 页面（`pages/` 下实现 [`Page`](page_host::Page) 的组件）与任务对话框（`tasks`）都是
//! 独立组件、各带自己的消息；页面的打开/关闭/保活交给 [`PageHost`](page_host::PageHost)，
//! 共享能力（跑任务、弹提示、打开任务对话框）在 [`Shell`] 里。这里没有实例业务逻辑。

mod config_warning;
mod header;
pub(crate) mod page_host;
pub(crate) mod shell;
mod tasks;
mod toast;

pub(crate) use shell::Shell;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;
use relm4::Sender;

use crate::config::{Config, LoadedConfig, MinecraftRoot};
use crate::gui::gtk4::pages::download::common;
use crate::gui::gtk4::pages::download::custom::CustomPage;
use crate::gui::gtk4::pages::home::{HomeOut, HomePage};
use header::{Header, HeaderMsg, HeaderOut};
use page_host::PageHost;
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
    /// 某个主页标签已经被移除。消息只带 root 名字，避免把 GObject 放进 Debug 消息。
    HomeClosed(String),
    /// 主窗口收到关闭请求。
    CloseRequested,
}

/// 一个已打开的主页标签。
struct HomeEntry {
    _controller: Controller<HomePage>,
    page: adw::TabPage,
}

struct AppModel {
    // 子组件靠持有 Controller 存活。
    _tasks: Controller<TaskCenter>,
    header: Controller<Header>,
    /// 任务入口：标题栏的「任务」按钮转发给任务组件。
    tasks: Sender<TaskMsg>,
    // 宿主本身要活到窗口关闭（打开的页面由它保活）。
    page_host: Rc<PageHost>,
    /// 主页标签 → root 名字；关闭标签时用来清理对应的 HomeEntry。
    home_pages: Rc<RefCell<Vec<(adw::TabPage, String)>>>,
    /// 已打开的主页。
    home_entries: BTreeMap<String, HomeEntry>,
    /// 打开「新实例」页时要用。
    shell: Rc<Shell>,
    config: Config,
    tab_view: adw::TabView,
    /// 主窗口：任务组件确认可以退出后销毁它。
    window: adw::ApplicationWindow,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for AppModel {
    type Init = (LoadedConfig, MinecraftRoot);
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
                // 标题栏（一行：根目录 / 标签页栏 / 账号任务设置主菜单 / 窗口按钮）
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
        (loaded, startup_root): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let LoadedConfig { config, error } = loaded;
        let tab_view = adw::TabView::new();

        // 页面宿主：把实现了 Page 的组件挂成标签页，并管它们的生命周期。
        let page_host = Rc::new(PageHost::new(tab_view.clone()));
        let home_pages = Rc::new(RefCell::new(Vec::<(adw::TabPage, String)>::new()));

        // 标题栏是独立组件；需要页面宿主 / 任务组件的动作通过 HeaderOut 回来。
        let roots: Vec<MinecraftRoot> = config.roots().collect();
        let header = Header::builder()
            .launch((
                tab_view.clone(),
                root.clone(),
                roots,
                config.minecraft_root.clone(),
            ))
            .forward(sender.input_sender(), AppMsg::Header);
        let header_widget = header.widget().clone();

        let tab_view_widget = &tab_view;
        let widgets = view_output!();
        let toast_layer = widgets.toast_layer.clone();

        // 配置加载有问题时，等主窗口显示后再弹汇总警告；关掉后按修复后的配置继续。
        if let Some(error) = error {
            let window = root.clone();
            let config = config.clone();
            gtk::glib::idle_add_local_once(move || config_warning::show(&window, &config, &error));
        }

        // 任务对话框：组件本身就是那个模态窗口，自己管卡片/确认/提示/关闭清理。
        let tasks = TaskCenter::builder()
            .launch((root.clone(), toast_layer.clone()))
            .forward(sender.input_sender(), AppMsg::Tasks);

        // 共享服务：任务入口 + 提示层
        let shell = Rc::new(Shell::new(tasks.sender().clone(), toast_layer.clone()));
        let tasks_sender = tasks.sender().clone();

        // 启动时只打开默认 root（或 CLI 覆盖的 root）的主页。
        let startup_name = startup_root.name.clone();
        let home = HomePage::builder()
            .launch((
                shell.clone(),
                config.clone(),
                startup_root.clone(),
                root.clone(),
            ))
            .forward(sender.input_sender(), move |out| AppMsg::Home {
                root: startup_name.clone(),
                out,
            });
        let home_page = tab_view.append(home.widget());
        home_page.set_title(&home_tab_title(&startup_root.name));
        home_pages
            .borrow_mut()
            .push((home_page.clone(), startup_root.name.clone()));
        let mut home_entries = BTreeMap::new();
        home_entries.insert(
            startup_root.name.clone(),
            HomeEntry {
                _controller: home,
                page: home_page,
            },
        );
        let _ = header.sender().send(HeaderMsg::SetOpenHomes(
            home_entries.keys().cloned().collect(),
        ));

        // 标签被移除时，清理 PageHost / root 映射。主页关闭后 HomePage 一起 drop，
        // 游戏进程不受影响；运行状态只属于本页，不跨页面保留。
        {
            let page_host = page_host.clone();
            let home_pages = home_pages.clone();
            let sender = sender.clone();
            tab_view.connect_page_detached(move |_, page, _| {
                page_host.close(page);
                let home_root = {
                    let mut pages = home_pages.borrow_mut();
                    pages
                        .iter()
                        .position(|(home_page, _)| home_page == page)
                        .map(|index| pages.remove(index).1)
                };
                if let Some(home_root) = home_root {
                    let _ = sender.input_sender().send(AppMsg::HomeClosed(home_root));
                }
            });
        }

        let model = AppModel {
            _tasks: tasks,
            header,
            tasks: tasks_sender,
            page_host,
            home_pages,
            home_entries,
            shell,
            config,
            tab_view,
            window: root.clone(),
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            AppMsg::Header(HeaderOut::OpenTasks) => {
                let _ = self.tasks.send(TaskMsg::Open);
            }
            AppMsg::Header(HeaderOut::OpenHome(name)) => self.open_home(&name, &sender),
            AppMsg::Header(HeaderOut::SetDefaultRoot(name)) => self.set_default_root(&name),
            AppMsg::Home {
                root,
                out: HomeOut::CreateCustom,
            } => self.open_custom(&root),
            AppMsg::Tasks(TaskCenterOut::RunningCount(count)) => {
                let _ = self.header.sender().send(HeaderMsg::SetRunningCount(count));
            }
            AppMsg::Tasks(TaskCenterOut::Quit) => self.window.destroy(),
            AppMsg::HomeClosed(name) => {
                self.home_entries.remove(&name);
                self.sync_header_homes();
            }
            AppMsg::CloseRequested => {
                let _ = self.tasks.send(TaskMsg::CloseMain);
            }
        }
    }
}

impl AppModel {
    /// 打开或聚焦某个 root 的主页。
    fn open_home(&mut self, name: &str, sender: &ComponentSender<Self>) {
        if let Some(entry) = self.home_entries.get(name) {
            self.tab_view.set_selected_page(&entry.page);
            return;
        }
        let root = match self.config.root(name) {
            Ok(root) => root,
            Err(err) => {
                self.shell.toast(&format!("无法打开主页：{err:#}"));
                return;
            }
        };
        let controller = HomePage::builder()
            .launch((
                self.shell.clone(),
                self.config.clone(),
                root.clone(),
                self.window.clone(),
            ))
            .forward(sender.input_sender(), {
                let name = root.name.clone();
                move |out| AppMsg::Home {
                    root: name.clone(),
                    out,
                }
            });
        let page = self.tab_view.append(controller.widget());
        page.set_title(&home_tab_title(&root.name));
        self.tab_view.set_selected_page(&page);
        self.home_pages
            .borrow_mut()
            .push((page.clone(), root.name.clone()));
        self.home_entries.insert(
            root.name.clone(),
            HomeEntry {
                _controller: controller,
                page,
            },
        );
        self.sync_header_homes();
    }

    /// 新建实例页绑定到发出请求的主页 root。
    fn open_custom(&mut self, root_name: &str) {
        let root = match self.config.root(root_name) {
            Ok(root) => root,
            Err(err) => {
                self.shell.toast(&format!("无法确定安装目录：{err:#}"));
                return;
            }
        };
        self.page_host
            .open_page::<CustomPage>((self.shell.clone(), self.config.clone(), root));
    }

    fn set_default_root(&mut self, name: &str) {
        let mut next = self.config.clone();
        if let Err(err) = next.select_minecraft_root(name) {
            self.shell.toast(&format!("无法设置默认主页：{err:#}"));
            return;
        }
        if let Err(err) = next.save_sync() {
            self.shell.toast(&format!("无法保存默认主页设置：{err:#}"));
            return;
        }
        self.config = next;
        let _ = self
            .header
            .sender()
            .send(HeaderMsg::SetDefaultRoot(name.to_string()));
        self.shell
            .toast(&format!("默认主页已设为 {name}，下次启动生效"));
    }

    fn sync_header_homes(&self) {
        let _ = self.header.sender().send(HeaderMsg::SetOpenHomes(
            self.home_entries.keys().cloned().collect(),
        ));
    }
}

fn home_tab_title(root_name: &str) -> String {
    format!("主页 · {root_name}")
}

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub(crate) fn run(loaded: LoadedConfig, startup_root: MinecraftRoot) -> anyhow::Result<()> {
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
