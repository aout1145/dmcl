//! 下载项。

use std::path::PathBuf;

/// 一个待下载项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    /// 具体地址。**不含** URL 模板 —— 模板由上层（安装流程）解析好再传进来。
    pub url: String,
    /// 目标文件路径。
    pub save_to: PathBuf,
    /// 期望的 sha1；给了就必须一致。
    pub sha1: Option<String>,
    /// 期望的大小；用于分块判断，也作为没有 sha1 时的完整性依据。
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
            sha1: None,
            size: None,
            name,
        }
    }

    pub fn with_sha1(mut self, sha1: Option<String>) -> Self {
        self.sha1 = sha1;
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

    #[test]
    fn items_default_their_name_to_the_file_name() {
        let item = DownloadItem::new("https://example.com/a/b.jar", "/tmp/b.jar");
        assert_eq!(item.name, "b.jar");
        assert_eq!(item.part_path(), PathBuf::from("/tmp/b.jar.part"));
        assert_eq!(item.clone().named("别的名字").name, "别的名字");
        assert_eq!(item.clone().with_size(Some(3)).size, Some(3));
        assert_eq!(item.with_sha1(Some("x".into())).sha1.as_deref(), Some("x"));
    }
}
