//! 统一的 Java 解析接口。

use anyhow::Result;

use crate::option::{JavaDetails, JavaOption};

/// 统一的 Java 解析接口：只负责解析，不产生副作用。
///
/// 安装流程和启动流程都只依赖它：
/// - 安装器 processor 只需要 `JavaOption.path`；
/// - 启动流程需要 `path` / `major` / `arch`。
#[async_trait::async_trait]
pub trait JavaResolver: Send + Sync {
    async fn resolve(&self, major: u32) -> Result<JavaOption>;
}

/// 自动安装 JRE 并探测成功后，把新 Java 交给调用方（例如 GUI 写回配置）。
///
/// 纯 CLI / 安装流程不设置这个 hook；`dmcl-java` 不认识配置。
#[async_trait::async_trait]
pub trait JavaInstalledHook: Send + Sync {
    async fn on_installed(&self, details: &JavaDetails) -> Result<()>;
}

/// 一个已经确定的 Java 可以直接当解析器用。
#[async_trait::async_trait]
impl JavaResolver for JavaOption {
    async fn resolve(&self, _major: u32) -> Result<JavaOption> {
        Ok(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::os::Arch;
    use std::path::PathBuf;

    #[tokio::test]
    async fn a_java_option_is_a_fixed_resolver() {
        let java = JavaOption {
            path: PathBuf::from("/jdk17"),
            major: 17,
            arch: Arch::X86_64,
        };
        assert_eq!(java.resolve(8).await.unwrap(), java);
    }
}
