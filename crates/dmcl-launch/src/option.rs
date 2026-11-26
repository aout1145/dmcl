//! 启动参数。

use std::fmt;
use std::sync::Arc;

use dmcl_game::VersionRef;
use dmcl_java::{JavaInstalledHook, JavaResolver};

use crate::account::AccountSource;

/// 启动器名字（替换 `${launcher_name}`）。
pub const LAUNCHER_NAME: &str = "dmcl";

/// 启动器版本（替换 `${launcher_version}`）。
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 游戏窗口的初始状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    /// 窗口化，使用给定的初始宽高（像素）。
    Windowed { width: u32, height: u32 },
    /// 全屏启动。
    Fullscreen,
}

/// 启动后直接进入的目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickPlay {
    /// 单人世界的文件夹名。
    Singleplayer(String),
    /// 多人服务器地址。
    Multiplayer(String),
    /// Realms ID。
    Realms(String),
}

impl QuickPlay {
    /// 对应的 Minecraft 参数名。
    pub fn argument(&self) -> &'static str {
        match self {
            Self::Singleplayer(_) => "--quickPlaySingleplayer",
            Self::Multiplayer(_) => "--quickPlayMultiplayer",
            Self::Realms(_) => "--quickPlayRealms",
        }
    }

    /// 参数值。
    pub fn value(&self) -> &str {
        match self {
            Self::Singleplayer(value) | Self::Multiplayer(value) | Self::Realms(value) => value,
        }
    }
}

/// 游戏进程的优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessPriority {
    /// 低于正常优先级。
    Low,
    /// 正常优先级。
    #[default]
    Normal,
    /// 高于正常优先级。
    High,
}

/// 启动一个版本所需要的全部信息。
///
/// 账户来源与 Java 解析都由调用者通过 `Arc<dyn …>` 传入，结构体因此仍是
/// `Send + Sync`，可以交给 GUI 的另一个线程。
#[derive(Clone)]
pub struct LaunchOption {
    pub account_source: Arc<dyn AccountSource>,
    /// 要启动的版本。
    pub version: VersionRef,
    /// 是否为每个版本使用独立的游戏目录（`versions/<版本名>`）。
    ///
    /// 关闭后游戏进程的工作目录和 `${game_directory}` 都改用根目录。
    pub version_isolation: bool,
    /// 窗口的初始状态；宽高不可能为负。
    pub window: WindowMode,
    /// 启动后直接进入的单人世界、服务器或 Realm；`None` 表示进入主菜单。
    pub quick_play: Option<QuickPlay>,
    /// 游戏进程的优先级。
    pub process_priority: ProcessPriority,
    /// 堆大小；`None` 表示自动分配。
    pub min_memory: Option<u32>,
    pub max_memory: Option<u32>,
    /// 传给游戏的客户端 ID（`${clientid}`，用于遥测）；默认空串。
    pub client_id: String,
    pub custom_info: String,
    pub additional_jvm: Vec<String>,
    pub additional_game: Vec<String>,
    /// 怎么找 java：由调用者提供的解析器决定。
    pub java: Arc<dyn JavaResolver>,
    /// 自动安装 JRE 并探测成功后调用；CLI / 无配置写回时为 `None`。
    pub java_installed_hook: Option<Arc<dyn JavaInstalledHook>>,
}

impl fmt::Debug for LaunchOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchOption")
            .field("account", &self.account_source.name())
            .field("version", &self.version)
            .field("window", &self.window)
            .field("min_memory", &self.min_memory)
            .field("max_memory", &self.max_memory)
            .finish_non_exhaustive()
    }
}

impl LaunchOption {
    /// 除必填项外，其余字段取各自的默认值。
    pub fn new(
        account_source: Arc<dyn AccountSource>,
        version: VersionRef,
        java: Arc<dyn JavaResolver>,
        java_installed_hook: Option<Arc<dyn JavaInstalledHook>>,
    ) -> Self {
        Self {
            account_source,
            version,
            version_isolation: true,
            window: WindowMode::Windowed {
                width: 854,
                height: 480,
            },
            quick_play: None,
            process_priority: ProcessPriority::Normal,
            min_memory: None,
            max_memory: None,
            client_id: String::new(),
            custom_info: LAUNCHER_NAME.to_string(),
            additional_jvm: vec![
                "-Dfml.ignoreInvalidMinecraftCertificates=True".to_string(),
                "-Dfml.ignorePatchDiscrepancies=True".to_string(),
            ],
            additional_game: Vec::new(),
            java,
            java_installed_hook,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::OfflineAccountSource;
    use dmcl_java::JavaOption;
    use dmcl_mcmeta::os::Arch;
    use std::path::PathBuf;

    fn fixed_java() -> JavaOption {
        JavaOption {
            path: PathBuf::from("/opt/jdk17/bin/java"),
            major: 17,
            arch: Arch::X86_64,
        }
    }

    #[test]
    fn quick_play_maps_to_minecraft_arguments() {
        let cases = [
            (
                QuickPlay::Singleplayer("world".into()),
                "--quickPlaySingleplayer",
            ),
            (
                QuickPlay::Multiplayer("example.org".into()),
                "--quickPlayMultiplayer",
            ),
            (QuickPlay::Realms("1234".into()), "--quickPlayRealms"),
        ];
        for (quick_play, argument) in cases {
            assert_eq!(quick_play.argument(), argument);
        }
    }

    #[tokio::test]
    async fn java_resolution_can_be_supplied_by_the_caller() {
        let java = fixed_java();
        let option = LaunchOption::new(
            Arc::new(OfflineAccountSource::new("p")),
            VersionRef::new("/mc", "1.20.1"),
            Arc::new(java.clone()),
            None,
        );
        assert_eq!(option.java.resolve(17).await.unwrap(), java);
        assert!(format!("{option:?}").contains('p'));
    }
}
