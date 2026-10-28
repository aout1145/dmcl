//! 首选项里的「Minecraft 目录」分类：列表、对话框与配置写入。
//!
//! 名称和路径只写配置：主页按名字常驻，重启 DMCL 后才会按新配置重建。

use std::path::{Path, PathBuf};
use std::rc::Rc;

use dmcl_mcmeta::layout::absolute;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gio;
use relm4::prelude::*;

use crate::config::Config;
use crate::gui::gtk4::app::{config_store, ConfigRef, PageToast, Shell};
use crate::gui::gtk4::pages::settings::common::drag_list;

/// 「路径格式」下拉里「相对路径」的下标；0 是「绝对路径」。
const RELATIVE_INDEX: u32 = 1;

/// 「路径格式」下拉里「绝对路径」的下标。
const ABSOLUTE_INDEX: u32 = 0;

/// 「Minecraft 目录」分类收到的消息。
#[derive(Debug)]
pub(super) enum DirectoryMsg {
    /// 共享配置变了：重新读取根目录。
    ConfigChanged,
    /// 打开添加目录的对话框。
    Add,
    /// 打开某个目录的编辑对话框。
    Edit(String),
    /// 请求删除某个目录（先弹确认框）。
    Remove(String),
    /// 确认框里点了删除。
    RemoveConfirmed(String),
    /// 把某个目录设为默认。
    SetDefault(String),
    /// 拖动排序。
    MoveRoot(drag_list::DragMove),
    /// 添加 / 编辑对话框确认后的结果；`original` 为空表示新增。
    SaveRoot {
        original: Option<String>,
        name: String,
        path: PathBuf,
    },
}

pub(super) struct DirectoryPage {
    config: ConfigRef,
    shell: Rc<Shell>,
    /// 页面级提示：只有首选项可见时才显示。
    page_toast: PageToast,
    /// 每个根目录一行；由配置顺序决定先后。
    roots: FactoryVecDeque<RootRow>,
    /// 固定在列表末尾的添加按钮；重建行后要重新移到最后。
    add_row: adw::ButtonRow,
}

#[relm4::component(pub(super))]
impl SimpleComponent for DirectoryPage {
    type Init = Rc<Shell>;
    type Input = DirectoryMsg;
    type Output = ();

    view! {
        adw::PreferencesPage {
            #[local_ref]
            roots_group -> adw::PreferencesGroup {},
        }
    }

    fn init(
        shell: Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = config_store::store().config();
        let group = adw::PreferencesGroup::builder()
            .title("Minecraft 目录")
            .description("管理启动器使用的 Minecraft 目录。部分修改将在下次打开启动器时生效。")
            .build();
        let roots = FactoryVecDeque::builder().launch(group.clone()).forward(
            sender.input_sender(),
            |out| match out {
                RootRowOut::Edit(name) => DirectoryMsg::Edit(name),
                RootRowOut::Remove(name) => DirectoryMsg::Remove(name),
                RootRowOut::SetDefault(name) => DirectoryMsg::SetDefault(name),
                RootRowOut::Move(mv) => DirectoryMsg::MoveRoot(mv),
            },
        );

        let add_row = adw::ButtonRow::builder()
            .title("添加目录")
            .start_icon_name("list-add-symbolic")
            .build();
        add_row.connect_activated({
            let sender = sender.clone();
            move |_| sender.input(DirectoryMsg::Add)
        });
        group.add(&add_row);

        let mut model = DirectoryPage {
            config,
            shell,
            page_toast: PageToast::new(&root_widget),
            roots,
            add_row,
        };
        let roots_group = model.roots.widget();
        let widgets = view_output!();
        model.reload();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            DirectoryMsg::ConfigChanged => self.reload(),
            DirectoryMsg::Add => show_add(&self.root_entries(), &sender),
            DirectoryMsg::Edit(name) => {
                let Some(path) = self.path_of(&name) else {
                    return;
                };
                show_edit(&name, path, &self.root_entries(), &sender);
            }
            DirectoryMsg::Remove(name) => confirm_remove(&name, &sender),
            DirectoryMsg::RemoveConfirmed(name) => {
                self.update_config(|config| config.remove_minecraft_root(&name));
                self.reload();
            }
            DirectoryMsg::SetDefault(name) => {
                {
                    let config = self.config.read();
                    if config.minecraft_root == name {
                        return;
                    }
                }
                self.update_config(|config| config.select_minecraft_root(&name));
                self.reload();
            }
            DirectoryMsg::MoveRoot(mv) => self.move_root(mv),
            DirectoryMsg::SaveRoot {
                original,
                name,
                path,
            } => {
                self.update_config(move |config| match &original {
                    Some(old) => {
                        config.rename_minecraft_root(old, &name)?;
                        config.set_minecraft_root_path(&name, path)
                    }
                    None => config.add_minecraft_root(&name, path),
                });
                self.reload();
            }
        }
    }
}

