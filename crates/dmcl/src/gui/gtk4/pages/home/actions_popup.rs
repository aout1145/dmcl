//! 单击实例弹出的功能卡片（非模态 popover）。

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::gui::gtk4::app::{AppModel, AppMsg};

use super::InstanceView;

/// 建一个挂在 `parent` 上的功能卡片（启动 / 停止、管理）。
pub(crate) fn build(
    parent: &impl IsA<gtk::Widget>,
    instance: &InstanceView,
    sender: &ComponentSender<AppModel>,
) -> gtk::Popover {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();

    let (start_label, start_icon) = if instance.running {
        ("停止", "media-playback-stop-symbolic")
    } else {
        ("启动", "media-playback-start-symbolic")
    };
    let start = flat_button(start_label, start_icon);
    {
        let sender = sender.clone();
        let name = instance.name.clone();
        start.connect_clicked(move |_| sender.input(AppMsg::Launch(name.clone())));
    }

    let manage = flat_button("管理", "document-properties-symbolic");
    {
        let sender = sender.clone();
        let name = instance.name.clone();
        manage.connect_clicked(move |_| sender.input(AppMsg::Manage(name.clone())));
    }

    content.append(&start);
    content.append(&manage);

    let popover = gtk::Popover::new();
    popover.set_child(Some(&content));
    popover.set_parent(parent);
    popover
}

/// 一行图标 + 文字的扁平按钮。
fn flat_button(label: &str, icon: &str) -> gtk::Button {
    let button = gtk::Button::builder().label(label).icon_name(icon).build();
    button.add_css_class("flat");
    button
}
