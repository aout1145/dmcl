//! 自制的提示堆叠。
//!
//! `AdwToastOverlay` 一次只显示一条，新的会顶掉旧的。这里用一个竖直的层：新的加在
//! 已有的**下面**，每条停留 `TOAST_SECONDS` 秒后自动消失；超过上限先删最老的。
//!
//! 每条提示只占一行，超长文字省略；颜色取自主题，深色 / 浅色模式都能跟着变。

use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;

/// 最多同时显示几条。
const MAX_TOASTS: usize = 4;
/// 每条停留多久（秒）。
const TOAST_SECONDS: u32 = 4;

/// 提示层的样式：圆角胶囊 + 阴影。
///
/// 背景和前景都取主题的 card 色，跟随深色 / 浅色模式切换；不要写死颜色。
pub(crate) const CSS: &str = "\
.dmcl-toast { padding: 8px 14px; border-radius: 9999px; box-shadow: 0 2px 8px 0 rgba(0, 0, 0, 0.18); background-color: @card_bg_color; color: @card_fg_color; }";

/// 往提示层里加一条。
pub(crate) fn show(layer: &gtk::Box, text: &str) {
    // 超过上限：先删最老的（层里的第一个子项）。
    while count_children(layer) >= MAX_TOASTS {
        match layer.first_child() {
            Some(oldest) => layer.remove(&oldest),
            None => break,
        }
    }

    let label = gtk::Label::new(Some(text));
    label.add_css_class("card");
    label.add_css_class("dmcl-toast");
    // 只占一行，超出宽度用省略号；`max_width_chars` 同时限制胶囊的自然宽度。
    label.set_wrap(false);
    label.set_lines(1);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(48);
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Start);
    layer.append(&label);

    // 到点自动移除（用弱引用，避免控件已经销毁）。
    let weak = label.downgrade();
    glib::timeout_add_seconds_local(TOAST_SECONDS, move || {
        if let Some(label) = weak.upgrade() {
            if let Some(parent) = label.parent() {
                if let Some(layer) = parent.downcast_ref::<gtk::Box>() {
                    layer.remove(&label);
                }
            }
        }
        glib::ControlFlow::Break
    });
}

fn count_children(layer: &gtk::Box) -> usize {
    let mut count = 0;
    let mut child = layer.first_child();
    while let Some(current) = child {
        count += 1;
        child = current.next_sibling();
    }
    count
}
