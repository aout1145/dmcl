//! 首选项里的「Java 管理」分类：列表、搜索、对话框与配置写入。

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dmcl_launch::{default_java_roots, find_java, JavaDetails};
use dmcl_mcmeta::layout::absolute;
use dmcl_mcmeta::os::Arch;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gio;
use relm4::prelude::*;

use crate::config::{
    configs_from_details, merge_javas, same_java_path, unique_java_name, Config, JavaConfig,
};
use crate::gui::gtk4::app::{config_store, ConfigRef, PageToast, Shell};
use crate::gui::gtk4::pages::settings::common::{
    details_arch_text, details_title, drag_list, java_summary, JavaProbeCache,
};

/// 搜索按钮的常态标题。
const SEARCH_TITLE: &str = "搜索已安装的 Java";

/// 搜索进行中的标题。
const SEARCHING_TITLE: &str = "正在搜索 Java…";

/// 路径已经被列表里的另一个 Java 占用时的提示。
const DUPLICATE_PATH: &str = "该路径已经被列表中的另一个 Java 使用。";

/// 「Java 管理」分类收到的消息。
#[derive(Debug)]
pub(super) enum JavaMsg {
    /// 页面显示出来了：重建列表并探测还没探测过的 Java。
    Shown,
    /// 页面隐藏了：停止发起新的探测。
    Hidden,
    /// 共享配置变了：重新读取 Java 表。
    ConfigChanged,
    /// 某个 Java 探测成功，记进本页缓存；不写配置。
    Probed { path: PathBuf, details: JavaDetails },
    /// 打开添加 Java 的对话框。
    Add,
    /// 打开某个 Java 的编辑对话框。
    Edit(String),
    /// 请求删除某个 Java（先弹确认框）。
    Remove(String),
    /// 确认框里点了删除。
    RemoveConfirmed(String),
    /// 拖动排序。
    MoveJava(drag_list::DragMove),
    /// 添加 / 编辑对话框确认后的结果；`original` 为空表示新增。
    SaveJava {
        original: Option<String>,
        java: JavaConfig,
        /// 新增时用来生成内部键；编辑时可以为空。
        details: Option<JavaDetails>,
    },
    /// 后台扫描本机 Java。
    Search,
    /// 扫描结束。
    SearchDone(Result<Vec<JavaDetails>, String>),
}

pub(super) struct JavaPage {
    config: ConfigRef,
    shell: Rc<Shell>,
    /// 页面级提示：只有首选项可见时才显示。
    page_toast: PageToast,
    /// 每个 Java 一行；由配置顺序决定先后。
    javas: FactoryVecDeque<JavaRow>,
    /// 固定在列表最前面的搜索行与最后面的添加行。
    search_row: adw::ButtonRow,
    add_row: adw::ButtonRow,
    /// 与游戏设置页共享的探测结果缓存。
    cache: Rc<JavaProbeCache>,
    /// 页面是否可见；不可见时不发起探测。
    visible: bool,
}

/// 「Java 管理」分类的初始化参数。
pub(crate) struct JavaPageInit {
    pub shell: Rc<Shell>,
    pub cache: Rc<JavaProbeCache>,
}

#[relm4::component(pub(super))]
impl SimpleComponent for JavaPage {
    type Init = JavaPageInit;
    type Input = JavaMsg;
    type Output = ();

    view! {
        adw::PreferencesPage {
            #[local_ref]
            javas_group -> adw::PreferencesGroup {},
        }
    }

