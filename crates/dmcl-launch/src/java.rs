//! Java 探测与选择。
//!
//! **这里不碰配置**：配置表由调用者传入 / 接收（见 [`JavaConfigs`]）。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use dmcl_mcmeta::os::{Arch, Os};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use walkdir::WalkDir;

/// java 配置表：名字 → 可执行文件。
pub type JavaConfigs = BTreeMap<String, PathBuf>;

/// 表示「自动选择」的哨兵字符串。
pub const JAVA_AUTOSELECT: &str = "autoselect";

/// 默认扫描的目录（仅 posix）。
pub const DEFAULT_JAVA_ROOTS: &[&str] = &["/lib/jvm/"];

/// `launch.java` —— 要么自动选择，要么是 [`JavaConfigs`] 里的一个键。
///
/// 序列化成普通字符串（`"autoselect"` 或配置项名字），因此配置文件里直接写即可。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum JavaChoice {
    /// 写成 `"autoselect"`。
    #[default]
    Auto,
    /// [`JavaConfigs`] 里某一项的名字。
    Named(String),
}

impl JavaChoice {
    pub fn as_str(&self) -> &str {
        match self {
            JavaChoice::Auto => JAVA_AUTOSELECT,
            JavaChoice::Named(name) => name,
        }
    }

    /// 是否自动选择。
    pub fn is_auto(&self) -> bool {
        matches!(self, JavaChoice::Auto)
    }
}

impl From<&str> for JavaChoice {
    fn from(raw: &str) -> Self {
        if raw == JAVA_AUTOSELECT {
            JavaChoice::Auto
        } else {
            JavaChoice::Named(raw.to_string())
        }
    }
}

impl fmt::Display for JavaChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for JavaChoice {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for JavaChoice {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(JavaChoice::from(
            String::deserialize(deserializer)?.as_str(),
        ))
    }
}

/// 一个探测过的 java 可执行文件。
#[derive(Debug, Clone)]
pub struct JavaOption {
    /// java 可执行文件。
    pub path: PathBuf,
    /// 厂商 / 类型，`java -version` 输出的第一个词（例如 `openjdk`）。
    pub type_: String,
    /// JVM 所在的操作系统（`os.name`）。
    pub os: Os,
    /// JVM 架构（`os.arch`）。
    pub arch: Arch,
    /// 大版本号（`1.8` ⇒ 8，`17.0.2` ⇒ 17）—— 不可能为负。
    pub major_version: u32,
    /// 完整版本字符串（`17.0.2`）。
    pub version_str: String,
}

impl JavaOption {
    /// 探测一个 java 可执行文件。
    ///
    /// 用 `-XshowSettings:properties -version`：一次调用既能拿到版本横幅，又能拿到
    /// `os.name` / `os.arch`（`java -version` 给不出 `linux` / `x86_64` 这种平台信息）。
    pub async fn probe(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let output = tokio::process::Command::new(&path)
            .arg("-XshowSettings:properties")
            .arg("-version")
            .output()
            .await
            .with_context(|| {
                format!(
                    "无法执行 {} -XshowSettings:properties -version",
                    path.display()
                )
            })?;

        // 属性与版本横幅通常都打在 stderr，有的实现写在 stdout，两个都收着
        let mut raw = String::from_utf8_lossy(&output.stderr).into_owned();
        raw.push_str(&String::from_utf8_lossy(&output.stdout));

        let property = |key: &str| {
            raw.lines()
                .find_map(|line| line.trim().strip_prefix(key))
                .unwrap_or_default()
                .trim()
                .to_string()
        };

        // 版本横幅那一行：不缩进、且带着 "version"（属性行都缩进，且首行是 "Property settings:"）
        let banner = raw
            .lines()
            .find(|line| {
                !line.starts_with(' ') && !line.starts_with('\t') && line.contains("version")
            })
            .unwrap_or("")
            .replace('"', "");

        let tokens: Vec<&str> = banner.split(' ').collect();
        let type_ = tokens.first().copied().unwrap_or("").to_string();
        let version_str = tokens.get(2).copied().unwrap_or("").to_string();
        let versions: Vec<&str> = version_str.split('.').collect();
        let major_version = if versions.first().copied().unwrap_or("") != "1" {
            versions
                .first()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0)
        } else {
            versions
                .get(1)
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0)
        };

        // 架构必须能从 `os.arch` 认出来：认不出就报错，让调用者跳过这个 java，
        // 而不是猜一个（natives 的 classifier 全押在它上面）
        let os_arch = property("os.arch = ");
        let arch = Arch::from_java_name(&os_arch).ok_or_else(|| {
            anyhow!(
                "无法确定 {} 的架构（os.arch = {os_arch:?}）",
                path.display()
            )
        })?;
        let os = Os::from_java_name(&property("os.name = ")).unwrap_or_else(Os::current);

        Ok(Self {
            path,
            type_,
            os,
            arch,
            major_version,
            version_str,
        })
    }

    /// 探测是否得到了可用的版本号。
    pub fn is_usable(&self) -> bool {
        self.major_version > 0
    }

    /// 探测结果对应的配置项名字（`{type}-{os}-{arch}-{version}`，如 `openjdk-linux-x86_64-17.0.2`）。
    pub fn config_name(&self) -> String {
        format!(
            "{}-{}-{}-{}",
            self.type_,
            self.os.mc_name(),
            self.arch.name(),
            self.version_str
        )
    }
}

