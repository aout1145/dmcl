//! 规则表：特征串 → 结论与建议。
//!
//! 表里每一行就是一条规则。固定结论用 [`Message::Fixed`]，需要从日志里取 mod 名、版本号
//! 的用 [`Message::Handler`]；处理函数返回 `None` 表示这条规则不成立。规则按 [`Tier`]
//! 分层，`lib.rs` 只取最靠前、非空的那一层。

use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::{push, Finding};

/// 一条结论里最多列出几个名字。
const MAX_NAMES: usize = 2;
/// 单个片段最多保留的字符数，避免异常长的行撑爆提示。
const MAX_NAME_CHARS: usize = 48;

/// 规则优先级。加载器问题命中后不再看运行时错误，避免把衍生报错当主因。
#[derive(Clone, Copy)]
pub(crate) enum Tier {
    Loader = 0,
    Runtime = 1,
    Generic = 2,
}

/// 规则的结论：固定文案，或从捕获组 / 日志里提取。
#[derive(Clone, Copy)]
enum Message {
    Fixed {
        cause: &'static str,
        advice: &'static str,
    },
    Handler(fn(&Captures, &str) -> Option<Finding>),
}

impl Message {
    fn finding(self, caps: &Captures, text: &str) -> Option<Finding> {
        match self {
            Self::Fixed { cause, advice } => Some(Finding::new(cause, advice)),
            Self::Handler(handler) => handler(caps, text),
        }
    }
}

const fn fixed(cause: &'static str, advice: &'static str) -> Message {
    Message::Fixed { cause, advice }
}

const fn handler(handler: fn(&Captures, &str) -> Option<Finding>) -> Message {
    Message::Handler(handler)
}

struct Rule {
    tier: Tier,
    pattern: &'static str,
    message: Message,
}

impl Rule {
    const fn new(tier: Tier, pattern: &'static str, message: Message) -> Self {
        Self {
            tier,
            pattern,
            message,
        }
    }
}

