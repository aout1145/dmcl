//! 账号、java 选择、命令行生成、启动与状态监控。
//!
//! 拆自原来的 `dmcl.launch` + `dmcl.account`，最重要的两点变化：
//!
//! 1. **启动与安装彻底解耦**。原来启动流程里"顺手把缺的库下了、把 natives 解了"；
//!    现在启动只做一件事：[`GameLauncher::check_integrity`] 返回
//!    [`IntegrityStatus`]（完整吗？缺哪些文件？），**修复交给调用者** ——
//!    由它决定是跑一次 [`dmcl_install`] 补全，还是照常启动。
//! 2. **不读全局配置**。java 的选择（[`JavaChoice`] + [`JavaConfigs`]）、
//!    窗口尺寸、内存上限等全部通过 [`LaunchOption`] 由调用者传入。
//!
//! 典型的启动流程（cli 就是这么写的）：
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_launch::{GameLauncher, LaunchOption, OfflineAccount};
//!
//! # async fn run(root: std::path::PathBuf) -> anyhow::Result<()> {
//! let option = LaunchOption::new(Arc::new(OfflineAccount::new("player")), root, "1.20.1");
//! let mut launcher = GameLauncher::new(option).await?;
//!
//! let status = launcher.check_integrity().await?;
//! if !status.is_complete() {
//!     println!("{}", status.summary());
//!     // 交给 dmcl-install 补全文件，然后再启动
//! }
//!
//! let mut monitor = launcher.launch_game().await?;
//! monitor.wait().await?;
//! # Ok(())
//! # }
//! ```

pub mod account;
pub mod integrity;
pub mod java;
pub mod launcher;
pub mod monitor;
pub mod option;
pub mod util;

pub use account::{sha1_uuid, Account, OfflineAccount};
pub use integrity::{
    check_integrity, version_json_exists, IntegrityStatus, MissingFile, MissingReason,
};
pub use java::{
    find_java, find_java_in_default_roots, select_java, JavaChoice, JavaConfigs, JavaOption,
    DEFAULT_JAVA_ROOTS, JAVA_AUTOSELECT,
};
pub use launcher::GameLauncher;
pub use monitor::GameMonitor;
pub use option::{LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION};
pub use util::{drive_name, escape_shell_command};
