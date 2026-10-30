//! 实例概要（download 页面公共组件）：图标 + 名称 + 分类 + 安装按钮。
//!
//! 名称与图标都有「当前值 + 用户是否改过」两个状态：外部推自动值
//! （[`SummaryIn::SetAutoName`] / [`SummaryIn::SetAutoIcon`]）只在用户没改过时生效，
//! 用户一改过就永不覆盖——CLI/GUI 共用的自动命名语义。名称被清空时视为「从未编辑」，
//! 自动命名可以重新接管。
//!
//! 自动命名规则（`dmcl_install::default_version_name`）由页面计算后推送，
//! 组件自身不知道版本与加载器怎么命名。名称冲突提示由页面检查后通过
//! [`SummaryIn::SetNameError`] 推进来；有冲突时在安装按钮左侧显示红色提示，输入框不变色。

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
    /// 名称冲突提示（如名称已存在）；`Some` 时在安装按钮左侧显示该文案。
    SetNameError(Option<String>),
    /// 用户在图标菜单里选了某项。
    PickIcon(IconSource),
    /// 用户在图标菜单里选了「自动」。
    PickAutoIcon,
    /// 用户点了「选择图片…」，发起文件选择。
    EditIconFile,
    /// 名称输入框的文本变化。
    NameEdited(String),
    /// 分类输入框的文本变化。
    CategoryEdited(String),
    /// 用户从已有分类里选了一项。
    SelectCategory(String),
    /// 取消当前分类，回到「未分类」。
    ClearCategory,
    /// 用户点了安装按钮。
    InstallClicked,
}

/// 组件输出。
#[derive(Debug)]
pub(crate) enum SummaryOut {
    /// 点了安装，带当前实例名与分类。
    Install { name: String, category: String },
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
    /// 用户改过名称后，自动命名永不再应用；清空名称会重置为 `false`。
    name_user_modified: bool,
    /// 名称错误提示（由页面检查）。
    name_error: Option<String>,
    /// 分类选择器：收起时显示当前分类，展开后是已有分类 + 自定义输入。
    category_expander: adw::ExpanderRow,
    /// 展开区最下面的自定义分类输入框。
    category_entry: adw::EntryRow,
    /// 分类非空时出现的「取消选择」按钮。
    category_clear: gtk::Button,
    /// 当前分类（原始文本；写入时空值表示未分类）。
    category: String,
    /// 已有分类选项。
    category_choices: FactoryVecDeque<CategoryChoice>,
    /// 当前图标 / 自动图标（用户点了「自动」时恢复成它）。
    icon: IconSource,
    /// 用户改过图标后，自动图标永不再应用。
    icon_user_modified: bool,
    auto_icon: IconSource,
    /// `File` 图标装载出来的纹理；装载失败回落为首字母头像。
    custom_image: Option<gdk::Texture>,
    install_enabled: bool,
}

const UNCATEGORIZED: &str = "未分类";

/// 分类选择器展开后的一项。
struct CategoryChoice {
    category: String,
    selected: bool,
}

#[relm4::factory]
impl FactoryComponent for CategoryChoice {
    type Init = (String, bool);
    type Input = bool;
    type Output = String;
    type CommandOutput = ();
    type ParentWidget = adw::ExpanderRow;
    type Index = DynamicIndex;

    view! {
        adw::ActionRow {
            add_css_class: "dmcl-category-choice",
            #[watch]
            set_title: &self.category,
            set_activatable: true,

            add_suffix = &gtk::Image {
                set_icon_name: Some("object-select-symbolic"),
                #[watch]
                set_visible: self.selected,
                add_css_class: "accent",
                set_valign: gtk::Align::Center,
            },

            connect_activated[sender] => move |_| {
                let _ = sender.output(category.clone());
            },
        }
    }

    fn init_model(
        (category, selected): Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        Self { category, selected }
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        _root: Self::Root,
        _returned_widget: &gtk::Widget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let category = self.category.clone();
        let widgets = view_output!();
        widgets
    }

    fn update(&mut self, selected: Self::Input, _sender: FactorySender<Self>) {
        self.selected = selected;
    }
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

