//! `launch.*` 段。

use std::path::PathBuf;

use dmcl_launch::{JavaChoice, JavaConfigs, LAUNCHER_NAME};
use serde::{Deserialize, Serialize};

/// `launch.*` 段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchConfig {
    /// 命名 java（名字 → 可执行文件），由 `dmcl java` 维护。
    pub java_configs: JavaConfigs,
    /// 用哪一个，或自动选择。
    pub java: JavaChoice,
    pub minecraft_root_path: PathBuf,
    /// 窗口尺寸（像素）—— 不可能为负。
    pub window_width: u32,
    pub window_height: u32,
    /// 堆大小（MiB）—— 不可能为负。
    pub min_memory: u32,
    pub max_memory: u32,
    pub custom_info: String,
    pub additional_jvm: String,
    pub additional_game: String,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            java_configs: JavaConfigs::new(),
            java: JavaChoice::Auto,
            minecraft_root_path: PathBuf::from(".minecraft"),
            window_width: 854,
            window_height: 480,
            min_memory: 512,
            max_memory: 2048,
            custom_info: LAUNCHER_NAME.to_string(),
            // 历史默认：这两个 `-D` 是给 FML/OptiFine 兼容用的，从旧实现继承下来。
            // 以后应改成空 / 更中性的默认，或交给用户配置。
            additional_jvm:
                "-Dfml.ignoreInvalidMinecraftCertificates=True -Dfml.ignorePatchDiscrepancies=True"
                    .to_string(),
            additional_game: String::new(),
        }
    }
}
