//! `accounts.*` 段：默认账户、解析、校验与取用。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_launch::{AccountSource, OfflineAccountSource};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Config, ConfigWarning, LaunchConfig};

/// 一个账户条目。
///
/// 目前只有离线账户；以后加微软 / Yggdrasil 登录时在这里增加变体即可。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccountConfig {
    Offline { username: String },
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self::Offline {
            username: "player".to_string(),
        }
    }
}

impl AccountConfig {
    /// 把这个配置条目变成一个登录源。
    ///
    /// 在线账户以后在这里返回对应的 `AccountSource`，配置层不需要知道登录细节。
    pub fn source(&self) -> Arc<dyn AccountSource> {
        match self {
            Self::Offline { username } => Arc::new(OfflineAccountSource::new(username.clone())),
        }
    }
}

/// 默认账户表。
pub(super) fn default_accounts() -> IndexMap<String, AccountConfig> {
    IndexMap::from([("player".to_string(), AccountConfig::default())])
}

/// 解析 `accounts` 表：条目逐个校验，坏条目丢弃；表整体替换默认表。
pub(super) fn parse_accounts(
    body: Value,
    warnings: &mut Vec<ConfigWarning>,
) -> IndexMap<String, AccountConfig> {
    super::parse_table("accounts", body, default_accounts(), warnings)
}

/// `launch.account` 指向不存在的账户时回退为 `null`。
pub(super) fn validate_selection(
    launch: &mut LaunchConfig,
    accounts: &IndexMap<String, AccountConfig>,
) -> Vec<ConfigWarning> {
    let mut warnings = Vec::new();

    if let Some(name) = launch.account.clone() {
        if !accounts.contains_key(&name) {
            launch.account = None;
            warnings.push(ConfigWarning {
                message: format!("配置项「launch.account」指向的账号不存在：{name}，已改为 null"),
                detail: "请编辑配置选择已有账号".to_string(),
            });
        }
    }

    warnings
}

impl Config {
    /// 按 `launch.account` 取一个账户登录源。
    ///
    /// 没有选择账户（`null`）或指向不存在的账户时返回错误，不静默使用默认账户。
    pub fn selected_account_source(&self, launch: &LaunchConfig) -> Result<Arc<dyn AccountSource>> {
        let name = launch
            .account
            .as_deref()
            .ok_or_else(|| anyhow!("实例没有选择账户（launch.account 为 null），请编辑配置"))?;
        let account = self
            .accounts
            .get(name)
            .ok_or_else(|| anyhow!("配置项「launch.account」指向的账号不存在：{name}"))?;
        Ok(account.source())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn default_accounts_contain_player() {
        let config = Config::default();
        assert_eq!(
            config.accounts.get("player"),
            Some(&AccountConfig::Offline {
                username: "player".to_string()
            })
        );
    }

    #[test]
    fn accounts_keep_the_file_order() {
        let text = r#"{
            "launch": { "account": "a" },
            "accounts": {
                "b": { "type": "offline", "username": "B" },
                "a": { "type": "offline", "username": "A" }
            }
        }"#;
        let (config, warnings) = Config::parse(text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            config
                .accounts
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );

        let saved = serde_json::to_string_pretty(&config).unwrap();
        let (reloaded, warnings) = Config::parse(&saved);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            reloaded
                .accounts
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
    }

    #[test]
    fn bad_account_entries_are_dropped() {
        let (config, warnings) = Config::parse(
            r#"{
                "launch": { "account": "steve" },
                "accounts": {
                    "steve": { "type": "offline", "username": "Steve" },
                    "missing_name": { "type": "offline" },
                    "bad_type": { "type": "online", "username": "x" }
                }
            }"#,
        );
        assert!(config.accounts.contains_key("steve"));
        assert!(!config.accounts.contains_key("player"));
        assert!(!config.accounts.contains_key("missing_name"));
        assert!(!config.accounts.contains_key("bad_type"));
        for key in ["accounts.missing_name", "accounts.bad_type"] {
            assert!(
                warnings.iter().any(|warning| warning.message.contains(key)),
                "缺少 {key} 的警告: {warnings:?}"
            );
        }
    }

    #[test]
    fn empty_account_table_stays_empty_and_clears_the_selection() {
        let (config, warnings) = Config::parse(r#"{"accounts": {}}"#);
        assert!(config.accounts.is_empty());
        assert_eq!(config.launch.account, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch.account"));
    }

    #[test]
    fn selected_account_source_requires_an_explicit_valid_selection() {
        let config = Config::default();
        let mut launch = config.launch.clone();
        launch.account = None;
        assert!(config.selected_account_source(&launch).is_err());

        launch.account = Some("nope".to_string());
        let err = config
            .selected_account_source(&launch)
            .err()
            .expect("应当报错");
        assert!(err.to_string().contains("账号不存在：nope"));

        launch.account = Some("player".to_string());
        assert_eq!(
            config.selected_account_source(&launch).unwrap().name(),
            "player"
        );
    }
}