    /// 分类文字变化后同步收起状态的副标题、取消按钮和对勾。
    fn sync_category_view(&mut self) {
        let selected = self.category.trim();
        self.category_expander.set_subtitle(if selected.is_empty() {
            UNCATEGORIZED
        } else {
            selected
        });
        self.category_clear.set_visible(!selected.is_empty());

        let mut guard = self.category_choices.guard();
        for choice in guard.iter_mut() {
            choice.selected = choice.category.as_str() == selected;
        }
    }

    /// 选中已有分类，写回输入框并收起展开区。
    fn select_category(&mut self, category: String) {
        if self.category_entry.text() != category {
            self.category_entry.set_text(&category);
        }
        self.category = category;
        self.sync_category_view();
        self.category_expander.set_expanded(false);
    }

    /// 取消当前分类，回到「未分类」。
    fn clear_category(&mut self) {
        if !self.category_entry.text().is_empty() {
            self.category_entry.set_text("");
        }
        self.category = String::new();
        self.sync_category_view();
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for InstanceSummary {
    type Init = Vec<String>;
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
                        set_selection_mode: gtk::SelectionMode::None,

                        #[local_ref]
                        name_row_widget -> adw::EntryRow {},

                        #[local_ref]
                        category_expander_widget -> adw::ExpanderRow {},
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
        categories: Self::Init,
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

        let category_expander = adw::ExpanderRow::builder()
            .title("分类")
            .subtitle(UNCATEGORIZED)
            .show_enable_switch(false)
            .build();
        let category_clear = gtk::Button::from_icon_name("window-close-symbolic");
        category_clear.add_css_class("flat");
        category_clear.set_valign(gtk::Align::Center);
        category_clear.set_tooltip_text(Some("取消选择（未分类）"));
        category_clear.set_visible(false);
        category_clear.connect_clicked({
            let sender = sender.clone();
            move |_| sender.input(SummaryIn::ClearCategory)
        });
        category_expander.add_suffix(&category_clear);

        let category_entry = adw::EntryRow::builder().title("自定义分类").build();
        category_entry.set_tooltip_text(Some("留空表示未分类"));
        {
            let sender = sender.clone();
            category_entry.connect_changed(move |row| {
                sender.input(SummaryIn::CategoryEdited(row.text().into()));
            });
        }

        let mut category_choices = FactoryVecDeque::builder()
            .launch(category_expander.clone())
            .forward(sender.input_sender(), SummaryIn::SelectCategory);
        {
            let mut guard = category_choices.guard();
            for category in categories {
                guard.push_back((category, false));
            }
        }
        // 自定义输入放最下面：已有分类先排，最后才是自由输入。
        category_expander.add_row(&category_entry);

        let model = InstanceSummary {
            name_row,
            name: String::new(),
            name_user_modified: false,
            name_error: None,
            category_expander,
            category_entry,
            category_clear,
            category: String::new(),
            category_choices,
            icon: IconSource::Initials,
            icon_user_modified: false,
            auto_icon: IconSource::Initials,
            custom_image: None,
            install_enabled: false,
        };
        let name_row_widget = model.name_row.clone();
        let category_expander_widget = model.category_expander.clone();
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
            SummaryIn::SetNameError(error) => self.name_error = error,
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
                // 清空 = 放弃手动命名，交还给自动命名；页面收到空名后会重新推自动值。
                self.name_user_modified = !text.trim().is_empty();
                self.name = if self.name_user_modified {
                    text
                } else {
                    String::new()
                };
                let _ = sender.output(SummaryOut::NameChanged(self.name.clone()));
            }
            SummaryIn::CategoryEdited(text) => {
                // 和名称一样，忽略程序 `set_text` 可能产生的中间态；只接受输入框当前文本。
                if text == self.category_entry.text() {
                    self.category = text;
                    self.sync_category_view();
                }
            }
            SummaryIn::SelectCategory(category) => self.select_category(category),
            SummaryIn::ClearCategory => self.clear_category(),
            SummaryIn::InstallClicked => {
                let _ = sender.output(SummaryOut::Install {
                    name: self.name.clone(),
                    category: self.category.clone(),
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
            // 文件对话框可能比页面活得久：页面关了就当用户放弃选择。
            let _ = sender
                .input_sender()
                .send(SummaryIn::PickIcon(IconSource::File(path)));
        }
    });
}
