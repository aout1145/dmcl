//! `launch.*` 段、实例覆盖，以及启动 Options 转换。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use dmcl_game::VersionRef;
use dmcl_java::{JavaInstalledHook, JavaResolver};
use dmcl_launch::{
    JvmCompatibility as LaunchJvmCompatibility, LaunchOption, ProcessPriority as LaunchPriority,
    QuickPlay as LaunchQuickPlay, WindowMode as LaunchWindowMode, LAUNCHER_NAME,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::{
    account, instance::instance_config_path, java, AccountConfig, Config, ConfigError,
    ConfigWarning, JavaChoice, JavaConfigs,
};

/// 快速游玩的目标类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickPlayTarget {
    /// 单人世界的文件夹名。
    Singleplayer,
    /// 多人服务器地址。
    Multiplayer,
    /// Realms ID。
    Realms,
}

/// 启动后直接进入的世界 / 服务器。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuickPlayConfig {
    pub target: QuickPlayTarget,
    pub value: String,
}

/// 游戏进程的优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessPriority {
    /// 低于正常优先级。
    Low,
    /// 正常优先级。
    #[default]
    Normal,
    /// 高于正常优先级。
    High,
}

/// 启动时对所选 Java 的兼容性检查。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JvmCompatibility {
    /// 不检查 Java 版本与架构，直接使用选中的 Java。
    Ignore,
    /// 按用户的 Java 选择检查版本与架构；不匹配时等用户确认。
    Check,
    /// 自动选择 Java 时，没有合适的 Java 就下载；手动指定时同 [`Self::Check`]。
    #[default]
    Complete,
}

