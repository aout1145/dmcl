//! 主菜单里的外观切换。
//!
//! 结构照 GNOME Text Editor 的 `EditorThemeSelector`：一个横向 `GtkBox`，三个
//! `GtkCheckButton` 都设 `hexpand` 并居中。圆形色板画在 checkbutton 本身，指示器
//! 透明并挪到右下角，选中时用 `object-select-symbolic` 显示对勾。

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;

/// 外观选择器的样式。
pub(crate) const CSS: &str = "\
.dmcl-style-switcher { margin: 9px; }
.dmcl-style-switcher checkbutton { padding: 0; min-height: 44px; min-width: 44px; padding: 1px; background-clip: content-box; border-radius: 9999px; box-shadow: inset 0 0 0 1px @borders; }
.dmcl-style-switcher checkbutton:checked { box-shadow: inset 0 0 0 2px @accent_bg_color; }
.dmcl-style-switcher checkbutton.system { background-image: linear-gradient(to bottom right, #fff 49.99%, #202020 50.01%); }
.dmcl-style-switcher checkbutton.light { background-color: #fff; }
.dmcl-style-switcher checkbutton.dark { background-color: #202020; }
.dmcl-style-switcher checkbutton radio { -gtk-icon-source: none; border: none; background: none; box-shadow: none; min-width: 12px; min-height: 12px; border-radius: 9999px; transform: translate(27px, 14px); padding: 2px; }
.dmcl-style-switcher checkbutton radio:checked { -gtk-icon-source: -gtk-icontheme(\"object-select-symbolic\"); background-color: @accent_bg_color; color: @accent_fg_color; }";

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
            set_spacing: 12,
            set_hexpand: true,

            #[name(theme_system)]
            gtk::CheckButton {
                add_css_class: "system",
                set_tooltip_text: Some("跟随系统"),
                set_hexpand: true,
                set_halign: gtk::Align::Center,
                set_focus_on_click: false,
                #[watch]
                set_active: model.theme == ThemeChoice::System,
                connect_toggled[sender] => move |button| {
                    if button.is_active() {
                        sender.input(ThemeChoice::System);
                    }
                },
            },

            #[name(theme_light)]
            gtk::CheckButton {
                add_css_class: "light",
                set_tooltip_text: Some("浅色"),
                set_hexpand: true,
                set_halign: gtk::Align::Center,
                set_focus_on_click: false,
                #[watch]
                set_active: model.theme == ThemeChoice::Light,
                connect_toggled[sender] => move |button| {
                    if button.is_active() {
                        sender.input(ThemeChoice::Light);
                    }
                },
            },

            #[name(theme_dark)]
            gtk::CheckButton {
                add_css_class: "dark",
                set_tooltip_text: Some("深色"),
                set_hexpand: true,
                set_halign: gtk::Align::Center,
                set_focus_on_click: false,
                #[watch]
                set_active: model.theme == ThemeChoice::Dark,
                connect_toggled[sender] => move |button| {
                    if button.is_active() {
                        sender.input(ThemeChoice::Dark);
                    }
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
