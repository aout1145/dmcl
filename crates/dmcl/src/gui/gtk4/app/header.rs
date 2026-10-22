//! 标题栏：一行里排「添加实例」+ 标签页栏 + 账号 / 任务 / 设置 / 主菜单 + 窗口按钮。
//!
//! 整行都放在 `AdwHeaderBar` 的 title widget 里自己排，原因见 [`Header`]：头栏把 title
//! widget 放在 `GtkCenterBox` 中间、左右各让出「较宽那一侧」的宽度，若只把标签栏当 title
//! widget，左边会被右侧的按钮挤出一大截空白。
//!
//! 账号 / 设置 是占位 popup；任务按钮与「添加实例 → 自定义」通过 [`HeaderOut`] 交给 root；
//! 外观切换与「关于」由标题栏自己处理。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

/// 标题栏的样式补丁（全局 CSS）：
/// * 收紧标签栏竖向内边距，让标题栏保持 libadwaita 正常的 47px 高度；
/// * 隐藏**主页**标签的关闭按钮——libadwaita 没有 per-page 的 closable，也没有公开句柄
///   拿到某个标签的关闭按钮，只能按 CSS 节点把 class 打在主页那个标签上（见
///   [`mark_home_tab`]），其余标签保持可关闭；
/// * 任务按钮的角标：用主题 accent 色的圆形小标签；
/// * 主菜单的外观选择：GNOME 应用里常见的「系统 / 浅色 / 深色」圆形色板。
///
/// 这是 GNOME Text Editor 等应用沿用的通用写法（libadwaita 没有现成控件）：一组互斥的
/// `GtkCheckButton`，把 `radio` 指示器画成圆形色板，再叠一个选中对勾。
pub(crate) const CSS: &str = "\
headerbar.dmcl-header tabbar tabbox { padding-top: 0; padding-bottom: 0; }
headerbar.dmcl-header tabbar .box { padding: 0; }
headerbar.dmcl-header tabbar tab.dmcl-home-tab button.image-button { min-width: 0; min-height: 0; padding: 0; margin: 0; -gtk-icon-size: 1px; opacity: 0; }
headerbar.dmcl-header .dmcl-task-badge { min-width: 16px; min-height: 16px; padding: 0 3px; border-radius: 9999px; background-color: @accent_bg_color; color: @accent_fg_color; font-size: 0.72em; font-weight: 700; }
.dmcl-style-switcher { padding: 6px; }
.dmcl-style-switcher .check { background-color: @accent_bg_color; color: @accent_fg_color; padding: 2px; border-radius: 17px; margin: 3px; }
.dmcl-style-switcher checkbutton { outline-offset: 1px; transition: none; }
.dmcl-style-switcher checkbutton radio { -gtk-icon-source: none; background: none; padding: 12px; min-height: 24px; min-width: 24px; border: none; transition: none; box-shadow: inset 0 0 0 1px @borders; }
.dmcl-style-switcher checkbutton radio:checked { box-shadow: inset 0 0 0 2px @accent_bg_color; }
.dmcl-style-switcher checkbutton.system radio { background: linear-gradient(-45deg, #1e1e1e 49.99%, white 50.01%); }
.dmcl-style-switcher checkbutton.light radio { background-color: white; }
.dmcl-style-switcher checkbutton.dark radio { background-color: #1e1e1e; }";

/// 标题栏发给 root 的消息：需要页面宿主 / 任务窗口配合的动作。
#[derive(Debug)]
pub(crate) enum HeaderOut {
    /// 「添加实例」下拉里的「自定义」：开一个「新实例」页。
    AddCustom,
    /// 点任务按钮：打开任务对话框。
    OpenTasks,
}

/// 标题栏自己的消息。
#[derive(Debug)]
pub(crate) enum HeaderMsg {
    /// 主页标签建好后给它打 class，隐藏关闭按钮。
    MarkHomeTab,
    /// 更新任务按钮上的运行中数量角标。
    SetRunningCount(usize),
    /// 切换主题。
    SetTheme(ThemeChoice),
    /// 打开「关于」。
    About,
}

/// 主菜单里的主题选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    fn from_color_scheme(scheme: adw::ColorScheme) -> Self {
        match scheme {
            adw::ColorScheme::ForceLight => Self::Light,
            adw::ColorScheme::ForceDark => Self::Dark,
            _ => Self::System,
        }
    }

    fn color_scheme(self) -> adw::ColorScheme {
        match self {
            Self::System => adw::ColorScheme::Default,
            Self::Light => adw::ColorScheme::ForceLight,
            Self::Dark => adw::ColorScheme::ForceDark,
        }
    }
}

pub(crate) struct Header {
    /// 标签页栏：等主页标签建好后用来打 class。
    tab_bar: adw::TabBar,
    /// 「关于」对话框的 parent。
    window: adw::ApplicationWindow,
    /// 当前运行中的任务数（任务按钮角标）。
    running: usize,
    /// 主菜单里当前选中的主题。
    theme: ThemeChoice,
}

/// 标题栏本身。
///
/// 整行（加号 / 标签栏 / 拖拽区 / 右侧按钮 / 窗口按钮）都放进 title widget 里自己排，而不是
/// 分头 `pack_start` / `pack_end`。因为 `AdwHeaderBar` 把 title widget 摆在 `GtkCenterBox`
/// 正中：中间宽度 = 总宽 − 2×max(左, 右)。左侧只有一个加号（40px），右侧是几个按钮加
/// 窗口按钮，于是标签栏右边贴着按钮、左边空出近 200px。把两侧内容也并进 title widget、
/// 并关掉头栏自带的 start/end 标题按钮之后，左右两侧宽度都是 0，标签栏紧贴加号并占满中间
/// 宽度，右侧留一小段固定的 `GtkWindowHandle` 当窗口拖拽区。
#[relm4::component(pub(crate))]
impl SimpleComponent for Header {
    type Init = (adw::TabView, adw::ApplicationWindow);
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

                // 「添加实例」：最左的 + 按钮，点开是下载类型列表（目前只有「自定义」）。
                gtk::MenuButton {
                    set_icon_name: "list-add-symbolic",
                    set_tooltip_text: Some("添加实例"),

                    #[wrap(Some)]
                    set_popover: add_popover = &gtk::Popover {
                        // 默认是「以按钮为中心」展开；加号在最左边，居中的话菜单会一半探到
                        // 窗口外再被挤回来。改成左对齐：菜单左边缘贴住加号。
                        set_halign: gtk::Align::Start,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,

                            gtk::Button {
                                set_label: "自定义",
                                add_css_class: "flat",
                                set_margin_top: 6,
                                set_margin_bottom: 6,
                                set_margin_start: 6,
                                set_margin_end: 6,
                                connect_clicked[sender, add_popover] => move |_| {
                                    // 自定义按钮不是 model button，popover 不会自己收起来。
                                    add_popover.popdown();
                                    let _ = sender.output(HeaderOut::AddCustom);
                                },
                            },
                        },
                    },
                },

                #[local_ref]
                tab_bar_widget -> adw::TabBar {},

                // 标签栏和右侧按钮之间留一小段固定的拖拽区。标签栏仍占满其余宽度；
                // 这段空白由 `GtkWindowHandle` 接管，拖它就能移动窗口。
                gtk::WindowHandle {
                    set_size_request: (28, -1),
                },

                gtk::MenuButton {
                    set_icon_name: "system-users-symbolic",
                    set_tooltip_text: Some("账号"),

                    #[wrap(Some)]
                    set_popover: account_popover = &gtk::Popover {
                        gtk::Label {
                            set_label: "账号功能尚未实现",
                            set_margin_top: 8,
                            set_margin_bottom: 8,
                            set_margin_start: 12,
                            set_margin_end: 12,
                        },
                    },
                },

                // 任务按钮：右上角叠一个运行中数量角标。
                gtk::Overlay {
                    gtk::Button {
                        set_icon_name: "view-list-symbolic",
                        set_tooltip_text: Some("任务"),
                        add_css_class: "flat",
                        connect_clicked[sender] => move |_| {
                            let _ = sender.output(HeaderOut::OpenTasks);
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

                gtk::MenuButton {
                    set_icon_name: "preferences-system-symbolic",
                    set_tooltip_text: Some("设置"),

                    #[wrap(Some)]
                    set_popover: settings_popover = &gtk::Popover {
                        gtk::Label {
                            set_label: "设置功能尚未实现",
                            set_margin_top: 8,
                            set_margin_bottom: 8,
                            set_margin_start: 12,
                            set_margin_end: 12,
                        },
                    },
                },

                // GNOME 应用常见的主菜单：外观圆形色板 + 关于。
                gtk::MenuButton {
                    set_icon_name: "open-menu-symbolic",
                    set_tooltip_text: Some("主菜单"),

                    #[wrap(Some)]
                    set_popover: menu_popover = &gtk::Popover {
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 6,
                            set_margin_top: 8,
                            set_margin_bottom: 6,
                            set_margin_start: 8,
                            set_margin_end: 8,

                            gtk::Box {
                                add_css_class: "dmcl-style-switcher",
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 18,
                                set_halign: gtk::Align::Center,
                                set_homogeneous: true,

                                gtk::Overlay {
                                    set_halign: gtk::Align::Center,

                                    #[name(theme_system)]
                                    gtk::CheckButton {
                                        add_css_class: "system",
                                        set_tooltip_text: Some("跟随系统"),
                                        #[watch]
                                        set_active: model.theme == ThemeChoice::System,
                                        connect_toggled[sender] => move |button| {
                                            if button.is_active() {
                                                sender.input(HeaderMsg::SetTheme(ThemeChoice::System));
                                            }
                                        },
                                    },

                                    add_overlay = &gtk::Image {
                                        add_css_class: "check",
                                        set_icon_name: Some("object-select-symbolic"),
                                        set_pixel_size: 13,
                                        set_halign: gtk::Align::End,
                                        set_valign: gtk::Align::End,
                                        #[watch]
                                        set_visible: model.theme == ThemeChoice::System,
                                    },
                                },

                                gtk::Overlay {
                                    set_halign: gtk::Align::Center,

                                    #[name(theme_light)]
                                    gtk::CheckButton {
                                        add_css_class: "light",
                                        set_tooltip_text: Some("浅色"),
                                        #[watch]
                                        set_active: model.theme == ThemeChoice::Light,
                                        connect_toggled[sender] => move |button| {
                                            if button.is_active() {
                                                sender.input(HeaderMsg::SetTheme(ThemeChoice::Light));
                                            }
                                        },
                                    },

                                    add_overlay = &gtk::Image {
                                        add_css_class: "check",
                                        set_icon_name: Some("object-select-symbolic"),
                                        set_pixel_size: 13,
                                        set_halign: gtk::Align::End,
                                        set_valign: gtk::Align::End,
                                        #[watch]
                                        set_visible: model.theme == ThemeChoice::Light,
                                    },
                                },

                                gtk::Overlay {
                                    set_halign: gtk::Align::Center,

                                    #[name(theme_dark)]
                                    gtk::CheckButton {
                                        add_css_class: "dark",
                                        set_tooltip_text: Some("深色"),
                                        #[watch]
                                        set_active: model.theme == ThemeChoice::Dark,
                                        connect_toggled[sender] => move |button| {
                                            if button.is_active() {
                                                sender.input(HeaderMsg::SetTheme(ThemeChoice::Dark));
                                            }
                                        },
                                    },

                                    add_overlay = &gtk::Image {
                                        add_css_class: "check",
                                        set_icon_name: Some("object-select-symbolic"),
                                        set_pixel_size: 13,
                                        set_halign: gtk::Align::End,
                                        set_valign: gtk::Align::End,
                                        #[watch]
                                        set_visible: model.theme == ThemeChoice::Dark,
                                    },
                                },
                            },

                            gtk::Separator {},

                            gtk::Button {
                                set_label: "关于",
                                add_css_class: "flat",
                                set_halign: gtk::Align::Fill,
                                connect_clicked[sender, menu_popover] => move |_| {
                                    menu_popover.popdown();
                                    sender.input(HeaderMsg::About);
                                },
                            },
                        },
                    },
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
        (tab_view, window): Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let tab_bar = adw::TabBar::new();
        tab_bar.set_view(Some(&tab_view));
        // 单个标签时也一直显示标签栏：否则从 2 个标签关到 1 个时，整条标签栏会跟着
        // `GtkRevealer`（slide-down，200ms）滑走，和标签自身的收缩动画叠在一起，看着不顺。
        tab_bar.set_autohide(false);

        let theme = ThemeChoice::from_color_scheme(adw::StyleManager::default().color_scheme());

        let model = Header {
            tab_bar,
            window,
            running: 0,
            theme,
        };

        let tab_bar_widget = &model.tab_bar;
        let widgets = view_output!();

        // 三个圆形色板互斥；分组后点击已选中的那个也不会把它取消。
        widgets.theme_light.set_group(Some(&widgets.theme_system));
        widgets.theme_dark.set_group(Some(&widgets.theme_system));

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            HeaderMsg::MarkHomeTab => mark_home_tab(&self.tab_bar),
            HeaderMsg::SetRunningCount(running) => self.running = running,
            HeaderMsg::SetTheme(theme) => {
                self.theme = theme;
                adw::StyleManager::default().set_color_scheme(theme.color_scheme());
            }
            HeaderMsg::About => {
                let about = adw::AboutDialog::new();
                about.set_application_name("DMCL");
                about.set_application_icon("io.github.aout1145.DMCL");
                about.set_version(env!("CARGO_PKG_VERSION"));
                about.set_developer_name("a_out_");
                about.set_comments("非官方 Minecraft 启动器");
                about.set_license_type(gtk::License::Gpl30);
                about.present(Some(&self.window));
            }
        }
    }
}

/// 给主页的标签节点打上 `.dmcl-home-tab`，好让 [`CSS`] 只隐藏它的关闭按钮。
///
/// 标签栏的 CSS 节点链是 `tabbar` → `tab` → `button.image-button`。主页是第一个标签，
/// 所以找第一个名为 `tab` 的后代即可（**要在标签建好之后调用**）。
fn mark_home_tab(tab_bar: &adw::TabBar) {
    if let Some(tab) = find_css_node(tab_bar.upcast_ref(), "tab") {
        tab.add_css_class("dmcl-home-tab");
    }
}

/// 深度优先找第一个 CSS 名为 `name` 的后代。
fn find_css_node(widget: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    let mut child = widget.first_child();
    while let Some(current) = child {
        if current.css_name().as_str() == name {
            return Some(current);
        }
        if let Some(found) = find_css_node(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