/// 规则表。同一层里越靠前越具体。
const RULES: &[Rule] = &[
    // ---------- 加载器 ----------
    Rule::new(
        Tier::Loader,
        r"Missing or unsupported mandatory dependencies|Mod resolution failed|Mod resolution encountered|Could not find required mod|Found conflicting mods",
        handler(missing_dependency),
    ),
    Rule::new(
        Tier::Loader,
        r"Found a duplicate mod|Found duplicate mods:|DuplicateModsFoundException|ModResolutionException: Duplicate",
        handler(duplicate_mod),
    ),
    Rule::new(
        Tier::Loader,
        r"Incompatible mods found!|Some of your mods are incompatible",
        handler(incompatible_mods),
    ),
    Rule::new(
        Tier::Loader,
        r"Mixin apply failed|Mixin apply for mod|Mixin prepare failed|Mixin prepare for mod|MixinApplyError|MixinTransformerError|mixin\.injection\.throwables\.",
        handler(mixin_failure),
    ),
    Rule::new(
        Tier::Loader,
        r"(?i)LoaderExceptionModCrash: Caught exception from|Failed to create mod instance\. ModId|Could not execute entrypoint stage",
        handler(mod_crash),
    ),
    Rule::new(
        Tier::Loader,
        r"Failed loading config file ([^\r\n]+?) of type [^\r\n]+? for modid ([^\r\n]+)",
        handler(mod_config),
    ),
    Rule::new(
        Tier::Loader,
        r"Multiple entries with same key: ([^=\r\n]+)",
        handler(multiple_entries),
    ),
    Rule::new(
        Tier::Loader,
        "maximum id range exceeded",
        fixed(
            "mod 数量超出 ID 上限",
            "减少 mod 数量，尤其是 1.12.2 的整合包。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        r"(?i)The Mod File [^\r\n]*OptiFine[^\r\n]* has mods that were not found",
        fixed(
            "OptiFine 与 Forge 不兼容",
            "确认 OptiFine 版本与 Forge 对应，或改用其它优化 mod。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        "Invalid Services found OptiFine",
        fixed(
            "OptiFine 与 NeoForge 不兼容",
            "移除 OptiFine，改用 NeoForge 的优化 mod。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        r"The directories below appear to be extracted jar files|Extracted mod jars found",
        fixed("mod 文件被解压", "把解压出来的目录删掉，只保留 .jar 文件。"),
    ),
    Rule::new(
        Tier::Loader,
        "ClassNotFoundException: org.spongepowered.asm.launch.MixinTweaker",
        fixed(
            "缺少 MixinBootstrap",
            "安装 MixinBootstrap 后重试，1.12.2 的旧整合包常见。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        "Invalid module name: '' is not a Java identifier",
        fixed(
            "mod 文件名包含特殊字符",
            "把 mod 文件名改成只含英文字母、数字、-、_ 和 .，再启动游戏。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        "com.electronwill.nightconfig.core.io.ParsingException: Not enough data available",
        fixed(
            "mod 配置文件损坏（NightConfig）",
            "删除 config 下对应的 .toml 文件，让 mod 重新生成。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        "Shaders Mod detected. Please remove it",
        fixed(
            "Shaders Mod 与 OptiFine 冲突",
            "删除 Shaders Mod，OptiFine 已内置光影支持。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        r"Cannot find launch target fmlclient|Invalid paths argument, contained no existing paths|Could not find net/minecraft/client/Minecraft\.class in classloader SecureModuleClassLoader",
        fixed("Forge 安装不完整", "重新安装该版本的 Forge / NeoForge。"),
    ),
    Rule::new(
        Tier::Loader,
        "MultipleArgumentsForOptionException: Found multiple arguments for option",
        fixed(
            "版本里重复安装了 Forge",
            "删除并重新安装该版本，只保留一个 Forge。",
        ),
    ),
    Rule::new(
        Tier::Loader,
        "Module optifine reads another module named optifine",
        fixed(
            "OptiFine 重复安装",
            "删除 mods 里多余的 OptiFine，只保留一个。",
        ),
    ),
    // ---------- 运行时与环境 ----------
    Rule::new(
        Tier::Runtime,
        r"java\.lang\.OutOfMemoryError(?:: ([^\r\n]+))?|insufficient memory for the Java Runtime Environment|out of physical RAM or swap space",
        handler(out_of_memory),
    ),
    Rule::new(
        Tier::Runtime,
        r"UnsupportedClassVersionError|Unsupported major\.minor version",
        handler(java_too_old),
    ),
    Rule::new(
        Tier::Runtime,
        r"Unable to make protected final java\.lang\.Class java\.lang\.ClassLoader\.defineClass|NoSuchFieldException: ucp|module java\.base does not export|NashornScriptEngineFactory|Unsupported class file major version|java\.lang\.ClassCastException: java\.base/jdk",
        fixed(
            "Java 版本过高",
            "老版本 Forge / 加载器不支持当前 Java；换回该版本要求的 Java，或更新加载器。",
        ),
    ),
    Rule::new(
        Tier::Runtime,
        r"Could not reserve enough space for|Invalid maximum heap size|Invalid initial heap size|The specified size exceeds the maximum representable size",
        fixed(
            "最大内存设置过大，或 Java 是 32 位",
            "在实例设置里调小最大内存；32 位 Java 最多只能用约 1.5 GB。",
        ),
    ),
    Rule::new(
        Tier::Runtime,
        r"Pixel format not accelerated|GLXBadFBConfig|The driver does not appear to support OpenGL|Failed to create window|GLFW error",
        fixed(
            "显卡驱动或 OpenGL 不可用",
            "更新显卡驱动；远程桌面、虚拟机里通常无法启动，关闭光影再试。",
        ),
    ),
    Rule::new(
        Tier::Runtime,
        r"1282: Invalid operation|Maybe try a lower resolution resourcepack",
        fixed(
            "光影或资源包导致 OpenGL 错误",
            "关闭光影，或降低资源包分辨率后再试。",
        ),
    ),
    Rule::new(
        Tier::Runtime,
        r"EXCEPTION_ACCESS_VIOLATION|A fatal error has been detected by the Java Runtime Environment|SIGSEGV",
        handler(native_crash),
    ),
    Rule::new(
        Tier::Runtime,
        r"UnsatisfiedLinkError: (?:Failed to locate library: )?([^\r\n]+)",
        handler(missing_library),
    ),
    Rule::new(
        Tier::Runtime,
        r"FileAlreadyExistsException: ([^\r\n]+)",
        handler(file_exists),
    ),
    Rule::new(
        Tier::Runtime,
        r"SHA1 digest error for ([^\r\n]+)|signer information does not match",
        handler(file_changed),
    ),
    Rule::new(
        Tier::Runtime,
        r"AccessDeniedException|Permission denied|Access is denied|拒绝访问",
        fixed(
            "文件访问被拒绝",
            "确认游戏目录可写，并把启动器与 Java 加入安全软件白名单。",
        ),
    ),
    Rule::new(
        Tier::Runtime,
        r"ConnectException|UnknownHostException|SocketTimeoutException",
        fixed(
            "网络连接失败",
            "检查网络和代理设置；联机与正版验证需要能访问验证服务器。",
        ),
    ),
    Rule::new(
        Tier::Runtime,
        r"Unrecognized option:|Unrecognized VM option",
        fixed(
            "Java 虚拟机参数有误",
            "检查实例或全局设置里的 JVM 参数，去掉无法识别的选项。",
        ),
    ),
    Rule::new(
        Tier::Runtime,
        r"Open ?J9 is (?:not supported|incompatible)|\.J9VMInternals\.",
        fixed(
            "游戏不支持 OpenJ9",
            "换用 HotSpot 的 Java，例如 Temurin、Zulu。",
        ),
    ),
    // ---------- 泛化 ----------
    Rule::new(
        Tier::Generic,
        r"NoClassDefFoundError|ClassNotFoundException|NoSuchMethodError|NoSuchFieldError|IncompatibleClassChangeError|AbstractMethodError|IllegalAccessError",
        fixed(
            "mod 或游戏文件版本不匹配",
            "检查 mod 是否缺少前置、版本是否与游戏对应；必要时校验游戏文件完整性。",
        ),
    ),
    Rule::new(
        Tier::Generic,
        r"Entity Type: ([^\r\n(]+)",
        handler(entity_crash),
    ),
    Rule::new(
        Tier::Generic,
        r"Block: Block\{([^}]+)\}",
        handler(block_crash),
    ),
    Rule::new(Tier::Generic, r"Suspected Mods:", handler(suspected_mods)),
    Rule::new(
        Tier::Generic,
        "An exception was thrown, the game will display an error screen and halt.",
        fixed(
            "Forge 在加载阶段报错",
            "查看日志里紧随其后的异常；通常是 mod 或前置版本不匹配。",
        ),
    ),
    Rule::new(
        Tier::Generic,
        "Manually triggered debug crash",
        fixed("手动触发的调试崩溃", "不是真实故障，忽略即可。"),
    ),
];

static COMPILED: LazyLock<Vec<(Tier, Regex, Message)>> = LazyLock::new(|| {
    RULES
        .iter()
        .map(|rule| {
            (
                rule.tier,
                Regex::new(rule.pattern).expect("内置正则必须合法"),
                rule.message,
            )
        })
        .collect()
});

/// 按层收集结论；同一层内相同结论只保留第一次出现的那条。
pub(crate) fn analyze_tiers(text: &str) -> [Vec<Finding>; 3] {
    let mut tiers: [Vec<Finding>; 3] = std::array::from_fn(|_| Vec::new());
    for (tier, regex, message) in COMPILED.iter() {
        let Some(caps) = regex.captures(text) else {
            continue;
        };
        if let Some(finding) = message.finding(&caps, text) {
            push(&mut tiers[*tier as usize], finding);
        }
    }
    tiers
}

// ---------- 需要提取信息的规则 ----------

fn missing_dependency(_: &Captures, text: &str) -> Option<Finding> {
    if let Some(finding) = conflicting_mods(text) {
        return Some(finding);
    }
    Some(
        forge_missing_dependency(text)
            .or_else(|| fabric_missing_dependency(text))
            .unwrap_or_else(|| {
                Finding::new(
                    "mod 依赖解析失败",
                    "加载器未能解析 mod 依赖，按日志里的提示调整 mod。",
                )
            }),
    )
}

/// Fabric 的 `Found conflicting mods: A conflicts with B`。
fn conflicting_mods(text: &str) -> Option<Finding> {
    static RE_CONFLICT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"Found conflicting mods: ([^\r\n]+?) conflicts with ([^\r\n]+)").unwrap()
    });

    let caps = RE_CONFLICT.captures(text)?;
    Some(Finding::new(
        format!(
            "mod {} 与 {} 冲突",
            truncate(caps[1].trim()),
            truncate(caps[2].trim())
        ),
        "移除或替换其中一个 mod。",
    ))
}

fn forge_missing_dependency(text: &str) -> Option<Finding> {
    static RE_MOD_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Mod ID: '([^']+)'").unwrap());

    let block = indented_block(text, "Missing or unsupported mandatory dependencies:")?;
    let names = names_in(&RE_MOD_ID, block);
    let cause = match format_names(&names) {
        Some(names) => format!("缺少前置 mod：{names}"),
        None => "缺少前置 mod 或版本不匹配".to_string(),
    };
    Some(Finding::new(
        cause,
        "安装缺失的前置 mod，并确认版本符合加载器的要求。",
    ))
}

fn fabric_missing_dependency(text: &str) -> Option<Finding> {
    static RE_MISSING: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"Mod '([^']+)' \([^)]*\)[^\r\n]*?requires[^\r\n]*? of '([^']+)' \([^)]*\)[^\r\n]*?which is missing",
        )
        .unwrap()
    });
    static RE_REQUIRED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"Could not find required mod: ([^\r\n]+?) requires ([^\r\n]+)").unwrap()
    });
    static RE_COLLECTION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"Could not resolve valid mod collection \(at: ([^\r\n]+?) requires ([^\r\n]+?)\)",
        )
        .unwrap()
    });

    let resolution_failed = [
        "Mod resolution failed",
        "Mod resolution encountered",
        "Could not find required mod",
        "Could not resolve valid mod collection",
    ]
    .iter()
    .any(|marker| text.contains(marker));
    if !resolution_failed {
        return None;
    }

    let mut missing = Vec::new();
    let mut requester = None;
    for regex in [&*RE_MISSING, &*RE_REQUIRED, &*RE_COLLECTION] {
        for caps in regex.captures_iter(text) {
            requester.get_or_insert_with(|| truncate(caps[1].trim()).to_string());
            push_name(&mut missing, &caps[2]);
        }
    }

    if missing.is_empty() {
        let incompatible =
            text.contains("incompatible mod set") || text.contains("Incompatible mod set");
        let cause = if incompatible {
            "mod 之间互不兼容"
        } else {
            "mod 依赖解析失败"
        };
        let advice = match install_solution(text) {
            Some(package) => format!("加载器建议安装 {package}，或调整 mod 版本组合。"),
            None => "加载器未能解析 mod 依赖，按日志里的提示调整 mod。".to_string(),
        };
        return Some(Finding::new(cause, advice));
    }

    let names = format_names(&missing)?;
    let cause = match requester {
        Some(requester) => format!("缺少前置 mod：{names}（由 {requester} 要求）"),
        None => format!("缺少前置 mod：{names}"),
    };
    Some(Finding::new(
        cause,
        "安装缺失的前置 mod，并确认版本符合加载器的要求。",
    ))
}

