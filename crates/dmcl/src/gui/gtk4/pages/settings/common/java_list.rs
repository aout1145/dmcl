//! 展开行里的 Java 选项列表。
//!
//! 游戏设置和下载设置都按全局 Java 表的顺序列出 Java。条目构造、下标与 [`JavaChoice`]
//! 的映射、探测调度和结果回填放在这里，保证两处显示同样的标题、路径和右侧信息。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use dmcl_java::JavaDetails;
use dmcl_mcmeta::layout::absolute;
use relm4::adw;
use relm4::prelude::*;

use crate::config::{Config, JavaChoice, JavaConfig};

use super::{
    details_arch_text, details_title, java_summary, ChoiceRow, ChoiceRowInit, ChoiceRowMsg,
    ChoiceRowOut, JavaProbeCache,
};

/// 一个可选的 Java：配置项、内部名，以及探测到的完整信息。
#[derive(Debug, Clone)]
pub(crate) struct JavaEntry {
    pub name: String,
    pub java: JavaConfig,
    pub details: Option<JavaDetails>,
}

/// 「自动选择」在选项列表里的下标。
const JAVA_AUTO_INDEX: u32 = 0;

/// 一个展开行里的 Java 选项列表。
pub(crate) struct JavaChoices {
    rows: FactoryVecDeque<ChoiceRow>,
    cache: Rc<JavaProbeCache>,
    javas: Vec<JavaEntry>,
    /// 正在探测的 Java 路径，避免重复执行。
    probing: HashSet<PathBuf>,
    /// 页面可见时才发起新的探测。
    visible: bool,
}

impl JavaChoices {
    /// 在 `expander` 里建立选项行；选中项通过 `input` 转发给调用方。
    pub(crate) fn new<M: Send + 'static>(
        expander: &adw::ExpanderRow,
        cache: Rc<JavaProbeCache>,
        input: &relm4::Sender<M>,
        map: impl Fn(ChoiceRowOut) -> M + Send + 'static,
    ) -> Self {
        let rows = FactoryVecDeque::builder()
            .launch(expander.clone())
            .forward(input, map);
        Self {
            rows,
            cache,
            javas: Vec::new(),
            probing: HashSet::new(),
            visible: false,
        }
    }

    /// 按全局 Java 表重建选项；`auto_subtitle` 是「自动选择」行下方的小字。
    pub(crate) fn reload(&mut self, config: &Config, auto_subtitle: &str) {
        self.javas = config
            .javas
            .iter()
            .map(|(name, java)| JavaEntry {
                name: name.clone(),
                java: java.clone(),
                details: self.cache.get(&java.path),
            })
            .collect();
        let paths: Vec<PathBuf> = self
            .javas
            .iter()
            .map(|entry| entry.java.path.clone())
            .collect();
        self.cache.retain(&paths);
        self.probing
            .retain(|path| paths.iter().any(|candidate| absolute(candidate) == *path));
        self.rebuild_rows(auto_subtitle);
    }

    pub(crate) fn javas(&self) -> &[JavaEntry] {
        &self.javas
    }

    pub(crate) fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// 刷新对勾；`selected` 为 false 时整列都不选中（例如整组继承全局）。
    pub(crate) fn sync(&self, index: u32, selected: bool) {
        for position in 0..self.rows.len() {
            self.rows.send(
                position,
                ChoiceRowMsg::SetSelected(selected && position as u32 == index),
            );
        }
    }

    /// 改写「自动选择」行下方的小字。
    pub(crate) fn set_auto_subtitle(&self, subtitle: String) {
        self.rows.send(
            JAVA_AUTO_INDEX as usize,
            ChoiceRowMsg::SetInfo {
                title: "自动选择".to_string(),
                subtitle,
                detail: String::new(),
            },
        );
    }

    /// `JavaChoice` → 选项下标；指向不存在的 Java 时回退到「自动选择」。
    pub(crate) fn index_for_choice(&self, choice: &JavaChoice) -> u32 {
        index_for_choice(&self.javas, choice)
    }

    /// 选项下标 → `JavaChoice`。
    pub(crate) fn choice_for_index(&self, index: u32) -> JavaChoice {
        choice_for_index(&self.javas, index)
    }

    /// 收起展开行时显示的一行摘要。
    pub(crate) fn label_of(&self, choice: &JavaChoice) -> String {
        match index_for_choice(&self.javas, choice) {
            JAVA_AUTO_INDEX => "自动选择".to_string(),
            index => self
                .javas
                .get((index - 1) as usize)
                .map(java_label)
                .unwrap_or_else(|| "自动选择".to_string()),
        }
    }

    /// 探测还没缓存、也还没在探测中的 Java；只在页面可见时调用。
    pub(crate) fn probe_missing<M: Send + 'static>(
        &mut self,
        input: &relm4::Sender<M>,
        message: fn(PathBuf, Result<JavaDetails, String>) -> M,
    ) {
        if !self.visible {
            return;
        }
        let missing: Vec<PathBuf> = self
            .javas
            .iter()
            .filter(|entry| entry.details.is_none())
            .map(|entry| absolute(&entry.java.path))
            .filter(|path| !self.probing.contains(path))
            .collect();

        for path in missing {
            self.probing.insert(path.clone());
            spawn_probe(path, input.clone(), message);
        }
    }

    /// 记录一次探测结果：成功时写缓存并刷新对应选项，失败时允许以后重试。
    pub(crate) fn apply_probe(&mut self, path: &Path, result: Result<JavaDetails, String>) {
        self.probing.remove(path);
        let Ok(details) = result else {
            return;
        };
        self.cache.insert(path.to_path_buf(), details.clone());
        let Some(position) = self
            .javas
            .iter()
            .position(|entry| absolute(&entry.java.path) == absolute(path))
        else {
            return;
        };
        self.javas[position].details = Some(details);
        let init = row_init(position as u32 + 1, &self.javas[position]);
        self.rows.send(
            position + 1,
            ChoiceRowMsg::SetInfo {
                title: init.title,
                subtitle: init.subtitle,
                detail: init.detail,
            },
        );
    }

    fn rebuild_rows(&mut self, auto_subtitle: &str) {
        let mut guard = self.rows.guard();
        guard.clear();
        guard.push_back(ChoiceRowInit {
            index: JAVA_AUTO_INDEX,
            title: "自动选择".to_string(),
            subtitle: auto_subtitle.to_string(),
            detail: String::new(),
            avatar_text: None,
            selected: false,
        });
        for (offset, entry) in self.javas.iter().enumerate() {
            guard.push_back(row_init(offset as u32 + 1, entry));
        }
    }
}

