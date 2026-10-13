//! 下载项。

use std::path::PathBuf;

use crate::hash::Checksum;

/// 一个待下载项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    /// 候选下载地址，**按优先级排列**（靠前的先试）；至少一个。
    ///
    /// 不含 URL 模板 —— 模板由上层（安装流程）按镜像解析成候选列表再传进来。
    pub urls: Vec<String>,
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
    /// 用**候选地址列表**构造（顺序即优先级）。候选为空时下载会直接报错。
    pub fn new(urls: impl IntoIterator<Item = String>, save_to: impl Into<PathBuf>) -> Self {
        let save_to = save_to.into();
        let name = save_to
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| save_to.to_string_lossy().into_owned());
        Self {
            urls: urls.into_iter().collect(),
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
    fn items_hold_their_candidate_urls_and_default_their_name() {
        let item = DownloadItem::new(
            [
                "https://bmclapi2.bangbang93.com/a/b.jar".to_string(),
                "https://libraries.minecraft.net/a/b.jar".to_string(),
            ],
            "/tmp/b.jar",
        );
        assert_eq!(
            item.urls,
            vec![
                "https://bmclapi2.bangbang93.com/a/b.jar".to_string(),
                "https://libraries.minecraft.net/a/b.jar".to_string(),
            ]
        );
        assert_eq!(item.name, "b.jar");
        assert_eq!(item.part_path(), PathBuf::from("/tmp/b.jar.part"));
        assert_eq!(item.clone().named("别的名字").name, "别的名字");
        assert_eq!(item.clone().with_size(Some(3)).size, Some(3));
        assert!(item.checksum.is_none());
    }

    #[test]
    fn checksums_can_be_attached_with_any_algorithm() {
        let hex = "ab".repeat(20);
        let item = DownloadItem::new(["u".to_string()], "/tmp/a")
            .with_sha1(Some(hex.clone()))
            .with_size(Some(1));
        let checksum = item.checksum.as_ref().unwrap();
        assert_eq!(checksum.algorithm(), HashAlgorithm::Sha1);
        assert_eq!(checksum.hex(), hex);

        let item =
            DownloadItem::new(["u".to_string()], "/tmp/a").with_sha256(Some("cd".repeat(32)));
        assert_eq!(
            item.checksum.as_ref().unwrap().algorithm(),
            HashAlgorithm::Sha256
        );

        let item = DownloadItem::new(["u".to_string()], "/tmp/a").with_md5(Some("ef".repeat(16)));
        assert_eq!(
            item.checksum.as_ref().unwrap().algorithm(),
            HashAlgorithm::Md5
        );

        // 显式传入 None 表示"不做校验"
        assert!(DownloadItem::new(["u".to_string()], "/tmp/a")
            .with_sha1(None::<String>)
            .checksum
            .is_none());
    }
}
