//! 解压 natives jar / zip。

use std::path::Path;

use anyhow::{Context, Result};

/// 解压一个 jar / zip 到目标目录。
///
/// 使用 `zip` 自带的安全解压：
/// - 拒绝绝对路径 / `..` / 盘符；
/// - 符号链接不创建；
/// - 流式拷贝，不按声明大小预分配。
pub(crate) async fn extract_zip(
    archive: impl AsRef<Path> + Send + 'static,
    target: impl AsRef<Path> + Send + 'static,
) -> Result<()> {
    let archive = archive.as_ref().to_path_buf();
    let target = target.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive)
            .with_context(|| format!("无法打开 {}", archive.display()))?;
        let mut zip = zip::ZipArchive::new(file)
            .with_context(|| format!("无法解析 {}", archive.display()))?;
        let options = zip::read::ExtractOptions::default()
            .symlink_action(zip::read::SymlinkExtractAction::NoExtract);
        zip.extract_with_options(&target, options)
            .with_context(|| format!("无法解压 {}", archive.display()))?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("解压任务崩溃")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[tokio::test]
    async fn extraction_rejects_entries_that_escape_the_target_directory() {
        let dir = std::env::temp_dir().join(format!("dmcl-game-zip-slip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).unwrap();
        let archive = dir.join("evil.jar");

        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default();
            writer.start_file("../escaped.txt", options).unwrap();
            writer.write_all(b"x").unwrap();
            writer.finish().unwrap();
        }
        std::fs::write(&archive, buffer).unwrap();

        assert!(extract_zip(archive, dir.join("out")).await.is_err());
        assert!(!dir.join("escaped.txt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
