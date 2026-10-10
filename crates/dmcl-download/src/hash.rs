//! sha1 校验与本地文件判断。

use std::path::Path;

use anyhow::{Context, Result};
use sha1::{Digest, Sha1};

/// 一个字节串的小写 hex sha1。
pub fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// 一个文件的小写 hex sha1。
pub async fn hash_file(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        use std::io::Read;

        let mut file =
            std::fs::File::open(&path).with_context(|| format!("无法打开 {}", path.display()))?;
        let mut hasher = Sha1::new();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .context("计算文件 sha1 的任务失败")?
}

/// 文件是否存在、且大小符合预期。
async fn size_matches(path: &Path, size: Option<u64>) -> bool {
    match size {
        None => true,
        Some(expected) => match tokio::fs::metadata(path).await {
            Ok(meta) => meta.len() == expected,
            Err(_) => false,
        },
    }
}

/// 文件是否就是我们要的那一个。
///
/// * 给了 `sha1` —— 必须一致；
/// * 只给了 `size` —— 大小一致即可；
/// * 两个都没给 —— 文件存在就算数。
pub async fn file_matches(path: impl AsRef<Path>, sha1: Option<&str>, size: Option<u64>) -> bool {
    let path = path.as_ref();
    if tokio::fs::metadata(path).await.is_err() {
        return false;
    }
    match sha1 {
        Some(expected) => match hash_file(path).await {
            Ok(actual) => actual == expected,
            Err(_) => false,
        },
        None => size_matches(path, size).await,
    }
}

/// `file_matches` 的别名，语义上更贴近调用点（"这个文件已经好了吗"）。
pub async fn check_file(path: impl AsRef<Path>, sha1: Option<&str>) -> bool {
    file_matches(path, sha1, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_hex_is_the_known_digest_of_empty_input() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[tokio::test]
    async fn files_are_checked_by_hash_or_size() {
        let dir = std::env::temp_dir().join(format!("dmcl-hash-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("a.txt");
        tokio::fs::write(&path, b"abc").await.unwrap();

        assert!(check_file(&path, None).await);
        assert!(check_file(&path, Some("a9993e364706816aba3e25717850c26c9cd0d89d")).await);
        assert!(!check_file(&path, Some("0000000000000000000000000000000000000000")).await);
        assert!(file_matches(&path, None, Some(3)).await);
        assert!(!file_matches(&path, None, Some(4)).await);
        assert!(!check_file(dir.join("missing"), None).await);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn hashing_streams_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("dmcl-hash-big-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("big.bin");
        let data = vec![7u8; 1024 * 1024 + 3];
        tokio::fs::write(&path, &data).await.unwrap();

        assert_eq!(hash_file(&path).await.unwrap(), sha1_hex(&data));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
