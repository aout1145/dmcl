//! jar / zip 处理。
//!
//! `zip` crate 是同步的，因此解压放到阻塞线程里跑。

use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 解压一个 jar / zip。
pub async fn extract_zip(
    filename: impl AsRef<Path> + Send + 'static,
    extract_to: impl AsRef<Path> + Send + 'static,
) -> Result<()> {
    let filename = filename.as_ref().to_path_buf();
    let extract_to = extract_to.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || extract_zip_blocking(&filename, &extract_to))
        .await
        .context("解压任务崩溃")?
}

fn extract_zip_blocking(filename: &Path, extract_to: &Path) -> Result<()> {
    let file = std::fs::File::open(filename)
        .with_context(|| format!("无法打开 {}", filename.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    std::fs::create_dir_all(extract_to)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name()?.to_string();
        if name.is_empty() {
            continue;
        }
        let out = extract_to.join(&name);
        if name.ends_with('/') {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut buffer = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buffer)?;
            std::fs::write(&out, buffer)?;
        }
    }
    Ok(())
}

/// 读一个内存里归档的条目。
pub fn zip_read_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("归档里没有 {name} 这一项"))?;
    let mut buffer = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// 归档里是否有这一项。
pub fn zip_has_entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> bool {
    archive.by_name(name).is_ok()
}

/// 读 zip 条目里 jar 的 `Main-Class`（处理器 jar 的入口类）。
pub fn main_class_of<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<String> {
    let manifest = zip_read_entry(archive, "META-INF/MANIFEST.MF")?;
    let manifest = String::from_utf8_lossy(&manifest).into_owned();
    Ok(manifest
        .split("Main-Class: ")
        .nth(1)
        .unwrap_or("")
        .split('\n')
        .next()
        .unwrap_or("")
        .trim()
        .to_string())
}

/// 打开内存里的归档。
pub fn open_archive(bytes: Vec<u8>) -> Result<zip::ZipArchive<Cursor<Vec<u8>>>> {
    Ok(zip::ZipArchive::new(Cursor::new(bytes))?)
}

/// 在 `PATH` 上找一个可执行文件（供 [`crate::PathJava`] 使用）。
pub fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Windows 上可执行文件带扩展名
        #[cfg(windows)]
        for extension in ["exe", "bat", "cmd"] {
            let candidate = dir.join(format!("{program}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_entries_can_be_read_and_probed() {
        // 造一个只有一个 manifest 的 zip
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default();
            writer.start_file("META-INF/MANIFEST.MF", options).unwrap();
            std::io::Write::write_all(
                &mut writer,
                b"Manifest-Version: 1.0\nMain-Class: com.example.Processor\n\n",
            )
            .unwrap();
            writer.finish().unwrap();
        }

        let mut archive = open_archive(buffer).unwrap();
        assert!(zip_has_entry(&mut archive, "META-INF/MANIFEST.MF"));
        assert!(!zip_has_entry(&mut archive, "nope"));
        assert_eq!(
            main_class_of(&mut archive).unwrap(),
            "com.example.Processor"
        );
        assert!(zip_read_entry(&mut archive, "nope").is_err());
    }
}