impl DirectoryPage {
    /// 按共享配置重建行；配置顺序就是行的顺序，添加按钮始终留在最后。
    fn reload(&mut self) {
        let config = self.config.read();
        let default = config.minecraft_root.clone();
        let roots = config.roots();
        drop(config);

        {
            let mut guard = self.roots.guard();
            guard.clear();
            for (name, path) in roots {
                let is_default = name == default;
                guard.push_back((name, path, is_default));
            }
        }

        self.roots.widget().remove(&self.add_row);
        self.roots.widget().add(&self.add_row);
    }

    /// 按拖动落点把源目录放到目标目录前或后。
    fn move_root(&mut self, mv: drag_list::DragMove) {
        let drag_list::DragMove {
            from,
            target,
            after,
        } = mv;
        if from == target {
            return;
        }

        let (from_index, target_index) = {
            let config = self.config.read();
            let names: Vec<String> = config.roots().into_iter().map(|(name, _)| name).collect();
            let Some(from_index) = names.iter().position(|name| name == &from) else {
                return;
            };
            let Some(target_index) = names.iter().position(|name| name == &target) else {
                return;
            };
            (from_index, target_index)
        };

        let to = drag_list::moved_index(from_index, target_index, after);
        if to == from_index {
            return;
        }

        self.update_config(|config| config.move_minecraft_root(&from, to));
        self.reload();
    }

    /// 当前根目录（名字 + 已展开的路径），供对话框校验重名 / 重复路径。
    fn root_entries(&self) -> Vec<(String, PathBuf)> {
        self.config.read().roots()
    }

    /// 配置里保存的原始路径，供编辑对话框原样显示。
    fn path_of(&self, name: &str) -> Option<PathBuf> {
        self.config.read().minecraft_roots.get(name).cloned()
    }

    /// 改配置并异步落盘；失败时在页面上提示。
    fn update_config(&self, f: impl FnOnce(&mut Config) -> anyhow::Result<()>) {
        let mut error = None;
        let shell = self.shell.clone();
        let page_toast = self.page_toast.clone();
        config_store::store().update(
            |config| {
                if let Err(err) = f(config) {
                    error = Some(err);
                }
            },
            move |err| shell.page_toast(&page_toast, &format!("保存配置失败：{err:#}")),
        );
        if let Some(err) = error {
            self.shell
                .page_toast(&self.page_toast, &format!("无法修改目录：{err:#}"));
        }
    }
}

/// 添加新目录。
fn show_add(roots: &[(String, PathBuf)], sender: &ComponentSender<DirectoryPage>) {
    show(RootDialogMode::Add, PathBuf::new(), roots, sender);
}

/// 编辑已有目录的名称和路径。
fn show_edit(
    name: &str,
    path: PathBuf,
    roots: &[(String, PathBuf)],
    sender: &ComponentSender<DirectoryPage>,
) {
    show(RootDialogMode::Edit(name.to_string()), path, roots, sender);
}

/// 删除前的确认框。
fn confirm_remove(name: &str, sender: &ComponentSender<DirectoryPage>) {
    let title = format!("删除「{name}」？");
    let dialog = adw::AlertDialog::new(Some(&title), Some("删除将在下次打开启动器时完全生效。"));
    dialog.add_response("cancel", "取消");
    dialog.add_response("delete", "删除");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let sender = sender.clone();
    let name = name.to_string();
    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            sender.input(DirectoryMsg::RemoveConfirmed(name.clone()));
        }
    });

    let parent = relm4::main_adw_application().active_window();
    dialog.present(parent.as_ref());
}

