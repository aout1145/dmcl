//! `accounts.*` 段：默认账户、解析、校验与取用。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_launch::{AccountSource, MicrosoftAccountSource, OfflineAccountSource, RefreshTokenStore};
use dmcl_task::BoxFuture;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Config, ConfigWarning, LaunchConfig};

/// 系统钥匙串里区分 dmcl 凭据的服务名；用户名是配置里的账户键。
const KEYRING_SERVICE: &str = "dmcl";

/// 一个账户条目。
///
/// 微软账户只把玩家名与 uuid 写进配置；refresh token 在系统钥匙串里，按账户键存取。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccountConfig {
    Offline { username: String },
    Microsoft { username: String, uuid: String },
}

impl AccountConfig {
    /// 把这个配置条目变成一个登录源。
    ///
    /// `name` 是配置里的账户键：微软账户用它定位钥匙串里的 refresh token。
    pub fn source(&self, name: &str) -> Arc<dyn AccountSource> {
        match self {
            Self::Offline { username } => Arc::new(OfflineAccountSource::new(username.clone())),
            Self::Microsoft { .. } => Arc::new(MicrosoftAccountSource::new(
                name,
                Arc::new(KeyringTokenStore::new(name)),
            )),
        }
    }
}

/// 用系统钥匙串保存某个微软账户的 refresh token。
///
/// 钥匙串调用是阻塞的，放进 `spawn_blocking`，不占异步运行时的工作线程。
pub(crate) struct KeyringTokenStore {
    name: String,
}

impl KeyringTokenStore {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// 在开始交互式登录前确认系统凭据库可用，免得用户走完浏览器流程才发现存不了。
    pub(crate) async fn ensure_available() -> Result<()> {
        tokio::task::spawn_blocking(|| match keyring::Entry::store_status() {
            Ok(()) => Ok(()),
            Err(err) => Err(keyring_unavailable(err)),
        })
        .await
        .map_err(|err| anyhow!("钥匙串操作异常终止：{err}"))?
    }
}

impl RefreshTokenStore for KeyringTokenStore {
    fn load<'a>(&'a self) -> BoxFuture<'a, Result<Option<String>>> {
        let name = self.name.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let entry = keyring_entry(&name)?;
                match entry.get_password() {
                    Ok(token) => Ok(Some(token)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(err) => Err(anyhow!("无法读取系统钥匙串中的登录凭据：{err}")),
                }
            })
            .await
            .map_err(|err| anyhow!("钥匙串操作异常终止：{err}"))?
        })
    }

    fn save<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<()>> {
        let name = self.name.clone();
        let token = token.to_string();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let entry = keyring_entry(&name)?;
                entry
                    .set_password(&token)
                    .map_err(|err| anyhow!("无法写入系统钥匙串：{err}"))
            })
            .await
            .map_err(|err| anyhow!("钥匙串操作异常终止：{err}"))?
        })
    }
}

/// 打开钥匙串条目；没有可用的系统凭据库时给一条能照着排查的错误。
fn keyring_entry(name: &str) -> Result<keyring::Entry> {
    if let Err(err) = keyring::Entry::store_status() {
        return Err(keyring_unavailable(err));
    }
    keyring::Entry::new(KEYRING_SERVICE, name).map_err(|err| {
        anyhow!(
            "无法访问系统钥匙串：{err}。请确认系统凭据服务可用（Linux 需要 Secret Service，例如 gnome-keyring 或 KWallet）"
        )
    })
}

fn keyring_unavailable(err: impl std::fmt::Display) -> anyhow::Error {
    anyhow!(
        "系统钥匙串不可用：{err}。请确认系统凭据服务已启动（Linux 需要 Secret Service，例如 gnome-keyring 或 KWallet）"
    )
}

/// 未指定账户键时，按玩家名生成一个不冲突的键。
///
/// 已经存在同名账户时依次尝试 `玩家名-2`、`玩家名-3`…，绝不覆盖已有条目。
pub(crate) fn unique_account_name(
    accounts: &IndexMap<String, AccountConfig>,
    base: &str,
) -> String {
    if !accounts.contains_key(base) {
        return base.to_string();
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !accounts.contains_key(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

/// 默认账户表：空的。账户要么由 `dmcl login` 创建，要么由用户自己写进配置。
pub(super) fn default_accounts() -> IndexMap<String, AccountConfig> {
    IndexMap::new()
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
        Ok(account.source(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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
    fn microsoft_accounts_round_trip_and_use_the_config_key_as_source_name() {
        let uuid = "069a79f444e94726a5befca90e38aaf5";
        let text = format!(
            r#"{{
                "launch": {{ "account": "steve" }},
                "accounts": {{
                    "steve": {{ "type": "microsoft", "username": "Steve", "uuid": "{uuid}" }}
                }}
            }}"#
        );
        let (config, warnings) = Config::parse(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        let account = &config.accounts["steve"];
        assert_eq!(
            account,
            &AccountConfig::Microsoft {
                username: "Steve".to_string(),
                uuid: uuid.to_string(),
            }
        );

        // 构造登录源不访问钥匙串：只在真正 login / refresh 时才读 token。
        let source = config.selected_account_source(&config.launch).unwrap();
        assert_eq!(source.name(), "steve");

        let saved = serde_json::to_string(&config).unwrap();
        let (reloaded, warnings) = Config::parse(&saved);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(&reloaded.accounts["steve"], account);
    }

    #[test]
    fn unique_account_names_never_overwrite_existing_entries() {
        let mut accounts = IndexMap::new();
        assert_eq!(unique_account_name(&accounts, "Steve"), "Steve");

        let offline = AccountConfig::Offline {
            username: "Steve".to_string(),
        };
        accounts.insert("Steve".to_string(), offline.clone());
        assert_eq!(unique_account_name(&accounts, "Steve"), "Steve-2");

        accounts.insert("Steve-2".to_string(), offline);
        assert_eq!(unique_account_name(&accounts, "Steve"), "Steve-3");
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
        assert_eq!(config.accounts.len(), 1);
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
    fn empty_account_table_clears_the_selection() {
        let (config, warnings) =
            Config::parse(r#"{"launch": {"account": "player"}, "accounts": {}}"#);
        assert!(config.accounts.is_empty());
        assert_eq!(config.launch.account, None);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("launch.account"));
    }

    #[test]
    fn selected_account_source_requires_an_explicit_valid_selection() {
        let mut config = Config::default();
        config.accounts.insert(
            "player".to_string(),
            AccountConfig::Offline {
                username: "player".to_string(),
            },
        );
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
