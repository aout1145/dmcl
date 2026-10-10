//! 平台相关的名字与约定。

/// `getOSName()` —— Minecraft 规则对象里使用的系统名。
///
/// 注意 macOS 必须叫 `osx`：这是游戏自己的规则文档里写死的名字，不是笔误。
pub fn get_os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

/// Java classpath 里的分隔符。
///
/// 这是 Java 自己的约定（Windows 用 `;`，其余用 `:`），不是操作系统的路径分隔符，
/// 所以 `std::path` 里没有对应物。
pub fn classpath_separator() -> char {
    if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_name_follows_the_game_rules() {
        // macOS 在规则文档里叫 osx
        assert!(matches!(
            get_os_name(),
            "windows" | "linux" | "osx" | "unknown"
        ));
        #[cfg(target_os = "macos")]
        assert_eq!(get_os_name(), "osx");
        #[cfg(target_os = "linux")]
        assert_eq!(get_os_name(), "linux");
        #[cfg(target_os = "windows")]
        assert_eq!(get_os_name(), "windows");
    }

    #[test]
    fn classpath_separator_matches_java_conventions() {
        if cfg!(target_os = "windows") {
            assert_eq!(classpath_separator(), ';');
        } else {
            assert_eq!(classpath_separator(), ':');
        }
    }
}