fn duplicate_mod(_: &Captures, text: &str) -> Option<Finding> {
    static RE_DUPLICATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Found a duplicate mod (.+?) at ").unwrap());
    static RE_MOD_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Mod ID: '([^']+)'").unwrap());

    let mut names = Vec::new();
    for caps in RE_DUPLICATE.captures_iter(text) {
        push_name(&mut names, &caps[1]);
    }
    if let Some(block) = indented_block(text, "Found duplicate mods:") {
        for name in names_in(&RE_MOD_ID, block) {
            push_name(&mut names, &name);
        }
    }

    let cause = match format_names(&names) {
        Some(names) => format!("存在重复安装的 mod：{names}"),
        None => "存在重复安装的 mod".to_string(),
    };
    Some(Finding::new(cause, "删除重复的 mod 文件，只保留一个版本。"))
}

fn incompatible_mods(_: &Captures, text: &str) -> Option<Finding> {
    static RE_REPLACE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"- Replace mod '([^']+)'").unwrap());

    let advice = match RE_REPLACE.captures(text) {
        Some(caps) => format!("加载器建议替换 mod：{}。", truncate(caps[1].trim())),
        None => match install_solution(text) {
            Some(package) => format!("加载器建议安装 {package}。"),
            None => "按日志中的建议调整或移除冲突的 mod。".to_string(),
        },
    };
    Some(Finding::new("mod 之间互不兼容", advice))
}

