//! 版本 JSON 的强类型合并。
//!
//! `self` 是父版本，`child` 是子版本。字段语义见 [`VersionJson::merge`]。

use crate::json::merge_json_map;
use crate::vanilla::rules::Rule;
use crate::vanilla::version::{Argument, Arguments, Library, VersionJson};

impl VersionJson {
    /// 把 `child` 合并进 `self`（`self` 是父版本）。
    ///
    /// * 标量 / 可选块：子版本有值就覆盖，没有就保留父版本；
    /// * `arguments`：每个 key 父在前、子在后追加；
    /// * `libraries`：子版本库在前、父版本追加在后，同键 + 同 rules 时版本号高者胜出；
    /// * `patches`：不合并历史，直接采用子版本的值（子版本没有就清空）。
    pub fn merge(&mut self, child: &VersionJson) {
        replace_if_some(&mut self.id, &child.id);
        replace_if_some(&mut self.jar, &child.jar);
        replace_if_some(&mut self.inherits_from, &child.inherits_from);
        replace_if_some(&mut self.main_class, &child.main_class);
        replace_if_some(&mut self.minecraft_arguments, &child.minecraft_arguments);
        replace_if_some(&mut self.asset_index, &child.asset_index);
        replace_if_some(&mut self.java_version, &child.java_version);
        replace_if_some(&mut self.downloads, &child.downloads);
        replace_if_some(&mut self.assets, &child.assets);
        replace_if_some(&mut self.compliance_level, &child.compliance_level);
        replace_if_some(
            &mut self.minimum_launcher_version,
            &child.minimum_launcher_version,
        );
        replace_if_some(&mut self.release_time, &child.release_time);
        replace_if_some(&mut self.time, &child.time);
        replace_if_some(&mut self.kind, &child.kind);
        replace_if_some(&mut self.logging, &child.logging);

        self.arguments = merge_arguments(self.arguments.take(), child.arguments.clone());

        let parent_libraries = std::mem::take(&mut self.libraries);
        self.libraries = merge_libraries(&parent_libraries, &child.libraries);

        // patches 不合并历史：子版本有什么就用什么，没有就清空
        self.patches = child.patches.clone();

        merge_json_map(&mut self.extra, &child.extra);
    }
}

fn replace_if_some<T: Clone>(target: &mut Option<T>, source: &Option<T>) {
    if source.is_some() {
        *target = source.clone();
    }
}

fn merge_arguments(parent: Option<Arguments>, child: Option<Arguments>) -> Option<Arguments> {
    match (parent, child) {
        (None, None) => None,
        (Some(parent), None) => Some(parent),
        (None, Some(child)) => Some(child),
        (Some(mut parent), Some(child)) => {
            append_arguments(&mut parent.jvm, &child.jvm);
            append_arguments(&mut parent.game, &child.game);
            append_arguments(&mut parent.default_user_jvm, &child.default_user_jvm);
            merge_json_map(&mut parent.extra, &child.extra);
            Some(parent)
        }
    }
}

fn append_arguments(target: &mut Option<Vec<Argument>>, source: &Option<Vec<Argument>>) {
    if let Some(source) = source {
        match target {
            Some(target) => target.extend(source.iter().cloned()),
            None => *target = Some(source.clone()),
        }
    }
}

/// 共同核心：child（子版本 / 新 patch）库在前，parent（父版本 / 当前值）追加在后。
/// 同键 + 同 rules 命中时，由 `parent_wins` 决定是否用 parent 替换 child。
pub(super) fn combine_libraries(
    parent: &[Library],
    child: &[Library],
    parent_wins: impl Fn(&Library, &Library) -> bool,
) -> Vec<Library> {
    let mut result = child.to_vec();

    for parent_lib in parent {
        let key = crate::maven::library_key(&parent_lib.name);
        let existing = result.iter().position(|child_lib| {
            crate::maven::library_key(&child_lib.name) == key
                && same_rules(&child_lib.rules, &parent_lib.rules)
        });

        match existing {
            Some(index) if parent_wins(parent_lib, &result[index]) => {
                // 父版本更高：替换，但保留子版本的位置
                result[index] = parent_lib.clone();
            }
            Some(_) => {
                // 否则保留子版本
            }
            None => result.push(parent_lib.clone()),
        }
    }

    result
}

/// 普通版本合并：同键 + 同 rules 时版本号高者胜出。
fn merge_libraries(parent: &[Library], child: &[Library]) -> Vec<Library> {
    combine_libraries(parent, child, |parent, child| {
        version_is_higher(&parent.name, &child.name)
    })
}

pub(super) fn same_rules(a: &Option<Vec<Rule>>, b: &Option<Vec<Rule>>) -> bool {
    a.as_deref().unwrap_or(&[]) == b.as_deref().unwrap_or(&[])
}

