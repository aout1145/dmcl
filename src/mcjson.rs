//! Typed views of every JSON document the launcher reads.
//!
//! The D implementation handled all of these through `std.json.JSONValue`,
//! which means every field access was a stringly-typed lookup that could (and
//! on missing keys did) throw at runtime. Here each document has a real Rust
//! type, so the compiler checks the accesses and a malformed remote document
//! produces a descriptive error instead of a panic deep inside the launcher.
//!
//! Two rules are followed throughout:
//!
//! * **Unknown keys are preserved.** Every struct keeps a
//!   `#[serde(flatten)] extra: Map<String, Value>` tail, so a version json that
//!   is written back after a Forge/Fabric install keeps whatever fields we do
//!   not model (a future Minecraft version cannot silently lose data).
//! * **Absent means absent.** Optional fields are `Option<T>` with
//!   `skip_serializing_if`, so a re-serialised document contains exactly the
//!   keys it contained before (no `null` padding).

use std::collections::BTreeMap;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::utils::get_os_name;

/// Convenience alias matching `serde_json`'s default map type.
pub type JsonMap = Map<String, Value>;

// ---------------------------------------------------------------------------
// rules
// ---------------------------------------------------------------------------

/// Declares a string-valued enum: a few known values plus an `Other(String)`
/// catch-all.
///
/// The catch-all is required because these values come from Minecraft's own
/// documents, which we parse *and write back* — silently dropping an action we
/// do not know about would corrupt the version json.
macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum $name {
            $(
                $variant,
            )+
            /// Any value this crate does not know about, kept verbatim.
            Other(String),
        }

        impl $name {
            /// The wire value of this variant.
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $text,)+
                    $name::Other(raw) => raw,
                }
            }
        }

        impl From<String> for $name {
            fn from(raw: String) -> Self {
                match raw.as_str() {
                    $($text => $name::$variant,)+
                    _ => $name::Other(raw),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let raw: String = serde::Deserialize::deserialize(deserializer)?;
                Ok(raw.into())
            }
        }
    };
}

string_enum! {
    /// The `action` of a rule — was a bare string compared against `"allow"`.
    RuleAction {
        Allow => "allow",
        Disallow => "disallow",
    }
}

/// `os` sub-object of a rule.
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

/// One element of a `rules` array.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Rule {
    /// `None` when the key is absent; an unknown action is kept as
    /// [`RuleAction::Other`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<RuleAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<JsonMap>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `checkRules` — decide whether a rule list applies to the current platform.
///
/// Faithful port of the original, including its two quirks: any rule carrying
/// `features` rejects the whole list, and an `allow` rule with an `os.version`
/// constraint also rejects it.
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
            // no action at all, or an action we do not know: never applies
            _ => {}
        }
    }
    true
}

/// `Option<Vec<Rule>>` helper mirroring `if ("rules" !in lib || checkRules(...))`.
pub fn rules_allow(rules: &Option<Vec<Rule>>) -> bool {
    match rules {
        None => true,
        Some(rules) => check_rules(rules),
    }
}

// ---------------------------------------------------------------------------
// downloads / libraries
// ---------------------------------------------------------------------------

/// A single downloadable file (`downloads.artifact`, `downloads.client`, ...).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Artifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    /// `name -> artifact`, keyed by e.g. `natives-linux`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<BTreeMap<String, Artifact>>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// One element of the `libraries` array.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibraryDownloads>,
    /// `os -> classifier name`, e.g. `{"linux": "natives-linux"}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<BTreeMap<String, String>>,
    /// Repository base url for "difficult" (non-downloads) libraries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Artifact>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

/// An argument `value`, which is either a single string or a list of strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrVec {
    Single(String),
    Many(Vec<String>),
}

impl StringOrVec {
    pub fn push_into(&self, out: &mut Vec<String>) {
        match self {
            StringOrVec::Single(s) => out.push(s.clone()),
            StringOrVec::Many(v) => out.extend(v.iter().cloned()),
        }
    }
}

