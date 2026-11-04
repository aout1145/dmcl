//! 主页启动时的 Java 选择。
//!
//! GUI 与 CLI 的策略不同：先用配置里仍然存在的 Java；一个都没有时扫描本机，把结果合并
//! 进 `javas` 后继续启动。写配置失败只影响下次启动，不会阻断本次启动。

use anyhow::{anyhow, Result};
use dmcl_launch::{auto_select_java, default_java_roots, find_java, JavaOption, JavaSelector};
use dmcl_task::BoxFuture;

use crate::config::{configs_from_details, merge_javas, JavaChoice};
use crate::gui::gtk4::app::{config_store, ConfigRef};

/// 按 GUI 策略选择 Java：读当前配置，必要时扫描本机并写回。
pub(crate) struct GuiJavaSelector {
    pub(crate) choice: JavaChoice,
    pub(crate) config: ConfigRef,
}

impl JavaSelector for GuiJavaSelector {
    fn select_java<'a>(&'a self, required_major: u32) -> BoxFuture<'a, Result<JavaOption>> {
        Box::pin(async move {
            match &self.choice {
                JavaChoice::Named(name) => {
                    let java = self.config.read().javas.get(name).cloned();
                    let Some(java) = java else {
                        return Err(anyhow!("Java 配置项不存在：{name}"));
                    };
                    if !java.path.is_file() {
                        return Err(anyhow!(
                            "Java 配置项「{name}」的可执行文件不存在：{}",
                            java.path.display()
                        ));
                    }
                    Ok(JavaOption {
                        path: java.path,
                        major: java.major,
                        arch: java.arch,
                    })
                }
                JavaChoice::Auto => {
                    let options: Vec<JavaOption> = self
                        .config
                        .read()
                        .javas
                        .values()
                        .filter(|java| java.path.is_file())
                        .map(|java| JavaOption {
                            path: java.path.clone(),
                            major: java.major,
                            arch: java.arch,
                        })
                        .collect();
                    if let Some(java) = auto_select_java(required_major, &options) {
                        return Ok(java);
                    }

                    let found = configs_from_details(find_java(&default_java_roots()).await);
                    if found.is_empty() {
                        return Err(anyhow!(
                            "未找到可用的 Java，请先安装 Java 或在配置中添加 Java 路径"
                        ));
                    }

                    let options: Vec<JavaOption> = found
                        .values()
                        .map(|java| JavaOption {
                            path: java.path.clone(),
                            major: java.major,
                            arch: java.arch,
                        })
                        .collect();
                    let selected = auto_select_java(required_major, &options);

                    // 扫描结果写回配置，供下次启动直接使用；保存失败不打断本次启动。
                    let _ = config_store::store()
                        .update_async(move |config| {
                            merge_javas(&mut config.javas, &found);
                        })
                        .await;
                    selected.ok_or_else(|| anyhow!("未找到合适的 Java，请先安装 Java"))
                }
            }
        })
    }

    fn allows_auto_install(&self) -> bool {
        false
    }
}
