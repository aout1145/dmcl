//! `launch.*` 段、实例覆盖，以及启动 Options 转换。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use dmcl_launch::{LaunchOption, LAUNCHER_NAME};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{
    account, java, AccountConfig, Config, ConfigError, ConfigWarning, JavaChoice, JavaConfigs,
};

/// 全局 `launch.*` 段，也是实例覆盖后的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchConfig {
    /// 用哪一个 Java，或自动选择。
    pub java: JavaChoice,
    /// `accounts` 里的账户名；`null` 表示没有选择账户。
    pub account: Option<String>,
    /// 窗口尺寸（像素）—— 不可能为负。
    pub window_width: u32,
    pub window_height: u32,
    /// 堆大小（MiB）；`None` 表示自动分配。
    pub min_memory: Option<u32>,
    pub max_memory: Option<u32>,
    pub custom_info: String,
    pub additional_jvm: String,
    pub additional_game: String,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            java: JavaChoice::Auto,
            account: Some("player".to_string()),
            window_width: 854,
            window_height: 480,
            min_memory: None,
            max_memory: None,
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

/// 实例配置文件路径：`<root>/versions/<version_name>/dmcl.json`。
pub fn instance_config_path(root: &Path, version_name: &str) -> PathBuf {
    dmcl_mcmeta::layout::version_dir(root, version_name).join("dmcl.json")
}

/// 一次实例配置加载的结果。
#[derive(Debug, Clone)]
pub struct LoadedInstanceConfig {
    /// 已经与全局 `launch` 合并、并校验过账户 / Java 的结果。
    pub launch: LaunchConfig,
    /// 实例文件自身的解析 / 合并问题；文件不存在时为 `None`。
    pub error: Option<ConfigError>,
}

impl Config {
    /// 读取某个实例的 `dmcl.json`，并逐字段覆盖全局 `launch`。
    ///
    /// 文件不存在时直接使用全局 `launch`，**不创建文件**。
    pub async fn load_instance(&self, root: &Path, version_name: &str) -> LoadedInstanceConfig {
        let path = instance_config_path(root, version_name);
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => self.parse_instance(&text, path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => LoadedInstanceConfig {
                launch: self.launch.clone(),
                error: None,
            },
            Err(err) => self.instance_read_failure(path, format!("无法读取实例配置：{err}")),
        }
    }

    /// [`Config::load_instance`] 的同步版本，给 GTK 回调这类不能等待 future 的地方用。
    pub fn load_instance_sync(&self, root: &Path, version_name: &str) -> LoadedInstanceConfig {
        let path = instance_config_path(root, version_name);
        match std::fs::read_to_string(&path) {
            Ok(text) => self.parse_instance(&text, path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => LoadedInstanceConfig {
                launch: self.launch.clone(),
                error: None,
            },
            Err(err) => self.instance_read_failure(path, format!("无法读取实例配置：{err}")),
        }
    }

    fn instance_read_failure(&self, path: PathBuf, detail: String) -> LoadedInstanceConfig {
        LoadedInstanceConfig {
            launch: self.launch.clone(),
            error: Some(ConfigError {
                path,
                warnings: vec![ConfigWarning {
                    message: "实例配置使用全局 launch".to_string(),
                    detail,
                }],
            }),
        }
    }

    fn parse_instance(&self, text: &str, path: PathBuf) -> LoadedInstanceConfig {
        let mut warnings = Vec::new();
        let mut launch = self.launch.clone();

        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(value) => value,
            Err(err) => {
                warnings.push(ConfigWarning {
                    message: "实例配置使用全局 launch".to_string(),
                    detail: format!("无法解析实例配置 JSON：{err}"),
                });
                return instance_result(launch, path, warnings);
            }
        };
        let serde_json::Value::Object(sections) = value else {
            warnings.push(ConfigWarning {
                message: "实例配置使用全局 launch".to_string(),
                detail: "实例配置的顶层不是 JSON 对象".to_string(),
            });
            return instance_result(launch, path, warnings);
        };

        for (section, body) in sections {
            if section != "launch" {
                warnings.push(ConfigWarning {
                    message: format!("配置项「{section}」已忽略"),
                    detail: "实例配置只支持 launch".to_string(),
                });
                continue;
            }
            let serde_json::Value::Object(entries) = body else {
                warnings.push(ConfigWarning {
                    message: "配置项「launch」使用全局值".to_string(),
                    detail: "该段不是 JSON 对象".to_string(),
                });
                continue;
            };

            for (field, raw) in entries {
                let mut candidate = match serde_json::to_value(&launch) {
                    Ok(serde_json::Value::Object(object)) => object,
                    _ => {
                        warnings.push(ConfigWarning {
                            message: "配置项「launch」使用全局值".to_string(),
                            detail: "无法展开全局 launch".to_string(),
                        });
                        break;
                    }
                };
                if !candidate.contains_key(&field) {
                    warnings.push(ConfigWarning {
                        message: format!("配置项「launch.{field}」已忽略"),
                        detail: "未知配置项".to_string(),
                    });
                    continue;
                }
                let fallback = candidate[field.as_str()].clone();
                candidate.insert(field.clone(), raw);
                match serde_json::from_value::<LaunchConfig>(serde_json::Value::Object(candidate)) {
                    Ok(value) => launch = value,
                    Err(err) => warnings.push(ConfigWarning {
                        message: format!("配置项「launch.{field}」使用全局值 {fallback}"),
                        detail: format!("值不合法：{err}"),
                    }),
                }
            }
        }

        warnings.extend(validate_launch_choices(
            &mut launch,
            &self.accounts,
            &self.javas,
        ));
        instance_result(launch, path, warnings)
    }

