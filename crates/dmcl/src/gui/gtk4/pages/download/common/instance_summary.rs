//! 实例概要（download 页面公共组件）：图标 + 名称 + 分类 + 安装按钮。
//!
//! 名称与图标都有「当前值 + 用户是否改过」两个状态：外部推自动值
//! （[`SummaryIn::SetAutoName`] / [`SummaryIn::SetAutoIcon`]）只在用户没改过时生效，
//! 用户一改过就永不覆盖——CLI/GUI 共用的自动命名语义。名称被清空时视为「从未编辑」，
//! 自动命名可以重新接管。
//!
//! 自动命名规则（`dmcl_install::default_version_name`）由页面计算后推送，
//! 组件自身不知道版本与加载器怎么命名。名称的格式校验、`versions/<名称>` 冲突检查
//! 和安装按钮可用性都在组件内完成；页面只通过 [`SummaryIn::SetReady`] 告知
//! 「除名称外的安装条件是否满足」（例如是否已选 Minecraft 版本）。

use std::path::PathBuf;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::prelude::*;

use dmcl_install::validate_version_name;
use dmcl_mcmeta::layout::version_dir;

use crate::gui::gtk4::widgets::category_picker::{
    CategoryPicker, CategoryPickerInit, CategoryPickerOut,
};
use crate::gui::gtk4::widgets::instance_icon::{choose_icon_file, IconKind, InstanceIcon};

/// 组件输入。
#[derive(Debug)]
pub(crate) enum SummaryIn {
    /// 外部自动命名（用户没改过名称时生效）。
    SetAutoName(String),
    /// 外部自动图标（用户没改过图标时生效）。
    SetAutoIcon(InstanceIcon),
    /// 除名称外的安装条件是否满足（如是否已选 Minecraft 版本）。
    SetReady(bool),
    /// 用户选好了自定义图片。
    PickCustomIcon(PathBuf),
    /// 用户在图标菜单里选了「自动」。
    PickAutoIcon,
    /// 用户点了「自定义…」，发起文件选择。
    EditIconFile,
    /// 名称输入框的文本变化。
    NameEdited(String),
    /// 分类选择器报告了新的分类（选择或编辑）。
    CategoryChanged(String),
    /// 用户点了安装按钮。
    InstallClicked,
}

/// 组件输出。
#[derive(Debug)]
pub(crate) enum SummaryOut {
    /// 点了安装，带当前实例名、分类与用户选的自定义图标。
    Install {
        name: String,
        category: String,
        icon: Option<PathBuf>,
    },
    /// 显示的名称变了（自动 / 用户编辑都会发），页面可用来同步标签标题。
    NameChanged(String),
}

/// 组件初始化参数。
pub(crate) struct SummaryInit {
    /// 已有分类建议。
    pub categories: Vec<String>,
    /// 安装根目录：用来检查 `versions/<名称>` 是否已存在。
    pub root_path: PathBuf,
}

pub(crate) struct InstanceSummary {
    /// 名称输入框。程序改文本时要直接写它，不能在 `view!` 里 `#[watch] set_text`：
    /// 那会在每次视图刷新时重设文本，GTK 还可能额外发一条中间态 `changed`（先清空后写入），
    /// 被当成用户清空 → 自动命名再写回 → 无限来回。
    name_row: adw::EntryRow,
    /// 显示中的名称。
    name: String,
    /// 用户改过名称后，自动命名永不再应用；清空名称会重置为 `false`。
    name_user_modified: bool,
    /// 名称错误提示（格式不合法或目录已存在）。
    name_error: Option<String>,
    /// 安装根目录，用来检查 `versions/<名称>`。
    root_path: PathBuf,
    /// 名称之外的安装条件是否满足（由页面推）。
    ready: bool,
    /// 分类选择器。
    category_picker: Controller<CategoryPicker>,
    /// 当前分类（原始文本；写入时空值表示未分类）。
    category: String,
    /// 当前图标 / 自动图标（用户点了「自动」时恢复成它）。
    icon: InstanceIcon,
    /// 用户选过自定义图片后，自动图标永不再应用。
    icon_user_modified: bool,
    auto_icon: InstanceIcon,
    /// 当前图标装载出来的纹理。
    custom_image: Option<gdk::Texture>,
    install_enabled: bool,
}

impl InstanceSummary {
    fn apply_icon(&mut self, icon: InstanceIcon) {
        self.custom_image = match &icon {
            InstanceIcon::File(path) => gdk::Texture::from_filename(path).ok(),
            InstanceIcon::Asset(kind) => kind.texture(),
        };
        self.icon = icon;
    }

