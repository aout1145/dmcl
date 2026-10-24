//! 顶层组件：窗口 + 标题栏 + 标签页；只做组装。
//!
//! 页面（`pages/` 下实现 [`Page`](page_host::Page) 的组件）与任务对话框（`tasks`）都是
//! 独立组件、各带自己的消息；页面的打开/关闭/保活交给 [`PageHost`](page_host::PageHost)，
//! 共享能力（跑任务、弹提示、打开任务对话框）在 [`Shell`] 里。这里没有业务逻辑。

mod config_warning;
mod header;
pub(crate) mod page_host;
pub(crate) mod shell;
mod tasks;
mod toast;

pub(crate) use shell::Shell;

use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;
use relm4::Sender;

use crate::config::{Config, LoadedConfig};
use crate::gui::gtk4::pages::download::common;
use crate::gui::gtk4::pages::download::custom::CustomPage;
use crate::gui::gtk4::pages::home::{self, HomePage};
use header::{Header, HeaderMsg, HeaderOut};
use page_host::PageHost;
use tasks::{TaskCenter, TaskCenterOut, TaskMsg};

/// 根组件收到的消息：来自标题栏、任务窗口或主窗口的关闭请求。
#[derive(Debug)]
enum AppMsg {
    Header(HeaderOut),
    Tasks(TaskCenterOut),
    /// 主窗口收到关闭请求。
    CloseRequested,
}

struct AppModel {
    // 子组件靠持有 Controller 存活。
    _tasks: Controller<TaskCenter>,
    _home: Controller<HomePage>,
    header: Controller<Header>,
    /// 任务入口：标题栏的「任务」按钮转发给任务组件。
    tasks: Sender<TaskMsg>,
    // 宿主本身要活到窗口关闭（打开的页面由它保活）。
    page_host: Rc<PageHost>,
    /// 打开「新实例」页时要用。
    shell: Rc<Shell>,
    config: Config,
    /// 主窗口：任务组件确认可以退出后销毁它。
    window: adw::ApplicationWindow,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for AppModel {
    type Init = LoadedConfig;
    type Input = AppMsg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_application: Some(&relm4::main_adw_application()),
            set_title: Some("DMCL"),
            set_default_width: 960,
            set_default_height: 640,

            // 交给任务组件判断：没任务直接放行，有任务先确认。
            connect_close_request[sender] => move |_| {
                sender.input(AppMsg::CloseRequested);
                gtk::glib::Propagation::Stop
            },

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                // 标题栏（一行：加号 / 标签页栏 / 账号任务设置主菜单 / 窗口按钮）
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
        loaded: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let LoadedConfig { config, error } = loaded;
        let tab_view = adw::TabView::new();

        // 页面宿主：把实现了 Page 的组件挂成标签页，并管它们的生命周期。
        let page_host = Rc::new(PageHost::new(tab_view.clone()));

        // 标题栏是独立组件；需要页面宿主 / 任务组件的动作通过 HeaderOut 回来。
        let header = Header::builder()
            .launch((tab_view.clone(), root.clone()))
            .forward(sender.input_sender(), AppMsg::Header);
        let header_widget = header.widget().clone();

        let tab_view_widget = &tab_view;
        let widgets = view_output!();
        let toast_layer = widgets.toast_layer.clone();

        // 配置加载有问题时，等主窗口显示后再弹汇总警告；关掉后按默认值继续。
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

        // 主页组件（自己管实例与运行状态）
        let home = HomePage::builder()
            .launch((shell.clone(), config.clone(), root.clone()))
            .detach();

        // 标签页：主页只用文字标题，且不能关闭（拒绝 close-page）。
        let home_tab = tab_view.append(home.widget());
        home_tab.set_title("主页");
        {
            let home_tab = home_tab.clone();
            tab_view.connect_close_page(move |view, page| {
                if page == &home_tab {
                    view.close_page_finish(page, false);
                    gtk::glib::Propagation::Stop
                } else {
                    gtk::glib::Propagation::Proceed
                }
            });
        }

        // 标签被移除时，宿主丢掉对应页面的保活。
        {
            let page_host = page_host.clone();
            tab_view.connect_page_detached(move |_, page, _| page_host.close(page));
        }

        // 主页的关闭按钮由 CSS 隐藏：等标签栏把标签建好再打 class。
        {
            let header_sender = header.sender().clone();
            gtk::glib::idle_add_local_once(move || {
                let _ = header_sender.send(HeaderMsg::MarkHomeTab);
            });
        }

        let model = AppModel {
            _tasks: tasks,
            _home: home,
            header,
            tasks: tasks_sender,
            page_host,
            shell,
            config,
            window: root.clone(),
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            AppMsg::Header(HeaderOut::AddCustom) => {
                self.page_host
                    .open_page::<CustomPage>((self.shell.clone(), self.config.clone()));
            }
            AppMsg::Header(HeaderOut::OpenTasks) => {
                let _ = self.tasks.send(TaskMsg::Open);
            }
            AppMsg::Tasks(TaskCenterOut::RunningCount(count)) => {
                let _ = self.header.sender().send(HeaderMsg::SetRunningCount(count));
            }
            AppMsg::Tasks(TaskCenterOut::Quit) => self.window.destroy(),
            AppMsg::CloseRequested => {
                let _ = self.tasks.send(TaskMsg::CloseMain);
            }
        }
    }
}

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub(crate) fn run(loaded: LoadedConfig) -> anyhow::Result<()> {
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
        home::CSS,
        common::CSS,
        tasks::CSS,
        toast::CSS,
        config_warning::CSS,
    ] {
        relm4::set_global_css_with_priority(css, relm4::gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    app.run::<AppModel>(loaded);
    Ok(())
}
