//! 首选项里的「全局游戏选项」分类，以及可复用到实例设置的游戏设置表单。
//!
//! 表单通过 [`GameOptionsTarget`] 读写启动配置：全局分类由 [`GlobalGameOptionsTarget`]
//! 绑定到共享配置，实例管理页用 [`InstanceGameOptionsTarget`] 把改动的字段写进实例
//! `dmcl.json` 的覆盖层。账户表和 Java 表是全局资源，表单直接从只读的 `ConfigRef` 读取。
//! 表单本身按 [`GameOptionGroup`] 拆在 `page/` 下。

mod global;
mod page;
mod target;

pub(crate) use global::GlobalGameOptionsTarget;
pub(crate) use page::{GameOptionsInit, GameOptionsMsg, GameOptionsPage};
pub(crate) use target::InstanceGameOptionsTarget;

use std::collections::BTreeSet;

use crate::config::{AccountConfig, LaunchConfig};

/// 表单一次提交的改动。
#[derive(Debug, Clone)]
pub(crate) struct GameOptionsUpdate {
    /// 表单里的完整值；只把 `fields` 里列出的字段写回目标。
    pub launch: LaunchConfig,
    /// 本次真正改过、需要写入目标的字段。
    pub fields: BTreeSet<GameOptionField>,
    /// 本次恢复继承全局、需要从目标删除的字段。
    pub inherit: BTreeSet<GameOptionField>,
}

/// 目标当前的启动配置和覆盖情况。
#[derive(Debug, Clone)]
pub(crate) struct GameOptionsState {
    /// 已经与全局合并、当前生效的启动配置。
    pub launch: LaunchConfig,
    /// 目标里被覆盖的字段；全局配置没有覆盖层，为空。
    pub overrides: BTreeSet<GameOptionField>,
}

/// 一个游戏窗口预设。
pub(crate) struct WindowPreset {
    pub width: u32,
    pub height: u32,
}

/// 游戏窗口提供的常用预设，沿用 HMCL 的挡位。
pub(crate) const WINDOW_PRESETS: &[WindowPreset] = &[
    WindowPreset {
        width: 854,
        height: 480,
    },
    WindowPreset {
        width: 1280,
        height: 720,
    },
    WindowPreset {
        width: 1600,
        height: 900,
    },
    WindowPreset {
        width: 1920,
        height: 1080,
    },
    WindowPreset {
        width: 2560,
        height: 1440,
    },
    WindowPreset {
        width: 3840,
        height: 2160,
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
    Fullscreen,
    WindowWidth,
    WindowHeight,
    QuickPlay,
    ProcessPriority,
    MinMemory,
    MaxMemory,
    CustomInfo,
    AdditionalJvm,
    JvmCompatibility,
    GameIntegrity,
    AdditionalGame,
    VersionIsolation,
}

impl GameOptionField {
    /// 实例 `dmcl.json` 的 `launch` 里的字段名。
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Account => "account",
            Self::Fullscreen => "fullscreen",
            Self::WindowWidth => "window_width",
            Self::WindowHeight => "window_height",
            Self::QuickPlay => "quick_play",
            Self::ProcessPriority => "process_priority",
            Self::MinMemory => "min_memory",
            Self::MaxMemory => "max_memory",
            Self::CustomInfo => "custom_info",
            Self::AdditionalJvm => "additional_jvm",
            Self::JvmCompatibility => "jvm_compatibility",
            Self::GameIntegrity => "game_integrity",
            Self::AdditionalGame => "additional_game",
            Self::VersionIsolation => "version_isolation",
        }
    }

    /// JSON 字段名 → 字段；不认识的字段返回 `None`。
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "java" => Self::Java,
            "account" => Self::Account,
            "fullscreen" => Self::Fullscreen,
            "window_width" => Self::WindowWidth,
            "window_height" => Self::WindowHeight,
            "quick_play" => Self::QuickPlay,
            "process_priority" => Self::ProcessPriority,
            "min_memory" => Self::MinMemory,
            "max_memory" => Self::MaxMemory,
            "custom_info" => Self::CustomInfo,
            "additional_jvm" => Self::AdditionalJvm,
            "jvm_compatibility" => Self::JvmCompatibility,
            "game_integrity" => Self::GameIntegrity,
            "additional_game" => Self::AdditionalGame,
            "version_isolation" => Self::VersionIsolation,
            _ => return None,
        })
    }
}

