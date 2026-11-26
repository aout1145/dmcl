//! 实例设置页的「实例」分类：图标、名称、分类、加载器与删除。
//!
//! 页面直接改实例目录里的文件：图标是 `logo.png`，分类在 `dmcl.json`，加载器通过版本
//! JSON 的 `dmcl-patches` 重建。所有文件操作都走后台任务，页面只维护一份展示状态。

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::glib;
use relm4::prelude::*;

use dmcl_game::{detect_version, VersionRef};
use dmcl_install::{
    update_loaders_task, validate_version_name, LoaderKind, LoaderRequest, LoaderUpdate,
};
use dmcl_mcmeta::layout::version_dir;

use crate::config::{
    clear_instance_category, read_instance_category_sync, write_instance_category,
};
use crate::gui::gtk4::app::{config_store, ConfigRef, PageToast, Shell};
use crate::gui::gtk4::pages::download::common::version_select::{
    SelectIn, SelectOut, VersionItem, VersionSelect,
};
use crate::gui::gtk4::pages::download::custom::loaders::{loader_init, LoaderSlot, LOADER_DEFS};
use crate::gui::gtk4::widgets::category_picker::{
    CategoryPicker, CategoryPickerInit, CategoryPickerMsg, CategoryPickerOut,
};
use crate::gui::gtk4::widgets::instance_icon::{choose_icon_file, save_png, IconKind};

use super::files::{delete_instance, rename_instance};
use super::loaders::{read_loader_support, CurrentLoader, LoaderSupport};

/// 分类输入防抖：用户连续打字时只写最后一次。
const CATEGORY_FLUSH_DELAY: Duration = Duration::from_millis(500);

pub(crate) struct GeneralInit {
    pub shell: Rc<Shell>,
    pub page_toast: PageToast,
    pub root_path: PathBuf,
    pub name: String,
    pub categories: Vec<String>,
}

/// 「实例」分类发给设置页的输出。
#[derive(Debug)]
pub(crate) enum GeneralOut {
    /// 实例目录已经改名。
    Renamed { old: String, new: String },
    /// 实例目录已经删除。
    Deleted { name: String },
}

/// 页面正在进行的文件操作；同一时间只允许一个。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    Icon,
    Rename,
    Delete,
    Loaders,
}

#[derive(Debug)]
pub(crate) enum GeneralMsg {
    RenameRequested,
    Rename(String),
    RenameDone {
        old: String,
        new: String,
    },
    RenameFailed(String),
    PickAutoIcon,
    IconPicked(PathBuf),
    IconSaved,
    IconSaveFailed(String),
    AutoIconCleared(Result<(), String>),
    AutoIcon(IconKind),
    CategorySelected(String),
    CategoryEdited(String),
    FlushCategory,
    CategoryWriteFailed {
        attempted: String,
        detail: String,
    },
    DeleteRequested,
    DeleteConfirmed,
    Deleted,
    DeleteFailed(String),
    LoaderChanged {
        slot: usize,
        item: Option<VersionItem>,
    },
    ApplyLoaders,
    LoadersFinished(Result<(), String>),
}

pub(crate) struct GeneralPage {
    shell: Rc<Shell>,
    config: ConfigRef,
    page_toast: PageToast,
    root_path: PathBuf,
    name: String,
    category: String,
    category_picker: Controller<CategoryPicker>,
    /// 分类输入防抖的挂起定时器。
    category_flush: Option<glib::SourceId>,
    /// 分类写入串行化：用户快速改分类时后一次不能先落盘。
    category_write_lock: Arc<tokio::sync::Mutex<()>>,
    /// 实例图标当前显示的内容。
    texture: Option<gdk::Texture>,
    auto_icon: IconKind,
    /// 是否有自定义 `logo.png`；自动图标随版本 / 加载器变化。
    custom_logo: bool,
    loaders: Vec<LoaderSlot>,
    loader_supported: bool,
    loader_reason: Option<String>,
    mc_version: Option<String>,
    current_loaders: Vec<CurrentLoader>,
    busy: Option<Busy>,
}