fn mixin_failure(_: &Captures, text: &str) -> Option<Finding> {
    static RE_MIXIN_MOD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:apply for mod|from mod) ([^\s\]/]+)").unwrap());

    let cause = match RE_MIXIN_MOD.captures(text) {
        Some(caps) => format!("Mixin 注入失败（{}）", truncate(caps[1].trim())),
        None => "Mixin 注入失败".to_string(),
    };
    Some(Finding::new(
        cause,
        "多为 mod 与游戏或其它 mod 版本不兼容；移除最近添加的 mod 逐个排查。",
    ))
}

fn mod_crash(_: &Captures, text: &str) -> Option<Finding> {
    static RE_CAUGHT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"LoaderExceptionModCrash: Caught exception from ([^\r\n(]+)\(([^)\r\n]+)\)")
            .unwrap()
    });
    static RE_INSTANCE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)Failed to create mod instance\. ModId:? ([^,\r\n]+)").unwrap()
    });
    static RE_ENTRYPOINT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"Could not execute entrypoint stage '[^']*' due to errors, provided by '([^']+)'",
        )
        .unwrap()
    });

    let name = RE_CAUGHT
        .captures(text)
        .map(|caps| {
            format!(
                "{}（{}）",
                truncate(caps[1].trim()),
                truncate(caps[2].trim())
            )
        })
        .or_else(|| {
            RE_INSTANCE
                .captures(text)
                .map(|caps| truncate(caps[1].trim()).to_string())
        })
        .or_else(|| {
            RE_ENTRYPOINT
                .captures(text)
                .map(|caps| truncate(caps[1].trim()).to_string())
        });
    let cause = match name {
        Some(name) => format!("mod {name} 导致游戏崩溃"),
        None => "mod 在加载阶段崩溃".to_string(),
    };
    Some(Finding::new(cause, "更新或移除该 mod 后重试。"))
}

