//! 顶层组件：窗口 + 标题栏（标签页 + 右侧 popup）+ 标签页（主页）。
//!
//! 用手写的 `SimpleComponent`（不用 `view!`）：界面部件都直接存在模型里，`update` 里
//! 按模型重建动态部分（实例网格）。任务列表、启动桥接等是下一步。

pub(crate) mod header;
pub(crate) mod message;

pub(crate) use message::AppMsg;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use dmcl_mcmeta::layout::installed_versions;

use crate::config::Config;
use crate::gui::gtk4::pages::home::{self, InstanceView};

/// 标题栏里内嵌的标签栏的样式补丁：
/// * 收紧竖向内边距，让标题栏保持 libadwaita 正常的 47px 高度；
/// * 隐藏标签的关闭按钮——libadwaita 没有 per-page 的 closable，唯一没有关闭按钮的方式是
///   `pinned`，但 `pinned` 又不显示标题；当前应用不支持关闭标签，直接隐藏 `×`。
const TABBAR_CSS: &str = "\
headerbar tabbar tabbox { padding-top: 0; padding-bottom: 0; }
headerbar tabbar .box { padding: 0; }
headerbar tabbar tab button.image-button { min-width: 0; min-height: 0; padding: 0; margin: 0; -gtk-icon-size: 1px; opacity: 0; }";

/// 应用顶层模型（窗口、网格等部件都存在这里）。
pub(crate) struct AppModel {
    config: Config,
    instances: Vec<InstanceView>,
    home_grid: gtk::FlowBox,
    toast_overlay: adw::ToastOverlay,
}

impl SimpleComponent for AppModel {
    type Init = Config;
    type Input = AppMsg;
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
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // 主页的实例网格
        let (home_scrolled, home_grid) = home::build();

        // 标签页：主页只用文字标题。**不能用 pinned**——pinned 标签只显示图标、
        // 不显示标题；改成拦截 `close-page` 来防止主页被关掉。
        let tab_view = adw::TabView::new();
        let home_page = tab_view.append(&home_scrolled);
        home_page.set_title("主页");
        {
            let home_page = home_page.clone();
            tab_view.connect_close_page(move |_, page| {
                if page == &home_page {
                    gtk::glib::Propagation::Stop
                } else {
                    gtk::glib::Propagation::Proceed
                }
            });
        }

        // 主界面提示层
        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&tab_view));

        // 标题栏（标签页栏 + 右侧 popup）；AdwApplicationWindow 的内容用
        // AdwToolbarView，把标题栏作为 top bar。
        let header = header::build(&tab_view);
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&toast_overlay));
        root.set_content(Some(&toolbar_view));

        let model = AppModel {
            config,
            instances: Vec::new(),
            home_grid,
            toast_overlay,
        };
        sender.input(AppMsg::ReloadInstances);
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            AppMsg::ReloadInstances => {
                // 读磁盘：放到后台任务里，读完把结果发回来。
                let root_path = self.config.launch.minecraft_root_path.clone();
                let sender = sender.clone();
                relm4::spawn(async move {
                    let result = installed_versions(&root_path)
                        .await
                        .map_err(|err| format!("{err:#}"));
                    sender.input(AppMsg::InstancesLoaded(result));
                });
            }
            AppMsg::InstancesLoaded(Ok(versions)) => {
                self.instances = versions
                    .into_iter()
                    .map(|version| InstanceView {
                        name: version.name,
                        running: false,
                    })
                    .collect();
                home::render(&self.home_grid, &self.instances, &sender);
            }
            AppMsg::InstancesLoaded(Err(err)) => {
                sender.input(AppMsg::Toast(format!("读取已安装版本失败：{err}")));
            }
            AppMsg::Launch(name) => {
                // 下一步接真正的启动任务链。
                sender.input(AppMsg::Toast(format!("启动 {name}（尚未实现）")));
            }
            AppMsg::Manage(name) => {
                sender.input(AppMsg::Toast(format!("打开 {name} 的管理页（尚未实现）")));
            }
            AppMsg::Toast(text) => {
                self.toast_overlay.add_toast(adw::Toast::new(&text));
            }
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
    // 标题栏里的标签栏：收紧竖向内边距，让标题栏保持 libadwaita 正常的 47px 高度
    // （否则 tabbar 自身 47px + 标题栏内层 13px 内边距会顶到 60px）。
    relm4::set_global_css_with_priority(
        TABBAR_CSS,
        relm4::gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    app.run::<AppModel>(config);
    Ok(())
}
