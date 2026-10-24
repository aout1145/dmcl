//! `accounts.*` 段。

use serde::{Deserialize, Serialize};

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