fn mod_config(caps: &Captures, _: &str) -> Option<Finding> {
    let file = truncate(caps[1].trim());
    let id = truncate(caps[2].trim());
    Some(Finding::new(
        format!("{id} 的配置文件无法加载"),
        format!("删除或修复 {file} 后重启游戏。"),
    ))
}

fn multiple_entries(caps: &Captures, _: &str) -> Option<Finding> {
    Some(Finding::new(
        format!("有 mod 重复注册同一个对象（{}）", truncate(caps[1].trim())),
        "移除重复注册的 mod，或调整加载顺序。",
    ))
}

fn out_of_memory(caps: &Captures, text: &str) -> Option<Finding> {
    if text.contains("There is insufficient memory for the Java Runtime Environment to continue")
        || text.contains("The system is out of physical RAM or swap space")
    {
        return Some(Finding::new(
            "物理内存不足",
            "关闭其它程序，或调小游戏最大内存与渲染距离。",
        ));
    }

    let detail = caps
        .get(1)
        .map(|detail| detail.as_str())
        .unwrap_or_default();
    let (cause, advice) =
        if detail.contains("Metaspace") || detail.contains("Compressed class space") {
            (
                "JVM 元空间不足",
                "减少 mod 数量，或给 Java 增加 -XX:MaxMetaspaceSize 参数。",
            )
        } else if detail.contains("GC overhead") {
            (
                "GC 压力过大（内存不足）",
                "调大最大内存，或减少 mod、渲染距离与资源包。",
            )
        } else {
            (
                "游戏内存不足",
                "在实例设置里调大最大内存，或减少 mod、渲染距离与资源包。",
            )
        };
    Some(Finding::new(cause, advice))
}

