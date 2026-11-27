//! 生成启动命令行。
//!
//! 真正起进程由 [`crate::LaunchTask`] 负责；这个模块只负责拼参数。
//! 两条设计要点：
//!
//! * **这里不解压 natives** —— 那是"把文件铺好"，由 [`dmcl_game`] 负责，
//!   这里只引用 natives 目录（[`dmcl_mcmeta::layout::natives_dir`]）；启动逻辑因此与安装解耦。
//! * **已选好的 Java 与确定的内存由调用者传入** —— [`generate_arguments`] 不再自己选
//!   Java，也不做内存的自动分配。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use dmcl_game::{main_jar_path, ResolvedVersion};
use dmcl_java::JavaOption;
use dmcl_mcmeta::layout::natives_dir;
use dmcl_mcmeta::maven::{lib_name_get_name, lib_name_get_version, lib_name_to_path};
use dmcl_mcmeta::os::{classpath_separator, Os};
use dmcl_mcmeta::vanilla::{rules_allow, Argument, RuleContext, VersionJson};
use dmcl_task::{Task, TaskContext};
use version_compare::{compare_to, Cmp};

use crate::memory::{resolve_memory, MemoryOptions, MemorySize};
use dmcl_account::Account;

/// 启动器名字（替换 `${launcher_name}`）。
pub const LAUNCHER_NAME: &str = "Dacite";

/// 启动器版本（替换 `${launcher_version}`）。
pub const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 游戏窗口的初始状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    /// 窗口化，使用给定的初始宽高（像素）。
    Windowed { width: u32, height: u32 },
    /// 全屏启动。
    Fullscreen,
}

/// 启动后直接进入的目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickPlay {
    /// 单人世界的文件夹名。
    Singleplayer(String),
    /// 多人服务器地址。
    Multiplayer(String),
    /// Realms ID。
    Realms(String),
}

impl QuickPlay {
    /// 对应的 Minecraft 参数名。
    pub fn argument(&self) -> &'static str {
        match self {
            Self::Singleplayer(_) => "--quickPlaySingleplayer",
            Self::Multiplayer(_) => "--quickPlayMultiplayer",
            Self::Realms(_) => "--quickPlayRealms",
        }
    }

    /// 参数值。
    pub fn value(&self) -> &str {
        match self {
            Self::Singleplayer(value) | Self::Multiplayer(value) | Self::Realms(value) => value,
        }
    }
}

/// 生成启动参数需要的配置。
#[derive(Debug, Clone)]
pub struct GameLaunchOptions {
    /// 窗口的初始状态；宽高不可能为负。
    pub window: WindowMode,
    /// 启动后直接进入的单人世界、服务器或 Realm；`None` 表示进入主菜单。
    pub quick_play: Option<QuickPlay>,
    /// 传给游戏的客户端 ID（`${clientid}`，用于遥测）；默认空串。
    pub client_id: String,
    /// `${version_type}`（`--versionType`）的文本，显示在游戏主菜单；空串表示使用版本 JSON 的 `type`。
    pub custom_info: String,
    /// 附加 JVM 参数。
    pub additional_jvm: Vec<String>,
    /// 附加游戏参数。
    pub additional_game: Vec<String>,
}

impl Default for GameLaunchOptions {
    fn default() -> Self {
        Self {
            window: WindowMode::Windowed {
                width: 854,
                height: 480,
            },
            quick_play: None,
            client_id: String::new(),
            custom_info: LAUNCHER_NAME.to_string(),
            additional_jvm: Vec::new(),
            additional_game: Vec::new(),
        }
    }
}