    fn init(
        init: Self::Init,
        root_widget: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let JavaPageInit { shell, cache } = init;
        let config = config_store::store().config();
        let group = adw::PreferencesGroup::builder()
            .title("Java 管理")
            .description("管理启动器可用的 Java。")
            .build();

        // 页面常驻，但只有真正显示时才去执行 Java，避免启动时探测。
        root_widget.connect_map({
            let sender = sender.clone();
            move |_| sender.input(JavaMsg::Shown)
        });
        root_widget.connect_unmap({
            let sender = sender.clone();
            move |_| sender.input(JavaMsg::Hidden)
        });
        let javas = FactoryVecDeque::builder().launch(group.clone()).forward(
            sender.input_sender(),
            |out| match out {
                JavaRowOut::Edit(name) => JavaMsg::Edit(name),
                JavaRowOut::Remove(name) => JavaMsg::Remove(name),
                JavaRowOut::Move(mv) => JavaMsg::MoveJava(mv),
                JavaRowOut::Probed { path, details } => JavaMsg::Probed { path, details },
            },
        );

        let search_row = adw::ButtonRow::builder()
            .title(SEARCH_TITLE)
            .start_icon_name("system-search-symbolic")
            .build();
        search_row.connect_activated({
            let sender = sender.clone();
            move |_| sender.input(JavaMsg::Search)
        });

        let add_row = adw::ButtonRow::builder()
            .title("手动添加 Java")
            .start_icon_name("list-add-symbolic")
            .build();
        add_row.connect_activated({
            let sender = sender.clone();
            move |_| sender.input(JavaMsg::Add)
        });
        group.add(&search_row);
        group.add(&add_row);

        let mut model = JavaPage {
            config,
            shell,
            page_toast: PageToast::new(&root_widget),
            javas,
            search_row,
            add_row,
            cache,
            visible: false,
        };
        let javas_group = model.javas.widget();
        let widgets = view_output!();
        model.reload();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            JavaMsg::Shown => {
                self.visible = true;
                self.reload();
            }
            JavaMsg::Hidden => self.visible = false,
            JavaMsg::ConfigChanged => self.reload(),
            JavaMsg::Probed { path, details } => {
                self.cache.insert(path, details);
            }
            JavaMsg::Add => show_add(&self.java_entries(), &sender),
            JavaMsg::Edit(name) => {
                let java = self.config.read().javas.get(&name).cloned();
                let Some(java) = java else {
                    return;
                };
                let details = self.cache.get(&java.path);
                show_edit(&name, java, details, &self.java_entries(), &sender);
            }
            JavaMsg::Remove(name) => {
                let title = self.remove_title(&name);
                confirm_remove(&name, &title, &sender);
            }
            JavaMsg::RemoveConfirmed(name) => {
                self.update_config(|config| config.remove_java(&name));
                self.reload();
            }
            JavaMsg::MoveJava(mv) => self.move_java(mv),
            JavaMsg::SaveJava {
                original,
                java,
                details,
            } => {
                let cached = details
                    .as_ref()
                    .map(|details| (absolute(&java.path), details.clone()));
                self.update_config(move |config| match &original {
                    Some(old) => config.set_java(old, java),
                    None => {
                        let details = details
                            .ok_or_else(|| anyhow::anyhow!("缺少探测结果，无法添加 Java"))?;
                        let name = unique_java_name(&config.javas, &details);
                        config.add_java(&name, java)
                    }
                });
                if let Some((path, details)) = cached {
                    self.cache.insert(path, details);
                }
                self.reload();
            }
            JavaMsg::Search => self.search(&sender),
            JavaMsg::SearchDone(result) => {
                self.search_row.set_sensitive(true);
                self.search_row.set_title(SEARCH_TITLE);
                self.finish_search(result);
            }
        }
    }
}

impl JavaPage {
    /// 按共享配置重建行；配置顺序就是行的顺序，搜索行在最前，添加行在最后。
    fn reload(&mut self) {
        let javas: Vec<(String, JavaConfig)> = {
            let config = self.config.read();
            config
                .javas
                .iter()
                .map(|(name, java)| (name.clone(), java.clone()))
                .collect()
        };

        let paths: Vec<PathBuf> = javas.iter().map(|(_, java)| absolute(&java.path)).collect();
        self.cache.retain(&paths);

        let group = self.javas.widget().clone();
        {
            let mut guard = self.javas.guard();
            guard.clear();
            group.remove(&self.search_row);
            group.remove(&self.add_row);
            group.add(&self.search_row);
            for (name, java) in javas {
                let details = self.cache.get(&java.path);
                guard.push_back((name, java, details));
            }
        }
        group.add(&self.add_row);

        // 页面可见时才探测；已经有结果的行走缓存，不会再执行 Java。
        if self.visible {
            for index in 0..self.javas.len() {
                self.javas.send(index, JavaRowMsg::Probe);
            }
        }
    }

    /// 删除确认框里用的标题；名字是内部键，不直接展示。
    fn remove_title(&self, name: &str) -> String {
        let config = self.config.read();
        let Some(java) = config.javas.get(name) else {
            return name.to_string();
        };
        match self.cache.get(&java.path) {
            Some(details) => details_title(&details),
            None => format!("Java {}", java.major),
        }
    }

    /// 按拖动落点把源 Java 放到目标 Java 前或后。
    fn move_java(&mut self, mv: drag_list::DragMove) {
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
            let Some(from_index) = config.javas.get_index_of(&from) else {
                return;
            };
            let Some(target_index) = config.javas.get_index_of(&target) else {
                return;
            };
            (from_index, target_index)
        };

