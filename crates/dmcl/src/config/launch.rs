//! `launch.*` 段，以及每个实例自己的覆盖配置。

use std::path::{Path, PathBuf};

use dmcl_launch::{JavaChoice, LAUNCHER_NAME};
use serde::{Deserialize, Serialize};

use super::{Config, ConfigError, ConfigWarning};

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
            java: JavaChoice::Auto,
            account: Some("player".to_string()),
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

        warnings.extend(super::validate_launch_choices(
            &mut launch,
            &self.accounts,
            &self.javas,
        ));
        instance_result(launch, path, warnings)
    }
}

fn instance_result(
    launch: LaunchConfig,
    path: PathBuf,
    warnings: Vec<ConfigWarning>,
) -> LoadedInstanceConfig {
    let error = (!warnings.is_empty()).then_some(ConfigError { path, warnings });
    LoadedInstanceConfig { launch, error }
}
