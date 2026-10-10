//! 版本 JSON 里的 `rules` 判定。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::macros::string_enum;
use crate::os::get_os_name;

/// 便捷别名，等同于 `serde_json` 默认的 map 类型。
pub type JsonMap = Map<String, Value>;

string_enum! {
    /// 规则的 `action` —— 原本是裸字符串，到处和 `"allow"` 比较。
    RuleAction {
        Allow => "allow",
        Disallow => "disallow",
    }
}

/// 规则里的 `os` 子对象。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OsRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `rules` 数组的一个元素。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rule {
    /// 键不存在时为 `None`；不认识的取值保留为 [`RuleAction::Other`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<RuleAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<JsonMap>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `checkRules` —— 判断一组规则在当前平台上是否成立。
///
/// 忠实保留原实现的两个怪癖：任何带 `features` 的规则都会让整组规则失效；
/// 带 `os.version` 约束的 `allow` 规则同样让整组失效。
pub fn check_rules(rules: &[Rule]) -> bool {
    for rule in rules {
        if rule.features.is_some() {
            // ignore features
            return false;
        }
        match rule.action.as_ref() {
            Some(RuleAction::Allow) => {
                if let Some(os) = &rule.os {
                    let name_mismatch = match os.name.as_deref() {
                        Some(name) => name != get_os_name(),
                        None => true, // ("name" !in rule["os"].object)
                    };
                    if name_mismatch || os.version.is_some() {
                        return false;
                    }
                }
            }
            Some(RuleAction::Disallow) => {
                if let Some(os) = &rule.os {
                    if os.name.as_deref() == Some(get_os_name()) {
                        return false;
                    }
                }
            }
            // 没有 action、或有我们不认识的 action：这条规则永远不生效
            _ => {}
        }
    }
    true
}

/// `Option<Vec<Rule>>` 的语法糖，对应 `if ("rules" !in lib || checkRules(...))`。
pub fn rules_allow(rules: &Option<Vec<Rule>>) -> bool {
    match rules {
        None => true,
        Some(rules) => check_rules(rules),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rules(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn rule_objects_follow_the_original_semantics() {
        let os = get_os_name();
        let other = if os == "windows" { "linux" } else { "windows" };

        assert!(check_rules(&parse_rules(&format!(
            r#"[{{"action":"allow","os":{{"name":"{os}"}}}}]"#
        ))));
        assert!(!check_rules(&parse_rules(&format!(
            r#"[{{"action":"allow","os":{{"name":"{other}"}}}}]"#
        ))));
        // 没有 os.name 的 allow 规则永远不成立
        assert!(!check_rules(&parse_rules(
            r#"[{"action":"allow","os":{}}]"#
        )));
        // 带 os.version 约束的 allow 规则同样不成立
        assert!(!check_rules(&parse_rules(&format!(
            r#"[{{"action":"allow","os":{{"name":"{os}","version":"^10"}}}}]"#
        ))));
        assert!(!check_rules(&parse_rules(&format!(
            r#"[{{"action":"disallow","os":{{"name":"{os}"}}}}]"#
        ))));
        // 任何带 features 的规则让整组失效
        assert!(!check_rules(&parse_rules(
            r#"[{"action":"allow","features":{"has_custom_resolution":true}}]"#
        )));
        // 没有规则就是允许
        assert!(check_rules(&[]));
        assert!(rules_allow(&None));
        assert!(rules_allow(&Some(vec![])));
    }

    #[test]
    fn string_enums_keep_unknown_values() {
        // `allow` / `disallow` 已经类型化，但不认识的取值必须能原样往返，
        // 否则回写版本 JSON 时会丢信息。
        let rule: Rule = serde_json::from_str(r#"{"action":"allow"}"#).unwrap();
        assert_eq!(rule.action, Some(RuleAction::Allow));
        let rule: Rule = serde_json::from_str(r#"{"action":"maybe"}"#).unwrap();
        assert_eq!(rule.action, Some(RuleAction::Other("maybe".to_string())));
        let written = serde_json::to_value(&rule).unwrap();
        assert_eq!(written["action"], "maybe");
        // 完全没有 action 时保持"缺键"而不是写出 null
        let rule: Rule = serde_json::from_str(r#"{"os":{"name":"linux"}}"#).unwrap();
        assert_eq!(rule.action, None);
        assert!(serde_json::to_value(&rule).unwrap().get("action").is_none());
    }
}