fn java_too_old(_: &Captures, text: &str) -> Option<Finding> {
    static RE_CLASS_VERSION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"class file version (\d+)[^\r\n]*only recognizes class file versions up to (\d+)",
        )
        .unwrap()
    });
    static RE_MAJOR_MINOR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Unsupported major\.minor version (\d+)").unwrap());

    if let Some(caps) = RE_CLASS_VERSION.captures(text) {
        let needed = java_release(&caps[1]);
        let current = java_release(&caps[2]);
        let cause = match (needed, current) {
            (Some(needed), Some(current)) => format!("需要 Java {needed}，当前 Java {current}"),
            _ => "Java 版本过低".to_string(),
        };
        return Some(Finding::new(cause, "在实例设置里换用更高版本的 Java。"));
    }

    // Forge 的 boot layer 读不了新版本编译的模块时只报 class 版本，没有 class file version 对比。
    if let Some(caps) = RE_MAJOR_MINOR.captures(text) {
        let cause = match java_release(&caps[1]) {
            Some(needed) => format!("需要 Java {needed} 或更高版本"),
            None => "Java 版本过低".to_string(),
        };
        return Some(Finding::new(cause, "在实例设置里换用更高版本的 Java。"));
    }

    Some(Finding::new("Java 版本过低", "换用该游戏版本要求的 Java。"))
}

fn native_crash(_: &Captures, text: &str) -> Option<Finding> {
    let cause = if text.contains("# C  [ig") {
        "显卡驱动崩溃（Intel）"
    } else if text.contains("# C  [atio") {
        "显卡驱动崩溃（AMD）"
    } else if text.contains("# C  [nvoglv") {
        "显卡驱动崩溃（NVIDIA）"
    } else {
        "JVM 发生原生崩溃"
    };
    Some(Finding::new(
        cause,
        "游戏目录下的 hs_err_pid*.log 有详细现场；多与显卡驱动、内存或 JVM 参数有关。",
    ))
}

fn missing_library(caps: &Captures, _: &str) -> Option<Finding> {
    Some(Finding::new(
        format!("缺少本机库：{}", truncate(caps[1].trim())),
        "重新补全游戏文件，确认 natives 已解压完整。",
    ))
}

fn file_exists(caps: &Captures, _: &str) -> Option<Finding> {
    Some(Finding::new(
        format!("文件已存在：{}", truncate(caps[1].trim())),
        "删除该文件后重试。",
    ))
}

fn file_changed(caps: &Captures, _: &str) -> Option<Finding> {
    let cause = match caps.get(1) {
        Some(file) => format!("游戏文件被修改：{}", truncate(file.as_str().trim())),
        None => "游戏文件被修改或签名不一致".to_string(),
    };
    Some(Finding::new(cause, "校验并补全游戏文件。"))
}

fn entity_crash(caps: &Captures, _: &str) -> Option<Finding> {
    Some(Finding::new(
        format!("实体 {} 导致崩溃", truncate(caps[1].trim())),
        "该实体可能来自某个 mod；在存档中移除它，或更新对应 mod。",
    ))
}

fn block_crash(caps: &Captures, _: &str) -> Option<Finding> {
    Some(Finding::new(
        format!("方块 {} 导致崩溃", truncate(caps[1].trim())),
        "该方块可能来自某个 mod；在存档中移除它，或更新对应 mod。",
    ))
}

