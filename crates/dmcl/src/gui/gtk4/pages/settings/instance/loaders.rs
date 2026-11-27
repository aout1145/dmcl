//! 读取实例的加载器 patch，供实例设置页展示与修改。
//!
//! 只认 Dacite 写入的 `dmcl-patches`：`[0]` 是原版快照，其后每条带 `name` / `version` 的
//! patch 对应一个加载器。没有 patch 历史的实例（其它启动器安装的）不支持调整加载器。

use std::path::Path;

use dmcl_install::LoaderKind;
use dmcl_mcmeta::layout::version_json_path;
use serde_json::Value;

/// 当前已安装的一个加载器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentLoader {
    pub kind: LoaderKind,
    /// 安装请求里使用的版本（Forge 是 `full_name`，OptiFine 不带 Minecraft 前缀）。
    pub id: String,
    /// 列表里显示的版本文案。
    pub label: String,
}

/// 实例能否调整加载器。
#[derive(Debug, Clone)]
pub(crate) enum LoaderSupport {
    Supported {
        mc_version: String,
        loaders: Vec<CurrentLoader>,
    },
    /// 不能调整的原因。
    Unsupported(String),
}

/// 读取实例的加载器信息；读不到或不是 Dacite 安装的实例时返回原因。
pub(crate) fn read_loader_support(root: &Path, name: &str) -> LoaderSupport {
    match read_support(root, name) {
        Ok(support) => support,
        Err(err) => LoaderSupport::Unsupported(format!("{err:#}")),
    }
}

fn read_support(root: &Path, name: &str) -> anyhow::Result<LoaderSupport> {
    let path = version_json_path(root, name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(LoaderSupport::Unsupported("无法读取版本 JSON".to_string()));
    };
    let Ok(document) = serde_json::from_str::<Value>(&text) else {
        return Ok(LoaderSupport::Unsupported("无法解析版本 JSON".to_string()));
    };
    let Some(patches) = document.get("dmcl-patches").and_then(Value::as_array) else {
        return Ok(LoaderSupport::Unsupported(
            "此实例不是由 Dacite 创建的，无法调整加载器".to_string(),
        ));
    };
    if patches.is_empty() {
        return Ok(LoaderSupport::Unsupported(
            "此实例不是由 Dacite 创建的，无法调整加载器".to_string(),
        ));
    }
    let Some(mc_version) = patches[0]
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(LoaderSupport::Unsupported(
            "无法从 dmcl-patches 判断 Minecraft 版本".to_string(),
        ));
    };

    let mut loaders: Vec<CurrentLoader> = Vec::new();
    for patch in &patches[1..] {
        let Some(patch_name) = patch.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(kind) = LoaderKind::from_patch_name(patch_name) else {
            continue;
        };
        if loaders.iter().any(|loader| loader.kind == kind) {
            return Ok(LoaderSupport::Unsupported(format!(
                "实例里有多个 {} patch，无法判断当前版本",
                kind.display_name()
            )));
        }
        let version = patch
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (id, label) = ui_version(kind, mc_version, version);
        loaders.push(CurrentLoader { kind, id, label });
    }

    Ok(LoaderSupport::Supported {
        mc_version: mc_version.to_string(),
        loaders,
    })
}

/// patch 里的版本 → 版本选择器用的 id / 文案。
fn ui_version(kind: LoaderKind, mc_version: &str, version: &str) -> (String, String) {
    match kind {
        // OptiFine 的 patch 版本是 `<mc>_<loader>`；安装请求只带 `<loader>`。
        LoaderKind::OptiFine => {
            let id = version
                .strip_prefix(&format!("{mc_version}_"))
                .unwrap_or(version)
                .to_string();
            (id.clone(), id)
        }
        // Forge 的 patch / 安装 id 都是 `full_name`；列表文案去掉 Minecraft 前缀。
        LoaderKind::Forge | LoaderKind::NeoForge => {
            let id = version.to_string();
            let label = version
                .strip_prefix(&format!("{mc_version}-"))
                .unwrap_or(version)
                .to_string();
            (id, label)
        }
        LoaderKind::Fabric => (version.to_string(), version.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::layout::version_dir;
    use serde_json::json;

    fn root(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dmcl-instance-loaders-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(version_dir(&root, "inst")).unwrap();
        root
    }

    fn write(root: &Path, document: Value) {
        std::fs::write(
            version_json_path(root, "inst"),
            serde_json::to_string(&document).unwrap(),
        )
        .unwrap();
    }

    fn loaders(support: LoaderSupport) -> (String, Vec<CurrentLoader>) {
        match support {
            LoaderSupport::Supported {
                mc_version,
                loaders,
            } => (mc_version, loaders),
            LoaderSupport::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn patch_versions_are_converted_to_install_ids() {
        let root = root("read");
        write(
            &root,
            json!({
                "id": "inst",
                "dmcl-patches": [
                    {"id": "1.20.1"},
                    {"name": "forge", "version": "1.20.1-47.2.0"},
                    {"name": "optifine", "version": "1.20.1_HD_U_I6"},
                    {"name": "custom", "version": "1"}
                ]
            }),
        );

        let (mc_version, loaders) = loaders(read_loader_support(&root, "inst"));
        assert_eq!(mc_version, "1.20.1");
        assert_eq!(loaders.len(), 2);
        assert_eq!(loaders[0].kind, LoaderKind::Forge);
        assert_eq!(loaders[0].id, "1.20.1-47.2.0");
        assert_eq!(loaders[0].label, "47.2.0");
        assert_eq!(loaders[1].kind, LoaderKind::OptiFine);
        assert_eq!(loaders[1].id, "HD_U_I6");

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn instances_without_patches_are_unsupported() {
        let root = root("bare");
        write(&root, json!({"id": "inst"}));
        assert!(matches!(
            read_loader_support(&root, "inst"),
            LoaderSupport::Unsupported(_)
        ));

        write(&root, json!({"id": "inst", "dmcl-patches": []}));
        assert!(matches!(
            read_loader_support(&root, "inst"),
            LoaderSupport::Unsupported(_)
        ));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn duplicate_loader_patches_are_unsupported() {
        let root = root("duplicate");
        write(
            &root,
            json!({
                "id": "inst",
                "dmcl-patches": [
                    {"id": "1.20.1"},
                    {"name": "fabric", "version": "0.15.0"},
                    {"name": "fabric", "version": "0.16.0"}
                ]
            }),
        );
        assert!(matches!(
            read_loader_support(&root, "inst"),
            LoaderSupport::Unsupported(_)
        ));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
