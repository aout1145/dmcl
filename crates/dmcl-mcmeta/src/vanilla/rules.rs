//! 版本 JSON 里的 `rules` 判定（标准 Mojang 语义）。

use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::json::JsonMap;
use crate::macros::string_enum;
use crate::os::{get_os_name, os_version};

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

/// 判定规则时的"平台 + 特性"上下文。
///
/// `RuleContext::current()` 取本机的系统名 / 架构 / 系统版本，特性默认为空。
#[derive(Debug, Clone)]
pub struct RuleContext {
    /// `windows` / `osx` / `linux`（见 [`get_os_name`]）。
    pub os_name: &'static str,
    /// `std::env::consts::ARCH`（`x86_64` / `aarch64` …）。
    pub arch: &'static str,
    /// 系统版本，用于匹配 `os.version` 里的正则。
    pub os_version: Option<String>,
    /// 启动器实际开启的特性（`has_custom_resolution` …）。
    pub features: BTreeMap<String, bool>,
}

impl RuleContext {
    /// 本机平台 + 空特性。
    pub fn current() -> Self {
        Self {
            os_name: get_os_name(),
            arch: std::env::consts::ARCH,
            os_version: Some(os_version()),
            features: BTreeMap::new(),
        }
    }
}

impl Rule {
    /// 这条规则的**条件**是否命中（与 [`Rule::action`] 无关）。
    pub fn matches(&self, ctx: &RuleContext) -> bool {
        if let Some(os) = &self.os {
            if let Some(name) = &os.name {
                if name != ctx.os_name {
                    return false;
                }
            }
            if let Some(arch) = &os.arch {
                if arch != ctx.arch {
                    return false;
                }
            }
            if let Some(pattern) = &os.version {
                let Ok(regex) = Regex::new(pattern) else {
                    return false;
                };
                match &ctx.os_version {
                    Some(version) if regex.is_match(version) => {}
                    _ => return false,
                }
            }
        }

        if let Some(features) = &self.features {
            for (name, expected) in features {
                let actual = ctx.features.get(name).copied().unwrap_or(false);
                let expected = expected.as_bool().unwrap_or(false);
                if actual != expected {
                    return false;
                }
            }
        }

        true
    }
}

/// `checkRules` —— 标准 Mojang 语义：
///
/// * 空列表 → 允许；
/// * 否则按顺序评估，**最后一条命中的规则的 `action`** 决定结果；
/// * 一条都没命中 / 命中的是未知 `action` → 不允许（保守）。
pub fn check_rules(rules: &[Rule], ctx: &RuleContext) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(ctx) {
            allowed = matches!(rule.action.as_ref(), Some(RuleAction::Allow));
        }
    }
    allowed
}

/// `Option<Vec<Rule>>` 的语法糖，对应 `if ("rules" !in lib || checkRules(...))`。
pub fn rules_allow(rules: &Option<Vec<Rule>>, ctx: &RuleContext) -> bool {
    match rules {
        None => true,
        Some(rules) => check_rules(rules, ctx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rules(json: &str) -> Vec<Rule> {
        serde_json::from_str(json).unwrap()
    }

    fn ctx(os_name: &'static str) -> RuleContext {
        RuleContext {
            os_name,
            arch: "x86_64",
            os_version: None,
            features: BTreeMap::new(),
        }
    }

    #[test]
    fn allow_rules_gate_on_the_os() {
        assert!(check_rules(
            &parse_rules(r#"[{"action":"allow","os":{"name":"linux"}}]"#),
            &ctx("linux")
        ));
        // 不命中 → 默认不允许
        assert!(!check_rules(
            &parse_rules(r#"[{"action":"allow","os":{"name":"linux"}}]"#),
            &ctx("windows")
        ));
    }

    #[test]
    fn the_last_matching_rule_wins() {
        let rules =
            parse_rules(r#"[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]"#);
        assert!(check_rules(&rules, &ctx("linux")));
        assert!(!check_rules(&rules, &ctx("osx")));

        // 后者覆盖前者
        let rules = parse_rules(r#"[{"action":"disallow"},{"action":"allow"}]"#);
        assert!(check_rules(&rules, &ctx("linux")));
    }

    #[test]
    fn empty_rules_and_no_rules_are_allowed() {
        assert!(check_rules(&[], &ctx("linux")));
        assert!(rules_allow(&None, &ctx("linux")));
        assert!(rules_allow(&Some(vec![]), &ctx("linux")));
    }

    #[test]
    fn features_must_match_then_actual_launcher_features() {
        let rules =
            parse_rules(r#"[{"action":"allow","features":{"has_custom_resolution":true}}]"#);
        let mut with = ctx("linux");
        assert!(!check_rules(&rules, &with));
        with.features.insert("has_custom_resolution".into(), true);
        assert!(check_rules(&rules, &with));
    }

    #[test]
    fn os_version_matches_as_a_regex() {
        let rules = parse_rules(r#"[{"action":"allow","os":{"name":"osx","version":"^10\\."}}]"#);
        let mut mac = ctx("osx");
        mac.os_version = Some("10.5.8".into());
        assert!(check_rules(&rules, &mac));
        mac.os_version = Some("11.0".into());
        assert!(!check_rules(&rules, &mac));
    }

    #[test]
    fn unknown_action_is_disallowed() {
        assert!(!check_rules(
            &parse_rules(r#"[{"action":"maybe"}]"#),
            &ctx("linux")
        ));
        assert!(!check_rules(&parse_rules(r#"[{}]"#), &ctx("linux")));
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
