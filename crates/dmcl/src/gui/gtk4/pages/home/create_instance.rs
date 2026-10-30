//! 「创建实例」模态对话框。
//!
//! 目前只有「自定义」一种创建方式；以后新增整合包 / 导入等入口时继续往列表里加行。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use super::{HomeOut, HomePage};

/// 弹出创建实例的方式列表。
pub(crate) fn show(
    parent: &impl IsA<gtk::Widget>,
    sender: &ComponentSender<HomePage>,
    categories: Vec<String>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title("创建实例");
    dialog.set_content_width(420);

    relm4::view! {
        toolbar = adw::ToolbarView {
            add_top_bar = &adw::HeaderBar {
                set_show_start_title_buttons: false,
                set_show_end_title_buttons: false,
                set_title_widget: Some(&adw::WindowTitle::new("创建实例", "")),

                pack_end = &gtk::Button::from_icon_name("window-close-symbolic") {
                    add_css_class: "flat",
                    set_tooltip_text: Some("关闭"),
                    connect_clicked[dialog] => move |_| {
                        dialog.close();
                    },
                },
            },

            #[wrap(Some)]
            set_content = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 12,
                set_margin_top: 6,
                set_margin_bottom: 18,
                set_margin_start: 18,
                set_margin_end: 18,

                append: list = &gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,

                    append = &adw::ActionRow {
                        set_title: "自定义",
                        set_subtitle: "创建自定义的 Minecraft 实例",
                        set_activatable: true,
                    },
                },
            },
        }
    }

    dialog.set_child(Some(&toolbar));
    {
        let dialog = dialog.clone();
        let sender = sender.clone();
        list.connect_row_activated(move |_, _| {
            dialog.close();
            let _ = sender.output(HomeOut::CreateCustom {
                categories: categories.clone(),
            });
        });
    }
    dialog.present(Some(parent));
}