        let to = drag_list::moved_index(from_index, target_index, after);
        if to == from_index {
            return;
        }

        self.update_config(|config| config.move_java(&from, to));
        self.reload();
    }

    /// 当前 Java 表，供对话框校验重复路径。
    fn java_entries(&self) -> Vec<(String, JavaConfig)> {
        self.config
            .read()
            .javas
            .iter()
            .map(|(name, java)| (name.clone(), java.clone()))
            .collect()
    }

    /// 后台扫描本机 Java；扫描完再合并进配置。
    fn search(&self, sender: &ComponentSender<Self>) {
        self.search_row.set_sensitive(false);
        self.search_row.set_title(SEARCHING_TITLE);

        let (tx, rx) = tokio::sync::oneshot::channel();
        relm4::spawn(async move {
            let _ = tx.send(find_java(&default_java_roots()).await);
        });

        let sender = sender.clone();
        relm4::spawn_local(async move {
            let result = rx.await.map_err(|_| "搜索任务意外结束".to_string());
            sender.input(JavaMsg::SearchDone(result));
        });
    }

    /// 合并扫描结果并提示新增数量。
    fn finish_search(&mut self, result: Result<Vec<JavaDetails>, String>) {
        let found = match result {
            Ok(found) => found,
            Err(err) => {
                self.shell
                    .page_toast(&self.page_toast, &format!("搜索 Java 失败：{err}"));
                return;
            }
        };
        if found.is_empty() {
            self.shell
                .page_toast(&self.page_toast, "没有找到已安装的 Java。");
            return;
        }

        // 搜索已经执行过这些 Java，直接记进缓存，列表不用再探测一遍。
        for details in &found {
            self.cache
                .insert(absolute(&details.option.path), details.clone());
        }

        let count = found.len();
        let found = configs_from_details(found);
        let added = Rc::new(Cell::new(0));
        let added_in_update = Rc::clone(&added);
        self.update_config(move |config| {
            added_in_update.set(merge_javas(&mut config.javas, &found));
            Ok(())
        });
        self.reload();

        let added = added.get();
        let text = format!("找到 {count} 个 Java，新增 {added} 个。");
        self.shell.page_toast(&self.page_toast, &text);
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
                .page_toast(&self.page_toast, &format!("无法修改 Java：{err:#}"));
        }
    }
}

/// 添加新 Java。
fn show_add(javas: &[(String, JavaConfig)], sender: &ComponentSender<JavaPage>) {
    show(JavaDialogMode::Add, None, None, javas, sender);
}

/// 编辑已有 Java 的路径；页面缓存里有探测结果时直接显示，不用重新执行 Java。
fn show_edit(
    name: &str,
    java: JavaConfig,
    details: Option<JavaDetails>,
    javas: &[(String, JavaConfig)],
    sender: &ComponentSender<JavaPage>,
) {
    show(
        JavaDialogMode::Edit(name.to_string()),
        Some(java),
        details,
        javas,
        sender,
    );
}

/// 删除前的确认框；`title` 是行上显示的类型与版本。
fn confirm_remove(name: &str, title: &str, sender: &ComponentSender<JavaPage>) {
    let title = format!("删除「{title}」？");
    let dialog = adw::AlertDialog::new(Some(&title), None);
    dialog.add_response("cancel", "取消");
    dialog.add_response("delete", "删除");
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");

    let sender = sender.clone();
    let name = name.to_string();
    dialog.connect_response(None, move |_, response| {
        if response == "delete" {
            sender.input(JavaMsg::RemoveConfirmed(name.clone()));
        }
    });

    let parent = relm4::main_adw_application().active_window();
    dialog.present(parent.as_ref());
}

#[derive(Clone)]
enum JavaDialogMode {
    Add,
    Edit(String),
}

impl JavaDialogMode {
    fn original(&self) -> Option<String> {
        match self {
            Self::Add => None,
            Self::Edit(name) => Some(name.clone()),
        }
    }
}

/// 已探测到的 Java：路径、缓存的版本 / 架构，以及本次探测的完整信息。
#[derive(Clone)]
struct Detected {
    path: PathBuf,
    major: u32,
    arch: Arch,
    /// 页面上次探测到的完整信息；还没探测过时为空。
    details: Option<JavaDetails>,
}