/// A conditional argument object: `{ "rules": [...], "value": ... }`.
///
/// The `flatten` tail must stay last: serde consumes the remaining keys into it
/// in field declaration order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuledArgument {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<StringOrVec>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// One element of `arguments.jvm` / `arguments.game`.
///
/// `Other` is the catch-all: an object with an unexpected shape is preserved
/// verbatim instead of failing the whole document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Ruled(RuledArgument),
    Other(Value),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Arguments {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jvm: Option<Vec<Argument>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<Vec<Argument>>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl Arguments {
    pub fn get(&self, key: &str) -> Option<&Vec<Argument>> {
        match key {
            "jvm" => self.jvm.as_ref(),
            "game" => self.game.as_ref(),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// version json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetIndexRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, rename = "totalSize", skip_serializing_if = "Option::is_none")]
    pub total_size: Option<u64>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JavaVersion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Major java version required by this version — never negative.
    #[serde(
        default,
        rename = "majorVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub major_version: Option<u32>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `versions/<name>/<name>.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Which `versions/<jar>/<jar>.jar` provides the client classes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar: Option<String>,
    #[serde(
        default,
        rename = "inheritsFrom",
        skip_serializing_if = "Option::is_none"
    )]
    pub inherits_from: Option<String>,
    #[serde(default, rename = "mainClass", skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
    #[serde(
        default,
        rename = "minecraftArguments",
        skip_serializing_if = "Option::is_none"
    )]
    pub minecraft_arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(
        default,
        rename = "assetIndex",
        skip_serializing_if = "Option::is_none"
    )]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(
        default,
        rename = "javaVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub java_version: Option<JavaVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<VersionDownloads>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    /// Everything we do not model, preserved verbatim.
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl VersionJson {
    /// Merge `child` on top of `self` with the original `mergeJSON` semantics.
    pub fn merge(&mut self, child: &VersionJson) -> anyhow::Result<()> {
        merge_typed(self, child)
    }

    pub fn asset_index_id(&self) -> &str {
        self.asset_index
            .as_ref()
            .and_then(|a| a.id.as_deref())
            .unwrap_or("")
    }

    /// The major java version this version needs (defaults to 8, like the D
    /// `unwrap_or`-style fallback for versions without a `javaVersion` block).
    pub fn required_java_major(&self) -> u32 {
        self.java_version
            .as_ref()
            .and_then(|j| j.major_version)
            .unwrap_or(8)
    }

    /// `arguments.<key>`, `None` when the version uses the legacy
    /// `minecraftArguments` string.
    pub fn arguments_for(&self, key: &str) -> Option<&Vec<Argument>> {
        self.arguments.as_ref().and_then(|a| a.get(key))
    }

    /// `if ("arguments" in json.object)`.
    pub fn uses_arguments(&self) -> bool {
        self.arguments.is_some()
    }
}

// ---------------------------------------------------------------------------
// installer profile (forge / neoforge)
// ---------------------------------------------------------------------------

/// `data.<key>` — normally `{client, server}`, but the file also carries a
/// free-form `_comment_` string, hence the untagged fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DataEntry {
    Pair(DataPair),
    Other(Value),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DataPair {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl DataEntry {
    /// `profile["data"][key]["client"]`.
    pub fn client(&self) -> Option<&str> {
        match self {
            DataEntry::Pair(pair) => pair.client.as_deref(),
            DataEntry::Other(value) => value.get("client").and_then(Value::as_str),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallEntry {
    #[serde(default, rename = "filePath", skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

string_enum! {
    /// The side an installer processor runs on.
    Side {
        Client => "client",
        Server => "server",
    }
}

/// One entry of `processors`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Processor {
    #[serde(default)]
    pub jar: String,
    #[serde(default)]
    pub classpath: Vec<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sides: Option<Vec<Side>>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl Processor {
    /// `isClient()` — a processor without `sides` runs on the client too.
    pub fn is_client_side(&self) -> bool {
        match &self.sides {
            None => true,
            Some(sides) => sides.contains(&Side::Client),
        }
    }
}

/// `install_profile.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<InstallEntry>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub processors: Vec<Processor>,
    #[serde(default)]
    pub data: BTreeMap<String, DataEntry>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl InstallProfile {
    pub fn file_path(&self) -> Option<&str> {
        self.install.as_ref().and_then(|i| i.file_path.as_deref())
    }
}

// ---------------------------------------------------------------------------
// metadata endpoints
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifest {
    #[serde(default)]
    pub versions: Vec<VersionManifestEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifestEntry {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub url: String,
}

/// `assets/indexes/<id>.json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AssetIndexFile {
    #[serde(default)]
    pub objects: BTreeMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    pub virtual_: Option<bool>,
    #[serde(default, rename = "map_to_resources")]
    pub map_to_resources: Option<bool>,
}

impl AssetIndexFile {
    /// `json["virtual"].boolean == true`, i.e. present *and* true.
    pub fn is_virtual(&self) -> bool {
        self.virtual_.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AssetObject {
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub size: u64,
}

/// `bmclapi .../forge/minecraft/<mcver>` entry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ForgeVersionEntry {
    #[serde(default)]
    pub mcversion: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// Build number — always positive.
    #[serde(default)]
    pub build: u64,
}

/// Official neoforged maven version listing.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NeoForgeVersionListing {
    #[serde(default)]
    pub versions: Vec<String>,
}

/// `bmclapi .../neoforge/list/<mcver>` entry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NeoForgeListEntry {
    #[serde(default)]
    pub version: String,
}

/// `meta.fabricmc.net/v2/versions/loader/<mcver>` entry.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoaderEntry {
    #[serde(default)]
    pub loader: FabricLoader,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoader {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub stable: Option<bool>,
}

impl FabricLoader {
    pub fn is_stable(&self) -> bool {
        self.stable.unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// merging
// ---------------------------------------------------------------------------

/// `mergeJSON` — recursive merge: objects key by key, arrays concatenated,
/// anything else overwritten by `source`.
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

/// Typed [`merge_value`]: `base` is merged with `child` following the exact
/// `mergeJSON` semantics, by round-tripping both through `serde_json`.
///
/// Because serialisation skips absent optional fields, "the child does not
/// mention this key" behaves exactly like the D version (the base value is
/// kept), and because unknown keys live in the structs' `extra` maps they take
/// part in the merge as well.
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
        // an allow rule with no os name never applies
        assert!(!check_rules(&parse_rules(
            r#"[{"action":"allow","os":{}}]"#
        )));
        // ... and neither does one carrying an os version constraint
        assert!(!check_rules(&parse_rules(&format!(
            r#"[{{"action":"allow","os":{{"name":"{os}","version":"^10"}}}}]"#
        ))));
        assert!(!check_rules(&parse_rules(&format!(
            r#"[{{"action":"disallow","os":{{"name":"{os}"}}}}]"#
        ))));
        // any rule with `features` rejects the whole list
        assert!(!check_rules(&parse_rules(
            r#"[{"action":"allow","features":{"has_custom_resolution":true}}]"#
        )));
        // no rules at all => allowed
        assert!(check_rules(&[]));
        assert!(rules_allow(&None));
        assert!(rules_allow(&Some(vec![])));
    }

    #[test]
    fn version_merge_follows_merge_json() {
        let mut merged: VersionJson = serde_json::from_str(
            r#"{
                "id": "base",
                "mainClass": "a.Main",
                "libraries": [{"name": "a:a:1"}],
                "arguments": {"game": ["--one"], "jvm": ["-Xa"]},
                "downloads": {"client": {"url": "u", "sha1": "s"}},
                "unknownKept": {"deep": {"a": 1, "b": 2}}
            }"#,
        )
        .unwrap();
        let child: VersionJson = serde_json::from_str(
            r#"{
                "id": "child",
                "libraries": [{"name": "b:b:2"}],
                "arguments": {"game": ["--two"]},
                "unknownKept": {"deep": {"b": 3, "c": 4}}
            }"#,
        )
        .unwrap();

        merged.merge(&child).unwrap();

        // scalars: the child wins
        assert_eq!(merged.id.as_deref(), Some("child"));
        // keys the child does not mention survive
        assert_eq!(merged.main_class.as_deref(), Some("a.Main"));
        assert!(merged.downloads.is_some());
        // arrays are concatenated, base first
        assert_eq!(merged.libraries.len(), 2);
        assert_eq!(merged.libraries[0].name, "a:a:1");
        assert_eq!(merged.libraries[1].name, "b:b:2");
        assert_eq!(
            merged
                .arguments
                .as_ref()
                .unwrap()
                .game
                .as_ref()
                .unwrap()
                .len(),
            2
        );
        // unmodelled keys merge recursively
        let kept = &merged.extra["unknownKept"]["deep"];
        assert_eq!(kept["a"], 1);
        assert_eq!(kept["b"], 3);
        assert_eq!(kept["c"], 4);
    }

    #[test]
    fn unknown_version_fields_round_trip() {
        let parsed: VersionJson = serde_json::from_str(
            r#"{"id":"x","mainClass":"m","complianceLevel":1,
                "logging":{"client":{"file":{"id":"c"}}},"extra":{"nested":[1,2]}}"#,
        )
        .unwrap();
        let written = serde_json::to_value(&parsed).unwrap();
        assert_eq!(written["complianceLevel"], 1);
        assert_eq!(written["logging"]["client"]["file"]["id"], "c");
        assert_eq!(written["extra"]["nested"][1], 2);
        // absent optional fields are not padded with nulls
        assert!(written.get("inheritsFrom").is_none());
        assert!(written.get("downloads").is_none());
        assert!(written.get("arguments").is_none());
    }

    #[test]
    fn arguments_parse_strings_objects_and_unknown_shapes() {
        let args: Arguments = serde_json::from_str(
            r#"{
                "jvm": ["-Xa", {"rules":[{"action":"allow"}],"value":["-Xb","-Xc"]}],
                "game": ["--one", {"rules":[{"action":"disallow","os":{"name":"linux"}}],"value":"--two"}, 5]
            }"#,
        )
        .unwrap();

        let jvm = args.jvm.unwrap();
        assert!(matches!(jvm[0], Argument::Plain(_)));
        match &jvm[1] {
            Argument::Ruled(ruled) => match ruled.value.as_ref().unwrap() {
                StringOrVec::Many(values) => assert_eq!(values.len(), 2),
                StringOrVec::Single(_) => panic!("expected a list value"),
            },
            _ => panic!("expected a ruled argument"),
        }

        let game = args.game.unwrap();
        assert!(matches!(game[0], Argument::Plain(_)));
        // an object without `rules` is preserved but never applied
        match &game[1] {
            Argument::Ruled(ruled) => {
                assert_eq!(ruled.rules.as_ref().unwrap().len(), 1);
                assert_eq!(ruled.value, Some(StringOrVec::Single("--two".into())));
            }
            _ => panic!("expected a ruled argument"),
        }
        // anything else falls back to the raw value
        assert!(matches!(game[2], Argument::Other(_)));
    }

    #[test]
    fn install_profile_handles_the_comment_entry() {
        let profile: InstallProfile = serde_json::from_str(
            r#"{
                "install": {"filePath": "net/minecraftforge/forge.jar"},
                "libraries": [{"name":"x:x:1","downloads":{"artifact":{"path":"p","url":"u","sha1":"s"}}}],
                "processors": [{"jar":"j","classpath":["c"],"args":["a"],"outputs":{"{A}":"b"},"sides":["client"]}],
                "data": {
                    "_comment_": "please do not automate",
                    "BINPATCH": {"client": "/data/client.lzma", "server": "/data/server.lzma"}
                }
            }"#,
        )
        .unwrap();

        assert_eq!(profile.file_path(), Some("net/minecraftforge/forge.jar"));
        assert_eq!(profile.data["BINPATCH"].client(), Some("/data/client.lzma"));
        // the free-form `_comment_` string must not break deserialisation
        assert_eq!(profile.data["_comment_"].client(), None);
        assert!(profile.processors[0].is_client_side());
        assert_eq!(
            profile.libraries[0]
                .downloads
                .as_ref()
                .unwrap()
                .artifact
                .as_ref()
                .unwrap()
                .path
                .as_deref(),
            Some("p")
        );
    }

    #[test]
    fn asset_index_reports_virtual_only_when_true() {
        let plain: AssetIndexFile =
            serde_json::from_str(r#"{"objects":{"a":{"hash":"aa","size":1}}}"#).unwrap();
        assert!(!plain.is_virtual());
        let virtual_index: AssetIndexFile =
            serde_json::from_str(r#"{"virtual":true,"objects":{}}"#).unwrap();
        assert!(virtual_index.is_virtual());
        let explicit_false: AssetIndexFile =
            serde_json::from_str(r#"{"virtual":false,"objects":{}}"#).unwrap();
        assert!(!explicit_false.is_virtual());
    }

    #[test]
    fn string_enums_keep_unknown_values() {
        // `allow` / `disallow` are typed now, but anything else survives a
        // round-trip so writing the version json back cannot lose information.
        let rule: Rule = serde_json::from_str(r#"{"action":"allow"}"#).unwrap();
        assert_eq!(rule.action, Some(RuleAction::Allow));
        let rule: Rule = serde_json::from_str(r#"{"action":"maybe"}"#).unwrap();
        assert_eq!(rule.action, Some(RuleAction::Other("maybe".to_string())));
        let written = serde_json::to_value(&rule).unwrap();
        assert_eq!(written["action"], "maybe");
        // no action at all stays absent
        let rule: Rule = serde_json::from_str(r#"{"os":{"name":"linux"}}"#).unwrap();
        assert_eq!(rule.action, None);
        assert!(serde_json::to_value(&rule).unwrap().get("action").is_none());

        // `side` on installer processors
        let side: Side = serde_json::from_str("\"client\"").unwrap();
        assert_eq!(side, Side::Client);
        let side: Side = serde_json::from_str("\"proxy\"").unwrap();
        assert_eq!(side, Side::Other("proxy".to_string()));
        assert_eq!(side.as_str(), "proxy");
        assert_eq!(Side::Server.as_str(), "server");
    }
}