/// 生成完整启动参数（java 可执行文件在最前）。
///
/// `memory` 是调用者已经确定下来的堆大小；`game_dir` 只用于替换 `${game_directory}`，
/// 与起进程用的工作目录没有推导关系。需要用户确认的警告通过 `ctx` 上报。
pub async fn generate_arguments(
    resolved: &ResolvedVersion,
    account: &Account,
    java: &JavaOption,
    game_dir: &Path,
    memory: MemorySize,
    options: &GameLaunchOptions,
    ctx: &TaskContext,
) -> Result<Vec<String>> {
    if memory.min > memory.max {
        ctx.log(format!(
            "警告：Xms（{} MiB）大于 Xmx（{} MiB），游戏可能无法启动",
            memory.min, memory.max
        ));
    }

    let mut args: Vec<String> = Vec::new();
    args.push(java.path.to_string_lossy().into_owned());

    // jvm 参数
    if resolved.json.uses_arguments() {
        // 1.13 及以后
        args.extend(arguments_for(&resolved.json, "jvm"));
    } else {
        // 1.12.2 及以前
        args.push("-Djava.library.path=${natives_directory}".to_string());
        args.push("-cp".to_string());
        args.push("${classpath}".to_string());
    }

    // 额外的 jvm 参数（调用者已经拆成独立参数）。
    args.extend(options.additional_jvm.iter().cloned());
    // 堆内存（初始 / 上限）
    args.push(format!("-Xms{}m", memory.min));
    args.push(format!("-Xmx{}m", memory.max));
    // 主类
    args.push(resolved.json.main_class.clone().unwrap_or_default());

    // 游戏参数
    if resolved.json.uses_arguments() {
        args.extend(arguments_for(&resolved.json, "game"));
    } else {
        args.extend(
            resolved
                .json
                .minecraft_arguments
                .as_deref()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string),
        );
    }

    // 快速游玩：启动后直接进入世界 / 服务器 / Realm。
    if let Some(quick_play) = &options.quick_play {
        args.push(quick_play.argument().to_string());
        args.push(quick_play.value().to_string());
    }

    // 窗口类型：窗口化传初始宽高，全屏交给游戏自己用显示器的全屏分辨率。
    match options.window {
        WindowMode::Windowed { width, height } => {
            args.push("--width".to_string());
            args.push(width.to_string());
            args.push("--height".to_string());
            args.push(height.to_string());
        }
        WindowMode::Fullscreen => args.push("--fullscreen".to_string()),
    }
    // 额外的游戏参数（调用者已经拆成独立参数）。
    args.extend(options.additional_game.iter().cloned());

    let classpath = libraries_classpath(resolved, ctx).await?;
    apply_templates(
        &mut args, resolved, account, java, game_dir, options, &classpath,
    );
    Ok(args)
}

/// 拼 classpath。
///
/// classpath 是**命令行值**，所以这里用平台分隔符拼成 `String`，只有每个条目是路径。
/// 版本号无法比较时经 `ctx` 上报一条警告。
async fn libraries_classpath(resolved: &ResolvedVersion, ctx: &TaskContext) -> Result<String> {
    let libraries_dir = resolved.reference.root.join("libraries");

    // 名字 -> (版本, 绝对路径)
    let mut libs: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();
    let rules = RuleContext::current();

    for lib in &resolved.json.libraries {
        if !rules_allow(&lib.rules, &rules) {
            continue;
        }
        let coord = lib.name.as_str();
        let name = lib_name_get_name(coord);
        let version = lib_name_get_version(coord);
        // `compareVersion(ver, libs[name].ver) > 0`，交给 `version-compare`；
        // 版本号解析不出来时保留已有的那一项
        let better = match libs.get(&name) {
            None => true,
            Some((existing, _)) => match compare_to(&version, existing, Cmp::Gt) {
                Ok(better) => better,
                Err(()) => {
                    ctx.confirm(format!(
                        "库 {name} 的版本号无法比较（{version} vs {existing}），先到的保留。"
                    ))
                    .await?;
                    false
                }
            },
        };

        match lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) {
            // 原版库：按 JSON 给的下载路径
            Some(artifact) => {
                if better {
                    libs.insert(
                        name,
                        (
                            version,
                            libraries_dir.join(artifact.path.as_deref().unwrap_or("")),
                        ),
                    );
                }
            }
            // 没有 artifact 的库：`downloads: {}` 的 Forge / modloader 库按 maven
            // 坐标拼路径；只有 natives 分类器的库不放进 classpath（由启动前解压）。
            None => {
                let natives_only = lib
                    .downloads
                    .as_ref()
                    .and_then(|d| d.classifiers.as_ref())
                    .is_some_and(|classifiers| !classifiers.is_empty());
                if better && !natives_only {
                    libs.insert(name, (version, libraries_dir.join(lib_name_to_path(coord))));
                }
            }
        }

        // natives 的解压不在这里做：启动流程的先导步骤已经把 natives 铺到
        // 该 java 架构对应的目录里，这里只需要在命令行里引用那个目录。
    }

    let mut result = String::new();
    for (_, path) in libs.values() {
        result.push_str(&path.to_string_lossy());
        result.push_str("${classpath_separator}");
    }
    let main_jar = main_jar_path(&resolved.reference, &resolved.json);
    if tokio::fs::metadata(&main_jar).await.is_ok() {
        result.push_str(&main_jar.to_string_lossy());
        result.push_str("${classpath_separator}");
    }

    // `replaceLast("${classpath_separator}", "")`
    const SEP: &str = "${classpath_separator}";
    if let Some(index) = result.rfind(SEP) {
        result.replace_range(index..index + SEP.len(), "");
    }
    Ok(result)
}

