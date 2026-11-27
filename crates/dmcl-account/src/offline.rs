//! 离线（未登录）账户。

use anyhow::Result;
use md5::{Digest, Md5};

use crate::account::{Account, AccountSource};

/// 离线（未登录）账户。
#[derive(Debug, Clone)]
pub struct OfflineAccountSource {
    pub username: String,
}

impl OfflineAccountSource {
    pub fn new(username: impl Into<String>) -> Self {
        Self {
            username: username.into(),
        }
    }

    /// 离线账户的快照规则都在这里：uuid 由名字派生，token 为空，类型是 legacy。
    fn account(&self) -> Account {
        Account {
            uuid: Self::offline_uuid(&self.username),
            username: self.username.clone(),
            access_token: String::new(),
            user_type: "legacy".to_string(),
            xuid: String::new(),
        }
    }

    /// 由 `md5("OfflinePlayer:" + name)` 构造的、基于名字的（version 3）RFC-4122 uuid。
    ///
    /// 与官方启动器、服务端的离线 UUID 算法一致：换启动器或换离线服务端时，玩家存档
    /// 与插件数据仍能对上同一个玩家。
    fn offline_uuid(name: &str) -> String {
        let mut hasher = Md5::new();
        hasher.update(b"OfflinePlayer:");
        hasher.update(name.as_bytes());
        let digest = hasher.finalize();

        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[0..16]);
        uuid::Builder::from_md5_bytes(bytes).into_uuid().to_string()
    }
}

#[async_trait::async_trait]
impl AccountSource for OfflineAccountSource {
    fn name(&self) -> &str {
        &self.username
    }

    async fn login(&self) -> Result<Account> {
        Ok(self.account())
    }

    async fn refresh(&self) -> Result<Account> {
        Ok(self.account())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_is_deterministic_and_shaped_like_v3() {
        let uuid = OfflineAccountSource::offline_uuid("player");
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "3"); // version 3
                                        // RFC-4122 variant：该半字节是 0b10xx，即 8/9/a/b 之一
        assert!(
            ["8", "9", "a", "b"].contains(&&uuid[19..20]),
            "variant nibble 不对: {uuid}"
        );
        assert_eq!(uuid, OfflineAccountSource::offline_uuid("player"));
        assert_ne!(uuid, OfflineAccountSource::offline_uuid("other"));
    }

    /// 固定期望值（`md5("OfflinePlayer:" + name)` 再加 version/variant 位），
    /// 用来锁定实现与官方离线的算法一致。
    #[test]
    fn offline_uuid_matches_fixed_expected_values() {
        assert_eq!(
            OfflineAccountSource::offline_uuid("player"),
            "b1033201-7292-3cac-9bf0-2059ac139adc"
        );
        assert_eq!(
            OfflineAccountSource::offline_uuid("Steve"),
            "5627dd98-e6be-3c21-b8a8-e92344183641"
        );
        assert_eq!(
            OfflineAccountSource::offline_uuid(""),
            "fc5bc365-aedf-30a8-8b89-04e462e29bde"
        );
    }

    #[tokio::test]
    async fn offline_login_fills_every_template() {
        let source = OfflineAccountSource::new("player");
        let account = source.login().await.unwrap();
        assert_eq!(source.name(), "player");
        assert_eq!(account.username, "player");
        assert_eq!(account.uuid, OfflineAccountSource::offline_uuid("player"));
        assert_eq!(account.access_token, "");
        assert_eq!(account.user_type, "legacy");
        assert_eq!(account.xuid, "");
    }

    #[tokio::test]
    async fn offline_refresh_returns_the_same_snapshot() {
        let source = OfflineAccountSource::new("Steve");
        assert_eq!(
            source.login().await.unwrap(),
            source.refresh().await.unwrap()
        );
    }
}
