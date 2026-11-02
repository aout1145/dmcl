//! 账户列表共用的展示文案与占位头像。

use crate::config::AccountConfig;

/// 账户的玩家名，充当列表里的 ID。
pub(crate) fn account_username(account: &AccountConfig) -> &str {
    match account {
        AccountConfig::Offline { username } | AccountConfig::Microsoft { username, .. } => username,
    }
}

/// 账户类型，显示在 ID 下方。
pub(crate) fn account_type_label(account: &AccountConfig) -> &'static str {
    match account {
        AccountConfig::Offline { .. } => "离线账户",
        AccountConfig::Microsoft { .. } => "微软账户",
    }
}

/// 占位头像的文字：玩家名的首字母；名字为空时用问号。
pub(crate) fn account_avatar_text(username: &str) -> String {
    username
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_string())
}
