//! Java 分组：默认 Java 选择与后台探测。

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use dmcl_java::JavaDetails;
use dmcl_mcmeta::layout::absolute;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::{configured_java, same_java_path, Config, JavaChoice, LaunchConfig};
use crate::gui::gtk4::pages::settings::common::{
    ChoiceRow, ChoiceRowInit, ChoiceRowMsg, ChoiceRowOut, FixedRow, JavaProbeCache,
};

use super::super::{
    java_choice_for_index, java_detail, java_index_for_choice, java_label, java_title,
    GameOptionField, GameOptionGroup, JavaEntry, JAVA_AUTO_INDEX,
};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct JavaGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    rows: FactoryVecDeque<ChoiceRow>,
    cache: Rc<JavaProbeCache>,
    /// 当前 Java 表；内存分组也要按选中的 Java 架构估算。
    pub(super) javas: Vec<JavaEntry>,
    /// 当前选中的下标；0 是「自动选择」。
    index: u32,
    /// 正在探测的 Java 路径，避免重复执行。
    probing: HashSet<PathBuf>,
    /// 实例版本需要的 Java 大版本；`None` 表示读不出来。
    required_major: Option<u32>,
    /// 是否已经拿到实例版本的 Java 要求；没拿到之前不显示失败文案。
    major_resolved: bool,
    /// 页面可见时才发起新的探测。
    visible: bool,
}

impl JavaGroup {
    pub(super) fn new(
        cache: Rc<JavaProbeCache>,
        required_major: Option<u32>,
        allow_inherit: bool,
        sender: &ComponentSender<GameOptionsPage>,
    ) -> Self {
        let expander = adw::ExpanderRow::builder()
            .title("Java")
            .show_enable_switch(false)
            .build();
        let inherit = FixedRow::builder("使用全局配置").build();
        inherit.row().connect_activated({
            let sender = sender.clone();
            move |_| notify(&sender, GameOptionsMsg::Inherit(GameOptionGroup::Java))
        });
        if allow_inherit {
            expander.add_row(inherit.row());
        }
        let rows = FactoryVecDeque::builder().launch(expander.clone()).forward(
            sender.input_sender(),
            |out| match out {
                ChoiceRowOut::Selected(index) => GameOptionsMsg::JavaSelected(index),
            },
        );
        Self {
            expander,
            inherit,
            rows,
            cache,
            javas: Vec::new(),
            index: JAVA_AUTO_INDEX,
            probing: HashSet::new(),
            required_major,
            major_resolved: !allow_inherit || required_major.is_some(),
            visible: false,
        }
    }

    pub(super) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }
}

/// 读取全局 Java 表与探测缓存，按当前配置定位选中项并重建选择行。
pub(super) fn reload(group: &mut JavaGroup, config: &Config, launch: &LaunchConfig) {
    let javas: Vec<JavaEntry> = config
        .javas
        .iter()
        .map(|(name, java)| JavaEntry {
            name: name.clone(),
            java: java.clone(),
            details: group.cache.get(&java.path),
        })
        .collect();
    group.javas = javas;
    group.index = java_index_for_choice(&group.javas, &launch.java);

    let paths: Vec<PathBuf> = group
        .javas
        .iter()
        .map(|entry| entry.java.path.clone())
        .collect();
    group.cache.retain(&paths);
    group
        .probing
        .retain(|path| paths.iter().any(|candidate| absolute(candidate) == *path));

    let mut guard = group.rows.guard();
    guard.clear();
    guard.push_back(ChoiceRowInit {
        index: JAVA_AUTO_INDEX,
        title: "自动选择".to_string(),
        subtitle: String::new(),
        detail: String::new(),
        avatar_text: None,
        selected: false,
    });
    for (offset, entry) in group.javas.iter().enumerate() {
        let index = offset as u32 + 1;
        guard.push_back(ChoiceRowInit {
            index,
            title: java_title(entry),
            subtitle: java_path(entry),
            detail: java_detail(entry),
            avatar_text: None,
            selected: false,
        });
    }
}

