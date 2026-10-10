//! `mergeJSON` —— 递归合并两份 JSON。

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

/// `mergeJSON` —— 对象逐键递归合并，数组拼接，其它情况由 `source` 覆盖。
pub fn merge_value(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(t), Value::Object(s)) => {
            for (key, value) in s {
                if let Some(existing) = t.get_mut(key) {
                    merge_value(existing, value);
                } else {
                    t.insert(key.clone(), value.clone());
                }
            }
        }
        (Value::Array(t), Value::Array(s)) => {
            t.extend(s.iter().cloned());
        }
        (t, s) => {
            *t = s.clone();
        }
    }
}

/// [`merge_value`] 的强类型版本：`base` 按 `mergeJSON` 的语义合并 `child`，
/// 实现方式是两边都走一遍 `serde_json`。
///
/// 因为序列化时会跳过不存在的可选字段，"子文档里没提到某个键"就等价于原实现的
/// "保留父文档的值"；未建模的字段都待在结构体的 `extra` 里，所以它们同样参与合并。
pub fn merge_typed<T>(base: &mut T, child: &T) -> anyhow::Result<()>
where
    T: Serialize + DeserializeOwned,
{
    let mut value = serde_json::to_value(&*base)?;
    merge_value(&mut value, &serde_json::to_value(child)?);
    *base = serde_json::from_value(value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_json_semantics_are_preserved() {
        let mut target = serde_json::json!({
            "a": 1,
            "obj": {"x": 1, "y": 2},
            "arr": [1],
        });
        let source = serde_json::json!({
            "a": 2,
            "obj": {"y": 3, "z": 4},
            "arr": [2, 3],
            "new": true,
        });
        merge_value(&mut target, &source);

        assert_eq!(target["a"], 2);
        // 对象递归合并
        assert_eq!(target["obj"]["x"], 1);
        assert_eq!(target["obj"]["y"], 3);
        assert_eq!(target["obj"]["z"], 4);
        // 数组拼接（父在前）
        assert_eq!(target["arr"], serde_json::json!([1, 2, 3]));
        assert_eq!(target["new"], true);
    }

    #[test]
    fn merge_typed_round_trips_through_the_same_algorithm() {
        #[derive(Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
        struct Doc {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            #[serde(default)]
            items: Vec<String>,
        }

        let mut base = Doc {
            name: Some("base".to_string()),
            items: vec!["a".to_string()],
        };
        // `name` 没被子文档提到 → 保留；`items` 拼接
        let child = Doc {
            name: None,
            items: vec!["b".to_string()],
        };
        merge_typed(&mut base, &child).unwrap();
        assert_eq!(base.name.as_deref(), Some("base"));
        assert_eq!(base.items, vec!["a".to_string(), "b".to_string()]);
    }
}
