//! 安装器档案（Forge 安装包里的 `install_profile.json`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json::JsonMap;
use crate::macros::string_enum;
use crate::source::{resolve_urls, templatize_urls, Mirror, UrlFields};
use crate::vanilla::version::{Library, VersionJson};

string_enum! {
    /// 安装器 processor 运行在哪一侧。
    Side {
        Client => "client",
        Server => "server",
    }
}

/// `data.<key>` —— 通常是 `{client, server}`，但文件里还带一个自由格式的
/// `_comment_` 字符串，所以需要 untagged 兜底。
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
    /// `profile["data"][key]["client"]`。
    pub fn client(&self) -> Option<&str> {
        match self {
            DataEntry::Pair(pair) => pair.client.as_deref(),
            DataEntry::Other(value) => value.get("client").and_then(Value::as_str),
        }
    }

    /// `profile["data"][key]["server"]`。
    pub fn server(&self) -> Option<&str> {
        match self {
            DataEntry::Pair(pair) => pair.server.as_deref(),
            DataEntry::Other(value) => value.get("server").and_then(Value::as_str),
        }
    }
}

/// `install` 块 —— 只有 1.6.4–1.12.2 的"经典"安装器才有。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallEntry {
    #[serde(default, rename = "filePath", skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 写进 `launcher_profiles.json` 的档案名（如 `forge`）。
    #[serde(
        default,
        rename = "profileName",
        skip_serializing_if = "Option::is_none"
    )]
    pub profile_name: Option<String>,
    /// 档案指向的"目标"版本 id（如 `1.10.2-forge1.10.2-12.18.3.2511`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// 档案里显示的版本名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<String>,
    #[serde(
        default,
        rename = "mirrorList",
        skip_serializing_if = "Option::is_none"
    )]
    pub mirror_list: Option<String>,
    /// `modList` —— 可能是字符串（`"none"`）或结构化的列表，形状随版本而异。
    #[serde(default, rename = "modList", skip_serializing_if = "Option::is_none")]
    pub mod_list: Option<Value>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `processors` 的一项。
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
    /// `isClient()` —— 没有 `sides` 的 processor 在客户端也要跑。
    pub fn is_client_side(&self) -> bool {
        match &self.sides {
            None => true,
            Some(sides) => sides.contains(&Side::Client),
        }
    }
}

/// `install_profile.json`。
///
/// 只有 `libraries` / `processors` / `data` 是三代安装器共有的；`spec` 以上的字段都是
/// "新式"（spec 0/1）安装器写的元信息。未被本结构体建模的键（目前只剩 `_comment_`
/// 这类注释）会留在 [`InstallProfile::extra`] 里。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallProfile {
    // ---- 新式安装器的元信息 ----
    /// `spec` —— 安装器规范版本（0 / 1）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<i64>,
    /// `profile` —— 写进 `launcher_profiles.json` 的档案名（如 `forge`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// 完整版本号（如 `1.20.1-forge-47.2.0`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// maven 坐标（如 `net.minecraftforge:forge:1.20.1-47.2.0`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// 目标原版版本号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft: Option<String>,
    /// 包内版本 JSON 的路径（如 `/version.json`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<String>,
    /// 服务端 jar 的路径模板（`{LIBRARY_DIR}` 等占位符）。
    #[serde(
        default,
        rename = "serverJarPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub server_jar_path: Option<String>,
    /// 是否隐藏"解压 natives"提示。
    #[serde(
        default,
        rename = "hideExtract",
        skip_serializing_if = "Option::is_none"
    )]
    pub hide_extract: Option<bool>,
    /// 镜像列表地址。
    #[serde(
        default,
        rename = "mirrorList",
        skip_serializing_if = "Option::is_none"
    )]
    pub mirror_list: Option<String>,
    /// 安装器窗口用的图标（data URI）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub welcome: Option<String>,
    // ---- 经典安装器 ----
    /// 经典安装器里的"可选步骤"（形状随版本而异，因此原样保留）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optionals: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<InstallEntry>,
    /// 经典安装器内嵌的整份版本 JSON。
    #[serde(
        default,
        rename = "versionInfo",
        skip_serializing_if = "Option::is_none"
    )]
    pub version_info: Option<Box<VersionJson>>,
    // ---- 三代共有 ----
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub processors: Vec<Processor>,
    #[serde(default)]
    pub data: BTreeMap<String, DataEntry>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

impl UrlFields for InstallProfile {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        for library in &mut self.libraries {
            library.for_each_url_mut(f);
        }
        if let Some(version_info) = &mut self.version_info {
            version_info.for_each_url_mut(f);
        }
    }
}

impl InstallProfile {
    /// 安装包内嵌的那个 jar 的路径（zip 内的条目名，不是文件系统路径）。
    pub fn file_path(&self) -> Option<&str> {
        self.install.as_ref().and_then(|i| i.file_path.as_deref())
    }

