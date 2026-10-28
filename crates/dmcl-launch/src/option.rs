//! 启动参数。

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use dmcl_install::PlanContext;

use crate::account::Account;
use crate::java::{JavaSelector, ScanJava};

/// 启动器名字（替换 `${launcher_name}`）。
pub const LAUNCHER_NAME: &str = "dmcl";

/// 启动器版本（替换 `${launcher_version}`）。
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 启动一个版本所需要的全部信息。
///
/// 账户用 `Arc<dyn Account>`（而不是裸接口引用）保存，结构体因此仍是
/// `Send + Sync`，可以交给 GUI 的另一个线程；java 的选择也在这里。
#[derive(Clone)]
pub struct LaunchOption {
    pub account: Arc<dyn Account>,
    /// `.minecraft` 根目录（绝对路径）。
    pub root_path: PathBuf,
    /// 版本目录的**名字**，不是路径。
    pub version_name: String,
    /// 窗口尺寸（像素）与堆大小（MiB）—— 不可能为负。
    pub window_width: u32,
    pub window_height: u32,
    /// 堆大小；`None` 表示自动分配。
    pub min_memory: Option<u32>,
    pub max_memory: Option<u32>,
    pub custom_info: String,
    pub additional_jvm: Vec<String>,
    pub additional_game: Vec<String>,
    /// 怎么选 java（默认扫描本机常见目录后自动选择）。
    pub java: Arc<dyn JavaSelector>,
    /// 跳过 `*.ogg` 资源（完整性检查时也要一致，否则会把音乐报成缺失）。
    pub skip_music: bool,
}

impl fmt::Debug for LaunchOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchOption")
            .field("account", &self.account.get_name())
            .field("root_path", &self.root_path)
            .field("version_name", &self.version_name)
            .field("window_width", &self.window_width)
            .field("window_height", &self.window_height)
            .field("min_memory", &self.min_memory)
            .field("max_memory", &self.max_memory)
            .finish_non_exhaustive()
    }
}

impl LaunchOption {
    /// 除三个必填项外，其余字段取各自的默认值。
    pub fn new(
        account: Arc<dyn Account>,
        root_path: impl Into<PathBuf>,
        version_name: impl Into<String>,
    ) -> Self {
        Self {
            account,
            root_path: root_path.into(),
            version_name: version_name.into(),
            window_width: 854,
            window_height: 480,
            min_memory: None,
            max_memory: None,
            custom_info: LAUNCHER_NAME.to_string(),
            additional_jvm: vec![
                "-Dfml.ignoreInvalidMinecraftCertificates=True".to_string(),
                "-Dfml.ignorePatchDiscrepancies=True".to_string(),
            ],
            additional_game: Vec::new(),
            java: Arc::new(ScanJava),
            skip_music: false,
        }
    }

    /// 指定 java 选择器。
    pub fn with_java(mut self, java: Arc<dyn JavaSelector>) -> Self {
        self.java = java;
        self
    }

    /// 指定本地文件布局相关的配置（与安装流程保持一致）。
    pub fn with_local_layout(mut self, skip_music: bool) -> Self {
        self.skip_music = skip_music;
        self
    }

    /// 完整性检查用的规划上下文。
    pub fn plan(&self) -> PlanContext {
        PlanContext {
            root_path: self.root_path.clone(),
            skip_music: self.skip_music,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::OfflineAccount;
    use crate::java::JavaOption;
    use dmcl_mcmeta::os::Arch;

    #[test]
    fn defaults_match_the_original_struct() {
        let option = LaunchOption::new(Arc::new(OfflineAccount::new("player")), "/mc", "1.20.1");
        assert_eq!(option.window_width, 854);
        assert_eq!(option.window_height, 480);
        assert_eq!(option.min_memory, None);
        assert_eq!(option.max_memory, None);
        assert_eq!(option.custom_info, LAUNCHER_NAME);
        assert!(option
            .additional_jvm
            .iter()
            .any(|arg| arg.contains("ignoreInvalidMinecraftCertificates")));
        assert!(option.additional_game.is_empty());
        assert_eq!(
            dmcl_mcmeta::layout::version_dir(&option.root_path, &option.version_name),
            PathBuf::from("/mc/versions/1.20.1")
        );
    }

    #[tokio::test]
    async fn java_selection_can_be_supplied_by_the_caller() {
        let java = JavaOption {
            path: PathBuf::from("/opt/jdk17/bin/java"),
            major: 17,
            arch: Arch::X86_64,
        };
        let option = LaunchOption::new(Arc::new(OfflineAccount::new("p")), "/mc", "1.20.1")
            .with_java(Arc::new(java.clone()));
        assert_eq!(option.java.select_java(17).await.unwrap(), java);
        assert!(format!("{option:?}").contains('p'));
    }
}
