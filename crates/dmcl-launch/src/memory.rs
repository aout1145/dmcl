//! 游戏堆内存的自动分配与解析。
//!
//! 配置里的 `min_memory` / `max_memory` 都可以为 `null`：`null` 表示交给启动器按系统
//! 可用内存和所选 Java 的架构决定。这里只做纯计算，系统内存由 [`system_available_mib`]
//! 读取。

use dmcl_mcmeta::os::Arch;

/// 1 GiB 的 MiB 数。
const GIB_MIB: u32 = 1024;

/// 32 位 Java 能可靠使用的堆上限（MiB）。
const X86_MAX_MIB: u32 = 1536;

/// 读不到系统可用内存时固定使用的堆上限（MiB）。
const UNKNOWN_AVAILABLE_MIB: u32 = 1536;

/// 当前系统可分配给新进程的内存（MiB）；读不到时返回 `None`。
pub fn system_available_mib() -> Option<u32> {
    let mut system = sysinfo::System::new().ok()?;
    system.refresh_memory();
    let mib = system.available_memory() / 1024 / 1024;
    (mib > 0).then(|| mib.min(u64::from(u32::MAX)) as u32)
}

/// 自动分配时的堆上限（MiB）：系统可用内存的 3/4，至少 1 GiB；32 位 Java 再压到 1.5 GiB。
/// 读不到系统可用内存时固定为 1.5 GiB。
pub fn auto_max_memory(java_arch: Option<Arch>) -> u32 {
    auto_max_memory_for(system_available_mib(), java_arch)
}

/// [`auto_max_memory`] 的纯计算版本，便于测试。
fn auto_max_memory_for(available_mib: Option<u32>, java_arch: Option<Arch>) -> u32 {
    let mut max = match available_mib {
        Some(available) => {
            let three_quarters = u64::from(available) * 3 / 4;
            u32::try_from(three_quarters)
                .unwrap_or(u32::MAX)
                .max(GIB_MIB)
        }
        None => UNKNOWN_AVAILABLE_MIB,
    };
    if java_arch == Some(Arch::X86) {
        max = max.min(X86_MAX_MIB);
    }
    max
}

/// 解析配置里的可空内存值，返回实际传给 JVM 的 `(Xms, Xmx)`（MiB）。
///
/// * `max_memory` 为 `None` 时用 [`auto_max_memory`]；32 位 Java 下显式上限也会压到 1.5 GiB。
/// * `min_memory` 为 `None` 时取最终的 `max_memory`。
/// * 显式 `min_memory` 大于最终上限时**原样保留**（由调用方决定是否告警）。
pub fn resolve_memory(
    min_memory: Option<u32>,
    max_memory: Option<u32>,
    java_arch: Option<Arch>,
) -> (u32, u32) {
    resolve_memory_for(min_memory, max_memory, system_available_mib(), java_arch)
}

/// [`resolve_memory`] 的纯计算版本，便于测试。
fn resolve_memory_for(
    min_memory: Option<u32>,
    max_memory: Option<u32>,
    available_mib: Option<u32>,
    java_arch: Option<Arch>,
) -> (u32, u32) {
    let mut max = max_memory.unwrap_or_else(|| auto_max_memory_for(available_mib, java_arch));
    if java_arch == Some(Arch::X86) {
        max = max.min(X86_MAX_MIB);
    }
    let min = min_memory.unwrap_or(max);
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_max_takes_three_quarters_with_a_one_gib_floor() {
        assert_eq!(auto_max_memory_for(Some(0), Some(Arch::X86_64)), GIB_MIB);
        assert_eq!(auto_max_memory_for(Some(1024), Some(Arch::X86_64)), GIB_MIB);
        assert_eq!(auto_max_memory_for(Some(8192), Some(Arch::X86_64)), 6144);
        assert_eq!(auto_max_memory_for(Some(10_000), None), 7500);
    }

    #[test]
    fn unknown_available_memory_falls_back_to_one_and_a_half_gib() {
        assert_eq!(auto_max_memory_for(None, Some(Arch::X86_64)), 1536);
        assert_eq!(auto_max_memory_for(None, None), 1536);
        assert_eq!(
            resolve_memory_for(None, None, None, Some(Arch::X86_64)),
            (1536, 1536)
        );
    }

    #[test]
    fn a_32_bit_java_caps_the_max_at_one_and_a_half_gib() {
        assert_eq!(
            auto_max_memory_for(Some(8192), Some(Arch::X86)),
            X86_MAX_MIB
        );
        assert_eq!(
            resolve_memory_for(Some(512), Some(4096), Some(8192), Some(Arch::X86)),
            (512, X86_MAX_MIB)
        );
        assert_eq!(
            resolve_memory_for(None, None, Some(8192), Some(Arch::X86)),
            (X86_MAX_MIB, X86_MAX_MIB)
        );
    }

    #[test]
    fn null_values_fall_back_to_the_auto_max() {
        assert_eq!(
            resolve_memory_for(None, None, Some(8192), Some(Arch::X86_64)),
            (6144, 6144)
        );
        assert_eq!(
            resolve_memory_for(None, Some(2048), Some(8192), Some(Arch::X86_64)),
            (2048, 2048)
        );
        assert_eq!(
            resolve_memory_for(Some(512), None, Some(8192), Some(Arch::X86_64)),
            (512, 6144)
        );
    }

    #[test]
    fn an_explicit_min_above_the_max_is_kept() {
        assert_eq!(
            resolve_memory_for(Some(4096), Some(2048), Some(8192), Some(Arch::X86_64)),
            (4096, 2048)
        );
    }
}