#[derive(Clone)]
enum RootDialogMode {
    Add,
    Edit(String),
}

impl RootDialogMode {
    fn original(&self) -> Option<String> {
        match self {
            Self::Add => None,
            Self::Edit(name) => Some(name.clone()),
        }
    }
}

fn show(
    mode: RootDialogMode,
    initial_path: PathBuf,
    roots: &[(String, PathBuf)],
    sender: &ComponentSender<DirectoryPage>,
) {
    let title = match &mode {
        RootDialogMode::Add => "添加目录",
        RootDialogMode::Edit(_) => "编辑目录",
    };
    let confirm_label = match &mode {
        RootDialogMode::Add => "添加",
        RootDialogMode::Edit(_) => "保存",
    };
    let initial_name = mode.original().unwrap_or_default();
    let initial_relative = match &mode {
        RootDialogMode::Add => false,
        RootDialogMode::Edit(_) => initial_path.is_relative(),
    };
    let initial_path = initial_path.display().to_string();

    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(460);
    dialog.set_content_height(320);

    relm4::view! {
        toolbar = adw::ToolbarView {
            add_top_bar = &adw::HeaderBar {
                set_show_start_title_buttons: false,
                set_show_end_title_buttons: false,
                set_title_widget: Some(&adw::WindowTitle::new(title, "")),

                pack_start = &gtk::Button::with_label("取消") {
                    connect_clicked[dialog] => move |_| {
                        dialog.close();
                    },
                },

                pack_end: save_button = &gtk::Button::with_label(confirm_label) {
                    add_css_class: "suggested-action",
                },
            },

            #[wrap(Some)]
            set_content = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,

                append = &adw::PreferencesPage {
                    set_vexpand: true,

                    add = &adw::PreferencesGroup {
                        add: name_row = &adw::EntryRow {
                            set_title: "目录名称",
                            set_text: &initial_name,
                        },

                        add: path_row = &adw::EntryRow {
                            set_title: "目录路径",
                            set_text: &initial_path,
                        },

                        add: style_row = &adw::ComboRow {
                            set_title: "路径格式",
                            set_model: Some(&gtk::StringList::new(&["绝对路径", "相对路径"])),
                            set_selected: u32::from(initial_relative),
                        },
                    },
                },

                append: notice_label = &gtk::Label {
                    set_visible: false,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_start: 18,
                    set_margin_end: 18,
                    set_margin_bottom: 18,
                },
            },
        }
    }

    dialog.set_child(Some(&toolbar));

    // 名称、路径或格式一变就重新判断确认按钮是否可用；不合法时按钮置灰。
    let original = mode.original();
    let roots = Rc::new(roots.to_vec());
    let refresh = {
        let name_row = name_row.clone();
        let path_row = path_row.clone();
        let style_row = style_row.clone();
        let save_button = save_button.clone();
        let original = original.clone();
        let roots = Rc::clone(&roots);
        Rc::new(move || {
            save_button.set_sensitive(
                validate(
                    original.as_deref(),
                    &name_row.text(),
                    &path_row.text(),
                    style_row.selected() == RELATIVE_INDEX,
                    &roots,
                )
                .is_ok(),
            );
        })
    };
    refresh();
    // 输入时就校验并显示原因；字段还空着时不提示，免得刚打开对话框就报错。
    let validate_live: Rc<dyn Fn()> = {
        let name_row = name_row.clone();
        let path_row = path_row.clone();
        let style_row = style_row.clone();
        let notice_label = notice_label.clone();
        let original = original.clone();
        let roots = Rc::clone(&roots);
        Rc::new(move || {
            if name_row.text().trim().is_empty() || path_row.text().trim().is_empty() {
                notice_label.set_visible(false);
                return;
            }
            match validate(
                original.as_deref(),
                &name_row.text(),
                &path_row.text(),
                style_row.selected() == RELATIVE_INDEX,
                &roots,
            ) {
                Ok(_) => notice_label.set_visible(false),
                Err(message) => show_notice(&notice_label, &message, true),
            }
        })
    };
    name_row.connect_changed({
        let refresh = Rc::clone(&refresh);
        let validate_live = Rc::clone(&validate_live);
        move |_| {
            refresh();
            validate_live();
        }
    });
    path_row.connect_changed({
        let refresh = Rc::clone(&refresh);
        let validate_live = Rc::clone(&validate_live);
        move |_| {
            refresh();
            validate_live();
        }
    });

    // 路径行右侧的文件夹按钮：用系统文件夹选择器填路径，也可以直接手输。
    let browse = gtk::Button::from_icon_name("folder-open-symbolic");
    browse.add_css_class("flat");
    browse.set_valign(gtk::Align::Center);
    browse.set_tooltip_text(Some("选择目录"));
    path_row.add_suffix(&browse);
    browse.connect_clicked({
        let path_row = path_row.clone();
        let style_row = style_row.clone();
        let notice_label = notice_label.clone();
        let parent = relm4::main_adw_application().active_window();
        move |_| pick_folder(parent.as_ref(), &path_row, &style_row, &notice_label)
    });

    // 路径输入完成（回车或离开该行）就按所选格式改写；切换格式也立即改写。
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let path_row = path_row.clone();
        let style_row = style_row.clone();
        let notice_label = notice_label.clone();
        let refresh = Rc::clone(&refresh);
        move |_| {
            normalize_entry(&path_row, &style_row, &notice_label);
            refresh();
        }
    });
    path_row.add_controller(focus);
    path_row.connect_apply({
        let path_row = path_row.clone();
        let style_row = style_row.clone();
        let notice_label = notice_label.clone();
        let save_button = save_button.clone();
        let refresh = Rc::clone(&refresh);
        move |_| {
            // 先改写再保存；改写失败时停在这里，错误已经显示在下方。
            normalize_entry(&path_row, &style_row, &notice_label);
            refresh();
            save_button.emit_clicked();
        }
    });
    style_row.connect_selected_notify({
        let path_row = path_row.clone();
        let style_row = style_row.clone();
        let notice_label = notice_label.clone();
        let refresh = Rc::clone(&refresh);
        move |_| {
            normalize_entry(&path_row, &style_row, &notice_label);
            refresh();
        }
    });

    save_button.connect_clicked({
        let dialog = dialog.clone();
        let notice_label = notice_label.clone();
        let name_row = name_row.clone();
        let path_row = path_row.clone();
        let style_row = style_row.clone();
        let sender = sender.clone();
        let roots = Rc::clone(&roots);
        let original = original.clone();
        move |_| {
            // 先按所选格式改写输入框；写不成相对路径时这里会切回绝对路径并提示。
            normalize_entry(&path_row, &style_row, &notice_label);
            let result = validate(
                original.as_deref(),
                &name_row.text(),
                &path_row.text(),
                style_row.selected() == RELATIVE_INDEX,
                &roots,
            );
            match result {
                Ok((name, path)) => {
                    dialog.close();
                    sender.input(DirectoryMsg::SaveRoot {
                        original: original.clone(),
                        name,
                        path,
                    });
                }
                Err(message) => show_notice(&notice_label, &message, true),
            }
        }
    });

    // 名称行回车等同于点确认按钮；路径行回车先规范化路径。
    name_row.connect_apply({
        let save_button = save_button.clone();
        move |_| save_button.emit_clicked()
    });

    let parent = relm4::main_adw_application().active_window();
    dialog.present(parent.as_ref());
}

