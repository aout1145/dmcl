//! 平台相关的名字与约定。

// 只支持这三个平台；其它平台直接编译失败，而不是运行时报错。
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
compile_error!("dmcl 目前只支持 Windows / macOS / Linux");

/// 操作系统。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    MacOs,
}

impl Os {
    /// 当前运行平台。
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }

    /// Minecraft 规则与 natives 分类器里用的名字（macOS 必须叫 `osx`）。
    pub fn mc_name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Linux => "linux",
            Os::MacOs => "osx",
        }
    }

    /// 从 java 的 `os.name` 属性（`Linux` / `Windows 10` / `Mac OS X` …）解析。
    pub fn from_java_name(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        if name.starts_with("windows") {
            Some(Os::Windows)
        } else if name.contains("mac") || name.contains("darwin") {
            Some(Os::MacOs)
        } else if name.contains("linux") {
            Some(Os::Linux)
        } else {
            None
        }
    }
}

/// CPU 架构。
///
/// 既要给 Minecraft natives 分类器的 `${arch}`（只有 32 / 64），也要给 natives
/// 目录名一个可读的架构名（`x86_64` 之类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86,
    X86_64,
    Aarch64,
}

impl Arch {
    /// Minecraft natives 分类器里 `${arch}` 的取值（`32` / `64`）。
    pub fn mc_bits(self) -> &'static str {
        match self {
            Arch::X86 => "32",
            Arch::X86_64 | Arch::Aarch64 => "64",
        }
    }

    /// 目录名 / 展示用的架构名。
    pub fn name(self) -> &'static str {
        match self {
            Arch::X86 => "x86",
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        }
    }

    /// 从 java 的 `os.arch` 属性解析；认不出来返回 `None`（调用者据此跳过这个 java，
    /// 而不是猜一个架构）。
    pub fn from_java_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "amd64" | "x86_64" | "x86-64" | "x64" => Some(Arch::X86_64),
            "aarch64" | "arm64" => Some(Arch::Aarch64),
            "x86" | "x86_32" | "i386" | "i486" | "i586" | "i686" => Some(Arch::X86),
            _ => None,
        }
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

/// 当前操作系统的版本号（用于匹配 `rules[].os.version` 里的正则）。
pub fn os_version() -> String {
    os_info::get().version().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_name_follows_the_game_rules() {
        // macOS 在规则文档里叫 osx
        let name = Os::current().mc_name();
        assert!(matches!(name, "windows" | "linux" | "osx"));
        #[cfg(target_os = "macos")]
        assert_eq!(name, "osx");
        #[cfg(target_os = "linux")]
        assert_eq!(name, "linux");
        #[cfg(target_os = "windows")]
        assert_eq!(name, "windows");
    }

    #[test]
    fn java_names_map_onto_the_platform_enums() {
        assert_eq!(Os::from_java_name("Linux"), Some(Os::Linux));
        assert_eq!(Os::from_java_name("Windows 10"), Some(Os::Windows));
        assert_eq!(Os::from_java_name("Mac OS X"), Some(Os::MacOs));
        assert_eq!(Os::from_java_name("SunOS"), None);

        assert_eq!(Arch::from_java_name("amd64"), Some(Arch::X86_64));
        assert_eq!(Arch::from_java_name("x86_64"), Some(Arch::X86_64));
        assert_eq!(Arch::from_java_name("aarch64"), Some(Arch::Aarch64));
        assert_eq!(Arch::from_java_name("arm64"), Some(Arch::Aarch64));
        assert_eq!(Arch::from_java_name("i386"), Some(Arch::X86));
        assert_eq!(Arch::from_java_name("sparc"), None);
    }

    #[test]
    fn arch_bits_and_names_match_minecraft() {
        assert_eq!(Arch::X86.mc_bits(), "32");
        assert_eq!(Arch::X86_64.mc_bits(), "64");
        assert_eq!(Arch::Aarch64.mc_bits(), "64");
        assert_eq!(Arch::X86_64.name(), "x86_64");
        assert_eq!(Arch::Aarch64.name(), "aarch64");
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
