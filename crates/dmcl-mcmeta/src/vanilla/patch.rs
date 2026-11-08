//! 版本 JSON 的 patch 与 patch 历史。
//!
//! patch 是一份只描述"要改什么"的文档：库、主类、tweaker。应用过的 patch 会追加到
//! [`VersionJson::patches`]；`patches[0]` 是应用前的原版快照。

use serde_json::Value;

use crate::json::JsonMap;
use crate::vanilla::merge::{combine_libraries, same_rules, version_is_higher};
use crate::vanilla::rules::Rule;
use crate::vanilla::version::{Argument, Library, VersionJson};

/// 一份版本 patch。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionJsonPatch {
    /// 必填。
    pub name: String,
    /// 必填。
    pub version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<Library>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tweakers: Vec<String>,
    #[serde(default, rename = "mainClass", skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
    /// 未知字段原样保留（例如 `priority`、`min_java_version`）。
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl VersionJson {
    /// 应用单个 patch。
    pub fn apply_patch(&mut self, patch: &VersionJsonPatch) -> anyhow::Result<()> {
        self.apply_patches(std::slice::from_ref(patch))
    }

    /// 按顺序应用多个 patch。
    pub fn apply_patches(&mut self, patches: &[VersionJsonPatch]) -> anyhow::Result<()> {
        if patches.is_empty() {
            return Ok(());
        }

        // 1. 先解析已有历史，并预先序列化新 patch，保证失败时不做任何修改。
        let mut forced: Vec<(String, Option<Vec<Rule>>)> = Vec::new();
        for value in self.patches.iter().skip(1) {
            let applied: VersionJsonPatch = serde_json::from_value(value.clone())
                .map_err(|err| anyhow::anyhow!("无法解析已应用的 patch 历史：{err}"))?;
            forced.extend(applied.libraries.iter().map(|library| {
                (
                    crate::maven::library_key(&library.name),
                    library.rules.clone(),
                )
            }));
        }

        let encoded_patches: Vec<Value> = patches
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()?;

        let original = if self.patches.is_empty() {
            Some(serde_json::to_value(&*self)?)
        } else {
            None
        };

        // 2. 以下步骤不再失败。
        if let Some(original) = original {
            self.patches.push(original);
        }

        for (patch, encoded) in patches.iter().zip(encoded_patches) {
            let current_libraries = std::mem::take(&mut self.libraries);
            self.libraries = apply_patch_libraries(&current_libraries, &patch.libraries, &forced);

            if let Some(main_class) = &patch.main_class {
                self.main_class = Some(main_class.clone());
            }

            for tweaker in &patch.tweakers {
                append_tweaker(self, tweaker);
            }

            // 3. 后面的 patch 命中这些库键 + rules 时要无条件覆盖
            forced.extend(patch.libraries.iter().map(|library| {
                (
                    crate::maven::library_key(&library.name),
                    library.rules.clone(),
                )
            }));

            self.patches.push(encoded);
        }

        Ok(())
    }
}

/// patch 的库合并规则：
/// 之前 patch 改过的同键 + 同 rules 库无条件用新 patch 覆盖；
/// 其它情况仍按版本号高者胜出。
fn apply_patch_libraries(
    current: &[Library],
    patch: &[Library],
    forced: &[(String, Option<Vec<Rule>>)],
) -> Vec<Library> {
    combine_libraries(current, patch, |current, patch| {
        !is_forced(patch, forced) && version_is_higher(&current.name, &patch.name)
    })
}

fn is_forced(library: &Library, forced: &[(String, Option<Vec<Rule>>)]) -> bool {
    let key = crate::maven::library_key(&library.name);
    forced.iter().any(|(forced_key, forced_rules)| {
        forced_key == &key && same_rules(forced_rules, &library.rules)
    })
}

const TWEAK_CLASS: &str = "--tweakClass";

fn append_tweaker(json: &mut VersionJson, tweaker: &str) {
    if let Some(arguments) = json.arguments.as_mut() {
        let game = arguments.game.get_or_insert_with(Vec::new);
        remove_plain_tweaker(game, tweaker);
        game.push(Argument::Plain(TWEAK_CLASS.to_string()));
        game.push(Argument::Plain(tweaker.to_string()));
        return;
    }

    let args = json.minecraft_arguments.get_or_insert_with(String::new);
    let mut tokens: Vec<String> = args.split_whitespace().map(str::to_owned).collect();
    let mut i = 0;
    while i + 1 < tokens.len() {
        if tokens[i] == TWEAK_CLASS && tokens[i + 1] == tweaker {
            tokens.drain(i..i + 2);
        } else {
            i += 1;
        }
    }
    tokens.push(TWEAK_CLASS.to_string());
    tokens.push(tweaker.to_string());
    *args = tokens.join(" ");
}