/// 表单里一起切换的一组字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GameOptionGroup {
    Account,
    Java,
    Window,
    QuickPlay,
    GameArgs,
    ProcessPriority,
    Memory,
    JvmArgs,
    JvmCompatibility,
    GameIntegrity,
    VersionIsolation,
}

impl GameOptionGroup {
    pub(crate) fn fields(self) -> &'static [GameOptionField] {
        match self {
            Self::Account => &[GameOptionField::Account],
            Self::Java => &[GameOptionField::Java],
            Self::Window => &[
                GameOptionField::Fullscreen,
                GameOptionField::WindowWidth,
                GameOptionField::WindowHeight,
            ],
            Self::QuickPlay => &[GameOptionField::QuickPlay],
            Self::GameArgs => &[GameOptionField::AdditionalGame],
            Self::ProcessPriority => &[GameOptionField::ProcessPriority],
            Self::Memory => &[GameOptionField::MinMemory, GameOptionField::MaxMemory],
            Self::JvmArgs => &[GameOptionField::AdditionalJvm],
            Self::JvmCompatibility => &[GameOptionField::JvmCompatibility],
            Self::GameIntegrity => &[GameOptionField::GameIntegrity],
            Self::VersionIsolation => &[GameOptionField::VersionIsolation],
        }
    }
}

/// 游戏设置表单的落点：全局配置，或某个实例的覆盖层。
pub(crate) trait GameOptionsTarget {
    /// 读取当前生效的启动配置和覆盖字段；账户 / Java 表由表单直接读全局配置。
    fn load(&self) -> GameOptionsState;

    /// 写回一次改动；保存失败由目标自己提示。
    fn save(&self, update: GameOptionsUpdate);
}

/// 把 `source` 里列出的字段复制到 `target`。
pub(crate) fn copy_fields<'a>(
    target: &mut LaunchConfig,
    source: &LaunchConfig,
    fields: impl IntoIterator<Item = &'a GameOptionField>,
) {
    for field in fields {
        match field {
            GameOptionField::Java => target.java = source.java.clone(),
            GameOptionField::Account => target.account = source.account.clone(),
            GameOptionField::Fullscreen => target.fullscreen = source.fullscreen,
            GameOptionField::WindowWidth => target.window_width = source.window_width,
            GameOptionField::WindowHeight => target.window_height = source.window_height,
            GameOptionField::QuickPlay => target.quick_play = source.quick_play.clone(),
            GameOptionField::ProcessPriority => target.process_priority = source.process_priority,
            GameOptionField::MinMemory => target.min_memory = source.min_memory,
            GameOptionField::MaxMemory => target.max_memory = source.max_memory,
            GameOptionField::CustomInfo => target.custom_info = source.custom_info.clone(),
            GameOptionField::AdditionalJvm => target.additional_jvm = source.additional_jvm.clone(),
            GameOptionField::JvmCompatibility => {
                target.jvm_compatibility = source.jvm_compatibility
            }
            GameOptionField::GameIntegrity => target.game_integrity = source.game_integrity,
            GameOptionField::AdditionalGame => {
                target.additional_game = source.additional_game.clone()
            }
            GameOptionField::VersionIsolation => {
                target.version_isolation = source.version_isolation
            }
        }
    }
}

/// 把一次改动里列出的字段写到 `launch` 上。
pub(crate) fn apply_update(launch: &mut LaunchConfig, update: &GameOptionsUpdate) {
    copy_fields(launch, &update.launch, &update.fields);
}

