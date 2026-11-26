//! Java 分组：默认 Java 选择与后台探测。

use std::path::PathBuf;
use std::rc::Rc;

use dmcl_java::JavaDetails;
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::prelude::*;

use crate::config::{configured_java, same_java_path, Config, LaunchConfig};
use crate::gui::gtk4::pages::settings::common::{
    java_label, ChoiceRowOut, FixedRow, JavaChoices, JavaEntry, JavaProbeCache,
};

use super::super::{GameOptionField, GameOptionGroup};
use super::{notify, GameOptionsMsg, GameOptionsPage, GroupView};

pub(super) struct JavaGroup {
    pub(super) expander: adw::ExpanderRow,
    inherit: FixedRow,
    list: JavaChoices,
    /// 当前选中的下标；0 是「自动选择」。
    index: u32,
    /// 实例版本需要的 Java 大版本；`None` 表示读不出来。
    required_major: Option<u32>,
    /// 是否已经拿到实例版本的 Java 要求；没拿到之前不显示失败文案。
    major_resolved: bool,
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
        let list = JavaChoices::new(&expander, cache, sender.input_sender(), |out| match out {
            ChoiceRowOut::Selected(index) => GameOptionsMsg::JavaSelected(index),
        });
        Self {
            expander,
            inherit,
            list,
            index: 0,
            required_major,
            major_resolved: !allow_inherit || required_major.is_some(),
        }
    }

    pub(super) fn set_visible(&mut self, visible: bool) {
        self.list.set_visible(visible);
    }

    /// 当前 Java 表；内存分组也要按选中的 Java 架构估算。
    pub(super) fn javas(&self) -> &[JavaEntry] {
        self.list.javas()
    }
}

/// 读取全局 Java 表与探测缓存，按当前配置定位选中项并重建选择行。
pub(super) fn reload(group: &mut JavaGroup, config: &Config, launch: &LaunchConfig) {
    group.list.reload(config, "");
    group.index = group.list.index_for_choice(&launch.java);
}

/// 同步对勾、继承状态与小字。
pub(super) fn sync(group: &JavaGroup, view: GroupView<'_>) {
    group.list.sync(group.index, !view.inherited);
    group.inherit.set_selected(view.inherited);
    group
        .expander
        .set_subtitle(&view.subtitle(group.list.label_of(&view.launch.java)));
    group
        .inherit
        .set_subtitle(&group.list.label_of(&view.global.java));
    group
        .list
        .set_auto_subtitle(auto_java_subtitle(group, view));
}

/// 选中 Java：更新模型、标成覆盖并收起展开行。
pub(super) fn selected(
    page: &mut GameOptionsPage,
    index: u32,
    sender: &ComponentSender<GameOptionsPage>,
) {
    if index as usize > page.java.javas().len() {
        return;
    }
    let inherited = page.inherited(GameOptionGroup::Java);
    if index != page.java.index || inherited {
        page.java.index = index;
        page.launch.java = page.java.list.choice_for_index(index);
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
    page.java.list.apply_probe(&path, result);
    sync(&page.java, page.view(GameOptionGroup::Java));
}

/// 探测还没缓存、也还没在探测中的 Java；只在页面可见时调用。
pub(super) fn probe_missing(group: &mut JavaGroup, sender: &ComponentSender<GameOptionsPage>) {
    group
        .list
        .probe_missing(sender.input_sender(), |path, result| {
            GameOptionsMsg::Probed { path, result }
        });
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
        .javas()
        .iter()
        .find(|entry| same_java_path(&entry.java.path, &java.path))
        .map(java_label)
        .unwrap_or_else(|| format!("Java {}（{}）", java.major, java.arch.name()))
}
