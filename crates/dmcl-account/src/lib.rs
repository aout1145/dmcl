//! 账户：启动快照、登录源，以及离线 / 微软账户实现。
//!
//! [`Account`] 是登录完成后用来拼命令行的只读快照；[`AccountSource`] 负责 `login` /
//! `refresh`，具体实现有 [`OfflineAccountSource`] 与 [`MicrosoftAccountSource`]。
//!
//! 账户层**不依赖启动链或任务系统**，也不读全局配置：凭据怎么持久化由调用方实现
//! [`RefreshTokenStore`]，交互式登录怎么展示由调用方实现 [`DeviceCodePresenter`]。
//! 启动链的登录节点、CLI 的 `login` 与图形界面的账户页都只是它的调用方。
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_account::{AccountSource, OfflineAccountSource};
//!
//! # async fn run() -> anyhow::Result<()> {
//! let source: Arc<dyn AccountSource> = Arc::new(OfflineAccountSource::new("player"));
//! let account = source.login().await?;
//! assert_eq!(account.username, "player");
//! # Ok(())
//! # }
//! ```

pub mod account;
pub mod microsoft;
pub mod offline;

pub use account::{Account, AccountSource};
pub use microsoft::{
    device_code_login, DeviceCodeLogin, DeviceCodePresenter, DeviceCodePrompt,
    MicrosoftAccountSource, RefreshTokenStore, StoredCredential, CLIENT_ID as MICROSOFT_CLIENT_ID,
};
pub use offline::OfflineAccountSource;