    /// 包内版本 JSON 的条目名（新式安装器的 `json`，去掉前导 `/`）；缺省 `version.json`。
    pub fn version_json_entry(&self) -> &str {
        self.json
            .as_deref()
            .map(|json| json.trim_start_matches('/'))
            .unwrap_or("version.json")
    }

    /// 把所有 url 归一成模板。
    pub fn templatize_urls(&mut self) -> usize {
        templatize_urls(self)
    }

    /// 把 url 模板解析成 `mirror` 下的真实链接。
    pub fn resolve_urls(&mut self, mirror: Mirror) -> usize {
        resolve_urls(self, mirror)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(profile.data["BINPATCH"].server(), Some("/data/server.lzma"));
        // 自由格式的 `_comment_` 字符串不能让解析失败
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
        // 经典安装器的 `version.json` 条目名默认值
        assert_eq!(profile.version_json_entry(), "version.json");
    }

    #[test]
    fn processors_without_sides_run_on_the_client() {
        let processor: Processor = serde_json::from_str(r#"{"jar":"j"}"#).unwrap();
        assert!(processor.is_client_side());
        let processor: Processor =
            serde_json::from_str(r#"{"jar":"j","sides":["server"]}"#).unwrap();
        assert!(!processor.is_client_side());
    }

    #[test]
    fn classic_installer_fields_are_modeled() {
        let profile: InstallProfile = serde_json::from_str(
            r#"{
                "install": {
                    "profileName": "forge",
                    "target": "1.10.2-forge1.10.2-12.18.3.2511",
                    "path": "net.minecraftforge:forge:1.10.2-12.18.3.2511",
                    "version": "forge 1.10.2-12.18.3.2511",
                    "filePath": "forge-1.10.2-12.18.3.2511-universal.jar",
                    "welcome": "Welcome to the simple forge installer.",
                    "minecraft": "1.10.2",
                    "mirrorList": "https://files.minecraftforge.net/mirror-brand.list",
                    "logo": "/big_logo.png",
                    "modList": "none"
                },
                "optionals": [],
                "versionInfo": {
                    "id": "1.10.2-forge",
                    "inheritsFrom": "1.10.2",
                    "jar": "1.10.2",
                    "mainClass": "m",
                    "minecraftArguments": "--x",
                    "libraries": [{"name": "a:a:1", "url": "https://libraries.minecraft.net/"}]
                }
            }"#,
        )
        .unwrap();

        let install = profile.install.as_ref().unwrap();
        assert_eq!(install.profile_name.as_deref(), Some("forge"));
        assert_eq!(
            install.target.as_deref(),
            Some("1.10.2-forge1.10.2-12.18.3.2511")
        );
        assert_eq!(
            install.file_path.as_deref(),
            Some("forge-1.10.2-12.18.3.2511-universal.jar")
        );
        assert_eq!(install.mod_list.as_ref().unwrap(), "none");
        assert!(install.extra.is_empty());
        assert_eq!(profile.file_path(), install.file_path.as_deref());
        assert!(profile.optionals.as_ref().unwrap().is_empty());

        let version_info = profile.version_info.as_ref().unwrap();
        assert_eq!(version_info.inherits_from.as_deref(), Some("1.10.2"));
        assert_eq!(version_info.libraries[0].name, "a:a:1");
    }

    #[test]
    fn modern_installer_fields_round_trip() {
        let profile: InstallProfile = serde_json::from_str(
            r#"{
                "_comment_": ["please do not automate"],
                "spec": 1,
                "profile": "forge",
                "version": "1.21.1-forge-52.1.16",
                "path": "net.minecraftforge:forge:1.21.1-52.1.16:shim",
                "minecraft": "1.21.1",
                "json": "/version.json",
                "serverJarPath": "{LIBRARY_DIR}/server.jar",
                "hideExtract": true,
                "mirrorList": "https://files.minecraftforge.net/mirrors-2.0.json",
                "icon": "data:image/png;base64,AAAA",
                "logo": "/big_logo.png",
                "welcome": "Welcome to the Forge installer.",
                "libraries": [],
                "processors": [],
                "data": {}
            }"#,
        )
        .unwrap();

        assert_eq!(profile.spec, Some(1));
        assert_eq!(profile.profile.as_deref(), Some("forge"));
        assert_eq!(profile.version.as_deref(), Some("1.21.1-forge-52.1.16"));
        assert_eq!(profile.minecraft.as_deref(), Some("1.21.1"));
        assert_eq!(profile.hide_extract, Some(true));
        assert_eq!(profile.version_json_entry(), "version.json");
        // 注释键留在 `extra` 里，不丢失
        assert!(profile.extra.contains_key("_comment_"));
        let written = serde_json::to_value(&profile).unwrap();
        assert_eq!(written["spec"], 1);
        assert_eq!(written["serverJarPath"], "{LIBRARY_DIR}/server.jar");
        assert!(written.get("install").is_none());
    }
}
