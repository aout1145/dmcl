//! 标题栏：一行里排「Minecraft 目录」+ 标签页栏 + 任务 / 主菜单 + 窗口按钮。
//!
//! 任务按钮、根目录菜单和主菜单里的设置入口通过 [`HeaderOut`] 交给 root；外观切换在
//! [`theme`]，点击「关于」打开 [`about`] 里的对话框。主菜单行带 GTK 助记符，应用级加速键
//! 由根组件注册。

use std::collections::BTreeSet;
use std::path::PathBuf;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::glib::variant::ToVariant;
use relm4::prelude::*;

mod about;
mod root_row;
mod theme;

use root_row::{RootRow, RootRowOut};
use theme::ThemeSwitcher;
pub(crate) use theme::CSS as THEME_CSS;

/// 标题栏的样式补丁（全局 CSS）：
/// * 收紧标签栏竖向内边距，让标题栏保持 libadwaita 正常的 47px 高度；
/// * 任务按钮的角标：用主题 accent 色的圆形小标签；
/// * 根目录行左右留一点内边距。
pub(crate) const CSS: &str = "\
headerbar.dmcl-header tabbar tabbox { padding-top: 0; padding-bottom: 0; }
headerbar.dmcl-header tabbar .box { padding: 0; }
headerbar.dmcl-header .dmcl-task-badge { min-width: 16px; min-height: 16px; padding: 0 3px; border-radius: 9999px; background-color: @accent_bg_color; color: @accent_fg_color; font-size: 0.72em; font-weight: 700; }
.dmcl-root-row { padding-left: 4px; padding-right: 4px; }";

/// 标题栏发给 root 的消息：需要页面宿主 / 任务窗口配合的动作。
#[derive(Debug)]
pub(crate) enum HeaderOut {
    /// 点任务按钮：打开任务对话框。
    Tasks,
    /// 打开或聚焦某个 root 的主页。
    Home(String),
}

/// 标题栏自己的消息。
#[derive(Debug)]
pub(crate) enum HeaderMsg {
    /// 更新任务按钮上的运行中数量角标。
    SetRunningCount(usize),
    /// 更新已打开的主页集合。
    SetOpenHomes(Vec<String>),
    /// 配置里的根目录列表变了，重建菜单行。
    SetRoots(Vec<(String, PathBuf)>),
    /// 根目录行发出的动作。
    RootAction(RootRowOut),
    /// 打开「关于」。
    About,
}

pub(crate) struct Header {
    /// 「关于」对话框的 parent。
    window: adw::ApplicationWindow,
    /// 当前运行中的任务数（任务按钮角标）。
    running: usize,
    /// 主菜单里的外观切换器。
    _theme: Controller<ThemeSwitcher>,
    /// 根目录菜单里的行。
    roots: FactoryVecDeque<RootRow>,
    /// 菜单行当前对应的配置内容；配置变化时用来判断要不要重建。
    configured_roots: Vec<(String, PathBuf)>,
    /// 根目录菜单按钮；选中某一行后主动收起。
    root_menu_button: Option<gtk::MenuButton>,
    /// 当前已经打开的主页名字。
    open_roots: BTreeSet<String>,
}

/// 标题栏本身。
///
/// 整行（目录菜单 / 标签栏 / 拖拽区 / 右侧按钮 / 窗口按钮）都放进 title widget 里自己排，
/// 而不是分头 `pack_start` / `pack_end`。因为 `AdwHeaderBar` 把 title widget 摆在
/// `GtkCenterBox` 正中：中间宽度 = 总宽 − 2×max(左, 右)。左侧只有一个目录按钮（40px），
/// 右侧是几个按钮加窗口按钮，于是标签栏右边贴着按钮、左边空出近 200px。把两侧内容也并进
/// title widget、并关掉头栏自带的 start/end 标题按钮之后，左右两侧宽度都是 0，标签栏紧贴
/// 目录按钮并占满中间宽度，右侧留一小段固定的 `GtkWindowHandle` 当窗口拖拽区。
#[relm4::component(pub(crate))]
impl SimpleComponent for Header {
    type Init = (adw::TabView, adw::ApplicationWindow, Vec<(String, PathBuf)>);
    type Input = HeaderMsg;
    type Output = HeaderOut;

