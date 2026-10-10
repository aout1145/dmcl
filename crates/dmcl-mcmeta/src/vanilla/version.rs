//! 版本 JSON（`versions/<name>/<name>.json`）及其组成部分。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json::JsonMap;
use crate::source::{resolve_urls, templatize_urls, Mirror, UrlFields};
use crate::vanilla::rules::Rule;

// ---------------------------------------------------------------------------
// downloads / libraries
// ---------------------------------------------------------------------------

/// 一个可下载文件（`downloads.artifact`、`downloads.client` …）。
///
/// `url` 里存的是**URL 模板**（见 [`crate::source`]）：从网络拿到的文档会被自动归一，
/// 未识别的第三方地址保持原样。
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

impl UrlFields for Artifact {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(url) = &mut self.url {
            f(url);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<Artifact>,
    /// `name -> artifact`，键形如 `natives-linux`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifiers: Option<std::collections::BTreeMap<String, Artifact>>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for LibraryDownloads {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(artifact) = &mut self.artifact {
            artifact.for_each_url_mut(f);
        }
        if let Some(classifiers) = &mut self.classifiers {
            for artifact in classifiers.values_mut() {
                artifact.for_each_url_mut(f);
            }
        }
    }
}

/// `libraries[].extract` —— 从 natives jar 里解压时排除的路径前缀。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `libraries` 数组的一个元素。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<LibraryDownloads>,
    /// `os -> classifier 名`，例如 `{"linux": "natives-linux"}`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub natives: Option<std::collections::BTreeMap<String, String>>,
    /// natives 解压时的排除规则。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extract>,
    /// "困难库"（没有 `downloads` 块）的仓库 base url。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    // ---- Fabric / 第三方加载器直接写在库上的校验信息 ----
    // 没有 `downloads` 块时，`url` + `name` 拼出地址，校验值就在这里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for Library {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(downloads) = &mut self.downloads {
            downloads.for_each_url_mut(f);
        }
        // `url` 是"仓库根"，同样参与镜像（否则困难库无法走镜像源）
        if let Some(url) = &mut self.url {
            f(url);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionDownloads {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Artifact>,
    /// 1.14+ 客户端混淆映射（`client_mappings`）的下载信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_mappings: Option<Artifact>,
    /// 1.14+ 服务端混淆映射（`server_mappings`）的下载信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_mappings: Option<Artifact>,
    /// 只有远古版本（≤1.5.2）才有的 Windows 专用服务端。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_server: Option<Artifact>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl VersionDownloads {
    /// 所有带下载信息的条目，顺序固定。
    pub fn artifacts(&self) -> impl Iterator<Item = &Artifact> {
        [
            &self.client,
            &self.server,
            &self.client_mappings,
            &self.server_mappings,
            &self.windows_server,
        ]
        .into_iter()
        .flatten()
    }

    /// 所有带下载信息的条目（可变），顺序固定。
    pub fn artifacts_mut(&mut self) -> impl Iterator<Item = &mut Artifact> {
        [
            &mut self.client,
            &mut self.server,
            &mut self.client_mappings,
            &mut self.server_mappings,
            &mut self.windows_server,
        ]
        .into_iter()
        .flatten()
    }
}

impl UrlFields for VersionDownloads {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        for artifact in self.artifacts_mut() {
            artifact.for_each_url_mut(f);
        }
    }
}

// ---------------------------------------------------------------------------
// arguments
// ---------------------------------------------------------------------------

/// 参数 `value`：既可以是单个字符串，也可以是字符串数组。
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

