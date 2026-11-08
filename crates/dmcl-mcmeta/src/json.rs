//! 共享的 JSON 别名。

use serde_json::{Map, Value};

/// 便捷别名，等同于 `serde_json` 默认的 map 类型（用于 `#[serde(flatten)] extra`）。
pub type JsonMap = Map<String, Value>;

/// 对象逐键递归合并，数组拼接，其它情况由 `source` 覆盖。
pub(crate) fn merge_json_map(target: &mut JsonMap, source: &JsonMap) {
    for (key, value) in source {
        match target.get_mut(key) {
            Some(existing) => merge_json_value(existing, value),
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// 单个 JSON 值的递归合并，语义同 [`merge_json_map`]。
pub(crate) fn merge_json_value(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(t), Value::Object(s)) => merge_json_map(t, s),
        (Value::Array(t), Value::Array(s)) => t.extend(s.iter().cloned()),
        (t, s) => *t = s.clone(),
    }
}
