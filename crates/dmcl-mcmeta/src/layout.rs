//! 本地 `.minecraft` 目录布局，以及读取版本 JSON。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use path_clean::PathClean;

use crate::os::{Arch, Os};
use crate::vanilla::version::VersionJson;

/// 把路径变成绝对路径并归一化（处理 `.` / `..` / 重复分隔符）。
pub fn absolute(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute.clean()
}

/// `versions/<version_name>` —— 一个版本自己的目录。
pub fn version_dir(root_path: &Path, version_name: &str) -> PathBuf {
    root_path.join("versions").join(version_name)
}

/// `versions/<version_name>/<version_name>.json`。
pub fn version_json_path(root_path: &Path, version_name: &str) -> PathBuf {
    version_dir(root_path, version_name).join(format!("{version_name}.json"))
}

/// `versions/<version_name>/<version_name>.jar`。
pub fn version_jar_path(root_path: &Path, version_name: &str) -> PathBuf {
    version_dir(root_path, version_name).join(format!("{version_name}.jar"))
}

/// 一个已安装的版本（`versions/<name>/` 目录）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledVersion {
    /// 本地版本名（目录名，也是启动时用的 `version-name`）。
    pub name: String,
    /// 版本目录 `versions/<name>`。
    pub dir: PathBuf,
    /// 版本 JSON 路径 `versions/<name>/<name>.json`（不保证存在）。
    pub json_path: PathBuf,
}

/// 列出 `<root_path>/versions/` 下的已安装版本，按名字排序。
///
/// `versions/` 不存在（还没装过任何版本）时返回空列表；只统计**目录**，
/// 忽略同名文件。目录里是否真的装了版本（有 `<name>.json`）不在这一步判断
/// —— 那是完整性检查的事。
pub async fn installed_versions(root_path: &Path) -> Result<Vec<InstalledVersion>> {
    let versions_dir = root_path.join("versions");
    let mut entries = match tokio::fs::read_dir(&versions_dir).await {
        Ok(entries) => entries,
        // 还没装过任何版本
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(err).with_context(|| format!("无法读取 {}", versions_dir.display()))
        }
    };

    let mut versions = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("无法读取 {}", versions_dir.display()))?
    {
        let file_type = entry
            .file_type()
            .await
            .with_context(|| format!("无法读取 {}", entry.path().display()))?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path();
        let json_path = dir.join(format!("{name}.json"));
        versions.push(InstalledVersion {
            name,
            dir,
            json_path,
        });
    }
    versions.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(versions)
}

pub fn libraries_dir(root_path: &Path) -> PathBuf {
    root_path.join("libraries")
}

pub fn assets_dir(root_path: &Path) -> PathBuf {
    root_path.join("assets")
}

/// `assets/indexes/<id>.json`。
pub fn assets_index_path(root_path: &Path, asset_index_id: &str) -> PathBuf {
    assets_dir(root_path)
        .join("indexes")
        .join(format!("{asset_index_id}.json"))
}

/// `assets/objects`。
pub fn assets_objects_dir(root_path: &Path) -> PathBuf {
    assets_dir(root_path).join("objects")
}

/// `assets/virtual/<id>` —— 1.6 及更早版本用的虚拟资源目录。
pub fn assets_virtual_dir(root_path: &Path, asset_index_id: &str) -> PathBuf {
    assets_dir(root_path).join("virtual").join(asset_index_id)
}

/// natives 解压目录（放在临时目录里）。
///
/// 目录名带上 os 与 java 架构（如 `dmcl-natives-linux-x86_64-<hash>`）：换平台或换 JVM
/// 架构时会落到不同目录，不会把上一次的 natives 混进来。版本名仍然经过 FNV-1a 散列，
/// 避免其中的路径分隔符等字符混进文件名。
pub fn natives_dir(version_name: &str, os: Os, arch: Arch) -> PathBuf {
    std::env::temp_dir().join(format!(
        "dmcl-natives-{}-{}-{:016x}",
        os.mc_name(),
        arch.name(),
        fnv1a64(version_name)
    ))
}

