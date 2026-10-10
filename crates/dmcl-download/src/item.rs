//! 下载项。

use std::path::PathBuf;

use crate::hash::Checksum;

/// 一个待下载项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    /// 具体地址。**不含** URL 模板 —— 模板由上层（安装流程）解析好再传进来。
    pub url: String,
    /// 目标文件路径。
    pub save_to: PathBuf,
    /// 期望的校验值（md5 / sha1 / sha256）；给了就必须一致。
    pub checksum: Option<Checksum>,
    /// 期望的大小；用于分块判断，也作为没有校验值时的完整性依据。
    pub size: Option<u64>,
    /// 进度里显示的名字，默认取文件名。
    pub name: String,
}

impl DownloadItem {
    pub fn new(url: impl Into<String>, save_to: impl Into<PathBuf>) -> Self {
        let save_to = save_to.into();
        let name = save_to
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| save_to.to_string_lossy().into_owned());
        Self {
            url: url.into(),
            save_to,
            checksum: None,
            size: None,
            name,
        }
    }

    /// 指定校验值（算法随 [`Checksum`]）。
    pub fn with_checksum(mut self, checksum: Option<Checksum>) -> Self {
        self.checksum = checksum;
        self
    }

    /// 指定 sha1（Minecraft 文档里给的就是这个）。
    pub fn with_sha1(mut self, hex: Option<impl Into<String>>) -> Self {
        self.checksum = hex.map(Checksum::sha1);
        self
    }

    /// 指定 sha256。
    pub fn with_sha256(mut self, hex: Option<impl Into<String>>) -> Self {
        self.checksum = hex.map(Checksum::sha256);
        self
    }

    /// 指定 md5。
    pub fn with_md5(mut self, hex: Option<impl Into<String>>) -> Self {
        self.checksum = hex.map(Checksum::md5);
        self
    }

    pub fn with_size(mut self, size: Option<u64>) -> Self {
        self.size = size;
        self
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// 临时文件路径：全部下载并校验通过之后才改名成 [`DownloadItem::save_to`]。
    pub fn part_path(&self) -> PathBuf {
        let file_name = self
            .save_to
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".to_string());
        self.save_to.with_file_name(format!("{file_name}.part"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::HashAlgorithm;

    #[test]
    fn items_default_their_name_to_the_file_name() {
        let item = DownloadItem::new("https://example.com/a/b.jar", "/tmp/b.jar");
        assert_eq!(item.name, "b.jar");
        assert_eq!(item.part_path(), PathBuf::from("/tmp/b.jar.part"));
        assert_eq!(item.clone().named("别的名字").name, "别的名字");
        assert_eq!(item.clone().with_size(Some(3)).size, Some(3));
        assert!(item.checksum.is_none());
    }

    #[test]
    fn checksums_can_be_attached_with_any_algorithm() {
        let hex = "ab".repeat(20);
        let item = DownloadItem::new("u", "/tmp/a")
            .with_sha1(Some(hex.clone()))
            .with_size(Some(1));
        let checksum = item.checksum.as_ref().unwrap();
        assert_eq!(checksum.algorithm(), HashAlgorithm::Sha1);
        assert_eq!(checksum.hex(), hex);

        let item = DownloadItem::new("u", "/tmp/a").with_sha256(Some("cd".repeat(32)));
        assert_eq!(
            item.checksum.as_ref().unwrap().algorithm(),
            HashAlgorithm::Sha256
        );

        let item = DownloadItem::new("u", "/tmp/a").with_md5(Some("ef".repeat(16)));
        assert_eq!(
            item.checksum.as_ref().unwrap().algorithm(),
            HashAlgorithm::Md5
        );

        // 显式传入 None 表示"不做校验"
        assert!(DownloadItem::new("u", "/tmp/a")
            .with_sha1(None::<String>)
            .checksum
            .is_none());
    }
}
