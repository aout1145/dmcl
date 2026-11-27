//! 账户：启动快照与登录源。
//!
//! [`Account`] 只承载登录结果，用来拼命令行；[`AccountSource`] 负责登录 / 刷新，
//! 决定怎么从配置产出一个可启动的账户。账户层不依赖启动链或任务系统：启动链的
//! `LoginTask` 和以后的账户管理页都只是它的调用方。
//!
//! 具体实现分在 [`crate::offline`] 与 [`crate::microsoft`]。

use anyhow::Result;

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
    /// Xbox User ID（`${auth_xuid}`）；离线账户为空。
    pub xuid: String,
}

/// 配置里的一个账户：负责登录 / 刷新，产出可启动的 [`Account`]。
///
/// 这一层不知道启动链或任务系统：`login` / `refresh` 只返回结果，进度与交互由调用方
/// 自己处理。启动链和账户管理页都可以直接使用它。
///
/// `login` 的语义是「确保拿到一个可用会话」：在线实现在内部静默 refresh；refresh
/// 不可用时返回错误，由调用方的交互式登录流程重新授权，而不是在启动游戏时
/// 突然要求用户去浏览器操作。
#[async_trait::async_trait]
pub trait AccountSource: Send + Sync {
    /// 账户名，用于日志和展示。
    fn name(&self) -> &str;

    /// 确保拿到一个可用会话。
    async fn login(&self) -> Result<Account>;

    /// 刷新已有会话。
    async fn refresh(&self) -> Result<Account>;
}
