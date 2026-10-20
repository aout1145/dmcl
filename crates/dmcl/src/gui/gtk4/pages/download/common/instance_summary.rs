//! 实例概要（download 页面公共组件）：图标 + 名称 + 分类 + 安装按钮。
//!
//! 名称与图标都有「当前值 + 用户是否改过」两个状态：外部推自动值
//! （[`SummaryIn::SetAutoName`] / [`SummaryIn::SetAutoIcon`]）只在用户没改过时生效，
//! 用户一改过就永不覆盖——CLI/GUI 共用的自动命名语义。
//!
//! 自动命名规则（`dmcl_install::default_version_name`）由页面计算后推送，
//! 组件自身不知道版本与加载器怎么命名。

use std::path::PathBuf;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::gio;
use relm4::prelude::*;

use dmcl_install::LoaderKind;

/// 实例图标的来源。
///
/// `Initials` = 首字母彩色头像；`Loader` = 按加载器取徽标字母（头像按文字自动配色，
/// Forge → F）；`File` = 用户自选的本地图片。
#[derive(Debug, Clone)]
pub(crate) enum IconSource {
    Initials,
    Loader(LoaderKind),
    File(PathBuf),
}

/// 组件输入。
#[derive(Debug)]
pub(crate) enum SummaryIn {
    /// 外部自动命名（用户没改过名称时生效）。
    SetAutoName(String),
    /// 外部自动图标（用户没改过图标时生效）。
    SetAutoIcon(IconSource),
    /// 安装按钮可用性。
    SetInstallEnabled(bool),
    /// 用户在图标菜单里选了某项。
    PickIcon(IconSource),
    /// 用户在图标菜单里选了「自动」。
    PickAutoIcon,
    /// 用户点了「选择图片…」，发起文件选择。
    EditIconFile,
    /// 名称输入框的文本变化。
    NameEdited(String),
    /// 用户点了安装按钮。
    InstallClicked,
}

/// 组件输出。
#[derive(Debug)]
pub(crate) enum SummaryOut {
    /// 点了安装，带当前实例名。
    Install { name: String },
    /// 显示的名称变了（自动 / 用户编辑都会发），页面可用来同步标签标题。
    NameChanged(String),
}

pub(crate) struct InstanceSummary {
    /// 名称输入框。程序改文本时要直接写它，不能在 `view!` 里 `#[watch] set_text`：
    /// 那会在每次视图刷新时重设文本，GTK 还可能额外发一条中间态 `changed`（先清空后写入），
    /// 被当成用户清空 → 自动命名再写回 → 无限来回。
    name_row: adw::EntryRow,
    /// 显示中的名称。
    name: String,
    /// 用户改过名称后，自动命名永不再应用。
    name_user_modified: bool,
    /// 当前图标 / 自动图标（用户点了「自动」时恢复成它）。
    icon: IconSource,
    /// 用户改过图标后，自动图标永不再应用。
    icon_user_modified: bool,
    auto_icon: IconSource,
    /// `File` 图标装载出来的纹理；装载失败回落为首字母头像。
    custom_image: Option<gdk::Texture>,
    install_enabled: bool,
}

impl InstanceSummary {
    fn apply_icon(&mut self, icon: IconSource) {
        self.custom_image = match &icon {
            IconSource::File(path) => gdk::Texture::from_filename(path).ok(),
            _ => None,
        };
        self.icon = icon;
    }

    /// 头像文字（`File` 已装载图像时头像不显示文字，这个值无所谓）。
    fn avatar_text(&self) -> String {
        let first = |text: &str| {
            text.chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "＋".to_string())
        };
        match &self.icon {
            IconSource::Initials | IconSource::File(_) => first(&self.name),
            IconSource::Loader(kind) => first(kind.display_name()),
        }
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for InstanceSummary {
    type Init = ();
    type Input = SummaryIn;
    type Output = SummaryOut;

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                // 图标按钮：点开菜单换图标。
                gtk::MenuButton {
                    add_css_class: "flat",
                    set_valign: gtk::Align::Start,
                    set_tooltip_text: Some("更换实例图标"),

                    #[wrap(Some)]
                    set_child = &adw::Avatar {
                        set_size: 64,
                        set_show_initials: true,
                        #[watch]
                        set_text: Some(&model.avatar_text()),
                        #[watch]
                        set_custom_image: model.custom_image.as_ref(),
                    },

                    #[wrap(Some)]
                    set_popover = &gtk::Popover {
                        set_autohide: true,

                        #[wrap(Some)]
                        set_child = &gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 4,
                            set_margin_all: 6,

                            gtk::Button {
                                add_css_class: "flat",
                                set_label: "自动",
                                connect_clicked[sender] => move |_| sender.input(SummaryIn::PickAutoIcon),
                            },
                            gtk::Button {
                                add_css_class: "flat",
                                set_label: LoaderKind::Forge.display_name(),
                                connect_clicked[sender] => move |_| {
                                    sender.input(SummaryIn::PickIcon(IconSource::Loader(LoaderKind::Forge)));
                                },
                            },
                            gtk::Button {
                                add_css_class: "flat",
                                set_label: LoaderKind::NeoForge.display_name(),
                                connect_clicked[sender] => move |_| {
                                    sender.input(SummaryIn::PickIcon(IconSource::Loader(LoaderKind::NeoForge)));
                                },
                            },
                            gtk::Button {
                                add_css_class: "flat",
                                set_label: LoaderKind::Fabric.display_name(),
                                connect_clicked[sender] => move |_| {
                                    sender.input(SummaryIn::PickIcon(IconSource::Loader(LoaderKind::Fabric)));
                                },
                            },
                            // 文件选择对话框要挂在应用的活动窗口上，逻辑在 update 里。
                            gtk::Button {
                                add_css_class: "flat",
                                set_label: "选择图片…",
                                connect_clicked[sender] => move |_| sender.input(SummaryIn::EditIconFile),
                            },
                        },
                    },
                },

