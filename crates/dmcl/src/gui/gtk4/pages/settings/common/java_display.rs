//! Java 探测结果的展示文案。
//!
//! Java 管理页和游戏设置页共用同一套标题 / 摘要格式，避免两处显示不一致。

use dmcl_launch::JavaDetails;

/// 标题：JDK / JRE + 完整版本。
pub(crate) fn details_title(details: &JavaDetails) -> String {
    format!(
        "{} {}",
        if details.is_jdk { "JDK" } else { "JRE" },
        details.version_str
    )
}

/// 右侧小字：供应商与架构；没有供应商时只显示架构。
pub(crate) fn details_arch_text(details: &JavaDetails) -> String {
    let arch = details.option.arch.name();
    if details.vendor.is_empty() {
        arch.to_string()
    } else {
        format!("{}, {arch}", details.vendor)
    }
}

/// 一行摘要：标题、供应商、架构。
pub(crate) fn java_summary(details: &JavaDetails) -> String {
    format!("{}, {}", details_title(details), details_arch_text(details))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dmcl_launch::{JavaDetails, JavaOption};
    use dmcl_mcmeta::os::Arch;

    use super::{details_title, java_summary};

    fn details(version: &str, vendor: &str, is_jdk: bool) -> JavaDetails {
        JavaDetails {
            option: JavaOption {
                path: PathBuf::from("java"),
                major: 8,
                arch: Arch::X86_64,
            },
            type_: "openjdk".to_string(),
            version_str: version.to_string(),
            vendor: vendor.to_string(),
            is_jdk,
        }
    }

    #[test]
    fn summaries_show_kind_version_vendor_and_arch() {
        let jdk = details("1.8.0_83", "Microsoft", true);
        assert_eq!(details_title(&jdk), "JDK 1.8.0_83");
        assert_eq!(java_summary(&jdk), "JDK 1.8.0_83, Microsoft, x86_64");

        let jre = details("1.8.0_83", "", false);
        assert_eq!(details_title(&jre), "JRE 1.8.0_83");
        assert_eq!(java_summary(&jre), "JRE 1.8.0_83, x86_64");
    }
}