/// 展开 `arguments.<key>`：套用每条参数的 rules，拼出命中项。
fn arguments_for(json: &VersionJson, key: &str) -> Vec<String> {
    let mut args = Vec::new();
    let Some(json_args) = json.arguments_for(key) else {
        // 1.12.2 及更早：根本没有 `arguments` 对象
        return args;
    };
    let rules = RuleContext::current();
    for arg in json_args {
        match arg {
            Argument::Plain(value) => args.push(value.clone()),
            Argument::Ruled(ruled) => {
                // 没有 rules 视为无条件；有 rules 则命中了才加上
                if rules_allow(&ruled.rules, &rules) {
                    if let Some(value) = &ruled.value {
                        value.push_into(&mut args);
                    }
                }
            }
            Argument::Other(_) => {}
        }
    }
    args
}

/// 把模板占位符替换成实际值。
fn apply_templates(
    args: &mut [String],
    resolved: &ResolvedVersion,
    account: &Account,
    java: &JavaOption,
    game_dir: &Path,
    options: &GameLaunchOptions,
    classpath: &str,
) {
    // natives 目录按运行平台的 os 与所选 java 的 arch 区分
    let natives_directory = natives_dir(&resolved.reference.name, Os::current(), java.arch)
        .to_string_lossy()
        .into_owned();
    let library_directory = resolved
        .reference
        .root
        .join("libraries")
        .to_string_lossy()
        .into_owned();
    let assets_root = resolved
        .reference
        .root
        .join("assets")
        .to_string_lossy()
        .into_owned();
    let asset_index_id = resolved.json.asset_index_id().to_string();
    let game_assets = if asset_index_id == "legacy" {
        resolved
            .reference
            .root
            .join("assets")
            .join("virtual")
            .join("legacy")
    } else {
        resolved.reference.root.join("assets")
    }
    .to_string_lossy()
    .into_owned();
    let version_id = resolved.json.id.clone().unwrap_or_default();
    let game_directory = game_dir.to_string_lossy().into_owned();
    // 自定义信息为空时沿用版本 JSON 的 `type`，与 `${version_type}` 原本的语义一致。
    let version_type = if options.custom_info.is_empty() {
        resolved.json.kind.clone().unwrap_or_default()
    } else {
        options.custom_info.clone()
    };
    let classpath_separator = classpath_separator().to_string();

    for arg in args.iter_mut() {
        *arg = arg
            .replace("${natives_directory}", &natives_directory)
            .replace("${launcher_name}", LAUNCHER_NAME)
            .replace("${launcher_version}", LAUNCHER_VERSION)
            .replace("${classpath}", classpath)
            .replace("${library_directory}", &library_directory)
            .replace("${auth_player_name}", &account.username)
            .replace("${auth_uuid}", &account.uuid)
            .replace("${uuid}", &account.uuid)
            .replace("${auth_xuid}", &account.xuid)
            .replace("${clientid}", &options.client_id)
            .replace("${version_name}", &version_id)
            .replace("${game_directory}", &game_directory)
            .replace("${assets_root}", &assets_root)
            .replace("${assets_index_name}", &asset_index_id)
            .replace("${auth_access_token}", &account.access_token)
            .replace("${auth_session}", &account.access_token)
            .replace("${user_type}", &account.user_type)
            .replace("${version_type}", &version_type)
            .replace("${user_properties}", "{}")
            .replace("${game_assets}", &game_assets)
            .replace("${classpath_separator}", &classpath_separator);
    }
}

