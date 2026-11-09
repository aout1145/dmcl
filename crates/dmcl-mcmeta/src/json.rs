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

/// 只补 `target` 缺的键，已有值优先；对象递归处理，其它类型不覆盖。
pub(crate) fn fill_json_map(target: &mut JsonMap, source: &JsonMap) {
    for (key, value) in source {
        match target.get_mut(key) {
            Some(existing) => fill_json_value(existing, value),
            None => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn fill_json_value(target: &mut Value, source: &Value) {
    if let (Value::Object(target), Value::Object(source)) = (target, source) {
        fill_json_map(target, source);
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
