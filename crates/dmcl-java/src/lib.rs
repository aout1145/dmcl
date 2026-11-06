//! Java 探测、扫描、自动选择，以及 Mojang JRE 的安装。
//!
//! 这个 crate 只处理 Java 运行时本身：
//!
//! - 探测一个 java 可执行文件的版本 / 架构 / 厂商（[`JavaDetails`]）；
//! - 扫描本机常见目录（[`find_java`] / [`default_java_roots`]）；
//! - 按大版本从候选里自动选择（[`auto_select_java`]）；
//! - 统一的解析接口 [`JavaResolver`]，安装流程和启动流程共用；
//! - 从 Mojang 安装 JRE（[`install_jre`]），以及官方启动器的 runtime 目录布局。
//!
//! 它不读配置文件：Java 选择与运行时目录都由调用者通过 [`JavaResolver`] /
//! [`JreInstallRequest`] 传入；自动安装成功后的配置写回由调用者实现
//! [`JavaInstalledHook`]。

mod jre;
mod option;
mod probe;
mod resolver;
mod roots;
mod runtime;
mod scan;

pub use jre::{
    fetch_jre_runtime_manifest, install_jre, resolve_jre_component, runtime_dir, JreAutoInstall,
    JreComponent, JreFileFormat, JreInstallRequest, JreInstalled,
};
pub use option::{JavaDetails, JavaOption};
pub use resolver::{JavaInstalledHook, JavaResolver};
pub use roots::{default_java_roots, official_runtime_root};
pub use runtime::JavaRuntimeOptions;
pub use scan::{auto_select_java, find_java};