/// 生成启动参数之后的产出。
#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    /// 已解析的版本。
    pub resolved: ResolvedVersion,
    /// 生成参数时使用的 `${game_directory}`。
    pub game_dir: PathBuf,
    pub args: Vec<String>,
    /// 本次实际使用的 Java。
    pub java: JavaOption,
}

/// 用已经选好的 java 与确定的内存生成启动参数。
#[derive(Clone)]
pub struct GenerateArgsTask {
    pub game: GameLaunchOptions,
    /// 生成 `${game_directory}` 用的路径，和进程工作目录没有推导关系。
    pub game_dir: PathBuf,
    /// 用户的内存偏好；在这里按已选 Java 的架构换算成确定值。
    pub memory: MemoryOptions,
}

#[async_trait::async_trait]
impl Task<(Arc<ResolvedVersion>, Arc<Account>, Arc<JavaOption>), PreparedLaunch>
    for GenerateArgsTask
{
    async fn run(
        &self,
        (resolved, account, java): (Arc<ResolvedVersion>, Arc<Account>, Arc<JavaOption>),
        ctx: TaskContext,
    ) -> Result<PreparedLaunch> {
        let memory = resolve_memory(&self.memory, Some(java.arch));
        let args = generate_arguments(
            &resolved,
            &account,
            &java,
            &self.game_dir,
            memory,
            &self.game,
            &ctx,
        )
        .await?;
        Ok(PreparedLaunch {
            resolved: (*resolved).clone(),
            game_dir: self.game_dir.clone(),
            args,
            java: (*java).clone(),
        })
    }
}

