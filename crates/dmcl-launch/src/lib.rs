//! 账号、命令行生成、启动与状态监控。
//!
//! 两条设计要点：
//!
//! 1. **底层启动与文件物化解耦**。[`GameLauncher`] 只负责拼命令行，起进程由 [`LaunchTask`]
//!    完成；文件完整性由 [`dmcl_game::check_integrity`] 负责，**是否修复交给调用者**。
//!    想要「缺了就补」的一站式流程时，用 [`launch_chain`]。
//! 2. **不读全局配置**。java 的选择由调用者实现 [`dmcl_java::JavaResolver`] 注入，
//!    窗口尺寸、内存设置等全部通过 [`LaunchOption`] 由调用者传入；内存可以留空，
//!    由 [`resolve_memory`] 按系统可用内存与所选 Java 的架构自动决定。
//!
//! 一站式启动把「检查游戏完整性 → 登录 → 启动」组成一张任务图（见 [`crate::prepare`]）：
//! 检查游戏完整性里依次是读取版本、检查并补全文件 / 选择 Java、解压 natives；启动里是
//! 生成参数与起子进程。[`launch_chain`] 返回 `TaskGraph<GameMonitor>`，由调用者运行、
//! 订阅事件，并在拿到**未 wait** 的 [`GameMonitor`] 后决定何时等待或停掉。
//!
//! [`Account`] 是登录完成后用来拼命令行的只读快照；[`AccountSource`] 负责 `login` /
//! `refresh`。登录节点等文件补全与 natives 都结束才开始，避免在线会话在长时间下载中过期。
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_game::{GameFilesOptions, VersionRef};
//! use dmcl_java::JavaOption;
//! use dmcl_launch::{launch_chain, LaunchOption, OfflineAccountSource};
//!
//! # async fn run(root: std::path::PathBuf, game: Arc<GameFilesOptions>) -> anyhow::Result<()> {
//! let java = JavaOption {
//!     path: "/usr/bin/java".into(),
//!     major: 17,
//!     arch: dmcl_mcmeta::os::Arch::X86_64,
//! };
//! let option = LaunchOption::new(
//!     Arc::new(OfflineAccountSource::new("player")),
//!     VersionRef::new(&root, "1.20.1"),
//!     Arc::new(java),
//!     None,
//! );
//!
//! // 检查游戏完整性 → 登录 → 启动
//! let monitor = launch_chain(option, game, None)
//!     .run(|event| println!("{event}"))
//!     .await?;
//! let mut monitor = std::sync::Arc::try_unwrap(monitor)
//!     .map_err(|_| anyhow::anyhow!("游戏监控句柄仍被共享"))?;
//! monitor.wait().await?;
//! # Ok(())
//! # }
//! ```

pub mod account;
pub mod launcher;
pub mod memory;
pub mod microsoft;
pub mod monitor;
pub mod option;
pub mod prepare;

pub use account::{Account, AccountSource, OfflineAccountSource};
pub use launcher::GameLauncher;
pub use memory::{auto_max_memory, resolve_memory, system_available_mib};
pub use microsoft::{
    device_code_login, DeviceCodeLogin, DeviceCodePresenter, DeviceCodePrompt,
    MicrosoftAccountSource, RefreshTokenStore, StoredCredential, CLIENT_ID as MICROSOFT_CLIENT_ID,
};
pub use monitor::GameMonitor;
pub use option::{LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION};
pub use prepare::{
    launch_chain, GenerateArgsTask, LaunchTask, LoginTask, PrepareLaunchFilesTask,
    PrepareNativesTask, PreparedLaunch, ReadVersionTask, SelectJavaTask,
};
