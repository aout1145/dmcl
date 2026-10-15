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
}
