//! 完整性检查：把本地缺了什么 / 坏了的文件列出来。
//!
//! **这个模块不会自己下载任何东西** —— 它只负责回答「完整吗？缺什么？」，
//! 修复由 [`crate::ensure_files`] 执行（它会跳过已存在的文件，只补缺的部分）。
//!
//! 「哪些文件应该有」用 [`crate::plan`] 的纯函数（与下载同一批），
//! 「本地这个文件好不好」用 [`dmcl_download::file_ok`]（与下载的跳过同一判定），
//! 并发调度交给 [`dmcl_download::files_ok`]，因此这里不会和下载流程对不上。

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::Result;
use dmcl_download::{files_ok, FileCheck};
use dmcl_mcmeta::layout::assets_index_path;
use dmcl_mcmeta::vanilla::AssetIndexFile;

use crate::options::GameFilesOptions;
use crate::plan::plan_files;
use crate::version::ResolvedVersion;

/// 一个文件为什么不合格。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingReason {
    /// 文件不存在。
    Missing,
    /// 文件在，但内容与预期不符（摘要 / 大小对不上）。
    Incomplete,
}

impl fmt::Display for MissingReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MissingReason::Missing => "缺失",
            MissingReason::Incomplete => "内容不符",
        })
    }
}

/// 一个需要修复的条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFile {
    pub path: PathBuf,
    /// 修复时用的 **URL 模板**（`${MC_MAVEN}/…`）；`None` 表示由安装流程自行重建。
    pub url: Option<String>,
    /// 期望的校验值。
    pub checksum: Option<dmcl_download::Checksum>,
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
pub async fn check_integrity(
    version: &ResolvedVersion,
    options: &GameFilesOptions,
) -> Result<IntegrityStatus> {
    let json = &version.json;
    let mut status = IntegrityStatus {
        complete: true,
        missing: Vec::new(),
        checked: 0,
    };

    // 资源对象要本地有索引才算得出来；索引本身也在下面被检查。
    let index = if json.asset_index.is_some() {
        read_local_asset_index(&assets_index_path(
            &version.reference.root,
            json.asset_index_id(),
        ))
        .await
    } else {
        None
    };

    let files = plan_files(version, index.as_ref(), options);
    let oks = files_ok(files.iter().map(|file| FileCheck {
        path: file.save_to.clone(),
        checksum: file.checksum.clone(),
        size: file.size,
    }))
    .await?;

    for (file, ok) in files.into_iter().zip(oks) {
        status.checked += 1;
        if ok {
            continue;
        }
        // 只区分「不存在」和「在但不完整」
        let reason = if tokio::fs::metadata(&file.save_to).await.is_ok() {
            MissingReason::Incomplete
        } else {
            MissingReason::Missing
        };
        status.missing.push(MissingFile {
            path: file.save_to,
            url: Some(file.url),
            checksum: file.checksum,
            reason,
        });
    }

    // natives 不在这里检查：它的分类器取决于启动时选中的 java 架构，由启动流程的
    // 「解压 natives」步骤负责下载 + 解压（见 [`crate::ensure_natives`]）。

    status.complete = status.missing.is_empty();
    Ok(status)
}