impl GeneralPage {
    fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    /// 设置当前操作，并禁用会写文件或起任务的控件。
    fn set_busy(&mut self, busy: Option<Busy>) {
        self.busy = busy;
        let enabled = busy.is_none();
        self.category_picker.widget().set_sensitive(enabled);
    }

    fn loader_dirty(&self) -> bool {
        let selected: Vec<(LoaderKind, &str)> = self
            .loaders
            .iter()
            .filter_map(|slot| {
                slot.selection
                    .as_ref()
                    .map(|item| (slot.kind, item.id.as_str()))
            })
            .collect();
        if selected.len() != self.current_loaders.len() {
            return true;
        }
        selected.iter().any(|(kind, id)| {
            self.current_loaders
                .iter()
                .find(|loader| loader.kind == *kind)
                .is_none_or(|loader| loader.id != *id)
        })
    }

    fn can_apply_loaders(&self) -> bool {
        self.loader_supported && !self.is_busy() && self.loader_dirty()
    }

    fn loader_hint(&self) -> String {
        if self.busy == Some(Busy::Loaders) {
            "正在应用加载器…".to_string()
        } else if self.loader_dirty() {
            "应用后会重写版本 JSON；未变化的加载器复用已有文件。".to_string()
        } else {
            "加载器与当前实例一致。".to_string()
        }
    }

    fn refresh_auto_icon(&self, sender: &ComponentSender<Self>) {
        let root = self.root_path.clone();
        let name = self.name.clone();
        let sender = sender.clone();
        relm4::spawn(async move {
            let kind = match detect_version(&VersionRef::new(root, name)).await {
                Ok(detected) => IconKind::from_detected(&detected),
                Err(_) => IconKind::Broken,
            };
            let _ = sender.input_sender().send(GeneralMsg::AutoIcon(kind));
        });
    }

    fn cancel_category_flush(&mut self) {
        if let Some(source) = self.category_flush.take() {
            source.remove();
        }
    }

    fn schedule_category_flush(&mut self, sender: &ComponentSender<Self>) {
        self.cancel_category_flush();
        let sender = sender.clone();
        self.category_flush = Some(gtk::glib::timeout_add_local_once(
            CATEGORY_FLUSH_DELAY,
            move || {
                let _ = sender.input_sender().send(GeneralMsg::FlushCategory);
            },
        ));
    }

    fn persist_category(&mut self, sender: &ComponentSender<Self>) {
        self.cancel_category_flush();
        let category = self.category.clone();
        let root = self.root_path.clone();
        let name = self.name.clone();
        let sender = sender.clone();
        let lock = Arc::clone(&self.category_write_lock);
        relm4::spawn(async move {
            let _writing = lock.lock().await;
            let result = write_category(&root, &name, &category).await;
            if let Err(err) = result {
                let _ = sender.input_sender().send(GeneralMsg::CategoryWriteFailed {
                    attempted: category,
                    detail: format!("{err:#}"),
                });
            }
        });
    }

    fn apply_loader_support(&mut self, support: LoaderSupport) {
        match support {
            LoaderSupport::Supported {
                mc_version,
                loaders,
            } => {
                self.loader_supported = true;
                self.loader_reason = None;
                let context_changed = self.mc_version.as_deref() != Some(mc_version.as_str());
                self.mc_version = Some(mc_version.clone());
                self.current_loaders = loaders.clone();
                for slot in &mut self.loaders {
                    if context_changed {
                        let _ = slot
                            .select
                            .sender()
                            .send(SelectIn::SetContext(Some(mc_version.clone())));
                    }
                    let item = loaders
                        .iter()
                        .find(|loader| loader.kind == slot.kind)
                        .map(current_item);
                    slot.selection = item.clone();
                    let _ = slot
                        .select
                        .sender()
                        .send(SelectIn::SetInitialSelection(item));
                }
            }
            LoaderSupport::Unsupported(reason) => {
                self.loader_supported = false;
                self.loader_reason = Some(reason);
                self.mc_version = None;
                self.current_loaders.clear();
            }
        }
    }