/// 读一个版本 JSON 文件。
pub async fn read_version_json(path: &Path) -> Result<VersionJson> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("无法读取 {}", path.display()))?;
    let mut json: VersionJson =
        serde_json::from_str(&text).with_context(|| format!("无法解析 {}", path.display()))?;
    // 从磁盘读出来的 url 是具体地址：归一成模板后才能按镜像下载。
    json.templatize_urls();
    Ok(json)
}

/// 读取版本 JSON，并沿 `inheritsFrom` 递归解析整条继承链。
///
/// 最终 JSON 从根祖先开始，依次 merge 到叶子版本。完全解析后清空 `inheritsFrom`；
/// 如果合并结果没有 `jar`，把它设成根祖先（叶子没有父版本时保持 `None`）。
pub async fn read_resolved_version_json(
    root_path: &Path,
    version_name: &str,
) -> Result<VersionJson> {
    // 叶子 -> ... -> 根祖先
    let mut chain: Vec<(String, VersionJson)> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut current = version_name.to_string();

    loop {
        if !visited.insert(current.clone()) {
            let mut names: Vec<&str> = chain.iter().map(|(name, _)| name.as_str()).collect();
            names.push(&current);
            anyhow::bail!("版本继承出现循环：{}", names.join(" -> "));
        }

        let json = read_version_json(&version_json_path(root_path, &current)).await?;
        let parent = json.inherits_from.clone();
        chain.push((current.clone(), json));

        match parent {
            Some(parent) if !parent.is_empty() => current = parent,
            _ => break,
        }
    }

    // chain 是叶子 -> 根祖先；pop 出来正好从根祖先开始。
    let (root_name, root_json) = chain
        .pop()
        .ok_or_else(|| anyhow::anyhow!("版本继承链为空"))?;
    let mut resolved = root_json;
    while let Some((_, child)) = chain.pop() {
        resolved.merge(&child);
    }

    // 完全解析后不再需要继承字段；主 jar 由 `jar` 表示。
    if resolved.jar.as_deref().is_some_and(str::is_empty) {
        resolved.jar = None;
    }
    if resolved.jar.is_none() && root_name != version_name {
        resolved.jar = Some(root_name);
    }
    resolved.inherits_from = None;

    Ok(resolved)
}

