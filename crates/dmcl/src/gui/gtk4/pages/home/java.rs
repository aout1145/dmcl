//! 主页启动时的 Java 选择与自动安装写回。
//!
//! GUI 与 CLI 的策略不同：先用配置里仍然存在的 Java；一个都没有时扫描本机，把结果合并
//! 进 `javas` 后继续启动。自动下载 JRE 成功后由 [`GuiJavaInstalledHook`] 写回配置。

use anyhow::{anyhow, Result};
use dmcl_java::{JavaDetails, JavaInstalledHook, JavaOption, JavaResolver};

use crate::config::{
    configured_java, merge_javas, resolve_from_config, scan_configs, unique_java_name, JavaChoice,
    JavaConfig, JavaConfigs,
};
use crate::gui::gtk4::app::{config_store, ConfigRef};

/// 按 GUI 策略选择 Java：读当前配置，必要时扫描本机并写回。
pub(crate) struct GuiJavaResolver {
    pub(crate) choice: JavaChoice,
    pub(crate) config: ConfigRef,
}

#[async_trait::async_trait]
impl JavaResolver for GuiJavaResolver {
    async fn resolve(&self, required_major: u32) -> Result<JavaOption> {
        let configs = self.config.read().javas.clone();
        if !self.choice.is_auto() {
            return resolve_from_config(required_major, &self.choice, &configs, false).await;
        }

        if let Some(java) = configured_java(required_major, &configs) {
            return Ok(java);
        }

        let found = scan_configs().await;
        if found.is_empty() {
            return Err(anyhow!(
                "未找到可用的 Java，请先安装 Java 或在配置中添加 Java 路径"
            ));
        }
        let selected = configured_java(required_major, &found);

        // 扫描结果写回配置，供下次启动直接使用；保存失败不打断本次启动。
        let _ = config_store::store()
            .update_async(move |config| {
                merge_javas(&mut config.javas, &found);
            })
            .await;
        selected.ok_or_else(|| anyhow!("未找到合适的 Java，请先安装 Java"))
    }
}

/// 自动下载 JRE 并探测成功后，把新 Java 写回 `javas`。
///
/// 写回失败只影响下次启动，不阻断本次启动。
pub(crate) struct GuiJavaInstalledHook {
    config: ConfigRef,
}

impl GuiJavaInstalledHook {
    pub(crate) fn new(config: ConfigRef) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl JavaInstalledHook for GuiJavaInstalledHook {
    async fn on_installed(&self, details: &JavaDetails) -> Result<()> {
        let name = unique_java_name(&self.config.read().javas, details);
        let found = JavaConfigs::from([(
            name,
            JavaConfig {
                path: details.option.path.clone(),
                major: details.option.major,
                arch: details.option.arch,
            },
        )]);
        let _ = config_store::store()
            .update_async(move |config| {
                merge_javas(&mut config.javas, &found);
            })
            .await;
        Ok(())
    }
}
