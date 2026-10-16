//! 顶层组件：目前只是一个空窗口。
//!
//! 之后这里会长出标题栏（标签页 + 右侧 popup）、标签页与主页/实例设置。

use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::config::Config;

/// 应用顶层模型。
struct AppModel {
    /// 启动时读到的配置；后续页面（主页 / 实例设置）会用到。
    #[allow(dead_code)]
    config: Config,
}

/// 顶层消息（暂时还没有）。
#[derive(Debug)]
enum AppMsg {}

#[relm4::component]
impl SimpleComponent for AppModel {
    type Init = Config;
    type Input = AppMsg;
    type Output = ();

    view! {
        adw::ApplicationWindow {
            set_title: Some("DMCL"),
            set_default_size: (960, 640),
        }
    }

    fn init(
        config: Self::Init,
        _root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = AppModel { config };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {}
    }
}

/// 启动 GTK4 界面；窗口关闭前一直阻塞在主线程。
pub(super) fn run(config: Config) -> anyhow::Result<()> {
    // 只把程序名传给 GApplication：`dmcl gui` 里的 `gui` 是给我们自己解析的，
    // 别让它被当成要打开的文件（否则 GLib 会警告 "can not open files"）。
    let mut args: Vec<String> = std::env::args().take(1).collect();
    if args.is_empty() {
        args.push("dmcl".to_string());
    }
    let app = RelmApp::new("io.github.aout1145.DMCL").with_args(args);
    app.run::<AppModel>(config);
    Ok(())
}
