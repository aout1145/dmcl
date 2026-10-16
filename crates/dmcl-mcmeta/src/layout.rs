//! 本地 `.minecraft` 目录布局，以及读取版本 JSON。

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

/// 读版本 JSON，并解析 `inheritsFrom`。
pub async fn read_version_json_safe(root_path: &Path, version_name: &str) -> Result<VersionJson> {
    let path = version_json_path(root_path, version_name);
    let mut json = read_version_json(&path).await?;

    if let Some(orig) = json.inherits_from.clone() {
        // 若基于另一个版本，则合并两者
        let mut orijson = read_version_json(&version_json_path(root_path, &orig)).await?;
        orijson.merge(&json)?;
        json = orijson;
    }
    Ok(json)
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
}
