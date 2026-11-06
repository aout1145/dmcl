//! Java 运行时信息与探测结果。

use std::path::PathBuf;

use anyhow::Result;
use dmcl_mcmeta::os::Arch;

/// 选择 Java 所需的最小信息：路径、大版本、架构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaOption {
    /// java 可执行文件。
    pub path: PathBuf,
    /// 大版本号（`1.8` ⇒ 8，`17.0.2` ⇒ 17）。
    pub major: u32,
    /// JVM 架构（`os.arch`）。
    pub arch: Arch,
}

/// 探测得到的 Java 详细信息。
#[derive(Debug, Clone)]
pub struct JavaDetails {
    /// 选择 Java 所需的信息。
    pub option: JavaOption,
    /// 厂商 / 类型，`java -version` 输出的第一个词（例如 `openjdk`）。
    pub type_: String,
    /// 完整版本字符串（`17.0.2`）。
    pub version_str: String,
    /// `java.vendor` 属性（例如 `Microsoft`、`Eclipse Adoptium`）。
    pub vendor: String,
    /// 可执行文件旁边是否有 `javac`：有就是 JDK，没有就是 JRE。
    pub is_jdk: bool,
}

impl JavaDetails {
    /// 探测一个 java 可执行文件。
    ///
    /// 用 `-XshowSettings:properties -version`：一次调用既能拿到版本横幅，又能拿到
    /// `os.arch`（`java -version` 给不出 `x86_64` 这种信息）。
    pub async fn probe(path: impl Into<PathBuf>) -> Result<Self> {
        crate::probe::probe(path.into()).await
    }

    /// 探测是否得到了可用的版本号。
    pub fn is_usable(&self) -> bool {
        self.option.major > 0
    }
}