fn remove_plain_tweaker(game: &mut Vec<Argument>, tweaker: &str) {
    let mut i = 0;
    while i + 1 < game.len() {
        let is_flag = matches!(&game[i], Argument::Plain(value) if value == TWEAK_CLASS);
        let is_tweaker = matches!(&game[i + 1], Argument::Plain(value) if value == tweaker);
        if is_flag && is_tweaker {
            game.drain(i..i + 2);
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(value: serde_json::Value) -> VersionJson {
        serde_json::from_value(value).unwrap()
    }

    fn patch(value: serde_json::Value) -> VersionJsonPatch {
        serde_json::from_value(value).unwrap()
    }

    fn library_names(json: &VersionJson) -> Vec<&str> {
        json.libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect()
    }

    fn plain_arguments(arguments: &Option<Vec<Argument>>) -> Vec<String> {
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
    fn a_single_patch_updates_libraries_main_class_and_tweakers() {
        let mut json = version(serde_json::json!({
            "mainClass": "A",
            "libraries": [{"name": "foo:bar:1"}],
            "arguments": {"game": ["--x"]}
        }));
        let patch = patch(serde_json::json!({
            "name": "p",
            "version": "1",
            "libraries": [{"name": "foo:bar:2"}],
            "mainClass": "B",
            "tweakers": ["optifine.OptiFineTweaker"]
        }));

        json.apply_patch(&patch).unwrap();

        assert_eq!(library_names(&json), ["foo:bar:2"]);
        assert_eq!(json.main_class.as_deref(), Some("B"));
        let game = plain_arguments(&json.arguments.unwrap().game);
        assert_eq!(
            &game[game.len() - 2..],
            ["--tweakClass", "optifine.OptiFineTweaker"]
        );
        assert_eq!(json.patches.len(), 2);
        assert_eq!(json.patches[0]["mainClass"], "A");
        assert!(json.patches[0].get("patches").is_none());
        assert_eq!(json.patches[1], serde_json::to_value(&patch).unwrap());
    }

    #[test]
    fn legacy_minecraft_arguments_get_the_tweaker_appended() {
        let mut json = version(serde_json::json!({
            "minecraftArguments": "--username x"
        }));
        let patch = patch(serde_json::json!({
            "name": "p",
            "version": "1",
            "tweakers": ["optifine.OptiFineTweaker"]
        }));

        json.apply_patch(&patch).unwrap();

        assert_eq!(
            json.minecraft_arguments.as_deref(),
            Some("--username x --tweakClass optifine.OptiFineTweaker")
        );
    }

    #[test]
    fn an_existing_tweaker_is_removed_and_appended_at_the_end() {
        let mut json = version(serde_json::json!({
            "arguments": {"game": ["--tweakClass", "A", "--x"]}
        }));
        let patch = patch(serde_json::json!({
            "name": "p",
            "version": "1",
            "tweakers": ["A"]
        }));

        json.apply_patch(&patch).unwrap();

        let game = plain_arguments(&json.arguments.unwrap().game);
        assert_eq!(game, ["--x", "--tweakClass", "A"]);
    }

    #[test]
    fn later_patches_override_earlier_patches_unconditionally() {
        let mut json = version(serde_json::json!({
            "libraries": [{"name": "foo:bar:1"}],
            "arguments": {"game": []}
        }));
        let first = patch(serde_json::json!({
            "name": "one",
            "version": "1",
            "libraries": [{"name": "foo:bar:2"}],
            "mainClass": "B",
            "tweakers": ["A"]
        }));
        let second = patch(serde_json::json!({
            "name": "two",
            "version": "1",
            "libraries": [{"name": "foo:bar:1"}],
            "mainClass": "C",
            "tweakers": ["B"]
        }));

        json.apply_patches(&[first, second]).unwrap();

        assert_eq!(library_names(&json), ["foo:bar:1"]);
        assert_eq!(json.main_class.as_deref(), Some("C"));
        let game = plain_arguments(&json.arguments.unwrap().game);
        assert_eq!(
            &game[game.len() - 4..],
            ["--tweakClass", "A", "--tweakClass", "B"]
        );
        assert_eq!(json.patches.len(), 3);
    }

    #[test]
    fn a_patch_still_loses_to_a_higher_version_from_the_original() {
        let mut json = version(serde_json::json!({
            "libraries": [{"name": "foo:bar:9"}]
        }));
        let patch = patch(serde_json::json!({
            "name": "p",
            "version": "1",
            "libraries": [{"name": "foo:bar:1"}]
        }));

        json.apply_patch(&patch).unwrap();

        assert_eq!(library_names(&json), ["foo:bar:9"]);
    }

    #[test]
    fn a_later_apply_call_still_overrides_an_earlier_patch() {
        let mut json = version(serde_json::json!({
            "libraries": [{"name": "foo:bar:1"}]
        }));
        let first = patch(serde_json::json!({
            "name": "one",
            "version": "1",
            "libraries": [{"name": "foo:bar:2"}]
        }));
        let second = patch(serde_json::json!({
            "name": "two",
            "version": "1",
            "libraries": [{"name": "foo:bar:1"}]
        }));

        json.apply_patch(&first).unwrap();
        json.apply_patch(&second).unwrap();

        assert_eq!(library_names(&json), ["foo:bar:1"]);
        assert_eq!(json.patches.len(), 3);

        let history: Vec<&str> = json.patches[1..]
            .iter()
            .map(|value| value["name"].as_str().unwrap())
            .collect();
        assert_eq!(history, ["one", "two"]);
    }

    #[test]
    fn different_rules_keep_both_variants_across_patches() {
        let mut json = version(serde_json::json!({}));
        let first = patch(serde_json::json!({
            "name": "one",
            "version": "1",
            "libraries": [{
                "name": "foo:bar:1",
                "rules": [{"action": "allow", "os": {"name": "linux"}}]
            }]
        }));
        let second = patch(serde_json::json!({
            "name": "two",
            "version": "1",
            "libraries": [{
                "name": "foo:bar:1",
                "rules": [{"action": "allow", "os": {"name": "windows"}}]
            }]
        }));

        json.apply_patches(&[first, second]).unwrap();

        assert_eq!(json.libraries.len(), 2);
        assert_eq!(
            json.libraries[0].rules.as_ref().unwrap()[0]
                .os
                .as_ref()
                .unwrap()
                .name
                .as_deref(),
            Some("windows")
        );
        assert_eq!(
            json.libraries[1].rules.as_ref().unwrap()[0]
                .os
                .as_ref()
                .unwrap()
                .name
                .as_deref(),
            Some("linux")
        );
    }

    #[test]
    fn an_unparsable_history_fails_without_modifying_the_version() {
        let mut json = version(serde_json::json!({
            "mainClass": "A",
            "patches": [{"id": "original"}, {"libraries": []}]
        }));
        let before = serde_json::to_value(&json).unwrap();
        let patch = patch(serde_json::json!({
            "name": "p",
            "version": "1",
            "mainClass": "B"
        }));

        let err = json.apply_patch(&patch).unwrap_err();
        assert!(format!("{err:#}").contains("patch 历史"), "{err:#}");
        assert_eq!(serde_json::to_value(&json).unwrap(), before);
    }

    #[test]
    fn an_empty_patch_list_is_a_no_op() {
        let mut json = version(serde_json::json!({
            "mainClass": "A",
            "libraries": [{"name": "foo:bar:1"}]
        }));
        let before = serde_json::to_value(&json).unwrap();

        json.apply_patches(&[]).unwrap();

        assert_eq!(serde_json::to_value(&json).unwrap(), before);
        assert!(json.patches.is_empty());
    }

    #[test]
    fn patch_history_urls_are_templatized_and_resolved() {
        let mut json = version(serde_json::json!({
            "libraries": [{
                "name": "foo:bar:1",
                "downloads": {"artifact": {
                    "url": "https://libraries.minecraft.net/foo/bar/1/bar-1.jar"
                }}
            }]
        }));
        let patch = patch(serde_json::json!({
            "name": "p",
            "version": "1",
            "libraries": [{
                "name": "foo:baz:1",
                "downloads": {"artifact": {
                    "url": "https://libraries.minecraft.net/foo/baz/1/baz-1.jar"
                }}
            }]
        }));
        json.apply_patch(&patch).unwrap();

        json.templatize_urls();
        assert_eq!(
            json.patches[0]["libraries"][0]["downloads"]["artifact"]["url"],
            "${MC_MAVEN}/foo/bar/1/bar-1.jar"
        );
        assert_eq!(
            json.patches[1]["libraries"][0]["downloads"]["artifact"]["url"],
            "${MC_MAVEN}/foo/baz/1/baz-1.jar"
        );

        json.resolve_official_urls();
        assert_eq!(
            json.patches[0]["libraries"][0]["downloads"]["artifact"]["url"],
            "https://libraries.minecraft.net/foo/bar/1/bar-1.jar"
        );
        assert_eq!(
            json.patches[1]["libraries"][0]["downloads"]["artifact"]["url"],
            "https://libraries.minecraft.net/foo/baz/1/baz-1.jar"
        );
    }
}