fn pick_folder(
    parent: Option<&gtk::Window>,
    path_row: &adw::EntryRow,
    style_row: &adw::ComboRow,
    notice_label: &gtk::Label,
) {
    let dialog = gtk::FileDialog::builder()
        .title("选择目录")
        .accept_label("选择")
        .build();
    let current = path_row.text();
    if !current.trim().is_empty() {
        dialog.set_initial_folder(Some(&gio::File::for_path(current.trim())));
    }

    let path_row = path_row.clone();
    let style_row = style_row.clone();
    let notice_label = notice_label.clone();
    dialog.select_folder(parent, None::<&gio::Cancellable>, move |result| {
        if let Some(path) = result.ok().and_then(|file| file.path()) {
            path_row.set_text(&path.display().to_string());
            normalize_entry(&path_row, &style_row, &notice_label);
        }
    });
}

/// 校验对话框输入，返回去掉首尾空白的名字和规范化后的路径。
///
/// 路径始终按所选格式规范化后写入：`canonicalize()` 之后，选相对路径时尽量写成相对
/// 当前工作目录的形式，不在其下时保留绝对路径。
fn validate(
    original: Option<&str>,
    name: &str,
    path: &str,
    relative: bool,
    roots: &[(String, PathBuf)],
) -> Result<(String, PathBuf), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("目录名称不能为空。".to_string());
    }
    if roots
        .iter()
        .any(|(existing, _)| existing == name && Some(existing.as_str()) != original)
    {
        return Err(format!("已存在同名目录：{name}。"));
    }

    let path = path.trim();
    if path.is_empty() {
        return Err("目录路径不能为空。".to_string());
    }
    let candidate = absolute(path);
    if let Some((existing, _)) = roots.iter().find(|(existing, existing_path)| {
        Some(existing.as_str()) != original && absolute(existing_path) == candidate
    }) {
        return Err(format!("该路径已被目录「{existing}」使用。"));
    }

    let (path, _) = normalize_path(path, relative)?;
    Ok((name.to_string(), path))
}