/// 读本地的资源索引；不存在或读不动时返回 `None`。
async fn read_local_asset_index(path: &Path) -> Option<AssetIndexFile> {
    let text = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::{resolve_version, VersionRef};
    use dmcl_mcmeta::layout::version_json_path;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};

    fn work_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dmcl-game-integrity-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn options() -> GameFilesOptions {
        GameFilesOptions::new(DownloadSource::new(Mirror::Official))
    }

    async fn write_json(root: &Path, json: serde_json::Value) {
        let path = version_json_path(root, "1.0");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&json).unwrap())
            .await
            .unwrap();
    }

    async fn resolved(root: &Path) -> ResolvedVersion {
        resolve_version(&VersionRef::new(root, "1.0"))
            .await
            .unwrap()
    }

    /// 一个最小版本：一个库、一个资源索引、没有 natives。
    async fn write_version(root: &Path, index_sha1: Option<&str>) {
        let mut index = serde_json::json!({
            "id": "test",
            "url": "https://piston-meta.mojang.com/v1/packages/abc/test.json"
        });
        if let Some(sha1) = index_sha1 {
            index["sha1"] = serde_json::json!(sha1);
        }
        write_json(
            root,
            serde_json::json!({
                "id": "1.0",
                "mainClass": "a.Main",
                "assetIndex": index,
                "libraries": [{
                    "name": "a:a:1",
                    "downloads": {"artifact": {
                        "url": "https://libraries.minecraft.net/a/a/1/a-1.jar",
                        "path": "a/a/1/a-1.jar", "sha1": "deadbeef"
                    }}
                }]
            }),
        )
        .await;
    }

    #[tokio::test]
    async fn everything_missing_is_reported_with_its_template_url() {
        let root = work_dir("missing");
        write_version(&root, None).await;

        let status = check_integrity(&resolved(&root).await, &options())
            .await
            .unwrap();
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

        // 造一个哈希正确的库文件
        let content = b"library bytes";
        let sha1 = dmcl_download::hash_bytes(dmcl_download::HashAlgorithm::Sha1, content);
        write_json(
            &root,
            serde_json::json!({
                "id": "1.0",
                "libraries": [{
                    "name": "a:a:1",
                    "downloads": {"artifact": {
                        "url": "https://libraries.minecraft.net/a/a/1/a-1.jar",
                        "path": "a/a/1/a-1.jar", "sha1": sha1
                    }}
                }]
            }),
        )
        .await;

        let library = root.join("libraries/a/a/1/a-1.jar");
        tokio::fs::create_dir_all(library.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&library, content).await.unwrap();

        let status = check_integrity(&resolved(&root).await, &options())
            .await
            .unwrap();
        assert!(status.is_complete(), "{:?}", status.missing);
        assert_eq!(status.checked, 1);

        // 内容被改坏后应当报「内容不符」
        tokio::fs::write(&library, b"corrupted").await.unwrap();
        let status = check_integrity(&resolved(&root).await, &options())
            .await
            .unwrap();
        assert_eq!(status.missing.len(), 1);
        assert_eq!(status.missing[0].reason, MissingReason::Incomplete);
    }

    #[tokio::test]
    async fn a_present_index_adds_its_objects_to_the_check() {
        let root = work_dir("index");
        let content = b"asset bytes";
        let sha1 = dmcl_download::hash_bytes(dmcl_download::HashAlgorithm::Sha1, content);
        write_version(&root, None).await;

        // 版本 JSON 里的索引没有 sha1：只检查它是否存在。
        let index = serde_json::json!({"objects": {
            "a/file.txt": {"hash": sha1, "size": content.len()}
        }});
        let index_path = assets_index_path(&root, "test");
        tokio::fs::create_dir_all(index_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&index_path, serde_json::to_string(&index).unwrap())
            .await
            .unwrap();

        // 库文件也补上，否则只会报库缺失。
        let library = root.join("libraries/a/a/1/a-1.jar");
        tokio::fs::create_dir_all(library.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&library, b"library").await.unwrap();

        let status = check_integrity(&resolved(&root).await, &options())
            .await
            .unwrap();
        // 索引存在后资源对象才进入检查：库 + 索引 + 1 个资源对象。
        assert_eq!(status.checked, 3);
        let object_path = root
            .join("assets/objects")
            .join(format!("{}/{}", &sha1[..2], sha1));
        let object = status
            .missing
            .iter()
            .find(|entry| entry.path == object_path)
            .expect("资源对象应当在缺失列表里");
        assert_eq!(object.reason, MissingReason::Missing);
        let expected_url = format!("${{MC_ASSETS}}/{}/{}", &sha1[..2], sha1);
        assert_eq!(object.url.as_deref(), Some(expected_url.as_str()));
    }

    #[tokio::test]
    async fn a_corrupt_index_is_reported_without_its_objects() {
        let root = work_dir("corrupt-index");
        let correct = b"not really an index";
        let sha1 = dmcl_download::hash_bytes(dmcl_download::HashAlgorithm::Sha1, correct);
        write_version(&root, Some(&sha1)).await;

        let index_path = assets_index_path(&root, "test");
        tokio::fs::create_dir_all(index_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&index_path, b"corrupt").await.unwrap();

        let status = check_integrity(&resolved(&root).await, &options())
            .await
            .unwrap();
        // 库 + 索引；索引读不出来时不报告资源对象。
        assert_eq!(status.checked, 2);
        let index = status
            .missing
            .iter()
            .find(|entry| entry.path.ends_with("test.json"))
            .expect("索引本身应当在缺失列表里");
        assert_eq!(index.reason, MissingReason::Incomplete);
        assert!(status
            .missing
            .iter()
            .all(|entry| !entry.path.starts_with(root.join("assets/objects"))));
    }
}
