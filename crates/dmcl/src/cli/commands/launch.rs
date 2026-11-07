//! `launch` —— 检查完整性、补全文件、生成参数、启动。
//!
//! 这里同时放着 CLI 的 Java 解析器 [`ConfigJavaResolver`]：只认全局配置里的 `javas`，
//! 不扫描、不下载。

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use dmcl_game::VersionRef;
use dmcl_java::JavaResolver;

use super::handle_event;
use crate::cli::progress::ProgressPrinter;
use crate::config::{resolve_from_config, Config, JavaChoice, JavaConfigs};

/// 只认配置的 Java 解析器：不会扫描本机，也不会自动安装。
///
/// 只服务于 `launch`：安装流程有独立的 `InstallJavaResolver`，按版本要求自动选择，
/// 与这里的 `launch.java` 无关。
pub(crate) struct ConfigJavaResolver {
    pub(crate) choice: JavaChoice,
    pub(crate) configs: JavaConfigs,
}

#[async_trait::async_trait]
impl JavaResolver for ConfigJavaResolver {
    async fn resolve(&self, required_major: u32) -> Result<dmcl_java::JavaOption> {
        resolve_from_config(required_major, &self.choice, &self.configs, false).await
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
    let java = Arc::new(ConfigJavaResolver {
        choice: launch.java.clone(),
        configs: config.javas.clone(),
    });
    let version = VersionRef::new(root_path.to_path_buf(), vername);
    let option = config.launch_option(&launch, version, java, None)?;
    let game = Arc::new(config.game_files_options(config.download_source()));

    let mut printer = ProgressPrinter::new();
    let monitor = dmcl_launch::launch_chain(option, game, None)
        .run(|event| handle_event(&mut printer, event))
        .await?;
    printer.finish();

    let mut monitor =
        Arc::try_unwrap(monitor).map_err(|_| anyhow::anyhow!("游戏监控句柄仍被其他地方共享"))?;
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

    fn file_entry(dir: &Path, name: &str, major: u32) -> crate::config::JavaConfig {
        let path = dir.join(name);
        std::fs::write(&path, b"").unwrap();
        crate::config::JavaConfig {
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

        let auto = ConfigJavaResolver {
            choice: JavaChoice::Auto,
            configs: configs.clone(),
        };
        let selected = JavaResolver::resolve(&auto, 17).await.unwrap();
        assert_eq!(selected.path, dir.join("seventeen"));

        let named = ConfigJavaResolver {
            choice: JavaChoice::Named("twentyone".into()),
            configs,
        };
        let selected = JavaResolver::resolve(&named, 8).await.unwrap();
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
                crate::config::JavaConfig {
                    path: dir.join("missing"),
                    major: 21,
                    arch: Arch::X86_64,
                },
            ),
            ("valid".to_string(), valid.clone()),
        ]);
        let resolver = ConfigJavaResolver {
            choice: JavaChoice::Auto,
            configs,
        };

        // 21 的路径不存在，只能选仍然存在的 17
        let selected = JavaResolver::resolve(&resolver, 21).await.unwrap();
        assert_eq!(selected.path, valid.path);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn auto_selection_without_a_usable_entry_points_at_search_java() {
        let dir = temp_java_dir("java-missing");
        let missing = dir.join("java");
        let configs = JavaConfigs::from([(
            "gone".to_string(),
            crate::config::JavaConfig {
                path: missing.clone(),
                major: 17,
                arch: Arch::X86_64,
            },
        )]);

        // 空表与「全部路径失效」都只认配置，不扫描本机。
        for configs in [&JavaConfigs::new(), &configs] {
            let resolver = ConfigJavaResolver {
                choice: JavaChoice::Auto,
                configs: configs.clone(),
            };
            let err = JavaResolver::resolve(&resolver, 17)
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("dmcl search-java"), "{err}");
        }

        let named = ConfigJavaResolver {
            choice: JavaChoice::Named("gone".into()),
            configs,
        };
        let err = JavaResolver::resolve(&named, 17)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("gone"), "{err}");
        assert!(err.contains(&missing.display().to_string()), "{err}");

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn unknown_named_java_is_reported_even_without_entries() {
        let resolver = ConfigJavaResolver {
            choice: JavaChoice::Named("nope".into()),
            configs: JavaConfigs::new(),
        };
        let err = JavaResolver::resolve(&resolver, 17)
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
        let java = ConfigJavaResolver {
            choice: JavaChoice::Named("jdk".into()),
            configs,
        };

        let option = JavaResolver::resolve(&java, 17).await.unwrap();
        assert_eq!(option.path, dir.join("jdk"));
        assert_eq!(option.major, 17);
        assert_eq!(option.arch, Arch::X86_64);

        std::fs::remove_dir_all(dir).unwrap();
    }
}