/// 在对话框底部显示提示；错误用报错样式，回退提示用次要文字样式。
fn show_notice(notice_label: &gtk::Label, message: &str, is_error: bool) {
    notice_label.set_label(message);
    if is_error {
        notice_label.add_css_class("error");
        notice_label.remove_css_class("dim-label");
    } else {
        notice_label.add_css_class("dim-label");
        notice_label.remove_css_class("error");
    }
    notice_label.set_visible(true);
}

/// 校验并改写路径输入框，返回是否成功；失败时在对话框里显示原因。
///
/// 请求相对路径、但目标不在当前工作目录下时，自动切回绝对路径并提示。
fn normalize_entry(
    path_row: &adw::EntryRow,
    style_row: &adw::ComboRow,
    notice_label: &gtk::Label,
) -> bool {
    let text = path_row.text();
    if text.trim().is_empty() {
        return false;
    }
    match normalize_path(&text, style_row.selected() == RELATIVE_INDEX) {
        Ok((path, reverted)) => {
            path_row.set_text(&path.display().to_string());
            if reverted {
                // 切回绝对路径会触发 selected_notify 再规范化一次，提示放在其后显示。
                style_row.set_selected(ABSOLUTE_INDEX);
                show_notice(
                    notice_label,
                    "该路径不在当前工作目录下，已改为绝对路径。",
                    false,
                );
            } else {
                notice_label.set_visible(false);
            }
            true
        }
        Err(message) => {
            show_notice(notice_label, &message, true);
            false
        }
    }
}

/// 路径必须存在且是目录；返回 `canonicalize()` 后的路径和「是否回退成了绝对路径」。
///
/// `relative` 为真时尽量改写成相对当前工作目录的形式；目标是工作目录本身时写成 `.`。
/// 不在工作目录下（或 Windows 跨盘符）时只能保留绝对路径，并把第二个返回值置为真。
fn normalize_path(text: &str, relative: bool) -> Result<(PathBuf, bool), String> {
    let path = absolute(text);
    ensure_directory(&path)?;
    let path = path
        .canonicalize()
        .map(strip_verbatim)
        .map_err(|err| format!("无法规范化目录路径：{err}。"))?;
    if !relative {
        return Ok((path, false));
    }

    let base = std::env::current_dir()
        .ok()
        .and_then(|cwd| cwd.canonicalize().ok())
        .map(strip_verbatim);
    match base.and_then(|base| relative_to(&path, &base)) {
        Some(path) => Ok((path, false)),
        None => Ok((path, true)),
    }
}

