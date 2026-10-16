//! 单个实例卡片：头像 + 名称 + 状态；单击弹功能卡片、双击启动。

use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::gui::gtk4::app::{AppModel, AppMsg};

use super::{actions_popup, InstanceView};

/// 建一个实例卡片。返回的还是 `gtk::Widget`，由调用者塞进网格。
pub(crate) fn build(instance: &InstanceView, sender: &ComponentSender<AppModel>) -> gtk::Widget {
    // 首字母彩色头像（暂无实例图标来源）。
    let avatar = adw::Avatar::new(48, Some(instance.name.as_str()), true);

    let name = gtk::Label::builder()
        .label(&instance.name)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .max_width_chars(14)
        .build();

    let status = gtk::Label::new(Some(if instance.running {
        "运行中"
    } else {
        "未运行"
    }));
    status.add_css_class("caption");
    status.add_css_class(if instance.running {
        "success"
    } else {
        "dim-label"
    });

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    content.append(&avatar);
    content.append(&name);
    content.append(&status);

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(120)
        .build();
    card.add_css_class("card");
    card.set_margin_all(6);
    card.append(&content);

    // 单击弹功能卡片、双击启动。
    let popover = actions_popup::build(&card, instance, sender);
    let gesture = gtk::GestureClick::new();
    {
        let sender = sender.clone();
        let name = instance.name.clone();
        let popover = popover.clone();
        gesture.connect_pressed(move |_, n_press, _, _| {
            if n_press >= 2 {
                popover.popdown();
                sender.input(AppMsg::Launch(name.clone()));
            } else if n_press == 1 {
                popover.popup();
            }
        });
    }
    card.add_controller(gesture);

    card.upcast()
}
