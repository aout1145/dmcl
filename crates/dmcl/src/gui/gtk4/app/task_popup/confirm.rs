//! 模态确认框：任务通过 `TaskEvent::Warning` 发来一条需要用户拍板的警告。

use dmcl_task::WarningDecision;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

/// 弹一个模态确认框；用户选择后调 `decision.accept()` / `abort()`。
///
/// `title` 用任务名；任务此时正阻塞在 `WarningGate` 上等答复，等价于等这个对话框。
/// 用 `AdwAlertDialog`（libadwaita 1.5；`AdwMessageDialog` 自 1.6 起已弃用）。
pub(crate) fn ask(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    message: &str,
    decision: WarningDecision,
) {
    let dialog = adw::AlertDialog::new(Some(title), Some(message));
    dialog.add_response("abort", "中止");
    dialog.add_response("continue", "继续");
    dialog.set_response_appearance("abort", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("continue"));
    dialog.set_close_response("abort");
    dialog.connect_response(None, move |_, response| {
        if response == "continue" {
            decision.accept();
        } else {
            decision.abort();
        }
    });
    // `present` 要的是 `IsA<gtk::Widget>`；把窗口 upcast 成 `gtk::Window` 再传。
    let parent: &gtk::Window = parent.upcast_ref();
    dialog.present(Some(parent));
}
