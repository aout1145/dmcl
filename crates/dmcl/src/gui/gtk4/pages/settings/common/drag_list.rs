//! 可拖动排序的设置列表。
//!
//! 行组件把拖动手柄和整行交给 [`attach`]；拖动结束后得到 [`DragMove`]，由调用方调整
//! 顺序并重建列表。

use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::glib;
use relm4::gtk::glib::prelude::ToValue;
use relm4::gtk::prelude::*;

/// 拖动时源行淡化，落点行上方或下方显示提示线。
pub(crate) const CSS: &str = "\
.dmcl-drag-source { opacity: 0.45; }
.dmcl-drop-before { box-shadow: inset 0 2px 0 0 @accent_bg_color; }
.dmcl-drop-after { box-shadow: inset 0 -2px 0 0 @accent_bg_color; }";

/// 一次拖动落点：把 `from` 放到 `target` 前或后。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DragMove {
    pub from: String,
    pub target: String,
    pub after: bool,
}

/// 让 `handle` 可以拖动 `row`，并在 `row` 上接收落点。
///
/// `key` 是这一行的稳定标识，拖动结果会原样带回；`on_move` 在放下时调用。
pub(crate) fn attach(
    row: &gtk::Widget,
    handle: &gtk::Image,
    key: String,
    on_move: impl Fn(DragMove) + 'static,
) {
    handle.set_icon_name(Some("list-drag-handle-symbolic"));
    handle.set_tooltip_text(Some("拖动排序"));
    handle.set_cursor_from_name(Some("grab"));
    handle.add_css_class("dim-label");

    let drag_source = gtk::DragSource::builder()
        .actions(gdk::DragAction::MOVE)
        .build();
    drag_source.connect_prepare({
        let key = key.clone();
        move |_, _, _| {
            let value = key.to_value();
            Some(gdk::ContentProvider::for_value(&value))
        }
    });
    drag_source.connect_drag_begin({
        let row = row.clone();
        let handle = handle.clone();
        move |_, _| {
            row.add_css_class("dmcl-drag-source");
            handle.set_cursor_from_name(Some("grabbing"));
        }
    });
    drag_source.connect_drag_end({
        let row = row.clone();
        let handle = handle.clone();
        move |_, _, _| {
            row.remove_css_class("dmcl-drag-source");
            handle.set_cursor_from_name(Some("grab"));
        }
    });
    handle.add_controller(drag_source);

    let drop_target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    drop_target.connect_enter({
        let row = row.clone();
        move |_, _, y| {
            set_drop_side(&row, y);
            gdk::DragAction::MOVE
        }
    });
    drop_target.connect_motion({
        let row = row.clone();
        move |_, _, y| {
            set_drop_side(&row, y);
            gdk::DragAction::MOVE
        }
    });
    drop_target.connect_leave({
        let row = row.clone();
        move |_| clear_drop_side(&row)
    });
    drop_target.connect_drop({
        let row = row.clone();
        move |_, value, _, y| {
            clear_drop_side(&row);
            let Ok(from) = value.get::<String>() else {
                return false;
            };
            if from == key {
                return true;
            }
            let after = y >= f64::from(row.height()) / 2.0;
            on_move(DragMove {
                from,
                target: key.clone(),
                after,
            });
            true
        }
    });
    row.add_controller(drop_target);
}

/// 把拖动落点换算成移动后的下标：源在目标之前时要扣掉自己占的位置。
pub(crate) fn moved_index(from: usize, target: usize, after: bool) -> usize {
    let insert_at = target + usize::from(after);
    if from < insert_at {
        insert_at - 1
    } else {
        insert_at
    }
}

/// 按落点在行内的高度，切换上 / 下插入提示线。
fn set_drop_side(row: &gtk::Widget, y: f64) {
    row.remove_css_class("dmcl-drop-before");
    row.remove_css_class("dmcl-drop-after");
    if y < f64::from(row.height()) / 2.0 {
        row.add_css_class("dmcl-drop-before");
    } else {
        row.add_css_class("dmcl-drop-after");
    }
}

fn clear_drop_side(row: &gtk::Widget) {
    row.remove_css_class("dmcl-drop-before");
    row.remove_css_class("dmcl-drop-after");
}

#[cfg(test)]
mod tests {
    use super::moved_index;

    #[test]
    fn drop_index_compensates_for_the_removed_source() {
        // 源在目标之前：先移除源，目标随之前移一位。
        assert_eq!(moved_index(0, 2, false), 1);
        assert_eq!(moved_index(0, 2, true), 2);
        // 源在目标之后：目标和插入位置都不受移除影响。
        assert_eq!(moved_index(2, 0, false), 0);
        assert_eq!(moved_index(2, 0, true), 1);
        // 落在相邻位置且顺序不变时，下标保持原样。
        assert_eq!(moved_index(0, 1, false), 0);
        assert_eq!(moved_index(1, 0, true), 1);
    }
}
