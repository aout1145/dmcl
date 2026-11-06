//! 读取安装器 jar / zip 里的条目。
//!
//! 解压本身属于「把游戏文件铺好」，在 `dmcl-game` 里；这里只处理安装器包内的内存读取。

use std::io::{Cursor, Read, Seek};

use anyhow::{Context, Result};

/// 读一个内存里归档的条目。
pub fn read_entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("归档里没有 {name} 这一项"))?;
    let mut buffer = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// 归档里是否有这一项。
pub fn has_entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> bool {
    archive.by_name(name).is_ok()
}

/// 读 zip 条目里 jar 的 `Main-Class`（处理器 jar 的入口类）。
pub fn main_class_of<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<String> {
    let manifest = read_entry(archive, "META-INF/MANIFEST.MF")?;
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
        assert!(has_entry(&mut archive, "META-INF/MANIFEST.MF"));
        assert!(!has_entry(&mut archive, "nope"));
        assert_eq!(
            main_class_of(&mut archive).unwrap(),
            "com.example.Processor"
        );
        assert!(read_entry(&mut archive, "nope").is_err());
    }
}