    /// 配置 + 传入的启动配置 → [`LaunchOption`]。
    pub fn launch_option(
        &self,
        launch: &LaunchConfig,
        version_name: &str,
        root: &Path,
    ) -> Result<LaunchOption> {
        let account_source = self.selected_account_source(launch)?;
        let mut option = LaunchOption::new(account_source, root.to_path_buf(), version_name)
            .with_java(Arc::new(java::ConfigJava {
                choice: launch.java.clone(),
                configs: self.javas.clone(),
            }))
            .with_local_layout(self.download.skip_music);
        option.window_width = launch.window_width;
        option.window_height = launch.window_height;
        option.min_memory = launch.min_memory;
        option.max_memory = launch.max_memory;
        option.custom_info = launch.custom_info.clone();
        option.additional_jvm = launch
            .additional_jvm
            .split_whitespace()
            .map(str::to_string)
            .collect();
        option.additional_game = launch
            .additional_game
            .split_whitespace()
            .map(str::to_string)
            .collect();
        Ok(option)
    }
}

/// 账户 / Java 选择项回退；全局配置和实例配置共用。
pub(super) fn validate_launch_choices(
    launch: &mut LaunchConfig,
    accounts: &IndexMap<String, AccountConfig>,
    javas: &JavaConfigs,
) -> Vec<ConfigWarning> {
    let mut warnings = account::validate_selection(launch, accounts);
    warnings.extend(java::validate_selection(launch, javas));
    warnings
}