                // 右侧两行（名称 / 分类）+ 右下角的安装按钮。
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    set_hexpand: true,

                    gtk::ListBox {
                        add_css_class: "boxed-list",
                        set_hexpand: true,

                        #[local_ref]
                        name_row_widget -> adw::EntryRow {},

                        append = &adw::ComboRow {
                            set_title: "分类",
                            set_model: Some(&gtk::StringList::new(&["未分类"])),
                            // TODO: 分类先禁用——等实例元数据存储设计好后启用并接入持久化。
                            set_sensitive: false,
                            set_subtitle: "尚未支持",
                        },
                    },

                    gtk::Button {
                        set_label: "安装",
                        add_css_class: "suggested-action",
                        set_halign: gtk::Align::End,
                        set_margin_top: 4,
                        #[watch]
                        set_sensitive: model.install_enabled,
                        connect_clicked[sender] => move |_| sender.input(SummaryIn::InstallClicked),
                    },
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let name_row = adw::EntryRow::builder().title("实例名称").build();
        {
            let sender = sender.clone();
            name_row.connect_changed(move |row| {
                sender.input(SummaryIn::NameEdited(row.text().into()));
            });
        }
        let model = InstanceSummary {
            name_row,
            name: String::new(),
            name_user_modified: false,
            icon: IconSource::Initials,
            icon_user_modified: false,
            auto_icon: IconSource::Initials,
            custom_image: None,
            install_enabled: false,
        };
        let name_row_widget = model.name_row.clone();
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            SummaryIn::SetAutoName(auto) => {
                if !self.name_user_modified && auto != self.name {
                    // 用户没改过名称才跟随自动值；只在这里写输入框，避免视图刷新反复触发 changed。
                    self.name = auto.clone();
                    if self.name_row.text() != auto {
                        self.name_row.set_text(&auto);
                    }
                    let _ = sender.output(SummaryOut::NameChanged(self.name.clone()));
                }
            }
            SummaryIn::SetAutoIcon(icon) => {
                self.auto_icon = icon;
                if !self.icon_user_modified {
                    self.apply_icon(self.auto_icon.clone());
                }
            }
            SummaryIn::SetInstallEnabled(enabled) => self.install_enabled = enabled,
            SummaryIn::PickIcon(icon) => {
                self.icon_user_modified = true;
                self.apply_icon(icon);
            }
            SummaryIn::PickAutoIcon => {
                self.icon_user_modified = false;
                self.apply_icon(self.auto_icon.clone());
            }
            SummaryIn::EditIconFile => {
                pick_icon_file(sender.clone());
            }
            SummaryIn::NameEdited(text) => {
                // 用户输入只在「事件文本 == 输入框当前文本」时成立：程序 `set_text` 时
                // GTK 可能发一条中间态（先清空再写入）的 `changed`，不能当成用户清空。
                if text != self.name_row.text() || text == self.name {
                    return;
                }
                self.name = text.clone();
                self.name_user_modified = true;
                let _ = sender.output(SummaryOut::NameChanged(text));
            }
            SummaryIn::InstallClicked => {
                let _ = sender.output(SummaryOut::Install {
                    name: self.name.clone(),
                });
            }
        }
    }
}

/// 弹出「选择图片」对话框（挂在应用当前的活动窗口上）。
fn pick_icon_file(sender: ComponentSender<InstanceSummary>) {
    let Some(parent) = relm4::main_adw_application().active_window() else {
        return;
    };
    let dialog = gtk::FileDialog::builder()
        .title("选择实例图标")
        .accept_label("选择")
        .build();
    dialog.open(Some(&parent), None::<&gio::Cancellable>, move |result| {
        if let Some(path) = result.ok().and_then(|file| file.path()) {
            sender.input(SummaryIn::PickIcon(IconSource::File(path)));
        }
    });
}