/// 在后台探测一个 Java，结束后把结果发回页面。
fn spawn_probe<M: Send + 'static>(
    path: PathBuf,
    input: relm4::Sender<M>,
    message: fn(PathBuf, Result<JavaDetails, String>) -> M,
) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    relm4::spawn({
        let path = path.clone();
        async move {
            let _ = tx.send(JavaDetails::probe(path).await);
        }
    });
    relm4::spawn_local(async move {
        let result = match rx.await {
            Ok(result) => result.map_err(|err| format!("{err:#}")),
            Err(_) => Err("Java 探测任务意外结束".to_string()),
        };
        let _ = input.send(message(path, result));
    });
}

fn row_init(index: u32, entry: &JavaEntry) -> ChoiceRowInit {
    ChoiceRowInit {
        index,
        title: java_title(entry),
        subtitle: java_path(entry),
        detail: java_detail(entry),
        avatar_text: None,
        selected: false,
    }
}

/// 选项的主标题：优先用探测结果，没探测到时退回配置里的大版本。
fn java_title(entry: &JavaEntry) -> String {
    match &entry.details {
        Some(details) => details_title(details),
        None => format!("Java {}", entry.java.major),
    }
}

/// 选项右侧的小字：供应商与架构；没探测到时只显示架构。
fn java_detail(entry: &JavaEntry) -> String {
    match &entry.details {
        Some(details) => details_arch_text(details),
        None => entry.java.arch.name().to_string(),
    }
}

/// 选项的一行摘要，用在收起状态的副标题里。
pub(crate) fn java_label(entry: &JavaEntry) -> String {
    match &entry.details {
        Some(details) => java_summary(details),
        None => format!("Java {}（{}）", entry.java.major, entry.java.arch.name()),
    }
}

/// Java 可执行文件的绝对路径，显示在选项下方。
fn java_path(entry: &JavaEntry) -> String {
    absolute(&entry.java.path).display().to_string()
}

fn index_for_choice(javas: &[JavaEntry], choice: &JavaChoice) -> u32 {
    match choice {
        JavaChoice::Auto => JAVA_AUTO_INDEX,
        JavaChoice::Named(name) => javas
            .iter()
            .position(|entry| &entry.name == name)
            .map(|index| index as u32 + 1)
            .unwrap_or(JAVA_AUTO_INDEX),
    }
}

fn choice_for_index(javas: &[JavaEntry], index: u32) -> JavaChoice {
    index
        .checked_sub(1)
        .and_then(|index| javas.get(index as usize))
        .map(|entry| JavaChoice::Named(entry.name.clone()))
        .unwrap_or(JavaChoice::Auto)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dmcl_java::{JavaDetails, JavaOption};
    use dmcl_mcmeta::os::Arch;

    use super::*;

    fn java(name: &str, major: u32) -> JavaEntry {
        JavaEntry {
            name: name.to_string(),
            java: JavaConfig {
                path: PathBuf::from(format!("/jvm/{name}/bin/java")),
                major,
                arch: Arch::X86_64,
            },
            details: None,
        }
    }

    #[test]
    fn index_and_choice_round_trip_and_fall_back_to_auto() {
        let javas = vec![java("a", 17), java("b", 21)];

        assert_eq!(index_for_choice(&javas, &JavaChoice::Auto), 0);
        assert_eq!(
            index_for_choice(&javas, &JavaChoice::Named("b".to_string())),
            2
        );
        assert_eq!(choice_for_index(&javas, 1), JavaChoice::Named("a".into()));
        assert_eq!(choice_for_index(&javas, 99), JavaChoice::Auto);

        assert_eq!(
            index_for_choice(&javas, &JavaChoice::Named("missing".to_string())),
            0
        );
    }

    #[test]
    fn labels_use_probe_details_and_fall_back_otherwise() {
        let mut entry = java("a", 17);
        assert_eq!(java_title(&entry), "Java 17");
        assert_eq!(java_detail(&entry), "x86_64");
        assert_eq!(java_label(&entry), "Java 17（x86_64）");

        entry.details = Some(JavaDetails {
            option: JavaOption {
                path: entry.java.path.clone(),
                major: 17,
                arch: Arch::X86_64,
            },
            type_: "openjdk".to_string(),
            version_str: "17.0.2".to_string(),
            vendor: "Microsoft".to_string(),
            is_jdk: true,
        });
        assert_eq!(java_title(&entry), "JDK 17.0.2");
        assert_eq!(java_detail(&entry), "Microsoft, x86_64");
        assert_eq!(java_label(&entry), "JDK 17.0.2, Microsoft, x86_64");
    }
}
