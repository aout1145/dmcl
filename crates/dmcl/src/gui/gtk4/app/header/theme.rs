//! 主菜单里的外观切换。
//!
//! GNOME 应用里常见的「跟随系统 / 浅色 / 深色」圆形色板：一组互斥的
//! `GtkCheckButton`，把 `radio` 指示器画成圆形，再叠一个选中对勾。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

/// 外观选择器的样式补丁。
pub(crate) const CSS: &str = "\
.dmcl-style-switcher { padding: 6px; }
.dmcl-style-switcher .check { background-color: @accent_bg_color; color: @accent_fg_color; padding: 2px; border-radius: 17px; margin: 3px; }
.dmcl-style-switcher checkbutton { outline-offset: 1px; transition: none; }
.dmcl-style-switcher checkbutton radio { -gtk-icon-source: none; background: none; padding: 12px; min-height: 24px; min-width: 24px; border: none; transition: none; box-shadow: inset 0 0 0 1px @borders; }
.dmcl-style-switcher checkbutton radio:checked { box-shadow: inset 0 0 0 2px @accent_bg_color; }
.dmcl-style-switcher checkbutton.system radio { background: linear-gradient(-45deg, #1e1e1e 49.99%, white 50.01%); }
.dmcl-style-switcher checkbutton.light radio { background-color: white; }
.dmcl-style-switcher checkbutton.dark radio { background-color: #1e1e1e; }";

/// 主菜单里的主题选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThemeChoice {
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

pub(super) struct ThemeSwitcher {
    theme: ThemeChoice,
}

#[relm4::component(pub(super))]
impl SimpleComponent for ThemeSwitcher {
    type Init = ();
    type Input = ThemeChoice;
    type Output = ();

    view! {
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
                            sender.input(ThemeChoice::System);
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
                            sender.input(ThemeChoice::Light);
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
                            sender.input(ThemeChoice::Dark);
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
        }
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = Self {
            theme: ThemeChoice::from_color_scheme(adw::StyleManager::default().color_scheme()),
        };
        let widgets = view_output!();

        // 三个圆形色板互斥；分组后点击已选中的那个也不会把它取消。
        widgets.theme_light.set_group(Some(&widgets.theme_system));
        widgets.theme_dark.set_group(Some(&widgets.theme_system));

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        self.theme = message;
        adw::StyleManager::default().set_color_scheme(message.color_scheme());
    }
}