/// 把 `path` 改成相对 `base` 的形式；不在 `base` 下时返回 `None`。
fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    match path.strip_prefix(base) {
        Ok(rest) if rest.as_os_str().is_empty() => Some(PathBuf::from(".")),
        Ok(rest) => Some(rest.to_path_buf()),
        Err(_) => None,
    }
}

/// 确认路径存在且是目录，否则给出可读的错误。
fn ensure_directory(path: &Path) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("该路径不是目录：{}。", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("目录不存在：{}。", path.display()))
        }
        Err(err) => Err(format!("无法访问目录：{}（{err}）。", path.display())),
    }
}

/// Windows 的 `canonicalize` 会加上 `\\?\` 前缀；写回配置前去掉，保持路径可读。
#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}

#[cfg(not(windows))]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    path
}

/// 一个根目录行：名称、路径和是否默认。
pub(super) struct RootRow {
    name: String,
    path: PathBuf,
    is_default: bool,
}

/// 目录行发出的动作；直接带上名字，避免列表顺序变化后索引失效。
#[derive(Debug)]
pub(super) enum RootRowOut {
    Edit(String),
    Remove(String),
    SetDefault(String),
    Move(drag_list::DragMove),
}

#[relm4::factory(pub(super))]
impl FactoryComponent for RootRow {
    type Init = (String, PathBuf, bool);
    type Input = bool;
    type Output = RootRowOut;
    type CommandOutput = ();
    type ParentWidget = adw::PreferencesGroup;
    type Index = DynamicIndex;

    view! {
        adw::ActionRow {
            set_title: &self.name,
            set_subtitle: &self.path.display().to_string(),

            #[name(drag_handle)]
            add_prefix = &gtk::Image {},

            #[name(default_button)]
            add_suffix = &gtk::Button {
                #[watch]
                set_icon_name: if self.is_default { "starred-symbolic" } else { "non-starred-symbolic" },
                #[watch]
                set_tooltip_text: Some(if self.is_default { "默认目录" } else { "设为默认" }),
                add_css_class: "flat",
                set_valign: gtk::Align::Center,
            },

            #[name(edit_button)]
            add_suffix = &gtk::Button {
                set_icon_name: "document-edit-symbolic",
                set_tooltip_text: Some("编辑"),
                add_css_class: "flat",
                set_valign: gtk::Align::Center,
            },

            #[name(remove_button)]
            add_suffix = &gtk::Button {
                set_icon_name: "user-trash-symbolic",
                set_tooltip_text: Some("删除"),
                add_css_class: "flat",
                add_css_class: "error",
                set_valign: gtk::Align::Center,
            },
        }
    }

