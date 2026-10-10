//! 杂项助手。

use std::path::Path;

/// `escapeShellCommand` —— 把 argv 渲染成给人看的命令行。
///
/// 交给 `shell_words::join`，它产生的 POSIX 单引号形式与原实现手写的一致。
pub fn escape_shell_command(args: &[String]) -> String {
    shell_words::join(args)
}

/// `driveName` —— 路径的 Windows 盘符前缀（`"C:"`），没有则返回 `""`。
///
/// 只用来回显 `cd <盘符>` 那一段；匹配 `Component::Prefix` 之后就不需要
/// 判断平台了。
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
    fn shell_escaping_quotes_only_when_needed() {
        assert_eq!(
            escape_shell_command(&["java".to_string(), "-Xmx2048m".to_string()]),
            "java -Xmx2048m"
        );
        assert_eq!(
            escape_shell_command(&["-Dfoo=bar baz".to_string()]),
            "'-Dfoo=bar baz'"
        );
    }

    #[test]
    fn drive_names_only_exist_on_windows_paths() {
        assert_eq!(drive_name(Path::new("/home/me/.minecraft")), "");
        assert_eq!(drive_name(Path::new("relative/path")), "");
        #[cfg(windows)]
        assert_eq!(drive_name(Path::new("C:\\mc")), "C:");
    }
}
