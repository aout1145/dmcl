//! 设置页：首选项。
//!
//! 页面目前只有空的 `AdwPreferencesPage` 骨架，具体分组后续补。页面由根组件在启动时
//! 创建，并通过 `PageSlot` 常驻：关闭标签只影响显示，重新打开仍是同一个页面。

use relm4::adw;
use relm4::prelude::*;

/// 首选项。
pub(crate) struct PreferencesPage;

#[relm4::component(pub(crate))]
impl SimpleComponent for PreferencesPage {
    type Init = ();
    type Input = ();
    type Output = ();

    view! {
        adw::PreferencesPage {}
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();
        ComponentParts {
            model: Self,
            widgets,
        }
    }

    fn update(&mut self, _message: Self::Input, _sender: ComponentSender<Self>) {}
}
