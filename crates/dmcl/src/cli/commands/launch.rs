//! `launch` —— 检查完整性、补全文件、生成参数、启动。
//!
//! 这里同时放着 CLI 的 Java 选择器 [`ConfigJava`]：只认全局配置里的 `javas`，不扫描、
//! 不下载。

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_launch::{auto_select_java, JavaOption, JavaSelector};

use super::handle_event;
use crate::cli::progress::ProgressPrinter;
use crate::config::{ensure_java_path, Config, JavaChoice, JavaConfig, JavaConfigs};

/// 只认配置的 Java 选择器：不会扫描本机，也不会自动安装。
///
/// 只服务于 `launch`：安装流程有独立的 JavaProvider，按版本要求自动选择，与这里的
/// `launch.java` 无关。
pub(crate) struct ConfigJava {
    pub(crate) choice: JavaChoice,
    pub(crate) configs: JavaConfigs,
}

impl JavaSelector for ConfigJava {
    fn select_java<'a>(
        &'a self,
        required_major: u32,
    ) -> dmcl_task::BoxFuture<'a, Result<JavaOption>> {
        Box::pin(async move { select_java(required_major, &self.choice, &self.configs).await })
    }

    fn allows_auto_install(&self) -> bool {
        false
    }
}

/// 按配置里的选择挑一个 Java；只读缓存的 `major` / `arch`，不执行 Java。
///
/// 自动选择只看 `javas` 里路径仍然存在的条目，不会扫描本机；没有可用条目时提示用户
/// 先执行 `dmcl search-java`。
async fn select_java(
    required_major: u32,
    choice: &JavaChoice,
    configs: &JavaConfigs,
) -> Result<JavaOption> {
    match choice {
        JavaChoice::Named(name) => {
            let java = configs
                .get(name)
                .ok_or_else(|| anyhow!("Java 配置项不存在：{name}"))?;
            ensure_java_path(name, java)?;
            Ok(java.option())
        }
        JavaChoice::Auto => {
            let options: Vec<JavaOption> = configs
                .values()
                .filter(|java| java.path.is_file())
                .map(JavaConfig::option)
                .collect();
            if options.is_empty() {
                return Err(anyhow!(
                    "配置中没有可用的 Java，请先执行 dmcl search-java 扫描本机 Java"
                ));
            }
            auto_select_java(required_major, &options)
                .ok_or_else(|| anyhow!("未找到合适的 Java，请先执行 dmcl search-java"))
        }
    }
}

/// `dmcl launch <version-name>`。
pub(crate) async fn run(vername: &str, config: &Config, root_path: &Path) -> Result<()> {
    let loaded = config.load_instance(root_path, vername).await;
    if let Some(error) = &loaded.error {
        for warning in &error.warnings {
            eprintln!("错误：{warning}");
        }
        return Err(error.clone().into());
    }
    let launch = loaded.launch;
    let java = Arc::new(ConfigJava {
        choice: launch.java.clone(),
        configs: config.javas.clone(),
    });
    let option = config.launch_option(&launch, vername, root_path, java)?;
    let install = Arc::new(config.install_options(config.download_source(), root_path)?);

    let mut printer = ProgressPrinter::new();
    let mut monitor = dmcl_launch::launch_chain(option, install)
        .run_with((), |event| handle_event(&mut printer, event))
        .await?;
    printer.finish();

    monitor.wait().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::os::Arch;
    use std::path::PathBuf;

    fn temp_java_dir(label: &str) -> PathBuf {
        let dir = crate::config::temp_dir(label);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_entry(dir: &Path, name: &str, major: u32) -> JavaConfig {
        let path = dir.join(name);
        std::fs::write(&path, b"").unwrap();
        JavaConfig {
            path,
            major,
            arch: Arch::X86_64,
        }
    }

    #[tokio::test]
    async fn selection_uses_cached_versions_without_probing() {
        // 文件都是空的；能选中说明选择过程只检查路径，没有执行 java
        let dir = temp_java_dir("java-selection");
        let configs = JavaConfigs::from([
            ("eight".to_string(), file_entry(&dir, "eight", 8)),
            ("seventeen".to_string(), file_entry(&dir, "seventeen", 17)),
            ("twentyone".to_string(), file_entry(&dir, "twentyone", 21)),
        ]);

        let selected = select_java(17, &JavaChoice::Auto, &configs).await.unwrap();
        assert_eq!(selected.path, dir.join("seventeen"));

        let selected = select_java(8, &JavaChoice::Named("twentyone".into()), &configs)
            .await
            .unwrap();
        assert_eq!(selected.path, dir.join("twentyone"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn auto_selection_ignores_missing_paths() {
        let dir = temp_java_dir("java-mixed");
        let valid = file_entry(&dir, "valid", 17);
        let configs = JavaConfigs::from([
            (
                "missing".to_string(),
                JavaConfig {
                    path: dir.join("missing"),
                    major: 21,
                    arch: Arch::X86_64,
                },
            ),
            ("valid".to_string(), valid.clone()),
        ]);

        // 21 的路径不存在，只能选仍然存在的 17
        let selected = select_java(21, &JavaChoice::Auto, &configs).await.unwrap();
        assert_eq!(selected.path, valid.path);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn auto_selection_without_a_usable_entry_points_at_search_java() {
        let dir = temp_java_dir("java-missing");
        let missing = dir.join("java");
        let configs = JavaConfigs::from([(
            "gone".to_string(),
            JavaConfig {
                path: missing.clone(),
                major: 17,
                arch: Arch::X86_64,
            },
        )]);

        // 空表与「全部路径失效」都只认配置，不扫描本机。
        for configs in [&JavaConfigs::new(), &configs] {
            let err = select_java(17, &JavaChoice::Auto, configs)
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("dmcl search-java"), "{err}");
        }

        let err = select_java(17, &JavaChoice::Named("gone".into()), &configs)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("gone"), "{err}");
        assert!(err.contains(&missing.display().to_string()), "{err}");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn unknown_named_java_is_reported_even_without_entries() {
        let err = select_java(17, &JavaChoice::Named("nope".into()), &JavaConfigs::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("nope"), "{err}");
        assert!(err.contains("不存在"), "{err}");
    }

    #[tokio::test]
    async fn config_java_converts_to_the_launch_types() {
        let dir = temp_java_dir("java-config-java");
        let configs = JavaConfigs::from([("jdk".to_string(), file_entry(&dir, "jdk", 17))]);
        let java = ConfigJava {
            choice: JavaChoice::Named("jdk".into()),
            configs,
        };

        let option = JavaSelector::select_java(&java, 17).await.unwrap();
        assert_eq!(option.path, dir.join("jdk"));
        assert_eq!(option.major, 17);
        assert_eq!(option.arch, Arch::X86_64);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