fn fnv1a64(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_absolute_and_normalised() {
        let path = absolute("a/./b/../c");
        assert!(path.is_absolute());
        assert!(path.ends_with("a/c"), "{path:?}");
    }

    #[test]
    fn the_layout_matches_the_official_launcher() {
        let root = Path::new("/mc");
        assert_eq!(
            version_dir(root, "1.20.1"),
            Path::new("/mc/versions/1.20.1")
        );
        assert_eq!(
            version_json_path(root, "1.20.1"),
            Path::new("/mc/versions/1.20.1/1.20.1.json")
        );
        assert_eq!(
            version_jar_path(root, "1.20.1"),
            Path::new("/mc/versions/1.20.1/1.20.1.jar")
        );
        assert_eq!(libraries_dir(root), Path::new("/mc/libraries"));
        assert_eq!(
            assets_index_path(root, "5"),
            Path::new("/mc/assets/indexes/5.json")
        );
        assert_eq!(assets_objects_dir(root), Path::new("/mc/assets/objects"));
        assert_eq!(
            assets_virtual_dir(root, "legacy"),
            Path::new("/mc/assets/virtual/legacy")
        );
    }

    #[test]
    fn natives_directories_are_stable_and_distinct() {
        let (os, arch) = (Os::current(), Arch::X86_64);
        assert_eq!(
            natives_dir("1.20.1", os, arch),
            natives_dir("1.20.1", os, arch)
        );
        assert_ne!(
            natives_dir("1.20.1", os, arch),
            natives_dir("1.20.2", os, arch)
        );
        // 架构不同 ⇒ 目录不同（换 JVM 不会复用上一次的 natives）
        assert_ne!(
            natives_dir("1.20.1", os, Arch::X86_64),
            natives_dir("1.20.1", os, Arch::X86)
        );
        let name = natives_dir("1.20.1", os, Arch::X86_64)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with("dmcl-natives-"), "{name}");
        assert!(name.contains("x86_64"), "{name}");
    }

    #[tokio::test]
    async fn installed_versions_lists_version_directories_sorted() {
        let root = std::env::temp_dir().join(format!("dmcl-installed-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let versions = root.join("versions");
        tokio::fs::create_dir_all(versions.join("1.20.1"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(versions.join("1.20.1-forge"))
            .await
            .unwrap();
        // 同名文件不算版本
        tokio::fs::write(versions.join("note.txt"), "x")
            .await
            .unwrap();

        let found = installed_versions(&root).await.unwrap();
        let names: Vec<&str> = found.iter().map(|version| version.name.as_str()).collect();
        assert_eq!(names, vec!["1.20.1", "1.20.1-forge"]);
        assert_eq!(found[0].dir, version_dir(&root, "1.20.1"));
        assert_eq!(found[0].json_path, version_json_path(&root, "1.20.1"));

        // 目录不存在 ⇒ 空列表，而不是报错
        let empty = installed_versions(&root.join("nope")).await.unwrap();
        assert!(empty.is_empty());

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    fn work_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dmcl-mcmeta-resolve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    async fn write_version(root: &Path, name: &str, value: serde_json::Value) {
        let path = version_json_path(root, name);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&value).unwrap())
            .await
            .unwrap();
    }

    fn plain_arguments(arguments: Option<&Vec<crate::vanilla::Argument>>) -> Vec<String> {
        arguments
            .unwrap()
            .iter()
            .filter_map(|argument| match argument {
                crate::vanilla::Argument::Plain(value) => Some(value.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn resolves_the_whole_inheritance_chain_from_the_root_ancestor() {
        let root = work_dir("chain");
        write_version(
            &root,
            "vanilla",
            serde_json::json!({
                "id": "vanilla",
                "mainClass": "v.Main",
                "libraries": [{"name": "v:lib:1"}],
                "arguments": {"game": ["--vanilla"]},
                "downloads": {"client": {"url": "https://piston-data.mojang.com/x"}},
                "assetIndex": {"id": "5"},
                "assets": "5",
                "javaVersion": {"majorVersion": 21}
            }),
        )
        .await;
        write_version(
            &root,
            "forge",
            serde_json::json!({
                "id": "forge",
                "inheritsFrom": "vanilla",
                "libraries": [{"name": "f:lib:1"}],
                "arguments": {"game": ["--forge"]},
                "mainClass": "f.Main"
            }),
        )
        .await;
        write_version(
            &root,
            "optifine",
            serde_json::json!({
                "id": "optifine",
                "inheritsFrom": "forge",
                "libraries": [{"name": "o:lib:1"}],
                "arguments": {"game": ["--optifine"]}
            }),
        )
        .await;

        let resolved = read_resolved_version_json(&root, "optifine").await.unwrap();

        assert_eq!(resolved.main_class.as_deref(), Some("f.Main"));
        let names: Vec<&str> = resolved
            .libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect();
        assert_eq!(names, ["o:lib:1", "f:lib:1", "v:lib:1"]);
        assert_eq!(
            plain_arguments(resolved.arguments.as_ref().unwrap().game.as_ref()),
            ["--vanilla", "--forge", "--optifine"]
        );
        assert_eq!(resolved.jar.as_deref(), Some("vanilla"));
        assert!(resolved.inherits_from.is_none());
        // 叶子和中间版本没有的字段从根祖先保留
        assert!(resolved.downloads.is_some());
        assert_eq!(resolved.asset_index_id(), "5");
        assert_eq!(resolved.assets.as_deref(), Some("5"));
        assert_eq!(resolved.required_java_major(), 21);

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn the_leaf_can_override_ancestor_fields() {
        let root = work_dir("override");
        write_version(
            &root,
            "base",
            serde_json::json!({
                "id": "base",
                "mainClass": "a.Main",
                "minecraftArguments": "--old"
            }),
        )
        .await;
        write_version(
            &root,
            "leaf",
            serde_json::json!({
                "id": "leaf",
                "inheritsFrom": "base",
                "mainClass": "b.Main",
                "minecraftArguments": "--new"
            }),
        )
        .await;

        let resolved = read_resolved_version_json(&root, "leaf").await.unwrap();

        assert_eq!(resolved.main_class.as_deref(), Some("b.Main"));
        assert_eq!(resolved.minecraft_arguments.as_deref(), Some("--new"));

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn the_same_library_key_merges_level_by_level() {
        let root = work_dir("levels");
        write_version(
            &root,
            "vanilla",
            serde_json::json!({"libraries": [{"name": "x:lib:1"}]}),
        )
        .await;
        write_version(
            &root,
            "forge",
            serde_json::json!({
                "inheritsFrom": "vanilla",
                "libraries": [{"name": "x:lib:2"}]
            }),
        )
        .await;
        write_version(
            &root,
            "optifine",
            serde_json::json!({
                "inheritsFrom": "forge",
                "libraries": [{"name": "x:lib:1"}]
            }),
        )
        .await;

        let resolved = read_resolved_version_json(&root, "optifine").await.unwrap();

        let names: Vec<&str> = resolved
            .libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect();
        assert_eq!(names, ["x:lib:2"]);

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn an_explicit_intermediate_jar_is_kept() {
        let root = work_dir("jar");
        write_version(&root, "vanilla", serde_json::json!({"id": "vanilla"})).await;
        write_version(
            &root,
            "forge",
            serde_json::json!({"inheritsFrom": "vanilla", "jar": "forge-jar"}),
        )
        .await;
        write_version(
            &root,
            "optifine",
            serde_json::json!({"inheritsFrom": "forge"}),
        )
        .await;

        let resolved = read_resolved_version_json(&root, "optifine").await.unwrap();

        assert_eq!(resolved.jar.as_deref(), Some("forge-jar"));

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn a_single_parent_is_merged_and_becomes_the_jar() {
        let root = work_dir("single");
        write_version(
            &root,
            "parent",
            serde_json::json!({
                "id": "parent",
                "mainClass": "a.Main",
                "libraries": [{"name": "a:a:1"}]
            }),
        )
        .await;
        write_version(
            &root,
            "child",
            serde_json::json!({"id": "child", "inheritsFrom": "parent"}),
        )
        .await;

        let resolved = read_resolved_version_json(&root, "child").await.unwrap();

        assert_eq!(resolved.main_class.as_deref(), Some("a.Main"));
        assert_eq!(resolved.libraries.len(), 1);
        assert_eq!(resolved.jar.as_deref(), Some("parent"));
        assert!(resolved.inherits_from.is_none());

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn an_empty_inherits_from_means_no_parent() {
        let root = work_dir("empty-parent");
        write_version(
            &root,
            "solo",
            serde_json::json!({"id": "solo", "inheritsFrom": "", "mainClass": "m.Main"}),
        )
        .await;

        let resolved = read_resolved_version_json(&root, "solo").await.unwrap();

        assert_eq!(resolved.main_class.as_deref(), Some("m.Main"));
        assert!(resolved.inherits_from.is_none());
        assert!(resolved.jar.is_none());

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn a_missing_parent_reports_its_path() {
        let root = work_dir("missing");
        write_version(
            &root,
            "child",
            serde_json::json!({"id": "child", "inheritsFrom": "missing"}),
        )
        .await;

        let err = read_resolved_version_json(&root, "child")
            .await
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(
            message.contains(&version_json_path(&root, "missing").display().to_string()),
            "{message}"
        );

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn a_cyclic_inheritance_reports_the_whole_chain() {
        let root = work_dir("cycle");
        write_version(
            &root,
            "a",
            serde_json::json!({"id": "a", "inheritsFrom": "b"}),
        )
        .await;
        write_version(
            &root,
            "b",
            serde_json::json!({"id": "b", "inheritsFrom": "a"}),
        )
        .await;

        let err = read_resolved_version_json(&root, "a").await.unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("循环"), "{message}");
        assert!(message.contains("a -> b -> a"), "{message}");

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }
}
