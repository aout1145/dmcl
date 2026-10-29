//! 账户：启动快照与登录源。
//!
//! [`Account`] 只承载登录结果，用来拼命令行；[`AccountSource`] 负责登录 / 刷新，
//! 决定怎么从配置产出一个可启动的账户。账户层不依赖启动链或任务系统：启动链的
//! `LoginTask` 和以后的账号管理页都只是它的调用方。

use anyhow::Result;
use dmcl_task::BoxFuture;
use sha1::{Digest, Sha1};

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
            uuid: Self::sha1_uuid(&self.username),
            username: self.username.clone(),
            access_token: String::new(),
            user_type: "legacy".to_string(),
        }
    }

    /// 由 `sha1(name)` 前 16 字节构造的、基于名字的（version 5）RFC-4122 uuid。
    fn sha1_uuid(name: &str) -> String {
        let mut hasher = Sha1::new();
        hasher.update(name.as_bytes());
        let digest = hasher.finalize();

        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[0..16]);
        uuid::Builder::from_sha1_bytes(bytes)
            .into_uuid()
            .to_string()
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
    fn offline_uuid_is_deterministic_and_shaped_like_v5() {
        let uuid = OfflineAccountSource::sha1_uuid("player");
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "5"); // version 5
                                        // RFC-4122 variant：该半字节是 0b10xx，即 8/9/a/b 之一
        assert!(
            ["8", "9", "a", "b"].contains(&&uuid[19..20]),
            "variant nibble 不对: {uuid}"
        );
        assert_eq!(uuid, OfflineAccountSource::sha1_uuid("player"));
        assert_ne!(uuid, OfflineAccountSource::sha1_uuid("other"));
    }

    /// 固定期望值（`sha1(name)[0..16]` 再加 version/variant 位），用来锁定实现不漂移。
    #[test]
    fn offline_uuid_matches_fixed_expected_values() {
        assert_eq!(
            OfflineAccountSource::sha1_uuid("player"),
            "889c6853-a117-5ca8-bef9-d6523335dc06"
        );
        assert_eq!(
            OfflineAccountSource::sha1_uuid("Steve"),
            "2121f63f-a284-5c8a-9ce4-00bb6f83824f"
        );
        assert_eq!(
            OfflineAccountSource::sha1_uuid(""),
            "da39a3ee-5e6b-5b0d-b255-bfef95601890"
        );
    }

    #[tokio::test]
    async fn offline_login_fills_every_template() {
        let source = OfflineAccountSource::new("player");
        let account = source.login().await.unwrap();
        assert_eq!(source.name(), "player");
        assert_eq!(account.username, "player");
        assert_eq!(account.uuid, OfflineAccountSource::sha1_uuid("player"));
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