fn instance_result(
    launch: LaunchConfig,
    path: PathBuf,
    warnings: Vec<ConfigWarning>,
) -> LoadedInstanceConfig {
    let error = (!warnings.is_empty()).then_some(ConfigError { path, warnings });
    LoadedInstanceConfig { launch, error }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{temp_dir, JavaConfig};

    #[test]
    fn default_launch_uses_player_and_autoselect() {
        let launch = Config::default().launch;
        assert_eq!(launch.account.as_deref(), Some("player"));
        assert!(launch.java.is_auto());
        assert_eq!(launch.custom_info, LAUNCHER_NAME);
    }

    #[tokio::test]
    async fn a_missing_instance_file_uses_the_global_launch() {
        let dir = temp_dir("instance-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), dir.join("mc"));
        config.minecraft_root = "test".to_string();
        config.launch.max_memory = Some(3000);

        let root = dir.join("mc");
        let loaded = config.load_instance(&root, "1.20.1").await;
        assert!(loaded.error.is_none());
        assert_eq!(loaded.launch.max_memory, Some(3000));
        assert!(!instance_config_path(&root, "1.20.1").exists());
    }

    #[tokio::test]
    async fn instance_fields_override_only_what_they_mention() {
        let dir = temp_dir("instance-partial");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mc");
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), root.clone());
        config.minecraft_root = "test".to_string();
        config.launch.max_memory = Some(3000);

        let path = instance_config_path(&root, "1.20.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = r#"{"launch": {"max_memory": 4096, "account": null}}"#;
        std::fs::write(&path, text).unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.launch.max_memory, Some(4096));
        assert_eq!(loaded.launch.window_width, 854);
        assert_eq!(loaded.launch.account, None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        let sync = config.load_instance_sync(&root, "1.20.1");
        assert_eq!(sync.launch.max_memory, Some(4096));
        assert_eq!(sync.launch.account, None);
        assert!(sync.error.is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn invalid_instance_fields_fall_back_to_the_global_values() {
        let dir = temp_dir("instance-invalid");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mc");
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), root.clone());
        config.minecraft_root = "test".to_string();
        config.launch.max_memory = Some(3000);
        config.launch.window_width = 1000;

        let path = instance_config_path(&root, "1.20.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{
                "launch": {
                    "max_memory": -1,
                    "window_width": 1200,
                    "unknown": true
                },
                "download": {}
            }"#,
        )
        .unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert_eq!(loaded.launch.max_memory, Some(3000));
        assert_eq!(loaded.launch.window_width, 1200);
        let error = loaded.error.as_ref().expect("应当有警告");
        assert!(error
            .warnings
            .iter()
            .any(|warning| warning.message.contains("launch.max_memory")));
        assert!(error
            .warnings
            .iter()
            .any(|warning| warning.message.contains("launch.unknown")));
        assert!(error
            .warnings
            .iter()
            .any(|warning| warning.message.contains("download")));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn instance_account_and_java_selections_are_validated_after_merging() {
        let dir = temp_dir("instance-selection");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mc");
        let mut config = Config::default();
        config
            .minecraft_roots
            .insert("test".to_string(), root.clone());
        config.minecraft_root = "test".to_string();

        let path = instance_config_path(&root, "1.20.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"launch": {"account": "missing", "java": "missing"}}"#,
        )
        .unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert_eq!(loaded.launch.account, None);
        assert!(loaded.launch.java.is_auto());
        let error = loaded.error.as_ref().expect("应当有警告");
        assert_eq!(error.warnings.len(), 2);
        assert!(config
            .selected_account_source(&loaded.launch)
            .err()
            .expect("应当报错")
            .to_string()
            .contains("没有选择账户"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn launch_option_follows_the_config() {
        let dir = temp_dir("launch-options");
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mc");

        let mut config = Config::default();
        config.download.skip_music = true;
        config.accounts.insert(
            "steve".to_string(),
            AccountConfig::Offline {
                username: "Steve".to_string(),
            },
        );
        config.launch.account = Some("steve".to_string());
        config.launch.window_width = 1920;
        config.launch.min_memory = Some(1024);
        config.launch.max_memory = Some(4096);
        config.launch.custom_info = "test".to_string();
        config.launch.additional_jvm = "-Da=1  -Db=2".to_string();
        config.launch.additional_game = "--foo bar".to_string();
        config.javas.insert(
            "jdk".to_string(),
            JavaConfig {
                path: PathBuf::from("/nonexistent/bin/java"),
                major: 17,
                arch: dmcl_mcmeta::os::Arch::X86_64,
            },
        );
        config.launch.java = JavaChoice::Named("jdk".to_string());

        let option = config
            .launch_option(&config.launch, "1.20.1", &root)
            .unwrap();
        assert_eq!(option.root_path, root);
        assert_eq!(option.window_width, 1920);
        assert_eq!(option.min_memory, Some(1024));
        assert_eq!(option.max_memory, Some(4096));
        assert_eq!(option.custom_info, "test");
        assert!(option.skip_music);
        assert!(option.additional_jvm.contains(&"-Da=1".to_string()));
        assert!(option.additional_jvm.contains(&"-Db=2".to_string()));
        assert_eq!(option.additional_game, vec!["--foo", "bar"]);
        assert_eq!(option.account_source.name(), "Steve");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