/// 组里任意字段被覆盖时，整组按覆盖处理。
pub(crate) fn group_overridden(
    overrides: &BTreeSet<GameOptionField>,
    group: GameOptionGroup,
) -> bool {
    group.fields().iter().any(|field| overrides.contains(field))
}

/// 没有选中账户时的下标；下拉里没有对应的行。
pub(crate) const ACCOUNT_NONE_INDEX: u32 = 0;

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
        AccountConfig::Offline { username } | AccountConfig::Microsoft { username, .. }
            if username == name =>
        {
            username.clone()
        }
        AccountConfig::Offline { username } | AccountConfig::Microsoft { username, .. } => {
            format!("{username}（{name}）")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            max_memory: Some(1024),
            ..LaunchConfig::default()
        };

        let mut changed = launch.clone();
        changed.max_memory = Some(4096);
        changed.window_width = 1280;
        changed.version_isolation = false;
        let update = GameOptionsUpdate {
            launch: changed,
            fields: [GameOptionField::MaxMemory].into_iter().collect(),
            inherit: BTreeSet::new(),
        };
        apply_update(&mut launch, &update);

        assert_eq!(launch.max_memory, Some(4096));
        assert_eq!(launch.window_width, LaunchConfig::default().window_width);
        assert!(launch.version_isolation);

        let update = GameOptionsUpdate {
            launch: LaunchConfig {
                version_isolation: false,
                ..launch.clone()
            },
            fields: [GameOptionField::VersionIsolation].into_iter().collect(),
            inherit: BTreeSet::new(),
        };
        apply_update(&mut launch, &update);
        assert!(!launch.version_isolation);
    }

    #[test]
    fn field_keys_match_the_instance_json_names() {
        let keys = [
            GameOptionField::Java,
            GameOptionField::Account,
            GameOptionField::Fullscreen,
            GameOptionField::WindowWidth,
            GameOptionField::WindowHeight,
            GameOptionField::QuickPlay,
            GameOptionField::ProcessPriority,
            GameOptionField::MinMemory,
            GameOptionField::MaxMemory,
            GameOptionField::CustomInfo,
            GameOptionField::AdditionalJvm,
            GameOptionField::JvmCompatibility,
            GameOptionField::GameIntegrity,
            GameOptionField::AdditionalGame,
            GameOptionField::VersionIsolation,
        ];
        for field in keys {
            assert_eq!(GameOptionField::from_key(field.key()), Some(field));
        }
        assert_eq!(GameOptionField::from_key("unknown"), None);

        // 字段名必须和 LaunchConfig 的序列化键一致，否则实例覆盖写不对位置。
        let config = serde_json::to_value(LaunchConfig::default()).unwrap();
        let mut serialized: Vec<&str> = config
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        serialized.sort_unstable();
        let mut fields: Vec<&str> = keys.iter().map(|field| field.key()).collect();
        fields.sort_unstable();
        assert_eq!(serialized, fields);
    }

    #[test]
    fn a_group_is_overridden_when_any_of_its_fields_is() {
        let mut overrides = BTreeSet::new();
        assert!(!group_overridden(&overrides, GameOptionGroup::Window));

        overrides.insert(GameOptionField::WindowHeight);
        assert!(group_overridden(&overrides, GameOptionGroup::Window));
        assert!(!group_overridden(&overrides, GameOptionGroup::Memory));
    }

    #[test]
    fn window_presets_match_sizes() {
        assert_eq!(window_preset(854, 480), Some(0));
        assert_eq!(window_preset(3840, 2160), Some(5));
        assert_eq!(window_preset(800, 600), None);
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
        assert_eq!(
            account_label(
                "工作",
                &AccountConfig::Microsoft {
                    username: "steve".to_string(),
                    uuid: "069a79f444e94726a5befca90e38aaf5".to_string(),
                }
            ),
            "steve（工作）"
        );
    }
}
