//! 安装器档案（Forge / NeoForge 安装包里的 `install_profile.json`）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::json::JsonMap;
use crate::macros::string_enum;
use crate::source::{resolve_urls, templatize_urls, Mirror, UrlFields};
use crate::vanilla::version::Library;

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

impl UrlFields for InstallProfile {
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String)) {
        for library in &mut self.libraries {
            library.for_each_url_mut(f);
        }
    }
}

impl InstallProfile {
    /// 安装包内嵌的那个 jar 的路径（zip 内的条目名，不是文件系统路径）。
    pub fn file_path(&self) -> Option<&str> {
        self.install.as_ref().and_then(|i| i.file_path.as_deref())
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
    }

    #[test]
    fn processors_without_sides_run_on_the_client() {
        let processor: Processor = serde_json::from_str(r#"{"jar":"j"}"#).unwrap();
        assert!(processor.is_client_side());
        let processor: Processor =
            serde_json::from_str(r#"{"jar":"j","sides":["server"]}"#).unwrap();
        assert!(!processor.is_client_side());
    }
}
