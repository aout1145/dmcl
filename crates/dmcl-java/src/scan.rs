//! 扫描本机 Java，并从候选里自动选择。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dmcl_mcmeta::os::Arch;
use walkdir::WalkDir;

use crate::option::{JavaDetails, JavaOption};

/// 扫描目录里的 `java` 可执行文件，返回探测成功的详细信息。
///
/// **只返回结果，不写配置**；扫不到或全部探测失败时返回空列表。
pub async fn find_java(roots: &[impl AsRef<Path>]) -> Vec<JavaDetails> {
    let mut roots_to_scan: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for root in roots {
        let root = root.as_ref();
        let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        roots_to_scan
            .entry(key)
            .or_insert_with(|| root.to_path_buf());
    }

    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    let mut candidates: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for root in roots_to_scan.values() {
        for entry in WalkDir::new(root)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if entry.file_name() != executable || !entry.path().is_file() {
                continue;
            }
            let canonical = entry
                .path()
                .canonicalize()
                .unwrap_or_else(|_| entry.path().to_path_buf());
            candidates
                .entry(canonical)
                .or_insert_with(|| entry.path().to_path_buf());
        }
    }

    let mut found = Vec::new();
    for path in candidates.values() {
        // 跳过探测失败或版本不可用的文件：一个野文件不该让整次搜索失败
        let Ok(java) = JavaDetails::probe(path.clone()).await else {
            continue;
        };
        if java.is_usable() {
            found.push(java);
        }
    }
    found
}

/// 从候选里挑最合适的 Java。
///
/// 只读缓存的 `major` / `arch`，不执行任何 Java。优先级：
/// 1. 大版本完全相等；
/// 2. 高于要求且最接近的；
/// 3. 低于要求且最高的；
///    同一档内优先当前平台架构，再按候选顺序。
pub fn auto_select_java(required_major: u32, options: &[JavaOption]) -> Option<JavaOption> {
    let host_arch = Arch::current();
    options
        .iter()
        .min_by_key(|java| {
            let (tier, distance) = match java.major.cmp(&required_major) {
                std::cmp::Ordering::Equal => (0, 0),
                std::cmp::Ordering::Greater => (1, java.major - required_major),
                std::cmp::Ordering::Less => (2, required_major - java.major),
            };
            let arch_mismatch = host_arch.is_some_and(|host| java.arch != host);
            (tier, distance, arch_mismatch)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn java(path: &str, major: u32) -> JavaOption {
        java_with_arch(path, major, Arch::X86_64)
    }

    fn java_with_arch(path: &str, major: u32, arch: Arch) -> JavaOption {
        JavaOption {
            path: PathBuf::from(path),
            major,
            arch,
        }
    }

    fn other_arch(host: Arch) -> Arch {
        match host {
            Arch::X86_64 => Arch::Aarch64,
            Arch::Aarch64 | Arch::X86 => Arch::X86_64,
        }
    }

    #[test]
    fn auto_selection_uses_cached_versions_without_running_java() {
        // 路径都不存在；能选中说明选择过程完全没有执行 java
        let options = vec![java("/jdk8", 8), java("/jdk17", 17), java("/jdk21", 21)];
        assert_eq!(auto_select_java(17, &options), Some(java("/jdk17", 17)));
        assert_eq!(auto_select_java(11, &options), Some(java("/jdk17", 17)));
        assert_eq!(auto_select_java(30, &options), Some(java("/jdk21", 21)));
        assert_eq!(auto_select_java(17, &[]), None);

        // 没有满足要求的版本时取最高的，不依赖配置顺序
        let lower = [java("/jdk8", 8), java("/jdk11", 11)];
        assert_eq!(auto_select_java(17, &lower), Some(java("/jdk11", 11)));
        let lower = [java("/jdk11", 11), java("/jdk8", 8)];
        assert_eq!(auto_select_java(17, &lower), Some(java("/jdk11", 11)));
    }

    #[test]
    fn auto_selection_prefers_the_current_architecture_within_a_version_tier() {
        let Some(host) = Arch::current() else {
            return;
        };
        let other = other_arch(host);
        let options = [
            java_with_arch("/other", 17, other),
            java_with_arch("/host", 17, host),
        ];
        assert_eq!(
            auto_select_java(17, &options),
            Some(java_with_arch("/host", 17, host))
        );
        assert_eq!(
            auto_select_java(11, &options),
            Some(java_with_arch("/host", 17, host))
        );
        assert_eq!(
            auto_select_java(21, &options),
            Some(java_with_arch("/host", 17, host))
        );
    }

    #[test]
    fn auto_selection_prefers_the_version_tier_over_the_architecture() {
        let Some(host) = Arch::current() else {
            return;
        };
        let other = other_arch(host);

        // 精确匹配的异架构版本优先于更高版本的当前架构版本
        let exact_other = [
            java_with_arch("/host-21", 21, host),
            java_with_arch("/other-17", 17, other),
        ];
        assert_eq!(
            auto_select_java(17, &exact_other),
            Some(java_with_arch("/other-17", 17, other))
        );

        // 高的兜底版本优先于低的当前架构版本
        let higher_other = [
            java_with_arch("/host-8", 8, host),
            java_with_arch("/other-11", 11, other),
        ];
        assert_eq!(
            auto_select_java(17, &higher_other),
            Some(java_with_arch("/other-11", 11, other))
        );
    }

    #[tokio::test]
    async fn find_java_returns_an_empty_list_when_nothing_is_found() {
        let dir = std::env::temp_dir().join(format!("dmcl-java-empty-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let found = find_java(&[dir.as_path()]).await;
        assert!(found.is_empty());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn find_java_accepts_a_symlinked_executable() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = std::env::temp_dir().join(format!("dmcl-java-symlink-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();

        let real = root.join("fake-java");
        tokio::fs::write(
            &real,
            "#!/bin/sh\ncat <<'EOF'\n\
             Property settings:\n\
             java.version = 17.0.8\n\
             os.arch = amd64\n\
             os.name = Linux\n\
             openjdk version \"17.0.8\" 2023-07-18\n\
             EOF\n",
        )
        .await
        .unwrap();
        let mut permissions = tokio::fs::metadata(&real).await.unwrap().permissions();
        permissions.set_mode(0o755);
        tokio::fs::set_permissions(&real, permissions)
            .await
            .unwrap();
        symlink(&real, root.join("java")).unwrap();

        let found = find_java(&[root.as_path()]).await;
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].option.path,
            root.join("java"),
            "符号链接本身应作为配置路径"
        );
        assert_eq!(found[0].type_, "openjdk");
        assert_eq!(found[0].option.major, 17);
        assert_eq!(found[0].option.arch, Arch::X86_64);

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }
}
