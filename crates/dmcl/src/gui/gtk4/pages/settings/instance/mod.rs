//! 单个实例的设置页：左侧「启动」分类，右侧复用游戏设置表单。
//!
//! 页面是关闭即销毁的一次性标签页；实例文件的变化不走全局配置仓库，表单目标自己写盘。
//! 同一实例由根组件按 `(root, 实例名)` 去重，这个模块只负责页面本身。

mod target;

use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

use dmcl_game::{resolve_version, VersionRef};

use crate::gui::gtk4::app::page_host::{Page, PageInit};
use crate::gui::gtk4::app::{config_store, PageToast, Shell};
use crate::gui::gtk4::pages::settings::{
    GameOptionsInit, GameOptionsMsg, GameOptionsPage, JavaProbeCache,
};

use target::InstanceGameOptionsTarget;

/// 实例设置页收到的消息。
#[derive(Debug)]
pub(crate) enum InstanceMsg {
    /// 全局配置改过了：让表单重新读取实例覆盖。
    ConfigChanged,
}

pub(crate) struct InstancePage {
    game: Controller<GameOptionsPage>,
}

#[relm4::component(pub(crate))]
impl SimpleComponent for InstancePage {
    type Init = PageInit<(Rc<Shell>, PathBuf, String)>;
    type Input = InstanceMsg;
    type Output = ();

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

                        #[name(start_list)]
                        gtk::ListBox {
                            add_css_class: "navigation-sidebar",
                            set_selection_mode: gtk::SelectionMode::Single,

                            append = &adw::ActionRow {
                                set_title: "启动",
                                set_activatable: true,
                                add_prefix = &gtk::Image {
                                    set_icon_name: Some("media-playback-start-symbolic"),
                                },
                            },
                        },
                    },
                },
            },

            #[wrap(Some)]
            set_content = &adw::NavigationPage {
                set_title: "启动",

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,

                    #[local_ref]
                    game_widget -> adw::PreferencesPage {},
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root_widget: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let PageInit {
            tab,
            args: (shell, root_path, name),
        } = init;
        let config = config_store::store().config();
        let cache = Rc::new(JavaProbeCache::new());
        let target = Rc::new(InstanceGameOptionsTarget::new(
            config.clone(),
            Rc::clone(&shell),
            PageToast::new(&root_widget),
            root_path.clone(),
            name.clone(),
        ));
        let game = GameOptionsPage::builder()
            .launch(GameOptionsInit {
                target,
                config,
                cache,
                allow_inherit: true,
                required_java_major: None,
            })
            .detach();
        let game_widget = game.widget().clone();
        game_widget.set_vexpand(true);
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

        let widgets = view_output!();
        if let Some(row) = widgets.start_list.row_at_index(0) {
            widgets.start_list.select_row(Some(&row));
        }

        let model = Self { game };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            InstanceMsg::ConfigChanged => {
                let _ = self.game.sender().send(GameOptionsMsg::Reload);
            }
        }
    }
}

impl Page for InstancePage {
    type Args = (Rc<Shell>, PathBuf, String);
}

fn instance_tab_title(name: &str) -> String {
    format!("实例设置 · {name}")
}
