//! 顶层组件：窗口 + 标题栏 + 标签页；只做组装。
//!
//! 页面（`pages/` 下实现 [`Page`](page_host::Page) 的组件）与任务对话框（`task_popup`）都是
//! 独立组件、各带自己的消息；页面的打开/关闭/保活交给 [`PageHost`](page_host::PageHost)，
//! 共享能力（跑任务、弹提示、打开任务对话框）在 [`Shell`] 里。这里没有业务逻辑。

mod header;
pub(crate) mod page_host;
pub(crate) mod shell;
mod task_popup;
mod toast;

pub(crate) use shell::Shell;

use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use crate::config::Config;
use crate::gui::gtk4::pages::download::common;
use crate::gui::gtk4::pages::download::custom::CustomPage;
use crate::gui::gtk4::pages::home::{self, HomePage};
use page_host::PageHost;
use task_popup::{TaskCenter, TaskMsg};

struct AppModel {
    // 子组件靠持有 Controller 存活。
    _tasks: Controller<TaskCenter>,
    _home: Controller<HomePage>,
    // 宿主本身要活到窗口关闭（打开的页面由它保活）。
    _page_host: Rc<PageHost>,
}

impl SimpleComponent for AppModel {
    type Init = Config;
    type Input = ();
    type Output = ();
    type Widgets = ();
    type Root = adw::ApplicationWindow;

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .application(&relm4::main_adw_application())
            .title("DMCL")
            .default_width(960)
            .default_height(640)
            .build()
    }

    fn init(
        config: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let tab_view = adw::TabView::new();

        // 页面宿主：把实现了 Page 的组件挂成标签页，并管它们的生命周期。
        let page_host = Rc::new(PageHost::new(tab_view.clone()));

        // 主界面提示层：自制的堆叠 toast（AdwToastOverlay 一次只能显示一条）。
        let content = gtk::Overlay::new();
        content.set_child(Some(&tab_view));
        let toast_layer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        toast_layer.set_halign(gtk::Align::Start);
        toast_layer.set_valign(gtk::Align::End);
        toast_layer.set_margin_start(18);
        toast_layer.set_margin_bottom(18);
        content.add_overlay(&toast_layer);

        // 任务对话框：组件本身就是那个模态窗口，自己管卡片/确认/提示/关闭清理。
        let tasks = TaskCenter::builder()
            .launch((root.clone(), toast_layer.clone()))
            .detach();

        // 标题栏（一行：加号 / 标签页栏 / 账号任务设置 / 窗口按钮）
        let header = header::build(&tab_view);

        // 共享服务：任务入口 + 提示层
        let shell = Rc::new(Shell::new(tasks.sender().clone(), toast_layer.clone()));
        {
            let tasks_sender = tasks.sender().clone();
            header.tasks_button.connect_clicked(move |_| {
                let _ = tasks_sender.send(TaskMsg::Open);
            });
        }

        // 「添加实例」下拉里的「自定义」→ 开一个「新实例」页。
        {
            let page_host = page_host.clone();
            let shell = shell.clone();
            let config = config.clone();
            header.add_custom.connect_clicked(move |_| {
                page_host.open_page::<CustomPage>((shell.clone(), config.clone()));
            });
        }

        // 主页组件（自己管实例与运行状态）
        let home = HomePage::builder().launch((shell, config)).detach();

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
            let tab_bar = header.tab_bar.clone();
            gtk::glib::idle_add_local_once(move || header::mark_home_tab(&tab_bar));
        }

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header.bar);
        toolbar_view.set_content(Some(&content));
        root.set_content(Some(&toolbar_view));

        ComponentParts {
            model: AppModel {
                _tasks: tasks,
                _home: home,
                _page_host: page_host,
            },
            widgets: (),
        }
    }
}

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub(crate) fn run(config: Config) -> anyhow::Result<()> {
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
        task_popup::CSS,
        toast::CSS,
    ] {
        relm4::set_global_css_with_priority(css, relm4::gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    app.run::<AppModel>(config);
    Ok(())
}
