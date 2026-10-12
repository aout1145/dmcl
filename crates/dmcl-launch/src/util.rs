//! 杂项助手。

use std::path::Path;

/// 路径的 Windows 盘符前缀（`"C:"`），没有则返回 `""`。
///
/// 只用来回显 `cd <盘符>` 那一段；匹配 `Component::Prefix` 就不需要判断平台。
pub fn drive_name(path: &Path) -> String {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => {
            prefix.as_os_str().to_string_lossy().into_owned()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_names_only_exist_on_windows_paths() {
        assert_eq!(drive_name(Path::new("/home/me/.minecraft")), "");
        assert_eq!(drive_name(Path::new("relative/path")), "");
        #[cfg(windows)]
        assert_eq!(drive_name(Path::new("C:\\mc")), "C:");
    }
}
