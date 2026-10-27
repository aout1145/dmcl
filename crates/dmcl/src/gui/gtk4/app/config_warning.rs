//! 全局配置修复提示。
//!
//! 配置加载时坏掉的项已经回退成默认值。GUI 启动后立即把修复后的配置写回文件，
//! 并把原文件备份为同目录的 `dmcl.old.json`；弹窗只说明修复结果。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use crate::config::{ConfigError, ConfigWarning};
use crate::gui::gtk4::app::config_store;

/// 配置警告卡片的全局样式。
///
/// 不用 `boxed-list`：它在滚动容器里会把列表自身的圆角/阴影露出来，看起来像四角有黑底。
/// 这里每张卡片独立圆角、无阴影，父容器保持透明。
pub(crate) const CSS: &str = "\
.dmcl-config-warning-scroll, .dmcl-config-warning-scroll > viewport { background: none; }
.dmcl-config-warning-card {
    background-color: @card_bg_color;
    color: @card_fg_color;
    border-radius: 12px;
    padding: 12px;
}";

/// 写回修复后的配置，并弹出说明结果的模态框。
pub(crate) fn repair(parent: &impl IsA<gtk::Window>, error: &ConfigError) {
    let window: gtk::Window = parent.clone().upcast();
    let warnings = error.warnings.clone();
    config_store::store().save_with_backup(move |result| match result {
        Ok(()) => show_result(&window, &warnings, true),
        Err(_) => show_result(&window, &warnings, false),
    });
}

fn show_result(parent: &gtk::Window, warnings: &[ConfigWarning], repaired: bool) {
    let body = repaired.then_some("已自动修复，原文件备份为 dmcl.old.json");
    let dialog = adw::AlertDialog::new(Some("读取配置文件时发生错误"), body);
    dialog.add_response("ok", "确定");
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("ok");
    dialog.set_extra_child(Some(&warning_cards(warnings)));
    dialog.present(Some(parent));
}

fn warning_cards(warnings: &[ConfigWarning]) -> gtk::ScrolledWindow {
    relm4::view! {
        cards = gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
        },
    }

    for warning in warnings {
        relm4::view! {
            card = gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
                add_css_class: "dmcl-config-warning-card",

                gtk::Label {
                    set_label: &warning.message,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_max_width_chars: 48,
                    add_css_class: "heading",
                },

                gtk::Label {
                    set_label: &warning.detail,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_max_width_chars: 52,
                    add_css_class: "caption",
                    add_css_class: "dim-label",
                },
            }
        }
        cards.append(&card);
    }

    relm4::view! {
        scroll = gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_max_content_height: 320,
            set_propagate_natural_height: true,
            set_child: Some(&cards),
            add_css_class: "dmcl-config-warning-scroll",
        },
    }
    scroll
}
