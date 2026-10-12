//! Fabric 的元数据、地址与取数（loader 版本列表、版本 JSON）。

use anyhow::Result;
use serde::Deserialize;

use crate::json::JsonMap;
use crate::source::{DownloadSource, Root};
use crate::vanilla::version::{Library, VersionJson};

/// `meta.fabricmc.net/v2/versions/loader/<mcver>` 的一项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoaderEntry {
    #[serde(default)]
    pub loader: FabricLoader,
    #[serde(default)]
    pub intermediary: FabricIntermediary,
    /// Fabric 自己的启动元信息（启动器可选使用）。
    #[serde(default, rename = "launcherMeta")]
    pub launcher_meta: FabricLauncherMeta,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLoader {
    /// 版本号里的分隔符（`.` 或 `+`）。
    #[serde(default)]
    pub separator: Option<String>,
    /// 构建号。
    #[serde(default)]
    pub build: Option<u64>,
    /// maven 坐标（`net.fabricmc:fabric-loader:<version>`）。
    #[serde(default)]
    pub maven: Option<String>,
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

/// 与 loader 配对的 intermediary 映射版本。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricIntermediary {
    #[serde(default)]
    pub maven: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub stable: Option<bool>,
}

/// `launcherMeta` —— Fabric 建议启动器使用的完整启动信息。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLauncherMeta {
    /// launcherMeta 结构自身的版本号。
    #[serde(default)]
    pub version: Option<i64>,
    /// 需要的最低 Java 大版本。
    #[serde(default)]
    pub min_java_version: Option<i64>,
    #[serde(default)]
    pub libraries: FabricLibraries,
    /// 老版本是单个字符串，新版本是 `{client, server}`。
    #[serde(default, rename = "mainClass")]
    pub main_class: Option<FabricMainClass>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricLibraries {
    #[serde(default)]
    pub client: Vec<Library>,
    #[serde(default)]
    pub common: Vec<Library>,
    #[serde(default)]
    pub server: Vec<Library>,
    #[serde(default)]
    pub development: Vec<Library>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// `launcherMeta.mainClass` —— 新版是 `{client, server}`，老版只有一个字符串。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum FabricMainClass {
    Plain(String),
    Sided(FabricSidedMainClass),
}

impl FabricMainClass {
    /// 客户端主类。
    pub fn client(&self) -> Option<&str> {
        match self {
            FabricMainClass::Plain(class) => Some(class),
            FabricMainClass::Sided(sided) => sided.client.as_deref(),
        }
    }

    /// 服务端主类；只有单个字符串时与客户端相同。
    pub fn server(&self) -> Option<&str> {
        match self {
            FabricMainClass::Plain(class) => Some(class),
            FabricMainClass::Sided(sided) => sided.server.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FabricSidedMainClass {
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// Fabric 的 loader 版本列表：`${FABRIC_META}/v2/versions/loader/<mc>`。
pub fn fabric_loader_list_url(mc_version: &str) -> String {
    format!(
        "{}/v2/versions/loader/{mc_version}",
        Root::FabricMeta.placeholder()
    )
}

/// Fabric 的版本 JSON：`${FABRIC_META}/v2/versions/loader/<mc>/<loader>/profile/json`。
pub fn fabric_profile_url(mc_version: &str, loader_version: &str) -> String {
    format!(
        "{}/v2/versions/loader/{mc_version}/{loader_version}/profile/json",
        Root::FabricMeta.placeholder()
    )
}

impl DownloadSource {
    /// 取 Fabric 的 loader 版本列表（从新到旧的顺序由调用者决定）。
    pub async fn fetch_fabric_loaders(&self, mc_version: &str) -> Result<Vec<FabricLoaderEntry>> {
        self.get_json(&fabric_loader_list_url(mc_version)).await
    }

    /// 取 Fabric 的版本 JSON（需要合并进原版版本 JSON）。
    pub async fn fetch_fabric_profile(
        &self,
        mc_version: &str,
        loader_version: &str,
    ) -> Result<VersionJson> {
        self.fetch_version_json(&fabric_profile_url(mc_version, loader_version))
            .await
    }
}

/// 最新的**稳定** Fabric loader；没有稳定版时取列表第一个。
pub async fn latest_fabric_loader(source: &DownloadSource, mc_version: &str) -> Result<String> {
    let list = source.fetch_fabric_loaders(mc_version).await?;
    for entry in list.iter().rev() {
        if entry.loader.is_stable() {
            return Ok(entry.loader.version.clone());
        }
    }
    Ok(list
        .first()
        .map(|entry| entry.loader.version.clone())
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_urls_are_built_as_templates() {
        assert_eq!(
            fabric_profile_url("1.20.1", "0.15.0"),
            "${FABRIC_META}/v2/versions/loader/1.20.1/0.15.0/profile/json"
        );
    }

    #[test]
    fn fabric_loader_stability_defaults_to_false() {
        let list: Vec<FabricLoaderEntry> = serde_json::from_str(
            r#"[{"loader":{"version":"0.15.0","stable":true}},{"loader":{"version":"0.16.0"}}]"#,
        )
        .unwrap();
        assert!(list[0].loader.is_stable());
        assert!(!list[1].loader.is_stable());
    }

    #[test]
    fn fabric_loader_list_entry_is_fully_modeled() {
        let list: Vec<FabricLoaderEntry> = serde_json::from_str(
            r#"[{
                "loader": {"separator":".", "build":5, "maven":"net.fabricmc:fabric-loader:0.19.5",
                           "version":"0.19.5", "stable":true},
                "intermediary": {"maven":"net.fabricmc:intermediary:1.21.1",
                                 "version":"1.21.1", "stable":true},
                "launcherMeta": {
                    "version": 2,
                    "min_java_version": 8,
                    "libraries": {
                        "client": [],
                        "common": [{"name":"org.ow2.asm:asm:9.10.1",
                                    "url":"https://maven.fabricmc.net/","sha1":"x","size":1}],
                        "server": [],
                        "development": []
                    },
                    "mainClass": {"client":"C","server":"S"}
                }
            }]"#,
        )
        .unwrap();
        let entry = &list[0];
        assert_eq!(entry.loader.build, Some(5));
        assert_eq!(
            entry.loader.maven.as_deref(),
            Some("net.fabricmc:fabric-loader:0.19.5")
        );
        assert_eq!(entry.intermediary.version.as_deref(), Some("1.21.1"));
        assert_eq!(entry.launcher_meta.min_java_version, Some(8));
        assert_eq!(entry.launcher_meta.libraries.common.len(), 1);
        assert_eq!(
            entry.launcher_meta.libraries.common[0].sha1.as_deref(),
            Some("x")
        );
        assert_eq!(
            entry
                .launcher_meta
                .main_class
                .as_ref()
                .and_then(FabricMainClass::client),
            Some("C")
        );
    }
}
