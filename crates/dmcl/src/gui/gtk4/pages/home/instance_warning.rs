//! 实例配置警告的启动前确认框。
//!
//! 与全局配置警告不同，这里不写回任何文件：确认后只用内存里修复过的配置启动。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use crate::config::ConfigError;

/// 在启动链开始前弹一个模态确认框。
///
/// 「取消」是默认响应；只有点「继续启动」才调用 `on_continue`。
pub(crate) fn show<F>(parent: &impl IsA<gtk::Widget>, error: &ConfigError, on_continue: F)
where
    F: Fn() + 'static,
{
    let dialog = adw::AlertDialog::new(Some("实例配置有警告"), None);
    dialog.add_response("cancel", "取消");
    dialog.add_response("continue", "继续启动");
    dialog.set_response_appearance("continue", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for warning in &error.warnings {
        let item = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let message = gtk::Label::new(Some(&warning.message));
        message.set_xalign(0.0);
        message.set_wrap(true);
        message.set_max_width_chars(48);
        message.add_css_class("heading");
        let detail = gtk::Label::new(Some(&warning.detail));
        detail.set_xalign(0.0);
        detail.set_wrap(true);
        detail.set_max_width_chars(52);
        detail.add_css_class("caption");
        detail.add_css_class("dim-label");
        item.append(&message);
        item.append(&detail);
        list.append(&item);
    }

    let scroll = gtk::ScrolledWindow::new();
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_max_content_height(320);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&list));
    dialog.set_extra_child(Some(&scroll));

    dialog.connect_response(None, move |_, response| {
        if response == "continue" {
            on_continue();
        }
    });
    dialog.present(Some(parent));
}
