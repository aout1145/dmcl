//! 首选项里的「全局游戏选项」分类，以及可复用到实例设置的游戏设置表单。
//!
//! 表单通过 [`GameOptionsTarget`] 读写启动配置：全局分类由 [`GlobalGameOptionsTarget`]
//! 绑定到共享配置；实例管理页以后实现自己的 target，把改动的字段写进实例 `dmcl.json`
//! 的覆盖层。账户表和 Java 表是全局资源，表单直接从只读的 `ConfigRef` 读取。

mod choice;
mod global;
mod page;

pub(crate) use global::GlobalGameOptionsTarget;
pub(crate) use page::{GameOptionsInit, GameOptionsMsg, GameOptionsPage};

use std::collections::BTreeSet;

use dmcl_launch::JavaDetails;

use crate::config::{AccountConfig, JavaChoice, JavaConfig, LaunchConfig};
use crate::gui::gtk4::pages::settings::common::{details_arch_text, details_title, java_summary};

/// 表单一次提交的改动。
#[derive(Debug, Clone)]
pub(crate) struct GameOptionsUpdate {
    /// 表单里的完整值；只把 `fields` 里列出的字段写回目标。
    pub launch: LaunchConfig,
    /// 本次真正改过的字段。
    pub fields: BTreeSet<GameOptionField>,
}

/// 一个游戏窗口预设。
pub(crate) struct WindowPreset {
    /// 展开行里的大字描述。
    pub label: &'static str,
    pub width: u32,
    pub height: u32,
}

/// 游戏窗口提供的常用预设。
pub(crate) const WINDOW_PRESETS: &[WindowPreset] = &[
    WindowPreset {
        label: "默认",
        width: 854,
        height: 480,
    },
    WindowPreset {
        label: "720p",
        width: 1280,
        height: 720,
    },
    WindowPreset {
        label: "1080p",
        width: 1920,
        height: 1080,
    },
];

/// 当前宽高匹配的预设下标；不是任何预设时返回 `None`。
pub(crate) fn window_preset(width: u32, height: u32) -> Option<usize> {
    WINDOW_PRESETS
        .iter()
        .position(|preset| preset.width == width && preset.height == height)
}

/// 可以单独写回的启动配置字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GameOptionField {
    Java,
    Account,
    WindowWidth,
    WindowHeight,
    MinMemory,
    MaxMemory,
    CustomInfo,
    AdditionalJvm,
    AdditionalGame,
}

/// 游戏设置表单的落点：全局配置，或某个实例的覆盖层。
pub(crate) trait GameOptionsTarget {
    /// 读取当前生效的启动配置；账户 / Java 表由表单直接读全局配置。
    fn load(&self) -> LaunchConfig;

    /// 写回一次改动；保存失败由目标自己提示。
    fn save(&self, update: GameOptionsUpdate);
}

/// 把一次改动里列出的字段写到 `launch` 上。
pub(crate) fn apply_update(launch: &mut LaunchConfig, update: &GameOptionsUpdate) {
    for field in &update.fields {
        match field {
            GameOptionField::Java => launch.java = update.launch.java.clone(),
            GameOptionField::Account => launch.account = update.launch.account.clone(),
            GameOptionField::WindowWidth => launch.window_width = update.launch.window_width,
            GameOptionField::WindowHeight => launch.window_height = update.launch.window_height,
            GameOptionField::MinMemory => launch.min_memory = update.launch.min_memory,
            GameOptionField::MaxMemory => launch.max_memory = update.launch.max_memory,
            GameOptionField::CustomInfo => launch.custom_info = update.launch.custom_info.clone(),
            GameOptionField::AdditionalJvm => {
                launch.additional_jvm = update.launch.additional_jvm.clone();
            }
            GameOptionField::AdditionalGame => {
                launch.additional_game = update.launch.additional_game.clone();
            }
        }
    }
}

/// 一个可选的 Java：配置项、内部名，以及探测到的完整信息。
#[derive(Debug, Clone)]
pub(crate) struct JavaEntry {
    pub name: String,
    pub java: JavaConfig,
    pub details: Option<JavaDetails>,
}

/// 「自动选择」在下拉里的下标。
pub(crate) const JAVA_AUTO_INDEX: u32 = 0;

/// 「未选择」账户在下拉里的下标。
pub(crate) const ACCOUNT_NONE_INDEX: u32 = 0;

/// 下拉下标 → `launch.java`。
pub(crate) fn java_choice_for_index(javas: &[JavaEntry], index: u32) -> JavaChoice {
    match index
        .checked_sub(1)
        .and_then(|index| javas.get(index as usize))
    {
        Some(entry) => JavaChoice::Named(entry.name.clone()),
        None => JavaChoice::Auto,
    }
}

