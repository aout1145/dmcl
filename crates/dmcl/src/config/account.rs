//! `account.*` 段。

use serde::{Deserialize, Serialize};

/// `account.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountConfig {
    /// 仍未使用的配置项：账号目前只有离线实现。
    pub online_mode: bool,
    pub username: String,
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            online_mode: false,
            username: "player".to_string(),
        }
    }
}
