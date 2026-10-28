//! 首选项：GNOME 设置式的左右分栏。
//!
//! 左侧是分类列表，右侧显示当前分类；页面嵌在标签页里，没有自己的窗口标题栏，所以
//! 分栏保持常驻、不做折叠。「Minecraft 目录」「Java 管理」「全局游戏选项」可用，账号分类
//! 仍是置灰占位。

mod directory;
mod java;

use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use crate::gui::gtk4::app::{config_store, PageToast, Shell};

use super::common::JavaProbeCache;
use super::game::{GameOptionsInit, GameOptionsMsg, GameOptionsPage, GlobalGameOptionsTarget};
use directory::{DirectoryMsg, DirectoryPage};
use java::{JavaMsg, JavaPage, JavaPageInit};

/// 首选项页收到的消息。
#[derive(Debug)]
pub(crate) enum PreferencesMsg {
    /// 共享配置改过了，交给各分类重新读取。
    ConfigChanged,
}

pub(crate) struct PreferencesPage {
    /// 「Minecraft 目录」分类；控制器同时负责保活子组件。
    directory: Controller<DirectoryPage>,
    /// 「Java 管理」分类。
    java: Controller<JavaPage>,
    /// 「全局游戏选项」分类。
    game: Controller<GameOptionsPage>,
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

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                        set_hexpand: true,

                        #[name(minecraft_list)]
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
                        },

                        gtk::Separator {
                            set_margin_start: 12,
                            set_margin_end: 12,
                        },

                        #[name(game_list)]
                        gtk::ListBox {
                            add_css_class: "navigation-sidebar",
                            set_selection_mode: gtk::SelectionMode::Single,

                            append = &adw::ActionRow {
                                set_title: "全局游戏选项",
                                set_activatable: true,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("applications-games-symbolic"),
                                },
                            },

                            append = &adw::ActionRow {
                                set_title: "Java 管理",
                                set_activatable: true,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("application-x-executable-symbolic"),
                                },
                            },
                        },

                        gtk::Separator {
                            set_margin_start: 12,
                            set_margin_end: 12,
                        },

                        #[name(download_list)]
                        gtk::ListBox {
                            add_css_class: "navigation-sidebar",
                            set_selection_mode: gtk::SelectionMode::Single,

                            append = &adw::ActionRow {
                                set_title: "下载",
                                set_subtitle: "尚未实现",
                                set_activatable: false,
                                set_sensitive: false,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("folder-download-symbolic"),
                                },
                            },
                        },
                    },
                },
            },

            #[wrap(Some)]
            set_content = &adw::NavigationPage {
                set_title: "首选项",

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    #[local_ref]
                    content_stack -> gtk::Stack {},
                },
            },
        }
    }

    fn init(
        shell: Self::Init,
        root_widget: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Java 管理页和游戏设置页共用探测结果，切换分类不会重复执行 Java。
        let config = config_store::store().config();
        let cache = Rc::new(JavaProbeCache::new());
        let directory = DirectoryPage::builder().launch(shell.clone()).detach();
        let java = JavaPage::builder()
            .launch(JavaPageInit {
                shell: shell.clone(),
                cache: Rc::clone(&cache),
            })
            .detach();
        let target = Rc::new(GlobalGameOptionsTarget::new(
            shell,
            PageToast::new(&root_widget),
            config.clone(),
        ));
        let game = GameOptionsPage::builder()
            .launch(GameOptionsInit {
                target,
                config,
                cache,
            })
            .detach();

        // 右栏按左侧分类切换；各分类页都保活，切换只影响显示。
        let content_stack = gtk::Stack::new();
        content_stack.set_vexpand(true);
        content_stack.add_named(directory.widget(), Some("directory"));
        content_stack.add_named(java.widget(), Some("java"));
        content_stack.add_named(game.widget(), Some("game"));

        let widgets = view_output!();

        let lists = Rc::new(vec![
            widgets.minecraft_list.clone(),
            widgets.game_list.clone(),
            widgets.download_list.clone(),
        ]);
        wire_sidebar(
            &widgets.minecraft_list,
            0,
            |index| (index == 0).then_some("directory"),
            Rc::clone(&lists),
            content_stack.clone(),
        );
        wire_sidebar(
            &widgets.game_list,
            1,
            |index| match index {
                0 => Some("game"),
                1 => Some("java"),
                _ => None,
            },
            Rc::clone(&lists),
            content_stack.clone(),
        );
        wire_sidebar(
            &widgets.download_list,
            2,
            |_| None,
            Rc::clone(&lists),
            content_stack,
        );

        // 默认停在「Minecraft 目录」。
        if let Some(row) = widgets.minecraft_list.row_at_index(0) {
            widgets.minecraft_list.select_row(Some(&row));
        }

        let model = Self {
            directory,
            java,
            game,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            PreferencesMsg::ConfigChanged => {
                let _ = self.directory.sender().send(DirectoryMsg::ConfigChanged);
                let _ = self.java.sender().send(JavaMsg::ConfigChanged);
                let _ = self.game.sender().send(GameOptionsMsg::Reload);
            }
        }
    }
}

/// 连接一个侧栏分组：选中行时切换右栏，并清掉其它分组的选择。
fn wire_sidebar(
    list: &gtk::ListBox,
    group: usize,
    page_for: impl Fn(i32) -> Option<&'static str> + 'static,
    lists: Rc<Vec<gtk::ListBox>>,
    content_stack: gtk::Stack,
) {
    list.connect_row_selected(move |_, row| {
        let Some(row) = row else {
            return;
        };
        let Some(page) = page_for(row.index()) else {
            return;
        };
        for (index, other) in lists.iter().enumerate() {
            if index != group {
                other.unselect_all();
            }
        }
        content_stack.set_visible_child_name(page);
    });
}
