//! 游戏堆内存的自动分配与解析。
//!
//! 配置里的 `min` / `max` 都可以为 `null`：`null` 表示交给启动器按系统可用内存和
//! 所选 Java 的架构决定。这里只做纯计算，系统内存由 [`system_available_mib`] 读取；
//! 生成启动参数只接受已经确定下来的 [`MemorySize`]。

use dmcl_mcmeta::os::Arch;

/// 1 GiB 的 MiB 数。
const GIB_MIB: u32 = 1024;

/// 32 位 Java 能可靠使用的堆上限（MiB）。
const X86_MAX_MIB: u32 = 1536;

/// 读不到系统可用内存时固定使用的堆上限（MiB）。
const UNKNOWN_AVAILABLE_MIB: u32 = 1536;

/// 用户的内存偏好（MiB）；`None` 表示自动。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryOptions {
    pub min: Option<u32>,
    pub max: Option<u32>,
}

/// 确定下来的堆大小（MiB），对应 JVM 的 `-Xms` / `-Xmx`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySize {
    pub min: u32,
    pub max: u32,
}

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

/// 把用户偏好解析成实际传给 JVM 的堆大小（MiB）。
///
/// * `max` 为 `None` 时用 [`auto_max_memory`]；32 位 Java 下显式上限也会压到 1.5 GiB。
/// * `min` 为 `None` 时取最终的 `max`。
/// * 显式 `min` 大于最终上限时**原样保留**（由调用方决定是否告警）。
pub fn resolve_memory(options: &MemoryOptions, java_arch: Option<Arch>) -> MemorySize {
    resolve_memory_for(options, system_available_mib(), java_arch)
}

/// [`resolve_memory`] 的纯计算版本，便于测试。
fn resolve_memory_for(
    options: &MemoryOptions,
    available_mib: Option<u32>,
    java_arch: Option<Arch>,
) -> MemorySize {
    let mut max = options
        .max
        .unwrap_or_else(|| auto_max_memory_for(available_mib, java_arch));
    if java_arch == Some(Arch::X86) {
        max = max.min(X86_MAX_MIB);
    }
    let min = options.min.unwrap_or(max);
    MemorySize { min, max }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(min: Option<u32>, max: Option<u32>) -> MemoryOptions {
        MemoryOptions { min, max }
    }

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
            resolve_memory_for(&options(None, None), None, Some(Arch::X86_64)),
            MemorySize {
                min: 1536,
                max: 1536
            }
        );
    }

    #[test]
    fn a_32_bit_java_caps_the_max_at_one_and_a_half_gib() {
        assert_eq!(
            auto_max_memory_for(Some(8192), Some(Arch::X86)),
            X86_MAX_MIB
        );
        assert_eq!(
            resolve_memory_for(&options(Some(512), Some(4096)), Some(8192), Some(Arch::X86)),
            MemorySize {
                min: 512,
                max: X86_MAX_MIB
            }
        );
        assert_eq!(
            resolve_memory_for(&options(None, None), Some(8192), Some(Arch::X86)),
            MemorySize {
                min: X86_MAX_MIB,
                max: X86_MAX_MIB
            }
        );
    }

    #[test]
    fn null_values_fall_back_to_the_auto_max() {
        assert_eq!(
            resolve_memory_for(&options(None, None), Some(8192), Some(Arch::X86_64)),
            MemorySize {
                min: 6144,
                max: 6144
            }
        );
        assert_eq!(
            resolve_memory_for(&options(None, Some(2048)), Some(8192), Some(Arch::X86_64)),
            MemorySize {
                min: 2048,
                max: 2048
            }
        );
        assert_eq!(
            resolve_memory_for(&options(Some(512), None), Some(8192), Some(Arch::X86_64)),
            MemorySize {
                min: 512,
                max: 6144
            }
        );
    }

    #[test]
    fn an_explicit_min_above_the_max_is_kept() {
        assert_eq!(
            resolve_memory_for(
                &options(Some(4096), Some(2048)),
                Some(8192),
                Some(Arch::X86_64)
            ),
            MemorySize {
                min: 4096,
                max: 2048
            }
        );
    }
}
