//! 账户抽象。

use sha1::{Digest, Sha1};

/// 一个可以被启动器使用的账户。
///
/// 做成 trait 是为了以后能加微软 / Yggdrasil 登录，而不用动启动逻辑。
pub trait Account: Send + Sync {
    /// 送给游戏的玩家名（`${auth_player_name}`）。
    fn get_name(&self) -> &str;
    /// 玩家 uuid（`${auth_uuid}` / `${uuid}`）。
    fn get_uuid(&self) -> String;
    /// 会话令牌（`${auth_access_token}` / `${auth_session}`）。
    fn get_access_token(&self) -> String;
    /// 账户类型（`${user_type}`）。
    fn get_type(&self) -> &str;
}

/// 离线（未登录）账户。
#[derive(Debug, Clone)]
pub struct OfflineAccount {
    pub name: String,
}

impl OfflineAccount {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Account for OfflineAccount {
    fn get_name(&self) -> &str {
        &self.name
    }

    fn get_uuid(&self) -> String {
        sha1_uuid(&self.name)
    }

    fn get_access_token(&self) -> String {
        String::new()
    }

    fn get_type(&self) -> &str {
        "legacy"
    }
}

/// `std.uuid.sha1UUID(name).toString()`：由 `sha1(name)` 前 16 字节构造的
/// 基于名字的（version 5）RFC-4122 uuid。
///
/// 原 D 实现手工翻转 version / variant 位并手拼十六进制；这里交给
/// `uuid::Builder::from_sha1_bytes`，对任何名字产生的字符串都逐字节相同。
pub fn sha1_uuid(name: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[0..16]);
    uuid::Builder::from_sha1_bytes(bytes)
        .into_uuid()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_is_deterministic_and_shaped_like_v5() {
        let uuid = sha1_uuid("player");
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "5"); // version 5
                                        // RFC-4122 variant：该半字节是 0b10xx，即 8/9/a/b 之一
        assert!(
            ["8", "9", "a", "b"].contains(&&uuid[19..20]),
            "variant nibble 不对: {uuid}"
        );
        assert_eq!(uuid, sha1_uuid("player"));
        assert_ne!(uuid, sha1_uuid("other"));
    }

    /// 原 D 算法（`sha1(name)[0..16]` 再加 version/variant 位）留下的黄金值，
    /// 用来证明换成 `uuid::Builder` 之后逐字节没变。
    #[test]
    fn offline_uuid_matches_the_d_implementation_byte_for_byte() {
        assert_eq!(sha1_uuid("player"), "889c6853-a117-5ca8-bef9-d6523335dc06");
        assert_eq!(sha1_uuid("Steve"), "2121f63f-a284-5c8a-9ce4-00bb6f83824f");
        assert_eq!(sha1_uuid(""), "da39a3ee-5e6b-5b0d-b255-bfef95601890");
    }

    #[test]
    fn offline_accounts_fill_every_template() {
        let account = OfflineAccount::new("player");
        assert_eq!(account.get_name(), "player");
        assert_eq!(account.get_uuid(), sha1_uuid("player"));
        assert_eq!(account.get_access_token(), "");
        assert_eq!(account.get_type(), "legacy");
    }
}