    fn init_model(
        (name, path, is_default): Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        Self {
            name,
            path,
            is_default,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        root: Self::Root,
        _returned_widget: &gtk::Widget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();

        widgets.default_button.connect_clicked({
            let sender = sender.clone();
            let name = self.name.clone();
            move |_| {
                let _ = sender.output(RootRowOut::SetDefault(name.clone()));
            }
        });
        widgets.edit_button.connect_clicked({
            let sender = sender.clone();
            let name = self.name.clone();
            move |_| {
                let _ = sender.output(RootRowOut::Edit(name.clone()));
            }
        });
        widgets.remove_button.connect_clicked({
            let sender = sender.clone();
            let name = self.name.clone();
            move |_| {
                let _ = sender.output(RootRowOut::Remove(name.clone()));
            }
        });

        drag_list::attach(
            root.upcast_ref(),
            &widgets.drag_handle,
            self.name.clone(),
            {
                let sender = sender.clone();
                move |mv| {
                    let _ = sender.output(RootRowOut::Move(mv));
                }
            },
        );

        widgets
    }

    fn update(&mut self, is_default: bool, _sender: FactorySender<Self>) {
        self.is_default = is_default;
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use dmcl_mcmeta::layout::absolute;

    use super::{normalize_path, relative_to, strip_verbatim, validate};

    fn roots() -> Vec<(String, PathBuf)> {
        vec![
            ("a".to_string(), absolute("a")),
            ("b".to_string(), absolute("b")),
        ]
    }

    #[test]
    fn rejects_empty_names_and_paths() {
        assert!(validate(None, "  ", "a", false, &roots()).is_err());
        assert!(validate(None, "c", "  ", false, &roots()).is_err());
    }

    #[test]
    fn rejects_duplicate_names_unless_editing_that_root() {
        assert!(validate(None, "a", "c", false, &roots()).is_err());
        // 编辑自己时名称不变是允许的；路径要存在（这里用当前目录）。
        assert!(validate(Some("a"), "a", ".", false, &roots()).is_ok());
        // 改成别的已有名字仍然不行。
        assert!(validate(Some("a"), "b", ".", false, &roots()).is_err());
    }

    #[test]
    fn rejects_paths_used_by_another_root() {
        let error = validate(None, "c", "b", false, &roots()).unwrap_err();
        assert!(error.contains("「b」"), "{error}");
        assert!(validate(Some("b"), "b", ".", false, &roots()).is_ok());
    }

    #[test]
    fn rejects_paths_that_do_not_exist_or_are_not_directories() {
        let missing = std::env::temp_dir().join("dmcl-root-does-not-exist");
        let error = validate(None, "c", &missing.display().to_string(), false, &[]).unwrap_err();
        assert!(error.contains("不存在"), "{error}");

        let file = std::env::temp_dir().join("dmcl-root-validation-file");
        std::fs::write(&file, b"").unwrap();
        let error = validate(None, "c", &file.display().to_string(), false, &[]).unwrap_err();
        assert!(error.contains("不是目录"), "{error}");
        let _ = std::fs::remove_file(&file);

        // 已有目录可以正常通过。
        let dir = std::env::temp_dir();
        assert!(validate(None, "c", &dir.display().to_string(), false, &[]).is_ok());
    }

    #[test]
    fn trims_names_and_canonicalizes_paths() {
        let dir = std::env::temp_dir();
        let input = format!("{}/.", dir.display());
        let (name, path) = validate(None, "  c  ", &input, false, &[]).unwrap();
        assert_eq!(name, "c");
        assert_eq!(path, strip_verbatim(dir.canonicalize().unwrap()));
    }

    #[test]
    fn saving_always_writes_the_normalized_path() {
        // 即使输入只是当前目录的另一种写法，保存时也统一成「.」。
        let (_, path) = validate(None, "c", "./.", true, &[]).unwrap();
        assert_eq!(path, PathBuf::from("."));
    }

    #[test]
    fn switching_the_path_style_rewrites_the_path() {
        // 相对 → 绝对：选绝对后必须真的写成绝对路径。
        let (_, path) = validate(Some("a"), "a", ".", false, &[]).unwrap();
        assert!(path.is_absolute(), "{}", path.display());

        // 绝对 → 相对：当前目录本身写成「.」。
        let cwd = std::env::current_dir().unwrap();
        let (_, path) = validate(Some("a"), "a", &cwd.display().to_string(), true, &[]).unwrap();
        assert_eq!(path, Path::new("."));
    }

    #[test]
    fn relative_paths_are_used_only_under_the_base() {
        let base = absolute("dmcl-base");
        assert_eq!(
            relative_to(&base.join("mc"), &base),
            Some(PathBuf::from("mc"))
        );
        assert_eq!(relative_to(&base, &base), Some(PathBuf::from(".")));

        let outside = absolute("dmcl-elsewhere");
        assert_eq!(relative_to(&outside, &base), None);
    }

    #[test]
    fn requesting_relative_for_an_outside_path_reports_the_fallback() {
        // 当前工作目录本身可以写成「.」，不会回退。
        let (path, reverted) = normalize_path(".", true).unwrap();
        assert_eq!(path, PathBuf::from("."));
        assert!(!reverted);

        // 工作目录的上一级必然不在其下；只在该目录存在时构造这个用例。
        let Some(outside) = std::env::current_dir()
            .unwrap()
            .parent()
            .map(Path::to_path_buf)
        else {
            return;
        };
        let (path, reverted) = normalize_path(&outside.display().to_string(), true).unwrap();
        assert!(reverted);
        assert_eq!(path, strip_verbatim(outside.canonicalize().unwrap()));
    }
}
