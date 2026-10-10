//! 完整性检查：把本地缺了什么 / 坏了的文件列出来。
//!
//! **这个模块不会自己下载任何东西** —— 它只负责回答"完整吗？缺什么？"，
//! 修复交给 [`dmcl_install`]：调用者拿到 [`IntegrityStatus`] 之后，自己决定
//! 是跑一次安装补全，还是直接启动。
//!
//! "哪些文件应该存在"这件事直接复用 `dmcl-install` 的规划函数（纯函数），
//! 因此两边永远不会对不上。

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use dmcl_download::Checksum;
use dmcl_install::{
    plan_asset_objects, plan_assets_index, plan_client_jar, plan_libraries, plan_natives,
    PlanContext, PlannedFile,
};
use dmcl_mcmeta::layout::{
    natives_dir, read_version_json, read_version_json_safe, version_json_path,
};
use dmcl_mcmeta::vanilla::{AssetIndexFile, VersionJson};

/// 一个文件为什么不合格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingReason {
    /// 文件不存在。
    Missing,
    /// 文件在，但摘要对不上。
    WrongHash,
    /// 文件在，但大小对不上（没有校验值时的兜底判断）。
    WrongSize,
    /// 目录还没准备好（例如 natives 还没解压）。
    NotPrepared,
}

impl fmt::Display for MissingReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MissingReason::Missing => "缺失",
            MissingReason::WrongHash => "摘要校验失败",
            MissingReason::WrongSize => "大小不符",
            MissingReason::NotPrepared => "尚未生成",
        })
    }
}

/// 一个需要修复的条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFile {
    pub path: PathBuf,
    /// 修复时用的 **URL 模板**（`${MC_MAVEN}/…`）；`None` 表示由安装流程自行重建。
    pub url: Option<String>,
    /// 期望的校验值（由 SHA-1 摘要构造）。
    pub checksum: Option<Checksum>,
    pub reason: MissingReason,
}

impl fmt::Display for MissingFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}（{}）", self.path.display(), self.reason)
    }
}

/// 一次完整性检查的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntegrityStatus {
    /// 是否完整。
    pub complete: bool,
    /// 需要修复的条目。
    pub missing: Vec<MissingFile>,
    /// 检查了多少个文件。
    pub checked: usize,
}

impl IntegrityStatus {
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// 缺了多少项。
    pub fn missing_count(&self) -> usize {
        self.missing.len()
    }

    /// 一句话说明（CLI / GUI 都可以直接用）。
    pub fn summary(&self) -> String {
        if self.complete {
            format!("版本完整（检查了 {} 个文件）", self.checked)
        } else {
            format!(
                "版本不完整：{} 个条目需要修复（共检查 {} 个文件）",
                self.missing.len(),
                self.checked
            )
        }
    }

    /// 明细，最多 `limit` 行（用于打印）。
    pub fn detail(&self, limit: usize) -> String {
        let mut lines: Vec<String> = self
            .missing
            .iter()
            .take(limit)
            .map(|entry| entry.to_string())
            .collect();
        if self.missing.len() > limit {
            lines.push(format!("……另有 {} 项", self.missing.len() - limit));
        }
        lines.join("\n")
    }
}

/// 检查一个版本在本地是否完整。
///
/// 需要修复时，调用者应执行一次 `dmcl_install`（它会跳过已存在的文件，
/// 只补缺的部分）。
pub async fn check_integrity(plan: &PlanContext, version_name: &str) -> Result<IntegrityStatus> {
    let json = read_version_json_safe(&plan.root_path, version_name).await?;
    let mut status = IntegrityStatus {
        complete: true,
        missing: Vec::new(),
        checked: 0,
    };

    // 客户端 jar + 库文件 + natives jar
    let mut expected: Vec<PlannedFile> = Vec::new();
    if let Some(client) = plan_client_jar(plan, &json, version_name) {
        expected.push(client);
    }
    expected.extend(plan_libraries(plan, &json));

    // 资源：索引本身，加上索引里列出的对象（索引在本地才能算得出来）
    if let Some(index_file) = plan_assets_index(plan, &json) {
        let index_path = index_file.save_to.clone();
        expected.push(index_file);
        if let Some(index) = read_local_asset_index(&index_path).await {
            expected.extend(plan_asset_objects(plan, &json, &index));
        }
    }

    for file in expected {
        status.checked += 1;
        if let Some(reason) = check_planned(&file).await {
            status.missing.push(MissingFile {
                path: file.save_to,
                url: Some(file.url),
                checksum: file.checksum,
                reason,
            });
        }
    }

    // natives 是"解压出来的目录"，不是下载项，单独判断
    if !plan_natives(plan, &json).is_empty() {
        status.checked += 1;
        let dir = natives_dir(version_name);
        if !dir_is_populated(&dir).await {
            status.missing.push(MissingFile {
                path: dir,
                url: None,
                checksum: None,
                reason: MissingReason::NotPrepared,
            });
        }
    }

    status.complete = status.missing.is_empty();
    Ok(status)
}

