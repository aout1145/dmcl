//! 自制的提示堆叠。
//!
//! `AdwToastOverlay` 一次只显示一条，新的会顶掉旧的。这里用一个竖直的层：新的加在
//! 已有的**下面**，每条停留 `TOAST_SECONDS` 秒后自动消失；超过上限先删最老的。
//!
//! 提示分两种作用域：应用级始终显示；页面级只在页面可见时显示，页面隐藏时直接丢弃。
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

/// 提示服务：所有 toast 共用一个提示层，区别只在显示条件。
#[derive(Clone)]
pub(crate) struct ToastService {
    layer: gtk::Box,
}

impl ToastService {
    pub(crate) fn new(layer: gtk::Box) -> Self {
        Self { layer }
    }

    /// 应用级提示：不检查页面，直接显示。
    pub(crate) fn global_toast(&self, text: &str) {
        show(&self.layer, text);
    }

    /// 页面级提示：页面当前不可见时直接丢弃，不排队。
    pub(crate) fn page_toast(&self, page: &PageToast, text: &str) {
        if page.is_visible() {
            show(&self.layer, text);
        }
    }
}

/// 页面私有的 toast 句柄：只用来判断页面是否可见，不持有页面强引用。
#[derive(Clone)]
pub(crate) struct PageToast {
    root: glib::WeakRef<gtk::Widget>,
}

impl PageToast {
    pub(crate) fn new(root: &impl IsA<gtk::Widget>) -> Self {
        let root: gtk::Widget = root.clone().upcast();
        Self {
            root: root.downgrade(),
        }
    }

    /// 页面根控件还在、并且已经映射时才算可见。
    fn is_visible(&self) -> bool {
        self.root.upgrade().is_some_and(|root| root.is_mapped())
    }
}

/// 往提示层里加一条。
fn show(layer: &gtk::Box, text: &str) {
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