/// 带条件的参数对象：`{ "rules": [...], "value": ... }`。
///
/// `flatten` 尾巴必须是最后一个字段：serde 按声明顺序把剩余键灌进它。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuledArgument {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<StringOrVec>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `arguments.jvm` / `arguments.game` 的一个元素。
///
/// `Other` 是兜底：形状意外的对象原样保留，而不是让整份文档解析失败。
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
    /// 26.x 起新增：用户没有自定义 JVM 参数时使用的默认 JVM 参数。
    #[serde(
        default,
        rename = "default-user-jvm",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_user_jvm: Option<Vec<Argument>>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl Arguments {
    pub fn get(&self, key: &str) -> Option<&Vec<Argument>> {
        match key {
            "jvm" => self.jvm.as_ref(),
            "game" => self.game.as_ref(),
            "default-user-jvm" => self.default_user_jvm.as_ref(),
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

impl UrlFields for AssetIndexRef {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(url) = &mut self.url {
            f(url);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JavaVersion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// 这个版本需要的 java 大版本号 —— 不可能为负。
    #[serde(
        default,
        rename = "majorVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub major_version: Option<u32>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `logging` 块（1.7+ 的 log4j 配置）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Logging {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<LoggingClient>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingClient {
    /// 形如 `-Dlog4j.configurationFile=${path}`；`${path}` 由启动器替换成本地路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<LoggingFile>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl LoggingClient {
    /// 把 `argument` 里的 `${path}` 替换成本地日志配置文件路径。
    pub fn argument_with_path(&self, path: &str) -> Option<String> {
        self.argument
            .as_ref()
            .map(|argument| argument.replace("${path}", path))
    }
}

/// `logging.client.file` —— 与 [`Artifact`] 类似，但路径键名是 `id`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for LoggingFile {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(url) = &mut self.url {
            f(url);
        }
    }
}

/// `versions/<name>/<name>.json`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 由哪个 `versions/<jar>/<jar>.jar` 提供客户端类。
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
    // ---- 元信息（写回游戏不需要，但文档里存在） ----
    #[serde(
        default,
        rename = "complianceLevel",
        skip_serializing_if = "Option::is_none"
    )]
    pub compliance_level: Option<i64>,
    #[serde(
        default,
        rename = "minimumLauncherVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub minimum_launcher_version: Option<i64>,
    #[serde(
        default,
        rename = "releaseTime",
        skip_serializing_if = "Option::is_none"
    )]
    pub release_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// `type` —— `release` / `snapshot` / `unobfuscated` / `old_beta` / `old_alpha`。
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 1.7+ 的 log4j 配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logging: Option<Logging>,
    /// 所有未建模的字段，原样保留。
    ///
    /// 真实文档里通常只剩 Forge 写在 `version.json` 顶部的 `_comment_` / `_comment`
    /// 注释字段。详见 [`crate::vanilla`] 的说明。
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for VersionJson {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        if let Some(downloads) = &mut self.downloads {
            downloads.for_each_url_mut(f);
        }
        if let Some(asset_index) = &mut self.asset_index {
            asset_index.for_each_url_mut(f);
        }
        if let Some(logging) = &mut self.logging {
            if let Some(client) = &mut logging.client {
                if let Some(file) = &mut client.file {
                    file.for_each_url_mut(f);
                }
            }
        }
        for library in &mut self.libraries {
            library.for_each_url_mut(f);
        }
    }
}

impl VersionJson {
    /// 按原 `mergeJSON` 的语义把 `child` 合并到 `self` 上。
    pub fn merge(&mut self, child: &VersionJson) -> anyhow::Result<()> {
        crate::merge::merge_typed(self, child)
    }

    /// 把所有 url 归一成模板，返回改写的字段个数。
    pub fn templatize_urls(&mut self) -> usize {
        templatize_urls(self)
    }

    /// 把 url 模板解析成 `mirror` 下的真实链接，返回改写的字段个数。
    pub fn resolve_urls(&mut self, mirror: Mirror) -> usize {
        resolve_urls(self, mirror)
    }

    /// 模板化的副本（从磁盘读出的文档要先过这一步才能被镜像）。
    pub fn templatized(mut self) -> Self {
        self.templatize_urls();
        self
    }

    /// 解析成具体镜像链接的副本（写回磁盘时用官方源，保证文件可移植）。
    pub fn resolved(mut self, mirror: Mirror) -> Self {
        self.resolve_urls(mirror);
        self
    }

    pub fn asset_index_id(&self) -> &str {
        self.asset_index
            .as_ref()
            .and_then(|a| a.id.as_deref())
            .unwrap_or("")
    }

    /// 这个版本需要的 java 大版本（没有 `javaVersion` 块时退回 8）。
    pub fn required_java_major(&self) -> u32 {
        self.java_version
            .as_ref()
            .and_then(|j| j.major_version)
            .unwrap_or(8)
    }

    /// `arguments.<key>`；老版本用 `minecraftArguments` 字符串时为 `None`。
    pub fn arguments_for(&self, key: &str) -> Option<&Vec<Argument>> {
        self.arguments.as_ref().and_then(|a| a.get(key))
    }