/// 对话框里的探测状态：结果、进行中的路径和待保存标记。
#[derive(Default)]
struct ProbeState {
    detected: Option<Detected>,
    /// 每次路径变化或发起探测都加一，用来丢弃过时的探测结果。
    generation: u64,
    /// 正在探测的路径；同一路径重复触发时复用这次探测。
    in_flight: Option<PathBuf>,
    /// 探测成功后是否自动保存（路径行回车或点了确认按钮）。
    save_when_done: bool,
}

fn show(
    mode: JavaDialogMode,
    initial_java: Option<JavaConfig>,
    initial_details: Option<JavaDetails>,
    javas: &[(String, JavaConfig)],
    sender: &ComponentSender<JavaPage>,
) {
    let title = match &mode {
        JavaDialogMode::Add => "添加 Java",
        JavaDialogMode::Edit(_) => "编辑 Java",
    };
    let confirm_label = match &mode {
        JavaDialogMode::Add => "添加",
        JavaDialogMode::Edit(_) => "保存",
    };
    let initial_path = initial_java
        .as_ref()
        .map(|java| java.path.clone())
        .unwrap_or_default();
    let initial_detected = initial_java.as_ref().map(|java| Detected {
        path: absolute(&java.path),
        major: java.major,
        arch: java.arch,
        details: initial_details,
    });

    let dialog = adw::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(460);
    dialog.set_content_height(300);

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
                        add: path_row = &adw::EntryRow {
                            set_title: "可执行文件",
                            set_text: &initial_path.display().to_string(),
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

    let original = mode.original();
    let javas = Rc::new(javas.to_vec());
    let state = Rc::new(RefCell::new(ProbeState {
        detected: initial_detected,
        ..ProbeState::default()
    }));

    // 路径一变就重新判断确认按钮是否可用；不合法时按钮置灰。
    let refresh: Rc<dyn Fn()> = {
        let path_row = path_row.clone();
        let save_button = save_button.clone();
        let original = original.clone();
        let javas = Rc::clone(&javas);
        let state = Rc::clone(&state);
        Rc::new(move || {
            let state = state.borrow();
            save_button.set_sensitive(
                validate(
                    original.as_deref(),
                    &path_row.text(),
                    state.detected.as_ref(),
                    &javas,
                )
                .is_ok(),
            );
        })
    };
    refresh();
    path_row.connect_changed({
        let path_row = path_row.clone();
        let refresh = Rc::clone(&refresh);
        let notice_label = notice_label.clone();
        let state = Rc::clone(&state);
        let javas = Rc::clone(&javas);
        let original = original.clone();
        move |_| {
            // 旧结果对新路径无效；先置灰，等下面按新路径重新检测。
            {
                let mut state = state.borrow_mut();
                state.detected = None;
                state.save_when_done = false;
            }
            notice_label.set_visible(false);
            refresh();

            // 半截路径只做存在性检查，只有已经指向文件时才会真正执行 Java；
            // 路径被别的条目占用时也在这里直接给出提示。
            start_probe(
                &path_row,
                &notice_label,
                &state,
                &refresh,
                &javas,
                original.as_deref(),
                None,
            );
        }
    });

    // 可执行文件右侧的文件夹按钮：用系统文件选择器填路径，也可以直接手输。
    let browse = gtk::Button::from_icon_name("document-open-symbolic");
    browse.add_css_class("flat");
    browse.set_valign(gtk::Align::Center);
    browse.set_tooltip_text(Some("选择 Java 可执行文件"));
    path_row.add_suffix(&browse);
    browse.connect_clicked({
        let path_row = path_row.clone();
        let notice_label = notice_label.clone();
        let state = Rc::clone(&state);
        let refresh = Rc::clone(&refresh);
        let javas = Rc::clone(&javas);
        let original = original.clone();
        let parent = relm4::main_adw_application().active_window();
        move |_| {
            pick_java(
                parent.as_ref(),
                &path_row,
                &notice_label,
                &state,
                &refresh,
                &javas,
                original.as_deref(),
            )
        }
    });

    // 输入完成（回车或离开该行）就探测一次；回车探测成功后直接保存。
    let focus = gtk::EventControllerFocus::new();
    focus.connect_leave({
        let path_row = path_row.clone();
        let notice_label = notice_label.clone();
        let state = Rc::clone(&state);
        let refresh = Rc::clone(&refresh);
        let javas = Rc::clone(&javas);
        let original = original.clone();
        move |_| {
            // 已经有这次路径的结果（或正在探测）就不再重复执行 Java。
            let path = entry_path(&path_row.text());
            if !tracks_path(&state.borrow(), &path) {
                start_probe(
                    &path_row,
                    &notice_label,
                    &state,
                    &refresh,
                    &javas,
                    original.as_deref(),
                    None,
                );
            }
        }
    });
    path_row.add_controller(focus);
    path_row.connect_apply({
        let path_row = path_row.clone();
        let notice_label = notice_label.clone();
        let state = Rc::clone(&state);
        let refresh = Rc::clone(&refresh);
        let javas = Rc::clone(&javas);
        let original = original.clone();
        let save_button = save_button.clone();
        move |_| {
            start_probe(
                &path_row,
                &notice_label,
                &state,
                &refresh,
                &javas,
                original.as_deref(),
                Some(save_button.clone()),
            )
        }
    });

    save_button.connect_clicked({
        let dialog = dialog.clone();
        let notice_label = notice_label.clone();
        let path_row = path_row.clone();
        let sender = sender.clone();
        let javas = Rc::clone(&javas);
        let state = Rc::clone(&state);
        let refresh = Rc::clone(&refresh);
        let original = original.clone();
        let save_button = save_button.clone();
        move |_| {
            // 路径还没探测过（或已改过）：先探测，成功后再自动回到这里保存。
            let current = entry_path(&path_row.text());
            if !tracks_path(&state.borrow(), &current) {
                start_probe(
                    &path_row,
                    &notice_label,
                    &state,
                    &refresh,
                    &javas,
                    original.as_deref(),
                    Some(save_button.clone()),
                );
                return;
            }

            let result = validate(
                original.as_deref(),
                &path_row.text(),
                state.borrow().detected.as_ref(),
                &javas,
            );
            match result {
                Ok(java) => {
                    let details = state
                        .borrow()
                        .detected
                        .as_ref()
                        .and_then(|detected| detected.details.clone());
                    dialog.close();
                    sender.input(JavaMsg::SaveJava {
                        original: original.clone(),
                        java,
                        details,
                    });
                }
                Err(message) => show_notice(&notice_label, &message, true),
            }
        }
    });

    if let Some(detected) = state.borrow().detected.as_ref() {
        show_notice(&notice_label, &detected_summary(detected), false);
    }

    let parent = relm4::main_adw_application().active_window();
    dialog.present(parent.as_ref());
}

fn pick_java(
    parent: Option<&gtk::Window>,
    path_row: &adw::EntryRow,
    notice_label: &gtk::Label,
    state: &Rc<RefCell<ProbeState>>,
    refresh: &Rc<dyn Fn()>,
    javas: &[(String, JavaConfig)],
    original: Option<&str>,
) {
    let dialog = gtk::FileDialog::builder()
        .title("选择 Java 可执行文件")
        .accept_label("选择")
        .build();
    let current = entry_path(&path_row.text());
    let folder = if current.is_dir() {
        Some(current)
    } else {
        current.parent().map(std::path::Path::to_path_buf)
    };
    if let Some(folder) = folder {
        dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
    }

    let path_row = path_row.clone();
    let notice_label = notice_label.clone();
    let state = Rc::clone(state);
    let refresh = Rc::clone(refresh);
    let javas = javas.to_vec();
    let original = original.map(str::to_string);
    dialog.open(parent, None::<&gio::Cancellable>, move |result| {
        if let Some(path) = result.ok().and_then(|file| file.path()) {
            path_row.set_text(&path.display().to_string());
            start_probe(
                &path_row,
                &notice_label,
                &state,
                &refresh,
                &javas,
                original.as_deref(),
                None,
            );
        }
    });
}

/// 路径是否已经被列表里的另一个 Java 占用。
fn duplicate_java(javas: &[(String, JavaConfig)], original: Option<&str>, path: &Path) -> bool {
    javas
        .iter()
        .any(|(name, java)| Some(name.as_str()) != original && same_java_path(&java.path, path))
}

/// 清空探测结果并作废处理中的探测。
fn reset_probe(state: &Rc<RefCell<ProbeState>>) {
    let mut state = state.borrow_mut();
    state.detected = None;
    state.in_flight = None;
    state.generation += 1;
    state.save_when_done = false;
}

/// 当前路径是否已经探测过、或正在探测。
fn tracks_path(state: &ProbeState, path: &std::path::Path) -> bool {
    state
        .detected
        .as_ref()
        .is_some_and(|detected| same_java_path(&detected.path, path))
        || state
            .in_flight
            .as_ref()
            .is_some_and(|in_flight| same_java_path(in_flight, path))
}

/// 探测可执行文件；成功后写入状态，并可选地触发保存。
fn start_probe(
    path_row: &adw::EntryRow,
    notice_label: &gtk::Label,
    state: &Rc<RefCell<ProbeState>>,
    refresh: &Rc<dyn Fn()>,
    javas: &[(String, JavaConfig)],
    original: Option<&str>,
    save_button: Option<gtk::Button>,
) {
    let text = path_row.text();
    if text.trim().is_empty() {
        reset_probe(state);
        refresh();
        notice_label.set_visible(false);
        return;
    }
    let path = entry_path(&text);

    // 路径已经被别的条目用着：不用执行 Java，直接说明原因。
    if duplicate_java(javas, original, &path) {
        reset_probe(state);
        refresh();
        show_notice(notice_label, DUPLICATE_PATH, true);
        return;
    }

    // 同一条路径正在探测：复用那次探测，只记下是否需要接着保存。
    {
        let mut state = state.borrow_mut();
        if let Some(in_flight) = &state.in_flight {
            if same_java_path(in_flight, &path) {
                state.save_when_done |= save_button.is_some();
                return;
            }
        }
    }

    let error = match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => None,
        Ok(_) => Some(format!("该路径不是文件：{}。", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Some(format!("Java 可执行文件不存在：{}。", path.display()))
        }
        Err(err) => Some(format!(
            "无法访问 Java 可执行文件：{}（{err}）。",
            path.display()
        )),
    };
    if let Some(message) = error {
        reset_probe(state);
        refresh();
        show_notice(notice_label, &message, true);
        return;
    }

    let current = {
        let mut state = state.borrow_mut();
        state.detected = None;
        state.in_flight = Some(path.clone());
        state.generation += 1;
        state.save_when_done = save_button.is_some();
        state.generation
    };
    refresh();
    show_notice(notice_label, "正在检测 Java…", false);

    let (tx, rx) = tokio::sync::oneshot::channel();
    relm4::spawn(async move {
        let _ = tx.send(JavaDetails::probe(path).await);
    });

    let notice_label = notice_label.clone();
    let state = Rc::clone(state);
    let refresh = Rc::clone(refresh);
    relm4::spawn_local(async move {
        let result = match rx.await {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("检测任务意外结束")),
        };
        let mut guard = state.borrow_mut();
        // 期间路径又改过：这次结果已经过时。
        if guard.generation != current {
            return;
        }
        guard.in_flight = None;
        match result {
            Ok(details) if details.is_usable() => {
                let summary = java_summary(&details);
                let java = details.option.clone();
                guard.detected = Some(Detected {
                    path: java.path,
                    major: java.major,
                    arch: java.arch,
                    details: Some(details),
                });
                let save_now = guard.save_when_done;
                guard.save_when_done = false;
                drop(guard);

                show_notice(&notice_label, &summary, false);
                refresh();
                if save_now {
                    if let Some(save_button) = save_button {
                        save_button.emit_clicked();
                    }
                }
            }
            Ok(_) => {
                guard.save_when_done = false;
                drop(guard);
                show_notice(&notice_label, "无法确定该 Java 的版本。", true);
                refresh();
            }
            Err(err) => {
                guard.save_when_done = false;
                drop(guard);
                show_notice(&notice_label, &format!("无法检测该 Java：{err}"), true);
                refresh();
            }
        }
    });
}