/// 判断一个规划出来的文件是否合格；合格返回 `None`。
async fn check_planned(file: &PlannedFile) -> Option<MissingReason> {
    let metadata = match tokio::fs::metadata(&file.save_to).await {
        Ok(metadata) => metadata,
        Err(_) => return Some(MissingReason::Missing),
    };
    match file.checksum.as_ref() {
        // 算法由校验值自己决定（md5 / sha1 / sha256 一视同仁）
        Some(checksum) => match checksum.verify_file(&file.save_to).await {
            Ok(true) => None,
            Ok(false) => Some(MissingReason::WrongHash),
            Err(_) => Some(MissingReason::Missing),
        },
        None => match file.size {
            Some(size) if metadata.len() != size => Some(MissingReason::WrongSize),
            _ => None,
        },
    }
}

/// 读本地的资源索引；不存在或读不动时返回 `None`。
async fn read_local_asset_index(path: &Path) -> Option<AssetIndexFile> {
    let text = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&text).ok()
}

/// 目录存在且非空。
async fn dir_is_populated(path: &Path) -> bool {
    let mut entries = match tokio::fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    entries.next_entry().await.ok().flatten().is_some()
}

/// 检查一个版本目录是否至少有一份可用的版本 JSON。
pub async fn version_json_exists(root_path: &Path, version_name: &str) -> Result<VersionJson> {
    let path = version_json_path(root_path, version_name);
    read_version_json(&path)
        .await
        .with_context(|| format!("找不到版本 {}（{}）", version_name, path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dmcl-integrity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 一个最小版本：一个库、一个资源、没有 natives。
    async fn write_version(root: &Path) {
        use dmcl_mcmeta::layout::version_json_path;

        let json = serde_json::json!({
            "id": "1.0",
            "mainClass": "a.Main",
            "assetIndex": {
                "id": "test", "sha1": "abc",
                "url": "https://piston-meta.mojang.com/v1/packages/abc/test.json"
            },
            "libraries": [{
                "name": "a:a:1",
                "downloads": {"artifact": {
                    "url": "https://libraries.minecraft.net/a/a/1/a-1.jar",
                    "path": "a/a/1/a-1.jar", "sha1": "deadbeef"
                }}
            }]
        });
        let path = version_json_path(root, "1.0");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn everything_missing_is_reported_with_its_template_url() {
        let root = work_dir("missing");
        write_version(&root).await;
        let plan = PlanContext::new(&root);

        let status = check_integrity(&plan, "1.0").await.unwrap();
        assert!(!status.is_complete());
        // 库 + 资源索引（索引不在本地，所以资源对象暂时算不出来）
        assert_eq!(status.checked, 2);
        assert_eq!(status.missing.len(), 2);
        let library = status
            .missing
            .iter()
            .find(|entry| entry.path.ends_with("a-1.jar"))
            .expect("库文件应当在缺失列表里");
        assert_eq!(library.reason, MissingReason::Missing);
        assert_eq!(library.url.as_deref(), Some("${MC_MAVEN}/a/a/1/a-1.jar"));
        assert!(status.summary().contains("需要修复"));
        assert!(status.detail(10).contains("a-1.jar"));
    }

    #[tokio::test]
    async fn present_files_with_the_right_hash_are_accepted() {
        let root = work_dir("present");
        write_version(&root).await;

        // 造一个哈希正确的库文件
        let content = b"library bytes";
        let sha1 = dmcl_download::sha1_hex(content);
        let json = serde_json::json!({
            "id": "1.0",
            "libraries": [{
                "name": "a:a:1",
                "downloads": {"artifact": {
                    "url": "https://libraries.minecraft.net/a/a/1/a-1.jar",
                    "path": "a/a/1/a-1.jar", "sha1": sha1
                }}
            }]
        });
        let path = version_json_path(&root, "1.0");
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();

        let library = root.join("libraries/a/a/1/a-1.jar");
        tokio::fs::create_dir_all(library.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&library, content).await.unwrap();

        let plan = PlanContext::new(&root);
        let status = check_integrity(&plan, "1.0").await.unwrap();
        assert!(status.is_complete(), "{:?}", status.missing);
        assert_eq!(status.checked, 1);

        // 内容被改坏后应当报摘要不符
        tokio::fs::write(&library, b"corrupted").await.unwrap();
        let status = check_integrity(&plan, "1.0").await.unwrap();
        assert_eq!(status.missing.len(), 1);
        assert_eq!(status.missing[0].reason, MissingReason::WrongHash);
    }

    #[tokio::test]
    async fn a_missing_version_json_is_an_error() {
        let root = work_dir("noversion");
        let plan = PlanContext::new(&root);
        let err = check_integrity(&plan, "nope").await.unwrap_err();
        assert!(format!("{err:#}").contains("nope"));
    }
}
