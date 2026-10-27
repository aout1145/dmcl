//! 首选项：GNOME 设置式的左右分栏。
//!
//! 左侧是分类列表，右侧显示当前分类；页面嵌在标签页里，没有自己的窗口标题栏，所以
//! 分栏保持常驻、不做折叠。目前只有「Minecraft 目录」可用，其余分类置灰，等对应设置
//! 实现后再启用。

mod directory;

use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use crate::gui::gtk4::app::Shell;

use directory::{DirectoryMsg, DirectoryPage};

/// 首选项页收到的消息。
#[derive(Debug)]
pub(crate) enum PreferencesMsg {
    /// 共享配置改过了，交给各分类重新读取。
    ConfigChanged,
}

pub(crate) struct PreferencesPage {
    /// 「Minecraft 目录」分类；控制器同时负责保活子组件。
    directory: Controller<DirectoryPage>,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for PreferencesPage {
    type Init = Rc<Shell>;
    type Input = PreferencesMsg;
    type Output = ();

    view! {
        adw::NavigationSplitView {
            set_min_sidebar_width: 200.0,
            set_max_sidebar_width: 260.0,

            #[wrap(Some)]
            set_sidebar = &adw::NavigationPage {
                set_title: "首选项",

                #[wrap(Some)]
                set_child = &gtk::ScrolledWindow {
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_vexpand: true,

                    #[name(category_list)]
                    gtk::ListBox {
                        add_css_class: "navigation-sidebar",
                        set_selection_mode: gtk::SelectionMode::Single,

                        append = &adw::ActionRow {
                            set_title: "Minecraft 目录",
                            set_activatable: true,
                            add_prefix = &gtk::Image {
                                set_icon_name: Some("folder-symbolic"),
                            },
                        },

                        append = &adw::ActionRow {
                            set_title: "账号",
                            set_subtitle: "尚未实现",
                            set_activatable: false,
                            set_sensitive: false,
                            add_prefix = &gtk::Image {
                                set_icon_name: Some("avatar-default-symbolic"),
                            },
                        },

                        append = &adw::ActionRow {
                            set_title: "Java 管理",
                            set_subtitle: "尚未实现",
                            set_activatable: false,
                            set_sensitive: false,
                            add_prefix = &gtk::Image {
                                set_icon_name: Some("application-x-executable-symbolic"),
                            },
                        },

                        append = &adw::ActionRow {
                            set_title: "全局游戏选项",
                            set_subtitle: "尚未实现",
                            set_activatable: false,
                            set_sensitive: false,
                            add_prefix = &gtk::Image {
                                set_icon_name: Some("applications-games-symbolic"),
                            },
                        },
                    },
                },
            },

            #[wrap(Some)]
            set_content = &adw::NavigationPage {
                set_title: "Minecraft 目录",

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    #[local_ref]
                    directory_widget -> adw::PreferencesPage {},
                },
            },
        }
    }

    fn init(
        shell: Self::Init,
        _root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let directory = DirectoryPage::builder().launch(shell).detach();
        let directory_widget = directory.widget();
        let widgets = view_output!();

        // 默认停在第一个分类；其余分类置灰，切换逻辑等它们实现后再接。
        if let Some(row) = widgets.category_list.row_at_index(0) {
            widgets.category_list.select_row(Some(&row));
        }

        let model = Self { directory };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            PreferencesMsg::ConfigChanged => {
                let _ = self.directory.sender().send(DirectoryMsg::ConfigChanged);
            }
        }
    }
}