/// 校验对话框输入，返回探测到的 Java 条目。
fn validate(
    original: Option<&str>,
    path_text: &str,
    detected: Option<&Detected>,
    javas: &[(String, JavaConfig)],
) -> Result<JavaConfig, String> {
    let path_text = path_text.trim();
    if path_text.is_empty() {
        return Err("Java 路径不能为空。".to_string());
    }
    let path = entry_path(path_text);
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(format!("该路径不是文件：{}。", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("Java 可执行文件不存在：{}。", path.display()));
        }
        Err(err) => {
            return Err(format!(
                "无法访问 Java 可执行文件：{}（{err}）。",
                path.display()
            ));
        }
    }

    if duplicate_java(javas, original, &path) {
        return Err(DUPLICATE_PATH.to_string());
    }

    let Some(detected) = detected else {
        return Err("请先检测该 Java。".to_string());
    };
    if !same_java_path(&detected.path, &path) {
        return Err("请先检测该 Java。".to_string());
    }

    Ok(JavaConfig {
        path,
        major: detected.major,
        arch: detected.arch,
    })
}

/// 对话框里的探测结果摘要；没有探测详情时退回缓存的大版本与架构。
fn detected_summary(detected: &Detected) -> String {
    match &detected.details {
        Some(details) => java_summary(details),
        None => format!("Java {}", detected.major),
    }
}

