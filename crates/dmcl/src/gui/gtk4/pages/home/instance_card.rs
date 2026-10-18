//! 单个实例磁贴：头像 + 名称；运行中时右上角有小绿点。单击选中、双击启动。

use relm4::adw;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use super::{HomeMsg, HomePage, InstanceView};

/// 建一个实例磁贴。返回的还是 `gtk::Button`，由调用者塞进网格。
pub(crate) fn build(
    instance: &InstanceView,
    selected: bool,
    sender: &ComponentSender<HomePage>,
) -> gtk::Button {
    // 首字母彩色头像（暂无实例图标来源）。
    let avatar = adw::Avatar::new(48, Some(instance.name.as_str()), true);

    // 名称固定 12 字符宽、最多两行：让所有磁贴等宽。
    let name = gtk::Label::builder()
        .label(&instance.name)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .width_chars(12)
        .max_width_chars(12)
        .yalign(0.0)
        .build();
    // 名称区固定占两行高：所有磁贴一致，短名称下面只空一行。
    name.set_height_request(name.create_pango_layout(Some("M\nM")).pixel_size().1);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(8)
        .margin_end(8)
        .build();
    content.append(&avatar);
    content.append(&name);

    // 运行中：右上角的小绿点（symbolic 圆点 + success 颜色）。
    let dot = gtk::Image::from_icon_name("media-record-symbolic");
    dot.add_css_class("success");
    dot.set_pixel_size(20);
    dot.set_halign(gtk::Align::End);
    dot.set_valign(gtk::Align::Start);
    dot.set_margin_top(6);
    dot.set_margin_end(4);
    dot.set_visible(instance.is_running());

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&content));
    overlay.add_overlay(&dot);

    // 透明磁贴（类似 GNOME Files）：不画卡片背景，hover / 选中态由样式类提供。
    let card = gtk::Button::builder()
        .child(&overlay)
        .has_frame(false)
        .hexpand(false)
        .width_request(136)
        .build();
    card.add_css_class("flat");
    if selected {
        card.add_css_class("dmcl-instance-selected");
    }
    card.set_tooltip_text(Some(&instance.name));

    // 单击选中（底部出现操作栏）；双击启动。
    {
        let sender = sender.clone();
        let name = instance.name.clone();
        card.connect_clicked(move |_| sender.input(HomeMsg::Select(name.clone())));
    }
    let gesture = gtk::GestureClick::new();
    // 捕获阶段：否则按钮自己的点击手势会先把双击吞掉。
    gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let sender = sender.clone();
        let name = instance.name.clone();
        gesture.connect_pressed(move |_, n_press, _, _| {
            if n_press >= 2 {
                sender.input(HomeMsg::Launch(name.clone()));
            }
        });
    }
    card.add_controller(gesture);

    card
}
