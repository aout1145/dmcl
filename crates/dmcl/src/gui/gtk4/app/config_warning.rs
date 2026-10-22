//! 配置加载警告的汇总弹窗。
//!
//! 正文用卡片逐条列出「采取的措施 + 错误类型」；「确认」把修复后的配置写回文件，
//! 「忽略」只关窗口、不写文件。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;

use crate::config::{Config, ConfigError};

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

/// 弹一个汇总全部配置警告的模态框。
pub(crate) fn show(parent: &impl IsA<gtk::Window>, config: &Config, error: &ConfigError) {
    // AdwAlertDialog 没有按严重级别切换的内置外观，标题和图标都由 extra child 自己排。
    let dialog = adw::AlertDialog::new(None, None);
    dialog.add_response("ignore", "忽略");
    dialog.add_response("confirm", "确认");
    dialog.set_response_appearance("confirm", adw::ResponseAppearance::Suggested);
    // 写文件有副作用，默认响应放在「忽略」上，避免回车误写。
    dialog.set_default_response(Some("ignore"));
    dialog.set_close_response("ignore");

    relm4::view! {
        cards = gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
        },

        content = gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Center,

                gtk::Image::from_icon_name("dialog-warning-symbolic") {
                    set_pixel_size: 32,
                    add_css_class: "warning",
                },

                gtk::Label {
                    set_label: "警告",
                    add_css_class: "title-1",
                },
            },

            gtk::Label {
                set_label: "读取配置文件时发生错误",
                set_halign: gtk::Align::Center,
            },

            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_max_content_height: 320,
                set_propagate_natural_height: true,
                set_child: Some(&cards),
                add_css_class: "dmcl-config-warning-scroll",
            },
        }
    }

    for warning in &error.warnings {
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

    dialog.set_extra_child(Some(&content));

    let window: gtk::Window = parent.clone().upcast();
    let config = config.clone();
    let save_parent = window.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "confirm" {
            return;
        }
        if let Err(err) = config.save_sync() {
            let dialog = adw::AlertDialog::new(Some("保存配置失败"), Some(&format!("{err:#}")));
            dialog.add_response("ok", "知道了");
            dialog.set_default_response(Some("ok"));
            dialog.set_close_response("ok");
            dialog.present(Some(&save_parent));
        }
    });
    dialog.present(Some(&window));
}