/// 配置里存的是绝对路径；这里只做清理，不解析符号链接。
fn entry_path(text: &str) -> PathBuf {
    absolute(text.trim())
}

/// 在对话框底部显示提示；错误用报错样式，检测结果用次要文字样式。
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

/// 行的探测状态。
enum RowInfo {
    Pending,
    Ready(JavaDetails),
    Failed,
}

/// 一个 Java 行：标题显示 JDK/JRE 与完整版本，右侧供应商与架构，下方是路径。
pub(super) struct JavaRow {
    name: String,
    java: JavaConfig,
    state: RowInfo,
    /// 标题与右侧小字由 `state` 派生；`view!` 的 watch 直接读这两个字段。
    title: String,
    info_text: String,
    subtitle: String,
    /// 已经发起探测，避免重复执行 Java。
    probing: bool,
}

impl JavaRow {
    fn is_pending(&self) -> bool {
        matches!(self.state, RowInfo::Pending)
    }

    /// 根据探测状态刷新标题与右侧小字。
    fn refresh_texts(&mut self) {
        match &self.state {
            RowInfo::Pending => {
                self.title.clear();
                self.info_text.clear();
            }
            RowInfo::Ready(details) => {
                self.title = details_title(details);
                self.info_text = details_arch_text(details);
            }
            RowInfo::Failed => {
                self.title = format!("Java {}", self.java.major);
                self.info_text = self.java.arch.name().to_string();
            }
        }
    }
}