    view! {
        adw::HeaderBar {
            add_css_class: "dmcl-header",
            set_show_start_title_buttons: false,
            set_show_end_title_buttons: false,

            #[wrap(Some)]
            set_title_widget = &gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,

                // 目录按钮和标签栏放在一个无间距的盒子里，让标签栏紧贴目录按钮；
                // 外层 6px 间距只用于标签栏和右侧按钮之间。
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 0,
                    set_hexpand: true,

                    // Minecraft 根目录：打开其它 root 的主页。
                    #[name(root_menu_button)]
                    gtk::MenuButton {
                        set_icon_name: "folder-symbolic",
                        set_tooltip_text: Some("Minecraft 目录"),

                        #[wrap(Some)]
                        set_popover = &gtk::Popover {
                            set_halign: gtk::Align::Start,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 4,
                                set_margin_top: 6,
                                set_margin_bottom: 6,
                                set_margin_start: 2,
                                set_margin_end: 6,

                                #[local_ref]
                                roots_widget -> gtk::Box {},
                            },
                        },
                    },

                    #[local_ref]
                    tab_bar_widget -> adw::TabBar {
                        set_hexpand: true,
                    },
                },

                // 标签栏和右侧按钮之间留一小段固定的拖拽区。标签栏仍占满其余宽度；
                // 这段空白由 `GtkWindowHandle` 接管，拖它就能移动窗口。
                gtk::WindowHandle {
                    set_size_request: (28, -1),
                },

                // 任务按钮：右上角叠一个运行中数量角标。
                gtk::Overlay {
                    gtk::Button {
                        set_icon_name: "view-list-symbolic",
                        set_tooltip_text: Some("任务"),
                        add_css_class: "flat",
                        connect_clicked[sender] => move |_| {
                            let _ = sender.output(HeaderOut::Tasks);
                        },
                    },

                    add_overlay = &gtk::Label {
                        add_css_class: "dmcl-task-badge",
                        set_halign: gtk::Align::End,
                        set_valign: gtk::Align::Start,
                        set_margin_top: 1,
                        set_margin_end: 1,
                        set_can_target: false,
                        set_xalign: 0.5,
                        #[watch]
                        set_label: &model.running.to_string(),
                        #[watch]
                        set_visible: model.running > 0,
                    },
                },