/// `launch.java` → 下拉下标；指向不存在的 Java 时回退到「自动选择」。
pub(crate) fn java_index_for_choice(javas: &[JavaEntry], choice: &JavaChoice) -> u32 {
    match choice {
        JavaChoice::Auto => JAVA_AUTO_INDEX,
        JavaChoice::Named(name) => javas
            .iter()
            .position(|entry| &entry.name == name)
            .map(|index| index as u32 + 1)
            .unwrap_or(JAVA_AUTO_INDEX),
    }
}

/// 下拉下标 → `launch.account`。
pub(crate) fn account_for_index(
    accounts: &[(String, AccountConfig)],
    index: u32,
) -> Option<String> {
    index
        .checked_sub(1)
        .and_then(|index| accounts.get(index as usize))
        .map(|(name, _)| name.clone())
}

/// `launch.account` → 下拉下标。
pub(crate) fn account_index_for(
    accounts: &[(String, AccountConfig)],
    account: &Option<String>,
) -> u32 {
    match account {
        None => ACCOUNT_NONE_INDEX,
        Some(name) => accounts
            .iter()
            .position(|(key, _)| key == name)
            .map(|index| index as u32 + 1)
            .unwrap_or(ACCOUNT_NONE_INDEX),
    }
}

/// 账户下拉里的显示文字；内部键与用户名不同时一并显示，便于区分重名账户。
pub(crate) fn account_label(name: &str, account: &AccountConfig) -> String {
    match account {
        AccountConfig::Offline { username } if username == name => username.clone(),
        AccountConfig::Offline { username } => format!("{username}（{name}）"),
    }
}

/// Java 选择项的主标题：优先用探测结果，没探测到时退回配置里的大版本。
pub(crate) fn java_title(entry: &JavaEntry) -> String {
    match &entry.details {
        Some(details) => details_title(details),
        None => format!("Java {}", entry.java.major),
    }
}

/// Java 选择项右侧的小字：供应商与架构；没探测到时只显示架构。
pub(crate) fn java_detail(entry: &JavaEntry) -> String {
    match &entry.details {
        Some(details) => details_arch_text(details),
        None => entry.java.arch.name().to_string(),
    }
}

/// Java 选择器的当前值摘要，用在收起状态的副标题里。
pub(crate) fn java_label(entry: &JavaEntry) -> String {
    match &entry.details {
        Some(details) => java_summary(details),
        None => format!("Java {}（{}）", entry.java.major, entry.java.arch.name()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dmcl_launch::{JavaDetails, JavaOption};
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

    fn account(name: &str) -> (String, AccountConfig) {
        (
            name.to_string(),
            AccountConfig::Offline {
                username: name.to_string(),
            },
        )
    }

    #[test]
    fn apply_update_only_touches_listed_fields() {
        let mut launch = LaunchConfig {
            max_memory: 1024,
            ..LaunchConfig::default()
        };

        let mut changed = launch.clone();
        changed.max_memory = 4096;
        changed.window_width = 1280;
        let update = GameOptionsUpdate {
            launch: changed,
            fields: [GameOptionField::MaxMemory].into_iter().collect(),
        };
        apply_update(&mut launch, &update);

        assert_eq!(launch.max_memory, 4096);
        assert_eq!(launch.window_width, LaunchConfig::default().window_width);
    }

    #[test]
    fn window_presets_match_sizes() {
        assert_eq!(window_preset(854, 480), Some(0));
        assert_eq!(window_preset(1920, 1080), Some(2));
        assert_eq!(window_preset(800, 600), None);
    }

    #[test]
    fn java_combo_round_trips_and_falls_back_to_auto() {
        let javas = vec![java("a", 17), java("b", 21)];

        assert_eq!(java_index_for_choice(&javas, &JavaChoice::Auto), 0);
        assert_eq!(
            java_index_for_choice(&javas, &JavaChoice::Named("b".to_string())),
            2
        );
        assert_eq!(
            java_choice_for_index(&javas, 1),
            JavaChoice::Named("a".into())
        );
        assert_eq!(java_choice_for_index(&javas, 99), JavaChoice::Auto);

        assert_eq!(
            java_index_for_choice(&javas, &JavaChoice::Named("missing".to_string())),
            0
        );
    }

    #[test]
    fn account_combo_round_trips_and_handles_no_selection() {
        let accounts = vec![account("player"), account("steve")];

        assert_eq!(account_index_for(&accounts, &None), 0);
        assert_eq!(account_index_for(&accounts, &Some("steve".to_string())), 2);
        assert_eq!(account_for_index(&accounts, 1).as_deref(), Some("player"));
        assert_eq!(account_for_index(&accounts, 99), None);
        assert_eq!(
            account_index_for(&accounts, &Some("missing".to_string())),
            0
        );
    }

    #[test]
    fn account_labels_include_the_key_only_when_it_differs() {
        assert_eq!(account_label("player", &account("player").1), "player");
        assert_eq!(
            account_label(
                "工作",
                &AccountConfig::Offline {
                    username: "steve".to_string()
                }
            ),
            "steve（工作）"
        );
    }

    #[test]
    fn java_labels_use_probe_details_and_fall_back_otherwise() {
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