/// Java 行发出的动作；直接带上名字，避免列表顺序变化后索引失效。
#[derive(Debug)]
pub(super) enum JavaRowOut {
    Edit(String),
    Remove(String),
    Move(drag_list::DragMove),
    Probed { path: PathBuf, details: JavaDetails },
}

/// Java 行内部消息。
#[derive(Debug)]
pub(super) enum JavaRowMsg {
    /// 页面可见：还没探测过就执行一次 Java。
    Probe,
    /// 后台探测结束。
    Probed(anyhow::Result<JavaDetails>),
}

#[relm4::factory(pub(super))]
impl FactoryComponent for JavaRow {
    type Init = (String, JavaConfig, Option<JavaDetails>);
    type Input = JavaRowMsg;
    type Output = JavaRowOut;
    type CommandOutput = ();
    type ParentWidget = adw::PreferencesGroup;
    type Index = DynamicIndex;

    view! {
        adw::ActionRow {
            #[watch]
            set_title: &self.title,
            set_subtitle: &self.subtitle,

            #[name(drag_handle)]
            add_prefix = &gtk::Image {},

            #[name(info_label)]
            add_suffix = &gtk::Label {
                #[watch]
                set_label: &self.info_text,
                #[watch]
                set_visible: !self.info_text.is_empty(),
                add_css_class: "dim-label",
                add_css_class: "caption",
                set_valign: gtk::Align::Center,
            },

            #[name(spinner)]
            add_suffix = &gtk::Spinner {
                #[watch]
                set_spinning: self.is_pending(),
                #[watch]
                set_visible: self.is_pending(),
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
        (name, java, details): Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        let subtitle = absolute(&java.path).display().to_string();
        let state = match details {
            Some(details) => RowInfo::Ready(details),
            None => RowInfo::Pending,
        };
        let mut row = Self {
            name,
            java,
            state,
            title: String::new(),
            info_text: String::new(),
            subtitle,
            probing: false,
        };
        row.refresh_texts();
        row
    }

    fn init_widgets(
        &mut self,
        _index: &DynamicIndex,
        root: Self::Root,
        _returned_widget: &gtk::Widget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();

        widgets.edit_button.connect_clicked({
            let sender = sender.clone();
            let name = self.name.clone();
            move |_| {
                let _ = sender.output(JavaRowOut::Edit(name.clone()));
            }
        });
        widgets.remove_button.connect_clicked({
            let sender = sender.clone();
            let name = self.name.clone();
            move |_| {
                let _ = sender.output(JavaRowOut::Remove(name.clone()));
            }
        });

        drag_list::attach(
            root.upcast_ref(),
            &widgets.drag_handle,
            self.name.clone(),
            {
                let sender = sender.clone();
                move |mv| {
                    let _ = sender.output(JavaRowOut::Move(mv));
                }
            },
        );

        widgets
    }

    fn update(&mut self, message: JavaRowMsg, sender: FactorySender<Self>) {
        match message {
            JavaRowMsg::Probe => {
                if !self.is_pending() || self.probing {
                    return;
                }
                self.probing = true;
                let path = absolute(&self.java.path);
                let (tx, rx) = tokio::sync::oneshot::channel();
                relm4::spawn(async move {
                    let _ = tx.send(JavaDetails::probe(path).await);
                });
                let sender = sender.clone();
                relm4::spawn_local(async move {
                    let result = rx
                        .await
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("Java 探测任务意外结束")));
                    sender.input(JavaRowMsg::Probed(result));
                });
            }
            JavaRowMsg::Probed(result) => {
                self.probing = false;
                match result {
                    Ok(details) if details.is_usable() => {
                        self.state = RowInfo::Ready(details.clone());
                        self.refresh_texts();
                        let _ = sender.output(JavaRowOut::Probed {
                            path: absolute(&self.java.path),
                            details,
                        });
                    }
                    _ => {
                        self.state = RowInfo::Failed;
                        self.refresh_texts();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use dmcl_mcmeta::os::Arch;

    use super::{duplicate_java, entry_path, validate, Detected};

    fn java(path: &str, major: u32) -> crate::config::JavaConfig {
        crate::config::JavaConfig {
            path: PathBuf::from(path),
            major,
            arch: Arch::X86_64,
        }
    }

    fn detected(path: &Path, major: u32) -> Detected {
        Detected {
            path: entry_path(&path.display().to_string()),
            major,
            arch: Arch::X86_64,
            details: None,
        }
    }

    #[test]
    fn detects_paths_used_by_another_java() {
        let dir = std::env::temp_dir();
        let file = dir.join("dmcl-java-duplicate");
        std::fs::write(&file, b"").unwrap();
        let text = file.display().to_string();
        let javas = vec![("a".to_string(), java(&text, 17))];
        let path = entry_path(&text);

        assert!(duplicate_java(&javas, None, &path));
        // 编辑这条 Java 时，它自己的路径不算重复。
        assert!(!duplicate_java(&javas, Some("a"), &path));
        assert!(!duplicate_java(&javas, None, &entry_path("/other/java")));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn rejects_empty_paths() {
        let dir = std::env::temp_dir();
        let detected = detected(&dir.join("java"), 17);
        assert!(validate(None, "  ", Some(&detected), &[]).is_err());
    }

    #[test]
    fn rejects_paths_used_by_another_java() {
        let dir = std::env::temp_dir();
        let file = dir.join("dmcl-java-shared");
        std::fs::write(&file, b"").unwrap();
        let detected = detected(&file, 17);
        let text = file.display().to_string();
        let javas = vec![("a".to_string(), java(&text, 17))];

        let error = validate(None, &text, Some(&detected), &javas).unwrap_err();
        assert!(error.contains("另一个 Java"), "{error}");
        // 编辑同一条目时，路径不变不算冲突。
        assert!(validate(Some("a"), &text, Some(&detected), &javas).is_ok());

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn rejects_missing_files_and_directories() {
        let dir = std::env::temp_dir();
        let missing = dir.join("dmcl-java-does-not-exist");
        let error = validate(
            None,
            &missing.display().to_string(),
            Some(&detected(&missing, 17)),
            &[],
        )
        .unwrap_err();
        assert!(error.contains("不存在"), "{error}");

        let error = validate(
            None,
            &dir.display().to_string(),
            Some(&detected(&dir, 17)),
            &[],
        )
        .unwrap_err();
        assert!(error.contains("不是文件"), "{error}");
    }

    #[test]
    fn requires_a_matching_probe_result() {
        let dir = std::env::temp_dir();
        let file = dir.join("dmcl-java-probed");
        std::fs::write(&file, b"").unwrap();
        let text = file.display().to_string();

        assert!(validate(None, &text, None, &[]).is_err());

        let other = detected(&dir.join("other-java"), 17);
        assert!(validate(None, &text, Some(&other), &[]).is_err());

        let detected = detected(&file, 17);
        let java = validate(None, &text, Some(&detected), &[]).unwrap();
        assert_eq!(java.path, entry_path(&text));
        assert_eq!(java.major, 17);
        assert_eq!(java.arch, Arch::X86_64);

        let _ = std::fs::remove_file(&file);
    }
}