    fn apply_loaders(&mut self, sender: &ComponentSender<Self>) {
        if !self.can_apply_loaders() {
            return;
        }
        let loaders: Vec<LoaderRequest> = self
            .loaders
            .iter()
            .filter_map(|slot| {
                slot.selection.as_ref().map(|item| LoaderRequest {
                    kind: slot.kind,
                    version: item.id.clone(),
                })
            })
            .collect();
        let options = {
            let config = self.config.read();
            config.install_options(config.download_source(), &self.root_path)
        };
        let options = match options {
            Ok(options) => Arc::new(options),
            Err(err) => {
                self.shell
                    .page_toast(&self.page_toast, &format!("无法调整加载器：{err:#}"));
                return;
            }
        };
        let update = LoaderUpdate {
            version_name: self.name.clone(),
            loaders,
        };
        let graph = match update_loaders_task(options, update) {
            Ok(graph) => graph,
            Err(err) => {
                self.shell
                    .page_toast(&self.page_toast, &format!("无法调整加载器：{err:#}"));
                return;
            }
        };
        self.set_busy(Some(Busy::Loaders));
        let handle = self
            .shell
            .run(format!("调整 {} 的加载器", self.name), graph);
        self.shell.open_tasks();
        let sender = sender.clone();
        relm4::spawn(async move {
            let result = handle.finished().await.map(|_| ());
            let _ = sender
                .input_sender()
                .send(GeneralMsg::LoadersFinished(result));
        });
    }

    fn show_rename_dialog(&self, sender: &ComponentSender<Self>) {
        if self.is_busy() {
            return;
        }
        let Some(window) = relm4::main_adw_application().active_window() else {
            return;
        };
        let entry = adw::EntryRow::builder().title("实例名称").build();
        entry.set_text(&self.name);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.append(&entry);
        let error = gtk::Label::new(None);
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        let child = gtk::Box::new(gtk::Orientation::Vertical, 6);
        child.append(&list);
        child.append(&error);
        let dialog = adw::AlertDialog::builder()
            .heading("重命名实例")
            .extra_child(&child)
            .build();
        dialog.add_response("cancel", "取消");
        dialog.add_response("rename", "重命名");
        dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        dialog.set_response_enabled("rename", false);
        let root = self.root_path.clone();
        let current = self.name.clone();
        entry.connect_changed({
            let dialog = dialog.clone();
            move |row| {
                let name = row.text();
                let problem = rename_error(&root, &current, &name);
                error.set_label(problem.as_deref().unwrap_or_default());
                error.set_visible(problem.is_some());
                dialog.set_response_enabled("rename", problem.is_none() && name != current);
            }
        });
        dialog.connect_response(None, {
            let entry = entry.clone();
            let sender = sender.clone();
            move |_, response| {
                if response == "rename" {
                    sender.input(GeneralMsg::Rename(entry.text().to_string()));
                }
            }
        });
        entry.connect_apply({
            let dialog = dialog.clone();
            let sender = sender.clone();
            move |row| {
                if dialog.is_response_enabled("rename") {
                    sender.input(GeneralMsg::Rename(row.text().to_string()));
                    dialog.close();
                }
            }
        });
        dialog.present(Some(&window));
    }

    fn show_delete_dialog(&self, sender: &ComponentSender<Self>) {
        if self.is_busy() {
            return;
        }
        let Some(window) = relm4::main_adw_application().active_window() else {
            return;
        };
        let dialog = adw::AlertDialog::builder()
            .heading("删除实例？")
            .body(format!(
                "将永久删除实例「{}」及其目录，且无法恢复。\n如果实例正在运行，删除可能导致游戏异常。",
                self.name
            ))
            .build();
        dialog.add_response("cancel", "取消");
        dialog.add_response("delete", "删除");
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let sender = sender.clone();
        dialog.connect_response(None, move |_, response| {
            if response == "delete" {
                sender.input(GeneralMsg::DeleteConfirmed);
            }
        });
        dialog.present(Some(&window));
    }
}