/// 全局 `launch.*` 段，也是实例覆盖后的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LaunchConfig {
    /// 用哪一个 Java，或自动选择。
    pub java: JavaChoice,
    /// `accounts` 里的账户名；`null` 表示没有选择账户。
    pub account: Option<String>,
    /// 是否为每个版本使用独立的游戏目录（`versions/<版本名>`）；关闭后共用根目录。
    pub version_isolation: bool,
    /// 是否全屏启动；关闭时使用下面的窗口宽高。
    pub fullscreen: bool,
    /// 窗口化时的初始尺寸（像素）—— 不可能为负。
    pub window_width: u32,
    pub window_height: u32,
    /// 启动后直接进入的单人世界、服务器或 Realm；`None` 表示进入主菜单。
    pub quick_play: Option<QuickPlayConfig>,
    /// 游戏进程的优先级。
    pub process_priority: ProcessPriority,
    /// 堆大小（MiB）；`None` 表示自动分配。
    pub min_memory: Option<u32>,
    pub max_memory: Option<u32>,
    pub custom_info: String,
    /// 附加 JVM 参数；`None` 表示使用默认参数，`Some("")` 表示自定义但未填写。
    pub additional_jvm: Option<String>,
    /// 启动时对所选 Java 的兼容性检查。
    pub jvm_compatibility: JvmCompatibility,
    pub additional_game: String,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            java: JavaChoice::Auto,
            account: None,
            version_isolation: true,
            fullscreen: false,
            window_width: 854,
            window_height: 480,
            quick_play: None,
            process_priority: ProcessPriority::Normal,
            min_memory: None,
            max_memory: None,
            custom_info: LAUNCHER_NAME.to_string(),
            additional_jvm: None,
            jvm_compatibility: JvmCompatibility::Complete,
            additional_game: String::new(),
        }
    }
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
    /// 文件不存在时直接使用全局 `launch`，**不创建文件**。`instance` 段属于实例元数据，
    /// 不参与启动配置，由 [`read_instance_category`](crate::config::read_instance_category)
    /// 单独读取。
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
            match section.as_str() {
                "launch" => {}
                // 实例元数据（分类等）不参与启动配置，由 config::instance 读取。
                "instance" => continue,
                _ => {
                    warnings.push(ConfigWarning {
                        message: format!("配置项「{section}」已忽略"),
                        detail: "实例配置只支持 launch / instance".to_string(),
                    });
                    continue;
                }
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

    /// 配置 + 版本 + Java 解析器 → [`LaunchOption`]。
    pub fn launch_option(
        &self,
        launch: &LaunchConfig,
        version: VersionRef,
        java: Arc<dyn JavaResolver>,
        java_installed_hook: Option<Arc<dyn JavaInstalledHook>>,
    ) -> Result<LaunchOption> {
        let account_source = self.selected_account_source(launch)?;
        let mut option = LaunchOption::new(account_source, version, java, java_installed_hook);
        option.window = if launch.fullscreen {
            LaunchWindowMode::Fullscreen
        } else {
            LaunchWindowMode::Windowed {
                width: launch.window_width,
                height: launch.window_height,
            }
        };
        option.min_memory = launch.min_memory;
        option.max_memory = launch.max_memory;
        option.custom_info = launch.custom_info.clone();
        option.version_isolation = launch.version_isolation;
        option.quick_play = launch.quick_play.as_ref().and_then(|quick| {
            let value = quick.value.trim();
            if value.is_empty() {
                return None;
            }
            Some(match quick.target {
                QuickPlayTarget::Singleplayer => LaunchQuickPlay::Singleplayer(value.to_string()),
                QuickPlayTarget::Multiplayer => LaunchQuickPlay::Multiplayer(value.to_string()),
                QuickPlayTarget::Realms => LaunchQuickPlay::Realms(value.to_string()),
            })
        });
        option.process_priority = match launch.process_priority {
            ProcessPriority::Low => LaunchPriority::Low,
            ProcessPriority::Normal => LaunchPriority::Normal,
            ProcessPriority::High => LaunchPriority::High,
        };
        option.jvm_compatibility = match launch.jvm_compatibility {
            JvmCompatibility::Ignore => LaunchJvmCompatibility::Ignore,
            JvmCompatibility::Check => LaunchJvmCompatibility::Check,
            JvmCompatibility::Complete => LaunchJvmCompatibility::Complete,
        };
        option.additional_jvm = launch
            .additional_jvm
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        option.additional_game = shell_words::split(&launch.additional_game)
            .map_err(|_| anyhow!("游戏参数引号不配对：{}", launch.additional_game))?;
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
    fn default_jvm_args_serialize_as_null() {
        let value = serde_json::to_value(LaunchConfig::default()).unwrap();
        assert!(value["additional_jvm"].is_null());
        assert_eq!(value["jvm_compatibility"], "complete");
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
    async fn instance_section_does_not_become_a_launch_warning() {
        let dir = temp_dir("instance-metadata-section");
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
            r#"{"instance":{"category":"整合包"},"launch":{"max_memory":4096}}"#,
        )
        .unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.launch.max_memory, Some(4096));
        assert_eq!(
            crate::config::read_instance_category(&root, "1.20.1").await,
            Some("整合包".to_string())
        );

        std::fs::remove_dir_all(&dir).unwrap();
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
        config.launch.additional_jvm = Some("-Dglobal=1".to_string());

        let path = instance_config_path(&root, "1.20.1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = serde_json::json!({
            "launch": {
                "max_memory": 4096,
                "account": null,
                "additional_jvm": null,
                "version_isolation": false,
            }
        })
        .to_string();
        std::fs::write(&path, &text).unwrap();

        let loaded = config.load_instance(&root, "1.20.1").await;
        assert!(loaded.error.is_none(), "{:?}", loaded.error);
        assert_eq!(loaded.launch.max_memory, Some(4096));
        assert_eq!(loaded.launch.window_width, 854);
        assert_eq!(loaded.launch.account, None);
        // 显式 null 表示使用默认 JVM 参数，不能退回全局的自定义值。
        assert_eq!(loaded.launch.additional_jvm, None);
        assert!(!loaded.launch.version_isolation);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        let sync = config.load_instance_sync(&root, "1.20.1");
        assert_eq!(sync.launch.max_memory, Some(4096));
        assert_eq!(sync.launch.account, None);
        assert!(!sync.launch.version_isolation);
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
        config.launch.window_height = 1080;
        config.launch.min_memory = Some(1024);
        config.launch.max_memory = Some(4096);
        config.launch.custom_info = "test".to_string();
        config.launch.additional_jvm = Some("-Da=1  -Db=2".to_string());
        config.launch.additional_game = "--foo \"bar baz\"".to_string();
        config.launch.quick_play = Some(QuickPlayConfig {
            target: QuickPlayTarget::Multiplayer,
            value: "example.org".to_string(),
        });
        config.launch.process_priority = ProcessPriority::High;
        config.launch.jvm_compatibility = JvmCompatibility::Ignore;
        config.launch.version_isolation = false;
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
            .launch_option(
                &config.launch,
                VersionRef::new(&root, "1.20.1"),
                Arc::new(dmcl_java::JavaOption {
                    path: PathBuf::from("/nonexistent/bin/java"),
                    major: 17,
                    arch: dmcl_mcmeta::os::Arch::X86_64,
                }),
                None,
            )
            .unwrap();
        assert_eq!(option.version.root, root);
        assert_eq!(option.version.name, "1.20.1");
        assert_eq!(
            option.window,
            LaunchWindowMode::Windowed {
                width: 1920,
                height: 1080
            }
        );
        assert_eq!(option.min_memory, Some(1024));
        assert_eq!(option.max_memory, Some(4096));
        assert_eq!(option.custom_info, "test");
        assert!(option.additional_jvm.contains(&"-Da=1".to_string()));
        assert!(option.additional_jvm.contains(&"-Db=2".to_string()));
        assert_eq!(option.additional_game, vec!["--foo", "bar baz"]);
        assert_eq!(
            option.quick_play,
            Some(LaunchQuickPlay::Multiplayer("example.org".to_string()))
        );
        assert_eq!(option.process_priority, LaunchPriority::High);
        assert_eq!(option.jvm_compatibility, LaunchJvmCompatibility::Ignore);
        assert!(!option.version_isolation);
        assert_eq!(option.account_source.name(), "Steve");

        config.launch.fullscreen = true;
        let option = config
            .launch_option(
                &config.launch,
                VersionRef::new(&root, "1.20.1"),
                Arc::new(dmcl_java::JavaOption {
                    path: PathBuf::from("/nonexistent/bin/java"),
                    major: 17,
                    arch: dmcl_mcmeta::os::Arch::X86_64,
                }),
                None,
            )
            .unwrap();
        assert_eq!(option.window, LaunchWindowMode::Fullscreen);

        config.launch.additional_game = "--foo \"bar".to_string();
        let error = config
            .launch_option(
                &config.launch,
                VersionRef::new(&root, "1.20.1"),
                Arc::new(dmcl_java::JavaOption {
                    path: PathBuf::from("/nonexistent/bin/java"),
                    major: 17,
                    arch: dmcl_mcmeta::os::Arch::X86_64,
                }),
                None,
            )
            .unwrap_err();
        assert!(error.to_string().contains("引号不配对"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