                // GNOME 应用常见的主菜单：GtkPopoverMenu 由 GMenu 驱动，外观圆形色板
                // 作为 custom child 插进第一段；菜单项交给应用级 action。
                #[name(menu_button)]
                gtk::MenuButton {
                    set_icon_name: "open-menu-symbolic",
                    set_tooltip_text: Some("主菜单"),
                },
                // 窗口按钮：头栏自带的那套关掉了，这里自己加在行尾。
                // `decoration-layout` 默认跟随桌面设置；桌面不画窗口按钮时它自己宽度为 0。
                gtk::WindowControls {
                    set_side: gtk::PackType::End,
                },
            },
        }
    }

    fn init(
        (tab_view, window, roots): Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));
        // 单个标签时也一直显示标签栏：否则从 2 个标签关到 1 个时，整条标签栏会跟着
        // `GtkRevealer`（slide-down，200ms）滑走，和标签自身的收缩动画叠在一起，看着不顺。
        tab_bar.set_autohide(false);
        // 标签按内容取最小宽度、从左边依次排开（类似 Chrome），而不是平分标签栏的整条宽度。
        tab_bar.set_expand_tabs(false);

        let theme = ThemeSwitcher::builder().launch(()).detach();
        let theme_widget = theme.widget().clone();
        let mut root_rows: FactoryVecDeque<RootRow> = FactoryVecDeque::builder()
            .launch(gtk::Box::new(gtk::Orientation::Vertical, 4))
            .forward(sender.input_sender(), HeaderMsg::RootAction);
        for root in &roots {
            root_rows.guard().push_back(root.clone());
        }
        let roots_widget = root_rows.widget().clone();
        let mut model = Header {
            window,
            running: 0,
            _theme: theme,
            roots: root_rows,
            configured_roots: roots,
            root_menu_button: None,
            open_roots: BTreeSet::new(),
        };

        let tab_bar_widget = &tab_bar;
        let roots_widget = &roots_widget;
        let widgets = view_output!();
        model.root_menu_button = Some(widgets.root_menu_button.clone());

        // 主菜单用 GNOME 应用常见的 GtkPopoverMenu + GMenu：普通项由 GTK 生成标准
        // GtkModelButton，外观色板作为 custom child 替换第一段里的占位项。
        let menu = primary_menu();
        let menu_popover = gtk::PopoverMenu::from_model(Some(&menu));
        menu_popover.add_child(&theme_widget, "theme");
        widgets.menu_button.set_popover(Some(&menu_popover));

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            HeaderMsg::SetRunningCount(running) => self.running = running,
            HeaderMsg::SetOpenHomes(open) => {
                self.open_roots = open.into_iter().collect();
                self.sync_root_rows();
            }
            HeaderMsg::SetRoots(roots) => {
                if self.configured_roots == roots {
                    return;
                }
                self.configured_roots = roots;
                self.rebuild_root_rows();
            }
            HeaderMsg::RootAction(RootRowOut::Open(index)) => {
                let Some(row) = self.roots.get(index.current_index()) else {
                    return;
                };
                let name = row.name.clone();
                if let Some(button) = self.root_menu_button.clone() {
                    // 等这次点击完全结束再收菜单，避免 popover 消失后同一次点击漏到页面上。
                    gtk::glib::idle_add_local_once(move || button.set_active(false));
                }
                let _ = sender.output(HeaderOut::Home(name));
            }
            HeaderMsg::About => about::show(&self.window),
        }
    }
}

impl Header {
    /// 按配置重建根目录菜单行，并恢复「已打开」标记。
    fn rebuild_root_rows(&mut self) {
        {
            let mut guard = self.roots.guard();
            guard.clear();
            for root in &self.configured_roots {
                guard.push_back(root.clone());
            }
        }
        self.sync_root_rows();
    }

    fn sync_root_rows(&mut self) {
        let mut guard = self.roots.guard();
        for row in guard.iter_mut() {
            row.open = self.open_roots.contains(&row.name);
        }
    }
}

/// 主菜单的 GMenu 结构：外观色板占位项 / 两个设置入口 / 关于。
///
/// `accel` 是 GtkPopoverMenu 读取的菜单元数据；GTK 会把它渲染成菜单右侧的纯文本
/// 快捷键提示。真正的键盘绑定仍由根组件注册到 `app.*` action 上。
fn primary_menu() -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();

    let theme = gtk::gio::Menu::new();
    let theme_item = gtk::gio::MenuItem::new(None, None);
    theme_item.set_attribute_value("custom", Some(&"theme".to_variant()));
    theme.append_item(&theme_item);
    menu.append_section(None, &theme);

    let options = gtk::gio::Menu::new();
    options.append_item(&menu_item(
        "启动器选项(_L)",
        "app.launcher-options",
        Some("<Control>comma"),
    ));
    options.append_item(&menu_item(
        "全局游戏选项(_G)",
        "app.game-options",
        Some("<Control><Shift>comma"),
    ));
    menu.append_section(None, &options);

    let about = gtk::gio::Menu::new();
    about.append_item(&menu_item("关于(_A)", "app.about", None));
    menu.append_section(None, &about);

    menu
}

fn menu_item(label: &str, action: &str, accel: Option<&str>) -> gtk::gio::MenuItem {
    let item = gtk::gio::MenuItem::new(Some(label), Some(action));
    if let Some(accel) = accel {
        item.set_attribute_value("accel", Some(&accel.to_variant()));
    }
    item
}
