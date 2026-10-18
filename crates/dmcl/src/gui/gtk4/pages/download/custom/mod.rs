//! 「下载 / 自定义」页：新建实例的入口，目前只有一个空占位。

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::gui::gtk4::app::page_host::{Page, PageInit, TabTitle};

/// 自定义实例页。
pub(crate) struct CustomPage {
    /// 标签标题句柄：改标题直接调它。
    tab: TabTitle,
    /// 标签标题（页面状态）。以后再改标题时从这儿取值。
    title: String,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for CustomPage {
    type Init = PageInit<()>;
    type Input = ();
    type Output = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let PageInit { tab, args: () } = init;
        let model = CustomPage {
            tab,
            title: "新实例".to_string(),
        };
        model.tab.set_title(&model.title);
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }
}

impl Page for CustomPage {
    type Args = ();
}
