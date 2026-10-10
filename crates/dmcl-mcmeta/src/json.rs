//! 共享的 JSON 别名。

use serde_json::{Map, Value};

/// 便捷别名，等同于 `serde_json` 默认的 map 类型（用于 `#[serde(flatten)] extra`）。
pub type JsonMap = Map<String, Value>;
