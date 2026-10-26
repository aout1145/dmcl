//! 根目录菜单里的一行。

use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::prelude::*;

use crate::config::MinecraftRoot;

pub(super) struct RootRow {
    pub(super) root: MinecraftRoot,
    pub(super) open: bool,
}

/// 根目录行发出的动作；带索引，由 Header 查回 root 名字。
#[derive(Debug)]
pub(crate) enum RootRowOut {
    Open(DynamicIndex),
}

#[relm4::factory(pub(super))]
impl FactoryComponent for RootRow {
    type Init = MinecraftRoot;
    type Input = ();
    type Output = RootRowOut;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;
    type Index = DynamicIndex;

    view! {
        root = gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 4,
            add_css_class: "dmcl-root-row",

            gtk::Button {
                add_css_class: "flat",
                set_hexpand: true,
                set_tooltip_text: Some("打开这个根目录的主页"),
                connect_clicked[sender, index] => move |_| {
                    let _ = sender.output(RootRowOut::Open(index.clone()));
                },

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    // 勾选位固定占宽，未打开时也留空，保证所有根目录名字对齐。
                    gtk::Box {
                        set_width_request: 16,
                        set_halign: gtk::Align::Start,
                        set_valign: gtk::Align::Center,

                        gtk::Image {
                            set_icon_name: Some("object-select-symbolic"),
                            set_pixel_size: 16,
                            #[watch]
                            set_visible: self.open,
                        },
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 1,

                        gtk::Label {
                            set_label: &self.root.name,
                            set_xalign: 0.0,
                        },

                        gtk::Label {
                            set_label: &self.root.path.display().to_string(),
                            set_xalign: 0.0,
                            add_css_class: "caption",
                            add_css_class: "dim-label",
                        },
                    },
                },
            },
        }
    }

    fn init_model(root: Self::Init, _index: &DynamicIndex, _sender: FactorySender<Self>) -> Self {
        Self { root, open: false }
    }
}