fn suspected_mods(_: &Captures, text: &str) -> Option<Finding> {
    static RE_BLOCK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Suspected Mods:([\s\S]*?)(?:\n\s*Stacktrace|\z)").unwrap());
    static RE_NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^\s*([^(\r\n]+?)\s*\(").unwrap());

    let raw = RE_BLOCK.captures(text)?.get(1)?.as_str();
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
        return None;
    }

    let mut names = Vec::new();
    for caps in RE_NAME.captures_iter(raw) {
        push_name(&mut names, &caps[1]);
    }
    let names = match format_names(&names) {
        Some(names) => names,
        None => truncate(trimmed.lines().next().unwrap_or(trimmed)).to_string(),
    };
    Some(Finding::new(
        format!("怀疑 mod {names} 导致崩溃"),
        "逐个禁用或更新这些 mod 排查。",
    ))
}

// ---------- 文本工具 ----------

/// 取 `header` 之后连续的缩进行，用于读取加载器列出的依赖块。
fn indented_block<'a>(text: &'a str, header: &str) -> Option<&'a str> {
    let start = text.find(header)? + header.len();
    let mut end = start;
    for line in text[start..].split_inclusive('\n') {
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        end += line.len();
    }
    Some(&text[start..end])
}

fn names_in(regex: &Regex, text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for caps in regex.captures_iter(text) {
        push_name(&mut names, &caps[1]);
    }
    names
}

fn install_solution(text: &str) -> Option<&str> {
    static RE_INSTALL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"- Install ([^,\r\n]+)").unwrap());
    let caps = RE_INSTALL.captures(text)?;
    Some(truncate(caps.get(1)?.as_str().trim()))
}

fn push_name(names: &mut Vec<String>, name: &str) {
    let name = truncate(name.trim());
    if !name.is_empty() && !names.iter().any(|existing| existing == name) {
        names.push(name.to_string());
    }
}

fn format_names(names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let mut joined = names
        .iter()
        .take(MAX_NAMES)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if names.len() > MAX_NAMES {
        joined.push_str(" 等");
    }
    Some(joined)
}

fn truncate(value: &str) -> &str {
    match value.char_indices().nth(MAX_NAME_CHARS) {
        Some((index, _)) => &value[..index],
        None => value,
    }
}

fn java_release(major: &str) -> Option<i32> {
    let major: i32 = major.parse().ok()?;
    (major >= 45).then_some(major - 44)
}

#[cfg(test)]
mod tests {
    use crate::causes;

    #[test]
    fn forge_missing_dependency_lists_the_missing_mods() {
        let text = "[12:00:00] [main/ERROR]: Missing or unsupported mandatory dependencies:\n\
                    \tMod ID: 'ferritecore', Requested by: 'modernfix', Expected range: '[5.0,)', Actual version: '[MISSING]'\n\
                    \tMod ID: 'cloth-config', Requested by: 'modernfix', Expected range: '[1.0,)', Actual version: '[MISSING]'\n\
                    [12:00:00] [main/ERROR]: Loading failed\n";
        assert_eq!(causes(text), ["缺少前置 mod：ferritecore、cloth-config"]);
    }

    #[test]
    fn fabric_missing_dependency_names_the_requester() {
        let text = "net.fabricmc.loader.impl.FormattedException: Mod resolution failed\n\
                    - Mod 'Sodium' (sodium) 0.5.3 requires version 0.5.0 or later of 'Indium' (indium), which is missing!\n";
        assert_eq!(causes(text), ["缺少前置 mod：Indium（由 Sodium 要求）"]);
    }

    #[test]
    fn fabric_conflicts_are_named() {
        let text =
            "net.fabricmc.loader.impl.discovery.ModResolutionException: Found conflicting mods: \
                    sodium conflicts with optifabric\n";
        assert_eq!(causes(text), ["mod sodium 与 optifabric 冲突"]);
    }

    #[test]
    fn fabric_warnings_alone_are_not_a_cause() {
        assert!(causes("Incompatible mod set!\n").is_empty());
    }

    #[test]
    fn fabric_install_suggestion_is_used_when_no_mod_is_named() {
        let text = "Mod resolution encountered an incompatible mod set!\n\
                    A potential solution has been determined:\n\
                    \t - Install fabric-api, version 0.90.0 or later.\n";
        let findings = crate::analyze(text, &crate::Exit::Code(1));
        assert_eq!(findings[0].cause, "mod 之间互不兼容");
        assert!(findings[0].advice.contains("fabric-api"), "{findings:?}");
    }