/// 展示用的命令行：进入工作目录 + 启动参数。可直接复制进当前平台的 shell 执行。
pub(crate) fn command_preview(game_path: &Path, args: &[String]) -> String {
    #[cfg(windows)]
    {
        let dir = quote_windows(&game_path.to_string_lossy());
        let args = args
            .iter()
            .map(|arg| quote_windows(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("cd /D {dir} && {args}")
    }
    #[cfg(not(windows))]
    {
        let cd = format!("cd {}", quote_posix(&game_path.to_string_lossy()));
        let args = args
            .iter()
            .map(|arg| quote_posix(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{cd} && {args}")
    }
}

/// POSIX shell 单参数引号：只在需要时包单引号，单引号自身写成 `'\''`。
#[cfg(not(windows))]
fn quote_posix(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "-_./:=,@+".contains(ch));
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Windows（cmd / MSVCRT）单参数引号：需要时才包双引号，反斜杠靠近引号时按规则翻倍。
#[cfg(windows)]
fn quote_windows(arg: &str) -> String {
    // 需要引号：空串、含空白、含 `"`、或含 cmd 元字符（放进引号里 cmd 就不再解释）
    let needs = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || "\"&|<>^()%!".contains(c));
    if !needs {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                out.push('\\');
            }
            '"' => {
                for _ in 0..backslashes * 2 + 1 {
                    out.push('\\');
                }
                out.push('"');
                backslashes = 0;
            }
            _ => {
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    for _ in 0..backslashes * 2 {
        out.push('\\');
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_game::VersionRef;
    use dmcl_mcmeta::os::Arch;
    use dmcl_task::{task_fn, TaskEvent, TaskGraphBuilder};
    use std::sync::Arc;

    async fn run_node<O, F, Fut>(f: F) -> anyhow::Result<Arc<O>>
    where
        O: Send + Sync + 'static,
        F: Fn(TaskContext) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = anyhow::Result<O>> + Send + 'static,
    {
        let mut builder = TaskGraphBuilder::new("测试");
        let step = builder.step("测试");
        let sink = builder.add(step, "测试", (), task_fn(move |_: (), ctx| f(ctx)));
        builder.build(sink).unwrap().run(|_| {}).await
    }

    fn test_resolved(json: VersionJson, root: &Path) -> ResolvedVersion {
        ResolvedVersion {
            reference: VersionRef::new(root, "1.0"),
            json,
        }
    }

    fn test_account() -> Account {
        Account {
            username: "player".to_string(),
            uuid: "uuid".to_string(),
            access_token: String::new(),
            user_type: "legacy".to_string(),
            xuid: String::new(),
        }
    }

    fn test_java() -> JavaOption {
        JavaOption {
            path: PathBuf::from("/usr/bin/java"),
            major: 17,
            arch: Arch::X86_64,
        }
    }

    /// "同名库保留最新版本"这一步依赖 `version-compare`。这些是 Minecraft
    /// 版本 JSON 里真实出现过的版本形态，用来钉住我们依赖的语义。
    #[test]
    fn library_versions_are_ordered_by_version_compare() {
        assert!(compare_to("1.20.1", "1.20.10", Cmp::Lt).unwrap());
        assert!(compare_to("1.20.10", "1.20.1", Cmp::Gt).unwrap());
        assert!(compare_to("3.3.2", "3.3.1", Cmp::Gt).unwrap());
        assert!(compare_to("1.20.1", "1.20.1", Cmp::Eq).unwrap());
        // forge 那种带 build 号的坐标
        assert!(compare_to("1.20.1-47.3.0", "1.20.1-47.2.0", Cmp::Gt).unwrap());
        // 结尾的 0 不会让版本变大（"1.0" == "1.0.0"）
        assert!(compare_to("1.0", "1.0.0", Cmp::Eq).unwrap());
        // ……但多出来的非零部分会
        assert!(compare_to("1.0", "1.0.1", Cmp::Lt).unwrap());
    }

    #[tokio::test]
    async fn the_classpath_keeps_only_the_newest_duplicate() {
        let root = std::env::temp_dir().join(format!("dmcl-args-cp-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.unwrap();

        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "libraries": [
                    {"name": "org.lwjgl:lwjgl:3.3.1", "downloads": {"artifact":
                        {"path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar"}}},
                    {"name": "org.lwjgl:lwjgl:3.3.2", "downloads": {"artifact":
                        {"path": "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2.jar"}}},
                    {"name": "net.minecraftforge:forge:1.20.1-47.2.0"},
                    {"name": "com.example:modloader:1.0", "downloads": {}},
                    {"name": "net.java.jinput:jinput-platform:2.0.5",
                     "downloads": {"classifiers": {"natives-linux": {"path": "x/y/jinput-natives.jar"}}}},
                    {"name": "only:other:1", "rules": [{"action": "allow", "os": {"name": "plan9"}}]}
                ]
            }"#,
        )
        .unwrap();

        let resolved = Arc::new(test_resolved(json, &root));
        let resolved_for_task = resolved.clone();
        let classpath = run_node(move |ctx| {
            let resolved = resolved_for_task.clone();
            async move { libraries_classpath(&resolved, &ctx).await }
        })
        .await
        .unwrap();
        let classpath = (*classpath).clone();

        assert!(classpath.contains("lwjgl-3.3.2.jar"), "{classpath}");
        assert!(!classpath.contains("lwjgl-3.3.1.jar"), "{classpath}");
        // modloader 库按 maven 坐标推导路径
        assert!(
            classpath.contains("net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0.jar"),
            "{classpath}"
        );
        // `downloads: {}` 的 modloader 库同样要按 maven 坐标进 classpath
        assert!(
            classpath.contains("com/example/modloader/1.0/modloader-1.0.jar"),
            "{classpath}"
        );
        // 只有 natives 分类器的库不进 classpath
        assert!(!classpath.contains("jinput-platform"), "{classpath}");
        // 平台不匹配的库不在里面
        assert!(!classpath.contains("only/other/1"), "{classpath}");
        // 条目之间保留占位分隔符（最终由 generate_arguments 换成平台分隔符），
        // 但末端那个必须被去掉
        assert!(
            classpath.contains("${classpath_separator}"),
            "条目之间应当有占位分隔符: {classpath}"
        );
        assert!(
            !classpath.ends_with("${classpath_separator}"),
            "{classpath}"
        );
        assert!(classpath.ends_with(".jar"), "{classpath}");

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn the_classpath_only_contains_the_main_jar() {
        let root = std::env::temp_dir().join(format!("dmcl-args-mainjar-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        let versions = root.join("versions");
        for name in ["base", "1.0"] {
            let dir = versions.join(name);
            tokio::fs::create_dir_all(&dir).await.unwrap();
            tokio::fs::write(dir.join(format!("{name}.jar")), b"jar")
                .await
                .unwrap();
        }
        let base_jar = versions
            .join("base")
            .join("base.jar")
            .to_string_lossy()
            .into_owned();
        let self_jar = versions
            .join("1.0")
            .join("1.0.jar")
            .to_string_lossy()
            .into_owned();

        let json: VersionJson =
            serde_json::from_str(r#"{"id": "1.0", "inheritsFrom": "base"}"#).unwrap();
        let resolved = Arc::new(test_resolved(json, &root));
        let resolved_for_task = resolved.clone();
        let classpath = run_node(move |ctx| {
            let resolved = resolved_for_task.clone();
            async move { libraries_classpath(&resolved, &ctx).await }
        })
        .await
        .unwrap();
        let classpath = (*classpath).clone();

        // `inherits_from` 不再被当成父 jar 路径，只加当前版本自己的主 jar
        assert!(!classpath.contains(&base_jar), "{classpath}");
        assert!(classpath.contains(&self_jar), "{classpath}");

        let json: VersionJson = serde_json::from_str(r#"{"id": "1.0", "jar": "base"}"#).unwrap();
        let resolved = Arc::new(test_resolved(json, &root));
        let resolved_for_task = resolved.clone();
        let classpath = run_node(move |ctx| {
            let resolved = resolved_for_task.clone();
            async move { libraries_classpath(&resolved, &ctx).await }
        })
        .await
        .unwrap();
        let classpath = (*classpath).clone();

        assert!(classpath.contains(&base_jar), "{classpath}");
        assert!(!classpath.contains(&self_jar), "{classpath}");

        tokio::fs::remove_dir_all(&root).await.unwrap();
    }

    #[tokio::test]
    async fn arguments_apply_rules_and_templates() {
        let root = std::env::temp_dir().join(format!("dmcl-args-args-{}", std::process::id()));
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "mainClass": "com.example.Main",
                "arguments": {
                    "jvm": ["-XstartOnFirstThread"],
                    "game": [
                        "--username", "${auth_player_name}",
                        "--clientId", "${clientid}",
                        "--xuid", "${auth_xuid}",
                        {"rules": [{"action": "allow", "os": {"name": "plan9"}}], "value": ["--never"]},
                        {"rules": [{"action": "allow"}, {"action": "disallow", "os": {"name": "plan9"}}], "value": "--always"}
                    ]
                }
            }"#,
        )
        .unwrap();

        let resolved = test_resolved(json, &root);
        let mut account = test_account();
        account.xuid = "2535412345678901".to_string();
        let java = test_java();
        let game_dir = root.join("versions").join("1.0");
        let options = GameLaunchOptions {
            client_id: "test-client".to_string(),
            additional_jvm: vec!["-Ddmcl.test.extra=1.0".to_string()],
            quick_play: Some(QuickPlay::Multiplayer("example.org:25565".to_string())),
            ..GameLaunchOptions::default()
        };

        let args = run_node(move |ctx| {
            let resolved = resolved.clone();
            let account = account.clone();
            let java = java.clone();
            let game_dir = game_dir.clone();
            let options = options.clone();
            async move {
                generate_arguments(
                    &resolved,
                    &account,
                    &java,
                    &game_dir,
                    MemorySize {
                        min: 512,
                        max: 2048,
                    },
                    &options,
                    &ctx,
                )
                .await
            }
        })
        .await
        .unwrap();
        let args = (*args).clone();

        // 第一个参数是已选定的 java 可执行文件
        assert!(args[0].contains("java"), "{}", args[0]);
        assert!(args.contains(&"-XstartOnFirstThread".to_string()));
        assert!(args.contains(&"com.example.Main".to_string()));
        assert!(!args.contains(&"--never".to_string()));
        assert!(args.contains(&"--always".to_string()));
        // 模板被替换成账户里的名字
        assert!(args.contains(&"player".to_string()));
        // 1.18+ 的 clientId / xuid 也要替换，不能把占位符原样传给游戏
        assert!(args.contains(&"test-client".to_string()));
        assert!(args.contains(&"2535412345678901".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("${")), "{args:?}");
        // 内存参数按 MiB 展开
        assert!(args.contains(&"-Xmx2048m".to_string()));
        assert!(args.contains(&"-Xms512m".to_string()));
        // 窗口尺寸通过命令行参数传入
        assert!(args.contains(&"--width".to_string()));
        // 快速游玩参数紧跟在游戏参数之后
        assert!(args.contains(&"--quickPlayMultiplayer".to_string()));
        assert!(args.contains(&"example.org:25565".to_string()));
        // 默认不再附加历史 FML 兼容参数
        assert!(!args
            .iter()
            .any(|arg| arg.contains("ignoreInvalidMinecraftCertificates")));
        // 调用者追加的额外 JVM 参数原样带上
        assert!(args.contains(&"-Ddmcl.test.extra=1.0".to_string()));
    }

    #[tokio::test]
    async fn version_type_prefers_custom_info_and_falls_back_to_the_json_type() {
        let root = std::env::temp_dir().join(format!("dmcl-args-type-{}", std::process::id()));
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "type": "snapshot",
                "mainClass": "com.example.Main",
                "arguments": {"game": ["--versionType", "${version_type}"]}
            }"#,
        )
        .unwrap();

        for (custom_info, expected) in [("custom", "custom"), ("", "snapshot")] {
            let resolved = test_resolved(json.clone(), &root);
            let account = test_account();
            let java = test_java();
            let game_dir = root.clone();
            let options = GameLaunchOptions {
                custom_info: custom_info.to_string(),
                ..GameLaunchOptions::default()
            };

            let args = run_node(move |ctx| {
                let resolved = resolved.clone();
                let account = account.clone();
                let java = java.clone();
                let game_dir = game_dir.clone();
                let options = options.clone();
                async move {
                    generate_arguments(
                        &resolved,
                        &account,
                        &java,
                        &game_dir,
                        MemorySize { min: 512, max: 512 },
                        &options,
                        &ctx,
                    )
                    .await
                }
            })
            .await
            .unwrap();
            assert!(args.contains(&expected.to_string()), "{args:?}");
        }
    }

    #[tokio::test]
    async fn game_directory_uses_the_argument_not_the_working_directory() {
        let root = std::env::temp_dir().join(format!("dmcl-args-gamedir-{}", std::process::id()));
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "mainClass": "com.example.Main",
                "arguments": {"jvm": [], "game": ["--gameDir", "${game_directory}"]}
            }"#,
        )
        .unwrap();

        let resolved = test_resolved(json, &root);
        let account = test_account();
        let java = test_java();
        // 故意与任何“工作目录”都不同的路径
        let game_dir = root.join("somewhere-else");
        let options = GameLaunchOptions::default();

        let args = run_node(move |ctx| {
            let resolved = resolved.clone();
            let account = account.clone();
            let java = java.clone();
            let game_dir = game_dir.clone();
            let options = options.clone();
            async move {
                generate_arguments(
                    &resolved,
                    &account,
                    &java,
                    &game_dir,
                    MemorySize { min: 512, max: 512 },
                    &options,
                    &ctx,
                )
                .await
            }
        })
        .await
        .unwrap();
        assert!(
            args.contains(&root.join("somewhere-else").to_string_lossy().into_owned()),
            "{args:?}"
        );
    }

    #[tokio::test]
    async fn generate_args_task_resolves_the_memory_preferences() {
        let root = std::env::temp_dir().join(format!("dmcl-args-memory-{}", std::process::id()));
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "mainClass": "com.example.Main",
                "arguments": {"jvm": [], "game": []}
            }"#,
        )
        .unwrap();

        let resolved = Arc::new(test_resolved(json, &root));
        let account = Arc::new(test_account());
        let java = Arc::new(test_java());
        let task = GenerateArgsTask {
            game: GameLaunchOptions::default(),
            game_dir: root.clone(),
            memory: MemoryOptions {
                min: Some(512),
                max: Some(2048),
            },
        };

        let args = run_node(move |ctx| {
            let resolved = resolved.clone();
            let account = account.clone();
            let java = java.clone();
            let task = task.clone();
            async move { task.run((resolved, account, java), ctx).await }
        })
        .await
        .unwrap();

        assert!(args.args.contains(&"-Xms512m".to_string()));
        assert!(args.args.contains(&"-Xmx2048m".to_string()));
        assert_eq!(args.game_dir, root);
    }

    #[tokio::test]
    async fn an_xms_above_xmx_is_reported_through_the_context() {
        let root = std::env::temp_dir().join(format!("dmcl-args-warning-{}", std::process::id()));
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "mainClass": "com.example.Main",
                "arguments": {"jvm": [], "game": []}
            }"#,
        )
        .unwrap();
        let resolved = test_resolved(json, &root);
        let account = test_account();
        let java = test_java();
        let game_dir = root.clone();
        let options = GameLaunchOptions::default();

        let mut builder = TaskGraphBuilder::new("生成参数");
        let step = builder.step("步骤");
        let sink = builder.add(
            step,
            "生成参数",
            (),
            task_fn(move |_: (), ctx| {
                let resolved = resolved.clone();
                let account = account.clone();
                let java = java.clone();
                let game_dir = game_dir.clone();
                let options = options.clone();
                async move {
                    generate_arguments(
                        &resolved,
                        &account,
                        &java,
                        &game_dir,
                        MemorySize {
                            min: 4096,
                            max: 2048,
                        },
                        &options,
                        &ctx,
                    )
                    .await
                }
            }),
        );
        let graph = builder.build(sink).unwrap();
        let mut events = graph.subscribe();
        let running = tokio::spawn(async move { graph.run(|_| {}).await });
        let mut warned = false;
        while let Some(event) = events.recv().await {
            if let TaskEvent::Log { message, .. } = event {
                warned |= message.contains("Xms") && message.contains("Xmx");
            }
        }
        running.await.unwrap().unwrap();
        assert!(warned);
    }

    #[tokio::test]
    async fn uncomparable_library_versions_wait_for_confirmation() {
        assert!(compare_to("foo", "bar", Cmp::Gt).is_err());

        let root = std::env::temp_dir().join(format!("dmcl-args-compare-{}", std::process::id()));
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "mainClass": "com.example.Main",
                "libraries": [
                    {"name": "a:b:foo", "downloads": {"artifact":
                        {"path": "a/b/foo/b-foo.jar"}}},
                    {"name": "a:b:bar", "downloads": {"artifact":
                        {"path": "a/b/bar/b-bar.jar"}}}
                ],
                "arguments": {"jvm": [], "game": []}
            }"#,
        )
        .unwrap();
        let resolved = test_resolved(json, &root);
        let account = test_account();
        let java = test_java();
        let game_dir = root.clone();
        let options = GameLaunchOptions::default();

        let mut builder = TaskGraphBuilder::new("生成参数");
        let step = builder.step("步骤");
        let sink = builder.add(
            step,
            "生成参数",
            (),
            task_fn(move |_: (), ctx| {
                let resolved = resolved.clone();
                let account = account.clone();
                let java = java.clone();
                let game_dir = game_dir.clone();
                let options = options.clone();
                async move {
                    generate_arguments(
                        &resolved,
                        &account,
                        &java,
                        &game_dir,
                        MemorySize { min: 512, max: 512 },
                        &options,
                        &ctx,
                    )
                    .await
                }
            }),
        );
        let graph = builder.build(sink).unwrap();
        let mut events = graph.subscribe();
        let running = tokio::spawn(async move { graph.run(|_| {}).await });
        loop {
            match events.recv().await.unwrap() {
                TaskEvent::Warning {
                    message, decision, ..
                } => {
                    assert!(message.contains("无法比较"), "{message}");
                    decision.abort();
                    break;
                }
                _ => continue,
            }
        }
        let err = running.await.unwrap().unwrap_err();
        assert!(
            err.chain()
                .any(|cause| cause.downcast_ref::<dmcl_task::Cancelled>().is_some()),
            "{err:#}"
        );
    }

    #[test]
    fn command_preview_uses_the_platform_shell_rules() {
        let dir = std::path::Path::new("/mc/versions/1.0");
        let args = vec!["java".to_string(), "-Dfoo=bar baz".to_string()];
        let command = command_preview(dir, &args);

        #[cfg(not(windows))]
        {
            assert_eq!(command, "cd /mc/versions/1.0 && java '-Dfoo=bar baz'");
        }
        #[cfg(windows)]
        {
            assert!(command.starts_with("cd /D "), "{command}");
            assert!(command.contains("\"-Dfoo=bar baz\""), "{command}");
        }
    }
}