/// 同步对勾、继承状态与小字。
pub(super) fn sync(group: &JavaGroup, view: GroupView<'_>) {
    for index in 0..group.rows.len() {
        group.rows.send(
            index,
            ChoiceRowMsg::SetSelected(!view.inherited && index as u32 == group.index),
        );
    }
    group.inherit.set_selected(view.inherited);
    group
        .expander
        .set_subtitle(&view.subtitle(java_label_of(&group.javas, &view.launch.java)));
    group
        .inherit
        .set_subtitle(&java_label_of(&group.javas, &view.global.java));
    group.rows.send(
        JAVA_AUTO_INDEX as usize,
        ChoiceRowMsg::SetInfo {
            title: "自动选择".to_string(),
            subtitle: auto_java_subtitle(group, view),
            detail: String::new(),
        },
    );
}

/// 选中 Java：更新模型、标成覆盖并收起展开行。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    if index as usize > page.java.javas.len() {
        return;
    }
    let inherited = page.inherited(GameOptionGroup::Java);
    if index != page.java.index || inherited {
        page.java.index = index;
        page.launch.java = java_choice_for_index(&page.java.javas, index);
        page.mark_override([GameOptionField::Java]);
        page.schedule_flush(sender);
    }
    page.sync_ui();
}

/// 实例版本要求的 Java 大版本读出来了。
pub(super) fn required_major(page: &mut GameOptionsPage, required: Option<u32>) {
    page.java.required_major = required;
    page.java.major_resolved = true;
    sync(&page.java, page.view(GameOptionGroup::Java));
}

/// 某个 Java 探测结束：写入缓存并刷新对应的选择行。
pub(super) fn probed(
    page: &mut GameOptionsPage,
    path: PathBuf,
    result: Result<JavaDetails, String>,
) {
    page.java.probing.remove(&path);
    let Ok(details) = result else {
        return;
    };
    page.java.cache.insert(path.clone(), details.clone());
    let Some(position) = page
        .java
        .javas
        .iter()
        .position(|entry| absolute(&entry.java.path) == path)
    else {
        return;
    };
    page.java.javas[position].details = Some(details);
    let entry = &page.java.javas[position];
    page.java.rows.send(
        position + 1,
        ChoiceRowMsg::SetInfo {
            title: java_title(entry),
            subtitle: java_path(entry),
            detail: java_detail(entry),
        },
    );
    sync(&page.java, page.view(GameOptionGroup::Java));
}

/// 探测还没缓存、也还没在探测中的 Java；只在页面可见时调用。
pub(super) fn probe_missing(group: &mut JavaGroup, sender: &ComponentSender<GameOptionsPage>) {
    if !group.visible {
        return;
    }
    let missing: Vec<PathBuf> = group
        .javas
        .iter()
        .filter(|entry| entry.details.is_none())
        .map(|entry| absolute(&entry.java.path))
        .filter(|path| !group.probing.contains(path))
        .collect();

    for path in missing {
        group.probing.insert(path.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        relm4::spawn({
            let path = path.clone();
            async move {
                let _ = tx.send(JavaDetails::probe(path).await);
            }
        });
        let sender = sender.clone();
        relm4::spawn_local(async move {
            let result = match rx.await {
                Ok(result) => result.map_err(|err| format!("{err:#}")),
                Err(_) => Err("Java 探测任务意外结束".to_string()),
            };
            notify(&sender, GameOptionsMsg::Probed { path, result });
        });
    }
}

/// 「自动选择」行的小字：实例实际会选中的 Java。
fn auto_java_subtitle(group: &JavaGroup, view: GroupView<'_>) -> String {
    if !view.allow_inherit || !group.major_resolved {
        return String::new();
    }
    let Some(required_major) = group.required_major else {
        return "无法确定所需 Java 版本".to_string();
    };
    let configs = view.config.read().javas.clone();
    let Some(java) = configured_java(required_major, &configs) else {
        return "配置中没有可用的 Java".to_string();
    };
    group
        .javas
        .iter()
        .find(|entry| same_java_path(&entry.java.path, &java.path))
        .map(java_label)
        .unwrap_or_else(|| format!("Java {}（{}）", java.major, java.arch.name()))
}

fn java_label_of(javas: &[JavaEntry], choice: &JavaChoice) -> String {
    match java_index_for_choice(javas, choice) {
        JAVA_AUTO_INDEX => "自动选择".to_string(),
        index => javas
            .get((index - 1) as usize)
            .map(java_label)
            .unwrap_or_else(|| "自动选择".to_string()),
    }
}

/// Java 可执行文件的绝对路径，显示在选择项下方。
fn java_path(entry: &JavaEntry) -> String {
    absolute(&entry.java.path).display().to_string()
}
