//! 账户：启动快照与登录源。
//!
//! [`Account`] 只承载登录结果，用来拼命令行；[`AccountSource`] 负责登录 / 刷新，
//! 决定怎么从配置产出一个可启动的账户。账户层不依赖启动链或任务系统：启动链的
//! `LoginTask` 和以后的账号管理页都只是它的调用方。

use anyhow::Result;
use dmcl_task::BoxFuture;
use md5::{Digest, Md5};

/// 登录完成后用于拼命令行的只读快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// 送给游戏的玩家名（`${auth_player_name}`）。
    pub username: String,
    /// 玩家 uuid（`${auth_uuid}` / `${uuid}`）。
    pub uuid: String,
    /// 会话令牌（`${auth_access_token}` / `${auth_session}`）。
    pub access_token: String,
    /// 账户类型（`${user_type}`）：离线为 `"legacy"`，微软为 `"msa"`。
    pub user_type: String,
}

/// 配置里的一个账户：负责登录 / 刷新，产出可启动的 [`Account`]。
///
/// 这一层不知道启动链或任务系统：`login` / `refresh` 只返回结果，进度与交互由调用方
/// 自己处理。启动链和账号管理页都可以直接使用它。
///
/// `login` 的语义是「确保拿到一个可用会话」：在线实现可以在内部先尝试 refresh，
/// 只有 refresh 失效时才走完整登录。
pub trait AccountSource: Send + Sync {
    /// 账户名，用于日志和 [`crate::LaunchOption`] 的 Debug 输出。
    fn name(&self) -> &str;

    /// 确保拿到一个可用会话。
    fn login<'a>(&'a self) -> BoxFuture<'a, Result<Account>>;

    /// 刷新已有会话。
    fn refresh<'a>(&'a self) -> BoxFuture<'a, Result<Account>>;
}

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

impl AccountSource for OfflineAccountSource {
    fn name(&self) -> &str {
        &self.username
    }

    fn login<'a>(&'a self) -> BoxFuture<'a, Result<Account>> {
        Box::pin(async move { Ok(self.account()) })
    }

    fn refresh<'a>(&'a self) -> BoxFuture<'a, Result<Account>> {
        Box::pin(async move { Ok(self.account()) })
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