    /// `if ("arguments" in json.object)`。
    pub fn uses_arguments(&self) -> bool {
        self.arguments.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        // 标量：子文档获胜
        assert_eq!(merged.id.as_deref(), Some("child"));
        // 子文档没提到的键保留
        assert_eq!(merged.main_class.as_deref(), Some("a.Main"));
        assert!(merged.downloads.is_some());
        // 数组拼接，父在前
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
        // 未建模的键递归合并
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
        // 不存在的可选字段不会补出 null
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
                StringOrVec::Single(_) => panic!("期望一个数组 value"),
            },
            _ => panic!("期望一个带条件的参数"),
        }

        let game = args.game.unwrap();
        assert!(matches!(game[0], Argument::Plain(_)));
        // 没有 rules 的对象会被保留但永不生效
        match &game[1] {
            Argument::Ruled(ruled) => {
                assert_eq!(ruled.rules.as_ref().unwrap().len(), 1);
                assert_eq!(ruled.value, Some(StringOrVec::Single("--two".into())));
            }
            _ => panic!("期望一个带条件的参数"),
        }
        // 其它形状落进兜底
        assert!(matches!(game[2], Argument::Other(_)));
    }

    #[test]
    fn modern_and_legacy_document_fields_are_modeled() {
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "x",
                "type": "release",
                "releaseTime": "2026-09-15T11:23:02+00:00",
                "time": "2026-09-15T11:23:02+00:00",
                "complianceLevel": 1,
                "minimumLauncherVersion": 21,
                "logging": {"client": {
                    "argument": "-Dlog4j.configurationFile=${path}",
                    "file": {"id": "client-1.12.xml", "sha1": "s", "size": 888,
                             "url": "https://piston-data.mojang.com/x"},
                    "type": "log4j2-xml"
                }},
                "downloads": {
                    "client": {"url": "https://piston-data.mojang.com/c"},
                    "server": {"url": "https://piston-data.mojang.com/s"},
                    "client_mappings": {"url": "https://piston-data.mojang.com/cm"},
                    "windows_server": {"url": "https://piston-data.mojang.com/w"}
                },
                "arguments": {
                    "game": ["--one"],
                    "default-user-jvm": [
                        {"value": ["-Xmx2G"]},
                        {"rules": [{"action": "allow", "os": {"name": "windows",
                            "versionRange": {"min": "10.0.17134"}}}], "value": ["-XX:+UseZGC"]}
                    ]
                },
                "libraries": [{
                    "name": "a:a:1",
                    "extract": {"exclude": ["META-INF/"]},
                    "url": "https://maven.fabricmc.net/",
                    "sha1": "s1", "sha256": "s256", "sha512": "s512", "md5": "m", "size": 9
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(json.kind.as_deref(), Some("release"));
        assert_eq!(
            json.release_time.as_deref(),
            Some("2026-09-15T11:23:02+00:00")
        );
        assert_eq!(json.time.as_deref(), Some("2026-09-15T11:23:02+00:00"));
        assert_eq!(json.compliance_level, Some(1));
        assert_eq!(json.minimum_launcher_version, Some(21));

        let downloads = json.downloads.as_ref().unwrap();
        assert!(downloads.client_mappings.is_some());
        assert!(downloads.windows_server.is_some());
        assert!(downloads.server_mappings.is_none());
        assert_eq!(downloads.artifacts().count(), 4);

        let client = json.logging.as_ref().unwrap().client.as_ref().unwrap();
        assert_eq!(client.kind.as_deref(), Some("log4j2-xml"));
        assert_eq!(
            client.file.as_ref().unwrap().id.as_deref(),
            Some("client-1.12.xml")
        );
        assert_eq!(
            client.argument_with_path("/tmp/x.xml").as_deref(),
            Some("-Dlog4j.configurationFile=/tmp/x.xml")
        );

        // 26.x 的 default-user-jvm 能按 key 取到
        assert_eq!(json.arguments_for("default-user-jvm").unwrap().len(), 2);

        let lib = &json.libraries[0];
        assert_eq!(lib.extract.as_ref().unwrap().exclude, vec!["META-INF/"]);
        assert_eq!(lib.sha256.as_deref(), Some("s256"));
        assert_eq!(lib.sha512.as_deref(), Some("s512"));
        assert_eq!(lib.md5.as_deref(), Some("m"));
        assert_eq!(lib.size, Some(9));

        // 把这些新字段写回去也不能丢
        let written = serde_json::to_value(&json).unwrap();
        assert_eq!(written["type"], "release");
        assert_eq!(written["complianceLevel"], 1);
        assert_eq!(
            written["downloads"]["client_mappings"]["url"],
            "https://piston-data.mojang.com/cm"
        );
        assert_eq!(
            written["libraries"][0]["extract"]["exclude"][0],
            "META-INF/"
        );
        assert_eq!(written["libraries"][0]["sha256"], "s256");
        assert_eq!(
            written["arguments"]["default-user-jvm"][1]["value"][0],
            "-XX:+UseZGC"
        );
    }
}