/// 扫描目录里的 `java` 可执行文件。
///
/// **只返回结果，不写配置**；Windows 上暂不支持自动搜索。
pub async fn find_java(roots: &[impl AsRef<Path>]) -> Result<JavaConfigs> {
    if cfg!(target_os = "windows") {
        return Err(anyhow!("Windows 上暂不支持自动搜索 java"));
    }
    if !cfg!(unix) {
        return Err(anyhow!("当前平台不支持自动搜索 java"));
    }

    let mut configs: JavaConfigs = BTreeMap::new();
    for root in roots {
        for entry in WalkDir::new(root.as_ref())
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() || entry.file_name() != "java" {
                continue;
            }
            // 跳过探测失败或版本不可用的文件：一个野文件不该让整次搜索失败
            match JavaOption::probe(entry.path().to_path_buf()).await {
                Ok(java) if java.is_usable() => {
                    configs.insert(java.config_name(), java.path.clone());
                }
                _ => continue,
            }
        }
    }
    Ok(configs)
}

/// 按调用者给的选择挑一个 java。
///
/// * [`JavaChoice::Named`]：直接用配置表里的那一项；
/// * [`JavaChoice::Auto`]：找与 `required_major` 最接近的（优先完全相等）。
///
/// 配置表为空且是自动选择时，会现场扫一遍 [`DEFAULT_JAVA_ROOTS`]（**不写回配置**）。
pub async fn select_java(
    required_major: u32,
    choice: &JavaChoice,
    configs: &JavaConfigs,
) -> Result<JavaOption> {
    let scanned = if choice.is_auto() && configs.is_empty() {
        Some(find_java(DEFAULT_JAVA_ROOTS).await?)
    } else {
        None
    };
    let configs = scanned.as_ref().unwrap_or(configs);
    if configs.is_empty() {
        return Err(anyhow!("没有任何可用的 java 配置，请先用 dmcl java 搜索"));
    }

    let name = match choice {
        JavaChoice::Named(name) => name.clone(),
        JavaChoice::Auto => auto_select_java(required_major, configs).await?,
    };

    let path = configs
        .get(&name)
        .cloned()
        .ok_or_else(|| anyhow!("没有这个 java 配置: {name}"))?;
    let java = JavaOption::probe(path).await?;
    Ok(java)
}

/// `autoselect` 的搜索规则：完全相等优先，否则"至少满足要求里最接近的"，
/// 再否则取配置表的第一项。
async fn auto_select_java(required_major: u32, configs: &JavaConfigs) -> Result<String> {
    let mut best: Option<String> = None;
    let mut best_diff = u32::MAX;
    for (name, path) in configs {
        let java = JavaOption::probe(path.clone()).await?;
        if java.major_version == required_major {
            return Ok(name.clone());
        }
        // `major_version` 是无符号数，这里不需要 `abs()`
        let diff = java.major_version.saturating_sub(required_major);
        if java.major_version >= required_major && diff < best_diff {
            best_diff = diff;
            best = Some(name.clone());
        }
    }
    Ok(best.unwrap_or_else(|| configs.keys().next().cloned().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_choice_round_trips_as_a_plain_string() {
        assert_eq!(
            serde_json::to_string(&JavaChoice::Auto).unwrap(),
            "\"autoselect\""
        );
        assert_eq!(
            serde_json::to_string(&JavaChoice::Named("jdk17".to_string())).unwrap(),
            "\"jdk17\""
        );
        assert_eq!(JavaChoice::from("autoselect"), JavaChoice::Auto);
        assert_eq!(
            JavaChoice::from("jdk17"),
            JavaChoice::Named("jdk17".to_string())
        );
        assert_eq!(JavaChoice::Auto.as_str(), JAVA_AUTOSELECT);
        assert!(JavaChoice::Auto.is_auto());
        assert_eq!(JavaChoice::Named("x".into()).to_string(), "x");
    }

    #[test]
    fn config_name_uses_os_and_arch_not_bits() {
        let java = JavaOption {
            path: PathBuf::from("/opt/jdk/bin/java"),
            type_: "openjdk".to_string(),
            os: Os::Linux,
            arch: Arch::Aarch64,
            major_version: 17,
            version_str: "17.0.2".to_string(),
        };
        assert_eq!(java.config_name(), "openjdk-linux-aarch64-17.0.2");
    }

    #[tokio::test]
    async fn selecting_without_any_java_is_an_error() {
        let configs = JavaConfigs::new();
        let err = select_java(17, &JavaChoice::Named("nope".into()), &configs)
            .await
            .unwrap_err();
        // 有配置表时不会去扫描，直接报"没有这一项"
        assert!(format!("{err:#}").contains("没有任何可用的 java 配置"));
    }

    #[tokio::test]
    async fn a_named_choice_must_exist() {
        let mut configs = JavaConfigs::new();
        configs.insert("jdk".to_string(), PathBuf::from("/nonexistent/java"));
        let err = select_java(17, &JavaChoice::Named("other".into()), &configs)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("没有这个 java 配置"));
        // 存在的名字会真的去执行 → 报"无法执行"
        let err = select_java(17, &JavaChoice::Named("jdk".into()), &configs)
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("无法执行"));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn the_system_java_can_be_probed() {
        let Ok(java) = JavaOption::probe("/usr/bin/java").await else {
            return; // 这台机器没有 java 就跳过
        };
        if java.is_usable() {
            assert!(matches!(
                java.arch,
                Arch::X86 | Arch::X86_64 | Arch::Aarch64
            ));
            assert!(matches!(java.os, Os::Windows | Os::Linux | Os::MacOs));
            assert!(!java.config_name().is_empty());
        }
    }
}