pub(super) fn version_is_higher(candidate: &str, current: &str) -> bool {
    let candidate = crate::maven::lib_name_get_version(candidate);
    let current = crate::maven::lib_name_get_version(current);
    version_compare::compare_to(&candidate, &current, version_compare::Cmp::Gt).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: serde_json::Value) -> VersionJson {
        serde_json::from_value(value).unwrap()
    }

    fn library_names(json: &VersionJson) -> Vec<&str> {
        json.libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect()
    }

    fn argument_values(arguments: &Option<Vec<Argument>>) -> Vec<String> {
        arguments
            .as_ref()
            .unwrap()
            .iter()
            .filter_map(|argument| match argument {
                Argument::Plain(value) => Some(value.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn scalar_fields_and_legacy_arguments_follow_the_child() {
        let mut parent = parse(serde_json::json!({
            "id": "base",
            "mainClass": "a.Main",
            "minecraftArguments": "--old"
        }));
        let child = parse(serde_json::json!({
            "id": "child",
            "minecraftArguments": "--new"
        }));

        parent.merge(&child);

        assert_eq!(parent.id.as_deref(), Some("child"));
        assert_eq!(parent.main_class.as_deref(), Some("a.Main"));
        assert_eq!(parent.minecraft_arguments.as_deref(), Some("--new"));
    }

    #[test]
    fn arguments_append_parent_before_child_for_every_key() {
        let mut parent = parse(serde_json::json!({
            "arguments": {
                "game": ["--one"],
                "jvm": ["-Xa"],
                "default-user-jvm": ["-Da"]
            }
        }));
        let child = parse(serde_json::json!({
            "arguments": {
                "game": ["--two"],
                "jvm": ["-Xb"],
                "default-user-jvm": ["-Db"]
            }
        }));

        parent.merge(&child);

        let arguments = parent.arguments.as_ref().unwrap();
        assert_eq!(argument_values(&arguments.game), ["--one", "--two"]);
        assert_eq!(argument_values(&arguments.jvm), ["-Xa", "-Xb"]);
        assert_eq!(argument_values(&arguments.default_user_jvm), ["-Da", "-Db"]);
    }

    #[test]
    fn libraries_put_the_child_first_and_append_the_parent() {
        let mut parent = parse(serde_json::json!({
            "libraries": [{"name": "a:a:1"}]
        }));
        let child = parse(serde_json::json!({
            "libraries": [{"name": "b:b:2"}]
        }));

        parent.merge(&child);

        assert_eq!(library_names(&parent), ["b:b:2", "a:a:1"]);
    }

    #[test]
    fn the_higher_version_wins_and_keeps_the_child_position() {
        let mut parent = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:9"}]
        }));
        let child = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:2"}, {"name": "b:b:1"}]
        }));

        parent.merge(&child);

        assert_eq!(library_names(&parent), ["foo:bar:9", "b:b:1"]);
    }

    #[test]
    fn a_higher_child_version_is_kept() {
        let mut parent = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:1"}]
        }));
        let child = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:2"}]
        }));

        parent.merge(&child);

        assert_eq!(library_names(&parent), ["foo:bar:2"]);
    }

    #[test]
    fn an_incomparable_version_keeps_the_child() {
        let mut parent = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:9"}]
        }));
        let child = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:dev"}]
        }));

        parent.merge(&child);

        assert_eq!(library_names(&parent), ["foo:bar:dev"]);
    }

    #[test]
    fn different_rules_keep_both_variants() {
        let mut parent = parse(serde_json::json!({
            "libraries": [{
                "name": "foo:bar:1",
                "rules": [{"action": "allow", "os": {"name": "linux"}}]
            }]
        }));
        let child = parse(serde_json::json!({
            "libraries": [{
                "name": "foo:bar:2",
                "rules": [{"action": "allow", "os": {"name": "windows"}}]
            }]
        }));

        parent.merge(&child);

        assert_eq!(library_names(&parent), ["foo:bar:2", "foo:bar:1"]);
    }

    #[test]
    fn rules_are_not_concatenated() {
        let mut parent = parse(serde_json::json!({
            "libraries": [{
                "name": "foo:bar:9",
                "rules": [{"action": "allow"}]
            }]
        }));
        let child = parse(serde_json::json!({
            "libraries": [{
                "name": "foo:bar:2",
                "rules": [{"action": "allow"}]
            }]
        }));

        parent.merge(&child);

        let rules = parent.libraries[0].rules.as_ref().unwrap();
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn classifiers_are_part_of_the_library_key() {
        let mut parent = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:1:natives-linux"}]
        }));
        let child = parse(serde_json::json!({
            "libraries": [{"name": "foo:bar:1"}]
        }));

        parent.merge(&child);

        assert_eq!(
            library_names(&parent),
            ["foo:bar:1", "foo:bar:1:natives-linux"]
        );
    }

    #[test]
    fn unknown_top_level_fields_merge_recursively() {
        let mut parent = parse(serde_json::json!({
            "unknownKept": {"a": 1, "b": 2}
        }));
        let child = parse(serde_json::json!({
            "unknownKept": {"b": 3, "c": 4}
        }));

        parent.merge(&child);

        let kept = &parent.extra["unknownKept"];
        assert_eq!(kept["a"], 1);
        assert_eq!(kept["b"], 3);
        assert_eq!(kept["c"], 4);
    }

    #[test]
    fn patches_do_not_merge_and_follow_the_child() {
        let mut parent = parse(serde_json::json!({
            "patches": [{"id": "snapshot"}]
        }));
        let child = parse(serde_json::json!({"id": "child"}));

        parent.merge(&child);

        assert!(parent.patches.is_empty());
    }
}
