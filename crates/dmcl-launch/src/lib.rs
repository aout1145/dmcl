//! 账号、java 选择、命令行生成、启动与状态监控。
//!
//! 两条设计要点：
//!
//! 1. **底层启动与安装解耦**。[`GameLauncher`] 只负责拼命令行与启动进程，
//!    文件完整性由 [`dmcl_install::check_integrity`] 负责，**是否修复交给调用者**。
//!    想要「缺了就补」的一站式流程时，用 [`launch_chain`]。
//! 2. **不读全局配置**。java 的选择（[`JavaChoice`] + [`JavaConfigs`]）、
//!    窗口尺寸、内存上限等全部通过 [`LaunchOption`] 由调用者传入。
//!
//! 一站式启动把「检查并补全 → 生成参数 → 启动」串成一条任务链（见 [`crate::prepare`]）：
//! [`launch_chain`] 返回 `TaskChain<(), GameMonitor>`，由调用者运行、订阅事件，
//! 并在拿到**未 wait** 的 [`GameMonitor`] 后决定何时等待或停掉。
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_install::InstallOptions;
//! use dmcl_launch::{launch_chain, LaunchOption, OfflineAccount};
//!
//! # async fn run(root: std::path::PathBuf, install: Arc<InstallOptions>) -> anyhow::Result<()> {
//! let option = LaunchOption::new(Arc::new(OfflineAccount::new("player")), root, "1.20.1");
//!
//! // 检查完整性 →（缺件则补全）→ 生成启动参数 → 启动子进程
//! let mut monitor = launch_chain(option, install)
//!     .run_with((), |event| println!("{event}"))
//!     .await?;
//! monitor.wait().await?;
//! # Ok(())
//! # }
//! ```

pub mod account;
pub mod java;
pub mod launcher;
pub mod monitor;
pub mod option;
pub mod prepare;

pub use account::{sha1_uuid, Account, OfflineAccount};
pub use java::{
    find_java, select_java, JavaChoice, JavaConfigs, JavaOption, DEFAULT_JAVA_ROOTS,
    JAVA_AUTOSELECT,
};
pub use launcher::GameLauncher;
pub use monitor::GameMonitor;
pub use option::{LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION};
pub use prepare::{
    launch_chain, GenerateArgsTask, LaunchTask, PrepareLaunchFilesTask, PrepareNativesTask,
    PreparedLaunch,
};