    /// 重新校验名称格式与目录冲突，更新提示和安装按钮可用性。
    fn refresh_name_state(&mut self) {
        let empty = self.name.trim().is_empty();
        let invalid = !empty && validate_version_name(&self.name).is_err();
        // 名称不合法时不去探测磁盘，避免 `../` 之类拼出 versions/ 之外的路径。
        let exists = !empty && !invalid && version_dir(&self.root_path, &self.name).exists();
        self.name_error = if invalid {
            Some("实例名称不合法".to_string())
        } else if exists {
            Some("实例已经存在".to_string())
        } else {
            None
        };
        self.install_enabled = self.ready && !empty && !invalid && !exists;
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for InstanceSummary {
    type Init = SummaryInit;
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
                    set_child = &gtk::Box {
                        set_size_request: (64, 64),
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,

                        adw::Clamp {
                            set_maximum_size: 64,
                            set_tightening_threshold: 64,

                            gtk::Picture {
                                set_content_fit: gtk::ContentFit::Contain,
                                set_can_shrink: true,
                                #[watch]
                                set_paintable: model.custom_image.as_ref(),
                            },
                        },
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
                            // 文件选择对话框要挂在应用的活动窗口上，逻辑在 update 里。
                            gtk::Button {
                                add_css_class: "flat",
                                set_label: "自定义…",
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

                    #[name(detail_rows)]
                    gtk::ListBox {
                        add_css_class: "boxed-list",
                        set_hexpand: true,
                        set_selection_mode: gtk::SelectionMode::None,

                        #[local_ref]
                        name_row_widget -> adw::EntryRow {},
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        set_halign: gtk::Align::End,
                        set_margin_top: 4,

                        gtk::Label {
                            add_css_class: "error",
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_label: model.name_error.as_deref().unwrap_or_default(),
                            #[watch]
                            set_visible: model.name_error.is_some(),
                        },

                        gtk::Button {
                            set_label: "安装",
                            add_css_class: "suggested-action",
                            #[watch]
                            set_sensitive: model.install_enabled,
                            connect_clicked[sender] => move |_| sender.input(SummaryIn::InstallClicked),
                        },
                    },
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let SummaryInit {
            categories,
            root_path,
        } = init;
        let name_row = adw::EntryRow::builder().title("实例名称").build();
        {
            let sender = sender.clone();
            name_row.connect_changed(move |row| {
                sender.input(SummaryIn::NameEdited(row.text().into()));
            });
        }

        let category_picker = CategoryPicker::builder()
            .launch(CategoryPickerInit {
                categories,
                category: String::new(),
            })
            .forward(sender.input_sender(), |out| match out {
                CategoryPickerOut::Selected(category) | CategoryPickerOut::Edited(category) => {
                    SummaryIn::CategoryChanged(category)
                }
            });

        let model = InstanceSummary {
            name_row,
            name: String::new(),
            name_user_modified: false,
            name_error: None,
            root_path,
            ready: false,
            category_picker,
            category: String::new(),
            icon: InstanceIcon::Asset(IconKind::Release),
            icon_user_modified: false,
            auto_icon: InstanceIcon::Asset(IconKind::Release),
            custom_image: IconKind::Release.texture(),
            install_enabled: false,
        };
        let name_row_widget = model.name_row.clone();
        let category_widget = model.category_picker.widget().clone();
        let widgets = view_output!();
        widgets.detail_rows.append(&category_widget);
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
                    self.refresh_name_state();
                    let _ = sender.output(SummaryOut::NameChanged(self.name.clone()));
                }
            }
            SummaryIn::SetAutoIcon(icon) => {
                self.auto_icon = icon;
                if !self.icon_user_modified {
                    self.apply_icon(self.auto_icon.clone());
                }
            }
            SummaryIn::SetReady(enabled) => {
                self.ready = enabled;
                self.refresh_name_state();
            }
            SummaryIn::PickCustomIcon(path) => {
                self.icon_user_modified = true;
                self.apply_icon(InstanceIcon::File(path));
            }
            SummaryIn::PickAutoIcon => {
                self.icon_user_modified = false;
                self.apply_icon(self.auto_icon.clone());
            }
            SummaryIn::EditIconFile => {
                choose_icon_file(move |path| sender.input(SummaryIn::PickCustomIcon(path)));
            }
            SummaryIn::NameEdited(text) => {
                // 用户输入只在「事件文本 == 输入框当前文本」时成立：程序 `set_text` 时
                // GTK 可能发一条中间态（先清空再写入）的 `changed`，不能当成用户清空。
                if text != self.name_row.text() || text == self.name {
                    return;
                }
                // 清空 = 放弃手动命名，交还给自动命名；页面收到空名后会重新推自动值。
                self.name_user_modified = !text.trim().is_empty();
                self.name = if self.name_user_modified {
                    text
                } else {
                    String::new()
                };
                self.refresh_name_state();
                let _ = sender.output(SummaryOut::NameChanged(self.name.clone()));
            }
            SummaryIn::CategoryChanged(category) => {
                self.category = category;
            }
            SummaryIn::InstallClicked => {
                // 只有用户选过自定义图片才写 `logo.png`；自动图标不落盘。
                let icon = match &self.icon {
                    InstanceIcon::File(path) if self.icon_user_modified => Some(path.clone()),
                    InstanceIcon::Asset(_) | InstanceIcon::File(_) => None,
                };
                let _ = sender.output(SummaryOut::Install {
                    name: self.name.clone(),
                    category: self.category.clone(),
                    icon,
                });
            }
        }
    }
}
