//! 单个实例的设置页：左侧分两组分类，右侧按分类显示实例信息或游戏设置。
//!
//! 「实例」分类编辑图标、名称、分类、加载器并删除实例；「启动」复用全局游戏设置
//! 表单。页面是关闭即销毁的一次性标签页；实例文件的变化不走全局配置仓库，各子页面自己
//! 写盘。同一实例由根组件按 `(root, 实例名)` 去重，这个模块只负责页面本身。

mod files;
mod general;
mod loaders;
mod target;

use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use dmcl_game::{resolve_version, VersionRef};

use crate::gui::gtk4::app::page_host::{Page, PageInit, TabTitle};
use crate::gui::gtk4::app::{config_store, PageToast, Shell};
use crate::gui::gtk4::pages::settings::{
    GameOptionsInit, GameOptionsMsg, GameOptionsPage, JavaProbeCache,
};

use general::{GeneralInit, GeneralOut, GeneralPage};
use target::InstanceGameOptionsTarget;

/// 实例设置页发给根组件的动作。
#[derive(Debug)]
pub(crate) enum InstanceOut {
    /// 实例目录已经改名，根组件要同步标签索引。
    Renamed { old: String, new: String },
    /// 实例目录已经删除，根组件要关掉这个标签。
    Deleted { name: String },
}

/// 实例设置页收到的消息。
#[derive(Debug)]
pub(crate) enum InstanceMsg {
    /// 全局配置改过了：让表单重新读取实例覆盖。
    ConfigChanged,
    /// 「实例」分类的动作。
    General(GeneralOut),
}

pub(crate) struct InstancePage {
    tab: TabTitle,
    target: Rc<InstanceGameOptionsTarget>,
    general: Controller<GeneralPage>,
    game: Controller<GameOptionsPage>,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for InstancePage {
    type Init = PageInit<(Rc<Shell>, PathBuf, String, Vec<String>)>;
    type Input = InstanceMsg;
    type Output = InstanceOut;

    view! {
        adw::NavigationSplitView {
            set_min_sidebar_width: 200.0,
            set_max_sidebar_width: 260.0,

            #[wrap(Some)]
            set_sidebar = &adw::NavigationPage {
                set_title: "实例设置",

                #[wrap(Some)]
                set_child = &gtk::ScrolledWindow {
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_vexpand: true,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                        set_hexpand: true,

                        #[name(instance_list)]
                        gtk::ListBox {
                            add_css_class: "navigation-sidebar",
                            set_selection_mode: gtk::SelectionMode::Single,

                            append = &adw::ActionRow {
                                set_title: "实例",
                                set_activatable: true,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("package-x-generic-symbolic"),
                                },
                            },

                            append = &adw::ActionRow {
                                set_title: "启动",
                                set_activatable: true,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("media-playback-start-symbolic"),
                                },
                            },
                        },

                        gtk::Separator {
                            set_margin_start: 12,
                            set_margin_end: 12,
                        },

                        gtk::ListBox {
                            add_css_class: "navigation-sidebar",
                            set_selection_mode: gtk::SelectionMode::Single,

                            append = &adw::ActionRow {
                                set_title: "模组管理",
                                set_subtitle: "尚未实现",
                                set_activatable: false,
                                set_sensitive: false,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("application-x-addon-symbolic"),
                                },
                            },

                            append = &adw::ActionRow {
                                set_title: "资源包管理",
                                set_subtitle: "尚未实现",
                                set_activatable: false,
                                set_sensitive: false,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("image-x-generic-symbolic"),
                                },
                            },

                            append = &adw::ActionRow {
                                set_title: "存档管理",
                                set_subtitle: "尚未实现",
                                set_activatable: false,
                                set_sensitive: false,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("document-save-symbolic"),
                                },
                            },
                        },
                    },
                },
            },

            #[wrap(Some)]
            set_content = &adw::NavigationPage {
                set_title: "实例设置",

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,

                    #[name(content_stack)]
                    gtk::Stack {
                        set_vexpand: true,
                    },
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let PageInit {
            tab,
            args: (shell, root_path, name, categories),
        } = init;
        let config = config_store::store().config();
        let cache = Rc::new(JavaProbeCache::new());
        let page_toast = PageToast::new(&root_widget);
        let target = Rc::new(InstanceGameOptionsTarget::new(
            config.clone(),
            Rc::clone(&shell),
            page_toast.clone(),
            root_path.clone(),
            name.clone(),
        ));
        let game = GameOptionsPage::builder()
            .launch(GameOptionsInit {
                target: target.clone(),
                config,
                cache,
                allow_inherit: true,
                required_java_major: None,
            })
            .detach();
        let game_widget = game.widget().clone();
        game_widget.set_vexpand(true);

        let general = GeneralPage::builder()
            .launch(GeneralInit {
                shell: Rc::clone(&shell),
                page_toast,
                root_path: root_path.clone(),
                name: name.clone(),
                categories,
            })
            .forward(sender.input_sender(), InstanceMsg::General);
        let general_widget = general.widget().clone();
        general_widget.set_vexpand(true);

        tab.set_title(&instance_tab_title(&name));

        // 版本 JSON 可能带 `inheritsFrom`，用和启动流程相同的解析结果取 Java 要求。
        let version = VersionRef::new(root_path, name);
        let game_sender = game.sender().clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        relm4::spawn(async move {
            let required = resolve_version(&version)
                .await
                .ok()
                .map(|resolved| resolved.json.required_java_major());
            let _ = tx.send(required);
        });
        relm4::spawn_local(async move {
            if let Ok(required) = rx.await {
                let _ = game_sender.send(GameOptionsMsg::RequiredJavaMajor(required));
            }
        });

        let model = Self {
            tab,
            target,
            general,
            game,
        };

        let widgets = view_output!();
        widgets
            .content_stack
            .add_named(model.general.widget(), Some("general"));
        widgets
            .content_stack
            .add_named(model.game.widget(), Some("game"));
        {
            let stack = widgets.content_stack.clone();
            widgets.instance_list.connect_row_selected(move |_, row| {
                let Some(row) = row else {
                    return;
                };
                let page = match row.index() {
                    0 => "general",
                    1 => "game",
                    _ => return,
                };
                stack.set_visible_child_name(page);
            });
        }
        // 默认停在「实例」。
        if let Some(row) = widgets.instance_list.row_at_index(0) {
            widgets.instance_list.select_row(Some(&row));
        }
        widgets.content_stack.set_visible_child_name("general");

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            InstanceMsg::ConfigChanged => {
                let _ = self.game.sender().send(GameOptionsMsg::Reload);
            }
            InstanceMsg::General(GeneralOut::Renamed { old, new }) => {
                self.target.rename(&new);
                self.tab.set_title(&instance_tab_title(&new));
                let _ = sender.output(InstanceOut::Renamed { old, new });
            }
            InstanceMsg::General(GeneralOut::Deleted { name }) => {
                let _ = sender.output(InstanceOut::Deleted { name });
            }
        }
    }
}

impl Page for InstancePage {
    type Args = (Rc<Shell>, PathBuf, String, Vec<String>);
}

fn instance_tab_title(name: &str) -> String {
    format!("实例设置 · {name}")
}
