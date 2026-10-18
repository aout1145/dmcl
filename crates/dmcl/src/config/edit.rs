//! 配置的展开、写回与显示（交互编辑器与 `show` 共用）。
//!
//! 字段清单由 serde 派生自动给出（[`Config::fields`]），不手写。

use anyhow::{anyhow, Context, Result};

use super::Config;

impl Config {
    /// 人类可读的配置转储（字段名与 `dmcl.json` 的键一致）。
    ///
    /// 字段清单由 serde 派生自动给出（见 [`Config::fields`]），不手写。
    pub fn show(&self) {
        // 只有标题行是给人看的说明；下面的键名刻意保持与 dmcl.json 一致
        println!("配置文件：{}\n", Self::path().display());
        match self.fields() {
            Ok(fields) => {
                for (key, value) in fields {
                    println!("{key} = {}", display_value(&value));
                }
            }
            Err(err) => eprintln!("无法展开配置：{err:#}"),
        }
    }

    /// 把配置展开成「点分键 → JSON 值」的列表（`launch.window_width` …）。
    ///
    /// 键名来自 serde 序列化，因此新增字段不用改这里。只展开两层：顶层是各段
    /// （`launch` / `account` / `download`），段里的字段整体作为一个值
    /// （`launch.java_configs` 这种就按 JSON 编辑）。
    pub fn fields(&self) -> Result<Vec<(String, serde_json::Value)>> {
        let root = serde_json::to_value(self)?;
        let mut fields = Vec::new();
        let serde_json::Value::Object(sections) = root else {
            return Ok(fields);
        };
        for (section, body) in sections {
            match body {
                serde_json::Value::Object(entries) => {
                    for (field, value) in entries {
                        fields.push((format!("{section}.{field}"), value));
                    }
                }
                other => fields.push((section, other)),
            }
        }
        Ok(fields)
    }

    /// 按点分键写回一个 JSON 值。
    ///
    /// 写回后整体反序列化校验（`deny_unknown_fields`、类型等都在这道关卡上）；
    /// 校验失败时**不改动** `self`。
    pub fn set_field(&mut self, key: &str, value: serde_json::Value) -> Result<()> {
        let (section, field) = key
            .split_once('.')
            .ok_or_else(|| anyhow!("配置键格式应为 section.field，实际是 {key}"))?;
        let mut root = serde_json::to_value(&*self)?;
        let body = root
            .get_mut(section)
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| anyhow!("没有这一段配置：{section}"))?;
        body.insert(field.to_string(), value);
        *self = serde_json::from_value(root).with_context(|| format!("{key} 的值不合法"))?;
        Ok(())
    }
}

/// 配置值在菜单 / `show` 里的**单行**呈现：标量按普通文本，对象 / 数组按紧凑 JSON。
pub fn display_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => value.to_string(),
        other => other.to_string(),
    }
}

/// 对象 / 数组类型的值（结构化字段，**不支持**在交互编辑器里修改）。
pub fn is_json_value(value: &serde_json::Value) -> bool {
    matches!(
        value,
        serde_json::Value::Object(_) | serde_json::Value::Array(_)
    )
}

/// 把编辑后的文本解析成 JSON 值：类型按 `current` 推断（字符串不加引号、布尔写 true/false）。
///
/// 只处理标量；结构化值（对象 / 数组）不走这里。
pub fn parse_edited_value(text: &str, current: &serde_json::Value) -> Result<serde_json::Value> {
    match current {
        serde_json::Value::String(_) | serde_json::Value::Null => {
            Ok(serde_json::Value::String(text.to_string()))
        }
        serde_json::Value::Bool(_) => match text.trim() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            other => Err(anyhow!("需要 true 或 false，实际是 {other:?}")),
        },
        serde_json::Value::Number(_) => {
            let trimmed = text.trim();
            if let Ok(int) = trimmed.parse::<i64>() {
                Ok(int.into())
            } else if let Ok(uint) = trimmed.parse::<u64>() {
                Ok(uint.into())
            } else if let Ok(float) = trimmed.parse::<f64>() {
                serde_json::Number::from_f64(float)
                    .map(serde_json::Value::Number)
                    .ok_or_else(|| anyhow!("不是合法的数字：{trimmed:?}"))
            } else {
                Err(anyhow!("需要一个数字，实际是 {trimmed:?}"))
            }
        }
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            Err(anyhow!("结构化配置项不能在这里修改"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// 配置项清单由 serde 派生自动展开，不用手写；段里含的字段都在。
    #[test]
    fn fields_are_derived_from_serde() {
        let mut config = Config::default();
        config.launch.window_width = 1920;
        config
            .launch
            .java_configs
            .insert("jdk".to_string(), PathBuf::from("/usr/bin/java"));

        let fields = config.fields().unwrap();
        let keys: Vec<&str> = fields.iter().map(|(key, _)| key.as_str()).collect();
        for expected in [
            "launch.java",
            "launch.java_configs",
            "launch.minecraft_root_path",
            "launch.window_width",
            "account.username",
            "download.mirror",
        ] {
            assert!(keys.contains(&expected), "缺少 {expected}: {keys:?}");
        }
        let width = fields
            .iter()
            .find(|(key, _)| key == "launch.window_width")
            .map(|(_, value)| value.clone())
            .unwrap();
        assert_eq!(width, serde_json::json!(1920));
    }

    #[test]
    fn set_field_round_trips_and_rejects_invalid_values() {
        let mut config = Config::default();
        config
            .set_field("launch.window_width", serde_json::json!(1920))
            .unwrap();
        assert_eq!(config.launch.window_width, 1920);

        // 写回后整体反序列化校验：负数会被 `u32` 拒绝，且不改动原值
        assert!(config
            .set_field("launch.window_width", serde_json::json!(-1))
            .is_err());
        assert_eq!(config.launch.window_width, 1920);

        // 名字写错 / 段不存在都要报错
        assert!(config
            .set_field("launch.nope", serde_json::json!(1))
            .is_err());
        assert!(config.set_field("nope.a", serde_json::json!(1)).is_err());

        // java_configs 按 JSON 对象写回
        config
            .set_field(
                "launch.java_configs",
                serde_json::json!({ "jdk": "/usr/bin/java" }),
            )
            .unwrap();
        assert_eq!(
            config.launch.java_configs.get("jdk").unwrap(),
            &PathBuf::from("/usr/bin/java")
        );
    }

    #[test]
    fn edited_text_is_parsed_by_the_current_type() {
        use serde_json::json;
        assert_eq!(
            parse_edited_value("hello", &json!("x")).unwrap(),
            json!("hello")
        );
        assert_eq!(
            parse_edited_value("true", &json!(false)).unwrap(),
            json!(true)
        );
        assert_eq!(parse_edited_value(" 42 ", &json!(0)).unwrap(), json!(42));
        assert!(parse_edited_value("abc", &json!(0)).is_err());
        assert!(parse_edited_value("maybe", &json!(true)).is_err());
        // 结构化值不走这里（编辑器直接提示无法修改）
        assert!(parse_edited_value(r#"{"a":"b"}"#, &json!({})).is_err());
    }

    #[test]
    fn values_are_displayed_compactly() {
        use serde_json::json;
        assert_eq!(display_value(&json!("text")), "text");
        assert_eq!(display_value(&json!(854)), "854");
        assert_eq!(display_value(&json!(true)), "true");
        assert_eq!(display_value(&json!({"a": "b"})), r#"{"a":"b"}"#);
        assert!(is_json_value(&json!({})));
        assert!(!is_json_value(&json!("x")));
    }
}