    #[test]
    fn duplicate_mods_are_reported_in_several_formats() {
        assert_eq!(
            causes("java.lang.RuntimeException: Found a duplicate mod create-1.0.jar at mods/\n"),
            ["存在重复安装的 mod：create-1.0.jar"]
        );
        assert_eq!(
            causes("Found duplicate mods:\n\tMod ID: 'create' from mod files: a.jar, b.jar\n"),
            ["存在重复安装的 mod：create"]
        );
        assert_eq!(
            causes("net.fabricmc.loader.impl.discovery.ModResolutionException: Duplicate mods\n"),
            ["存在重复安装的 mod"]
        );
    }

    #[test]
    fn mod_loading_failures_name_the_mod() {
        assert_eq!(
            causes("LoaderExceptionModCrash: Caught exception from Create (create)\n"),
            ["mod Create（create） 导致游戏崩溃"]
        );
        assert_eq!(
            causes(
                "Could not execute entrypoint stage 'main' due to errors, provided by 'sodium'!\n"
            ),
            ["mod sodium 导致游戏崩溃"]
        );
        assert_eq!(
            causes("Failed to create mod instance. ModId modernfix,\n"),
            ["mod modernfix 导致游戏崩溃"]
        );
    }

    #[test]
    fn mixin_and_config_failures_name_the_mod() {
        assert_eq!(
            causes("Mixin apply for mod create failed: create.mixins.json:BlockMixin\n"),
            ["Mixin 注入失败（create）"]
        );
        assert_eq!(
            causes("Failed loading config file config/create.toml of type TOML for modid create\n"),
            ["create 的配置文件无法加载"]
        );
    }

    #[test]
    fn java_too_old_reports_the_needed_version() {
        assert_eq!(
            causes("UnsupportedClassVersionError: class file version 65.0, this version only recognizes class file versions up to 61.0"),
            ["需要 Java 21，当前 Java 17"]
        );
        // Forge 的 boot layer 只报 class 版本，没有当前 Java 的对照。
        assert_eq!(
            causes("Caused by: java.lang.module.InvalidModuleDescriptorException: Unsupported major.minor version 65.0"),
            ["需要 Java 21 或更高版本"]
        );
        assert_eq!(
            causes("java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main"),
            ["Java 版本过低"]
        );
    }

    #[test]
    fn memory_problems_are_distinguished_by_detail() {
        assert_eq!(
            causes("java.lang.OutOfMemoryError: Metaspace"),
            ["JVM 元空间不足"]
        );
        assert_eq!(
            causes("There is insufficient memory for the Java Runtime Environment to continue"),
            ["物理内存不足"]
        );
    }

    #[test]
    fn access_violation_points_at_the_graphics_driver() {
        assert_eq!(
            causes("EXCEPTION_ACCESS_VIOLATION\n# C  [nvoglv64.dll+0x1234]"),
            ["显卡驱动崩溃（NVIDIA）"]
        );
    }

    #[test]
    fn file_changes_are_reported_when_the_file_is_known() {
        assert_eq!(
            causes("java.lang.SecurityException: SHA1 digest error for net/minecraft/client/Main.class"),
            ["游戏文件被修改：net/minecraft/client/Main.class"]
        );
        assert_eq!(
            causes("SecurityException: signer information does not match signer information of other classes in the same package"),
            ["游戏文件被修改或签名不一致"]
        );
    }

    #[test]
    fn suspected_mods_are_listed_unless_none() {
        assert_eq!(
            causes("Suspected Mods: create (create.jar)\nStacktrace:\n"),
            ["怀疑 mod create 导致崩溃"]
        );
        assert_eq!(
            causes("Suspected Mods:\n\tcreate (create.jar)\n\tsodium (sodium.jar)\nStacktrace:\n"),
            ["怀疑 mod create、sodium 导致崩溃"]
        );
        assert!(causes("Suspected Mods: None\nStacktrace:\n").is_empty());
    }

    #[test]
    fn long_names_are_truncated() {
        let text = format!("Found a duplicate mod {}.jar at mods/\n", "x".repeat(200));
        assert!(causes(&text)[0].chars().count() < 100);
    }
}