#[relm4::component(pub(crate))]
impl SimpleComponent for GeneralPage {
    type Init = GeneralInit;
    type Input = GeneralMsg;
    type Output = GeneralOut;

    view! {
        adw::PreferencesPage {
            adw::PreferencesGroup {
                set_title: "实例",
                set_description: Some("管理实例的图标、名称与分类。"),

                adw::ActionRow {
                    set_title: "名称",
                    set_activatable: false,
                    #[watch]
                    set_subtitle: &model.name,
                    add_suffix = &gtk::Button {
                        add_css_class: "flat",
                        set_valign: gtk::Align::Center,
                        set_tooltip_text: Some("重命名实例"),
                        set_icon_name: "document-edit-symbolic",
                        #[watch]
                        set_sensitive: !model.is_busy(),
                        connect_clicked => GeneralMsg::RenameRequested,
                    },
                },

                adw::ActionRow {
                    set_title: "图标",
                    set_subtitle: "自动按版本与加载器选择",

                    add_suffix = &gtk::MenuButton {
                        add_css_class: "flat",
                        set_valign: gtk::Align::Center,
                        set_tooltip_text: Some("更换实例图标"),
                        #[watch]
                        set_sensitive: !model.is_busy(),

                        #[wrap(Some)]
                        set_child = &gtk::Box {
                            set_size_request: (40, 40),
                            set_halign: gtk::Align::Center,
                            set_valign: gtk::Align::Center,

                            adw::Clamp {
                                set_maximum_size: 40,
                                set_tightening_threshold: 40,

                                gtk::Picture {
                                    set_content_fit: gtk::ContentFit::Contain,
                                    set_can_shrink: true,
                                    #[watch]
                                    set_paintable: model.texture.as_ref(),
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
                                    connect_clicked[sender] => move |_| sender.input(GeneralMsg::PickAutoIcon),
                                },
                                gtk::Button {
                                    add_css_class: "flat",
                                    set_label: "自定义…",
                                    connect_clicked[sender] => move |_| {
                                        let sender = sender.clone();
                                        choose_icon_file(move |path| {
                                            sender.input(GeneralMsg::IconPicked(path));
                                        });
                                    },
                                },
                            },
                        },
                    },
                },

                #[local_ref]
                category_widget -> adw::ExpanderRow {},
            },

            #[name(loader_group)]
            adw::PreferencesGroup {
                set_title: "加载器",
                set_description: Some("查看 Minecraft 版本，添加或更换加载器。"),
                #[watch]
                set_visible: model.loader_supported,
                #[watch]
                set_sensitive: !model.is_busy(),

                adw::ActionRow {
                    set_title: "Minecraft 版本",
                    set_activatable: false,
                    #[watch]
                    set_subtitle: model.mc_version.as_deref().unwrap_or(""),
                },

                #[name(apply_row)]
                adw::ActionRow {
                    set_title: "应用加载器更改",
                    #[watch]
                    set_subtitle: &model.loader_hint(),
                    add_suffix = &gtk::Button {
                        set_label: "应用",
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_sensitive: model.can_apply_loaders(),
                        connect_clicked => GeneralMsg::ApplyLoaders,
                    },
                },
            },

            adw::PreferencesGroup {
                set_title: "加载器",
                set_description: Some("查看 Minecraft 版本，添加或更换加载器。"),
                #[watch]
                set_visible: !model.loader_supported,

                adw::ActionRow {
                    set_title: "无法调整加载器",
                    set_activatable: false,
                    #[watch]
                    set_subtitle: model.loader_reason.as_deref().unwrap_or(""),
                },
            },

            adw::PreferencesGroup {
                set_title: "危险操作",
                set_description: Some("这些操作不可撤销，请谨慎操作。"),

                adw::ActionRow {
                    set_title: "删除实例",
                    set_subtitle: "删除实例目录及其所有文件，无法恢复。",
                    add_suffix = &gtk::Button {
                        set_label: "删除",
                        set_valign: gtk::Align::Center,
                        add_css_class: "destructive-action",
                        #[watch]
                        set_sensitive: !model.is_busy(),
                        connect_clicked => GeneralMsg::DeleteRequested,
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
        let GeneralInit {
            shell,
            page_toast,
            root_path,
            name,
            categories,
        } = init;
        let config = config_store::store().config();
        let category = read_instance_category_sync(&root_path, &name).unwrap_or_default();
        let support = read_loader_support(&root_path, &name);
        let supported = matches!(support, LoaderSupport::Supported { .. });

        let category_picker = CategoryPicker::builder()
            .launch(CategoryPickerInit {
                categories,
                category: category.clone(),
            })
            .forward(sender.input_sender(), |out| match out {
                CategoryPickerOut::Selected(category) => GeneralMsg::CategorySelected(category),
                CategoryPickerOut::Edited(category) => GeneralMsg::CategoryEdited(category),
            });

        let logo = version_dir(&root_path, &name).join("logo.png");
        let custom_logo = logo.is_file();
        let texture = if custom_logo {
            gdk::Texture::from_filename(&logo).ok()
        } else {
            IconKind::Release.texture()
        };

        let source = config.read().download_source();
        let mut loaders = Vec::new();
        if supported {
            for def in LOADER_DEFS {
                let select = VersionSelect::builder()
                    .launch(loader_init(def.kind, source.clone()))
                    .forward(sender.input_sender(), move |out| {
                        GeneralMsg::LoaderChanged {
                            slot: loaders_index(def.kind),
                            item: take_selection(out),
                        }
                    });
                loaders.push(LoaderSlot {
                    kind: def.kind,
                    mutex_groups: def.mutex_groups,
                    select,
                    selection: None,
                });
            }
        }

        let model = GeneralPage {
            shell,
            config,
            page_toast,
            root_path,
            name,
            category,
            category_picker,
            category_flush: None,
            category_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            texture,
            auto_icon: IconKind::Release,
            custom_logo,
            loaders,
            loader_supported: false,
            loader_reason: None,
            mc_version: None,
            current_loaders: Vec::new(),
            busy: None,
        };

        let category_widget = model.category_picker.widget().clone();
        let widgets = view_output!();
        for slot in &model.loaders {
            widgets.loader_group.add(slot.select.widget());
        }
        // 加载器行在运行时才加入分组；把「应用加载器更改」重新挂到末尾。
        widgets.loader_group.remove(&widgets.apply_row);
        widgets.loader_group.add(&widgets.apply_row);

        let mut model = model;
        model.apply_loader_support(support);
        model.refresh_auto_icon(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            GeneralMsg::RenameRequested => self.show_rename_dialog(&sender),
            GeneralMsg::Rename(new_name) => {
                if self.is_busy() || new_name == self.name {
                    return;
                }
                if validate_version_name(&new_name).is_err() {
                    self.shell.page_toast(&self.page_toast, "实例名称不合法");
                    return;
                }
                if version_dir(&self.root_path, &new_name).exists() {
                    self.shell.page_toast(&self.page_toast, "实例已经存在");
                    return;
                }
                // 分类可能还在防抖窗口里；和改名共用一把锁，先在旧名字下写完分类。
                self.cancel_category_flush();
                let old = self.name.clone();
                let root = self.root_path.clone();
                let category = self.category.clone();
                let lock = Arc::clone(&self.category_write_lock);
                self.set_busy(Some(Busy::Rename));
                let sender = sender.clone();
                relm4::spawn(async move {
                    let _writing = lock.lock().await;
                    if let Err(err) = write_category(&root, &old, &category).await {
                        let _ = sender.input_sender().send(GeneralMsg::CategoryWriteFailed {
                            attempted: category,
                            detail: format!("{err:#}"),
                        });
                    }
                    let result = rename_instance(&root, &old, &new_name).await;
                    let message = match result {
                        Ok(()) => GeneralMsg::RenameDone { old, new: new_name },
                        Err(err) => GeneralMsg::RenameFailed(format!("{err:#}")),
                    };
                    let _ = sender.input_sender().send(message);
                });
            }
            GeneralMsg::RenameDone { old, new } => {
                self.set_busy(None);
                self.name = new.clone();
                self.refresh_auto_icon(&sender);
                self.shell.page_toast(&self.page_toast, "已重命名实例");
                let _ = sender.output(GeneralOut::Renamed { old, new });
            }
            GeneralMsg::RenameFailed(err) => {
                self.set_busy(None);
                self.shell
                    .page_toast(&self.page_toast, &format!("重命名实例失败：{err}"));
            }
            GeneralMsg::PickAutoIcon => {
                if self.is_busy() || !self.custom_logo {
                    return;
                }
                self.custom_logo = false;
                self.texture = self.auto_icon.texture();
                let logo = version_dir(&self.root_path, &self.name).join("logo.png");
                self.set_busy(Some(Busy::Icon));
                let sender = sender.clone();
                relm4::spawn(async move {
                    let result = match tokio::fs::remove_file(&logo).await {
                        Ok(()) => Ok(()),
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(err) => Err(format!("无法删除自定义图标：{err}")),
                    };
                    let _ = sender
                        .input_sender()
                        .send(GeneralMsg::AutoIconCleared(result));
                });
            }
            GeneralMsg::IconPicked(path) => {
                if self.is_busy() {
                    return;
                }
                let Some(texture) = gdk::Texture::from_filename(&path).ok() else {
                    self.shell.page_toast(&self.page_toast, "无法读取图标图片");
                    return;
                };
                self.texture = Some(texture);
                let destination = version_dir(&self.root_path, &self.name).join("logo.png");
                self.set_busy(Some(Busy::Icon));
                let sender = sender.clone();
                relm4::spawn_local(async move {
                    let result = save_png(&path, &destination);
                    let message = match result {
                        Ok(()) => GeneralMsg::IconSaved,
                        Err(err) => GeneralMsg::IconSaveFailed(err),
                    };
                    let _ = sender.input_sender().send(message);
                });
            }
            GeneralMsg::IconSaved => {
                self.set_busy(None);
                self.custom_logo = true;
            }
            GeneralMsg::IconSaveFailed(err) => {
                self.set_busy(None);
                let logo = version_dir(&self.root_path, &self.name).join("logo.png");
                self.custom_logo = logo.is_file();
                self.texture = if self.custom_logo {
                    gdk::Texture::from_filename(&logo)
                        .ok()
                        .or_else(|| self.auto_icon.texture())
                } else {
                    self.auto_icon.texture()
                };
                self.shell
                    .page_toast(&self.page_toast, &format!("写入实例图标失败：{err}"));
            }
            GeneralMsg::AutoIconCleared(result) => {
                self.set_busy(None);
                match result {
                    Ok(()) => {
                        self.custom_logo = false;
                        self.texture = self.auto_icon.texture();
                    }
                    Err(err) => {
                        self.custom_logo = true;
                        let logo = version_dir(&self.root_path, &self.name).join("logo.png");
                        self.texture = gdk::Texture::from_filename(&logo)
                            .ok()
                            .or_else(|| self.auto_icon.texture());
                        self.shell.page_toast(&self.page_toast, &err);
                    }
                }
            }
            GeneralMsg::AutoIcon(kind) => {
                self.auto_icon = kind;
                if !self.custom_logo {
                    self.texture = kind.texture();
                }
            }
            GeneralMsg::CategorySelected(category) => {
                if self.is_busy() {
                    return;
                }
                self.category = category;
                self.persist_category(&sender);
            }
            GeneralMsg::CategoryEdited(category) => {
                if self.is_busy() {
                    return;
                }
                self.category = category;
                self.schedule_category_flush(&sender);
            }
            GeneralMsg::FlushCategory => {
                if self.is_busy() {
                    return;
                }
                self.persist_category(&sender);
            }
            GeneralMsg::CategoryWriteFailed { attempted, detail } => {
                if self.category != attempted {
                    return;
                }
                let actual =
                    read_instance_category_sync(&self.root_path, &self.name).unwrap_or_default();
                self.category = actual.clone();
                let _ = self
                    .category_picker
                    .sender()
                    .send(CategoryPickerMsg::SetCategory(actual));
                self.shell
                    .page_toast(&self.page_toast, &format!("保存实例分类失败：{detail}"));
            }
            GeneralMsg::DeleteRequested => self.show_delete_dialog(&sender),
            GeneralMsg::DeleteConfirmed => {
                if self.is_busy() {
                    return;
                }
                self.set_busy(Some(Busy::Delete));
                let root = self.root_path.clone();
                let name = self.name.clone();
                let sender = sender.clone();
                relm4::spawn(async move {
                    let message = match delete_instance(&root, &name).await {
                        Ok(()) => GeneralMsg::Deleted,
                        Err(err) => GeneralMsg::DeleteFailed(format!("{err:#}")),
                    };
                    let _ = sender.input_sender().send(message);
                });
            }
            GeneralMsg::Deleted => {
                self.set_busy(None);
                self.shell.page_toast(&self.page_toast, "已删除实例");
                let _ = sender.output(GeneralOut::Deleted {
                    name: self.name.clone(),
                });
            }
            GeneralMsg::DeleteFailed(err) => {
                self.set_busy(None);
                self.shell
                    .page_toast(&self.page_toast, &format!("删除实例失败：{err}"));
            }
            GeneralMsg::LoaderChanged { slot, item } => {
                let groups = self.loaders[slot].mutex_groups;
                if item.is_some() {
                    for (index, other) in self.loaders.iter_mut().enumerate() {
                        if index != slot
                            && other.selection.is_some()
                            && other
                                .mutex_groups
                                .iter()
                                .any(|group| groups.contains(group))
                        {
                            other.selection = None;
                            let _ = other.select.sender().send(SelectIn::Deselect);
                        }
                    }
                }
                self.loaders[slot].selection = item;
            }
            GeneralMsg::ApplyLoaders => self.apply_loaders(&sender),
            GeneralMsg::LoadersFinished(Ok(())) => {
                self.set_busy(None);
                let support = read_loader_support(&self.root_path, &self.name);
                self.apply_loader_support(support);
                self.refresh_auto_icon(&sender);
                self.shell.page_toast(&self.page_toast, "已更新加载器");
            }
            GeneralMsg::LoadersFinished(Err(err)) => {
                self.set_busy(None);
                self.shell
                    .page_toast(&self.page_toast, &format!("调整加载器失败：{err}"));
            }
        }
    }
}

fn current_item(loader: &CurrentLoader) -> VersionItem {
    VersionItem {
        id: loader.id.clone(),
        label: loader.label.clone(),
        kind: String::new(),
        badges: Vec::new(),
        detail: None,
    }
}

/// 把分类写到指定实例：空分类删掉键。
async fn write_category(root: &Path, name: &str, category: &str) -> anyhow::Result<()> {
    if category.trim().is_empty() {
        clear_instance_category(root, name).await
    } else {
        write_instance_category(root, name, category).await
    }
}

/// 重命名对话框的实时校验：返回要显示的提示，`None` 表示名称可以提交。
fn rename_error(root: &Path, current: &str, name: &str) -> Option<String> {
    if name == current {
        return None;
    }
    if let Err(err) = validate_version_name(name) {
        return Some(err.to_string());
    }
    version_dir(root, name)
        .exists()
        .then(|| "实例已经存在。".to_string())
}

fn take_selection(out: SelectOut) -> Option<VersionItem> {
    match out {
        SelectOut::Changed(item) => item,
    }
}

/// 加载器在 [`LOADER_DEFS`] 里的下标。
fn loaders_index(kind: LoaderKind) -> usize {
    LOADER_DEFS
        .iter()
        .position(|def| def.kind == kind)
        .expect("创建的加载器一定来自 LOADER_DEFS")
}
