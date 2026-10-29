//! 生成启动命令行。
//!
//! 真正起进程由 [`crate::LaunchTask`] 负责；这个模块只负责拼参数。
//! 两条设计要点：
//!
//! * **这里不解压 natives** —— 那是"把文件铺好"，由 [`dmcl_install`] 负责，
//!   这里只引用 natives 目录（[`dmcl_mcmeta::layout::natives_dir`]）；启动逻辑因此与安装解耦。
//! * **java 的选择来自 [`LaunchOption`]**。

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use dmcl_mcmeta::layout::{natives_dir, read_version_json_safe, version_dir};
use dmcl_mcmeta::maven::{lib_name_get_name, lib_name_get_version, lib_name_to_path};
use dmcl_mcmeta::os::{classpath_separator, Arch, Os};
use dmcl_mcmeta::vanilla::{rules_allow, Argument, RuleContext, VersionJson};
use dmcl_task::TaskContext;
use version_compare::{compare_to, Cmp};

use crate::account::Account;
use crate::java::JavaOption;
use crate::memory::resolve_memory;
use crate::monitor::GameMonitor;
use crate::option::{LaunchOption, LAUNCHER_NAME, LAUNCHER_VERSION};

/// 启动一个版本。
pub struct GameLauncher {
    pub option: LaunchOption,
    /// 已经登录好的账户快照。
    pub account: Account,
    /// `versions/<version_name>`。
    pub version_path: PathBuf,
    pub java: Option<JavaOption>,
    pub json: VersionJson,
}

impl GameLauncher {
    /// `this(LaunchOption, Account)`。
    pub async fn new(option: LaunchOption, account: Account) -> Result<Self> {
        let version_path = version_dir(&option.root_path, &option.version_name);
        let json = read_version_json_safe(&option.root_path, &option.version_name).await?;
        Ok(Self {
            option,
            account,
            version_path,
            java: None,
            json,
        })
    }

    /// 拼 classpath。
    ///
    /// classpath 只算一次。它是**命令行值**，所以这里用平台分隔符拼成 `String`，
    /// 只有每个条目是路径。版本号无法比较时经 `ctx` 上报一条警告。
    pub async fn get_libraries(&self, ctx: &TaskContext) -> Result<String> {
        if self.java.is_none() {
            return Err(anyhow!("还没有选择 java"));
        }
        let libraries_dir = self.option.root_path.join("libraries");

        // 名字 -> (版本, 绝对路径)
        let mut libs: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();
        let rules = RuleContext::current();

        for lib in &self.json.libraries {
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

            match lib.downloads.as_ref() {
                // 原版库
                Some(downloads) => {
                    if let Some(artifact) = &downloads.artifact {
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
                }
                // modloader 库
                None => {
                    if better {
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
        if let Some(oriver) = self.json.inherits_from.as_deref() {
            // 若基于另一个版本，把父版本的 jar 也加进 classpath
            let parent_jar =
                version_dir(&self.option.root_path, oriver).join(format!("{oriver}.jar"));
            result.push_str(&parent_jar.to_string_lossy());
            result.push_str("${classpath_separator}");
        }
        let self_jar = self
            .version_path
            .join(format!("{}.jar", self.option.version_name));
        if tokio::fs::metadata(&self_jar).await.is_ok() {
            result.push_str(&self_jar.to_string_lossy());
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
    pub fn get_arguments(&self, key: &str) -> Vec<String> {
        let mut args = Vec::new();
        let Some(json_args) = self.json.arguments_for(key) else {
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

    /// 生成完整启动参数（java 可执行文件在最前）。
    pub async fn gen_args(&mut self) -> Result<Vec<String>> {
        self.gen_args_in(&TaskContext::detached("生成启动参数"))
            .await
    }

    /// 同 [`GameLauncher::gen_args`]，但把「Java 版本 / 架构不匹配」等需要用户确认的警告
    /// 通过 `ctx` 上报，并**等待前端决定**是否继续。
    pub async fn gen_args_in(&mut self, ctx: &TaskContext) -> Result<Vec<String>> {
        // Java 路径；已经选好就直接用，否则按配置选一个。版本或架构不匹配时先问用户。
        let required_major = self.json.required_java_major();
        let java = match &self.java {
            Some(java) => java.clone(),
            None => self.option.java.select_java(required_major).await?,
        };
        let mut mismatches = Vec::new();
        if java.major != required_major {
            mismatches.push(format!(
                "Java 版本不匹配（需要 {required_major}，实际为 {}）",
                java.major
            ));
        }
        if let Some(host) = Arch::current().filter(|host| *host != java.arch) {
            mismatches.push(format!(
                "Java 架构与当前平台不一致（Java 为 {}，平台为 {}）",
                java.arch.name(),
                host.name()
            ));
        }
        if !mismatches.is_empty() {
            ctx.confirm(mismatches.join("；")).await?;
        }

        // natives 目录按运行平台的 os 与所选 java 的 arch 区分
        let natives_directory = natives_dir(&self.option.version_name, Os::current(), java.arch)
            .to_string_lossy()
            .into_owned();

        // 内存要在 java 选定后才能定：自动分配依赖它的架构（32 位上限）。
        let (min_memory, max_memory) = resolve_memory(
            self.option.min_memory,
            self.option.max_memory,
            Some(java.arch),
        );
        if min_memory > max_memory {
            ctx.log(format!(
                "警告：Xms（{min_memory} MiB）大于 Xmx（{max_memory} MiB），游戏可能无法启动"
            ));
        }

        let mut args: Vec<String> = Vec::new();
        args.push(java.path.to_string_lossy().into_owned());
        self.java = Some(java);

        // jvm 参数
        if self.json.uses_arguments() {
            // 1.13 及以后
            args.extend(self.get_arguments("jvm"));
        } else {
            // 1.12.2 及以前
            args.push("-Djava.library.path=${natives_directory}".to_string());
            args.push("-cp".to_string());
            args.push("${classpath}".to_string());
        }

        // 额外的 jvm 参数（调用者已经拆成独立参数）。
        args.extend(self.option.additional_jvm.iter().cloned());
        // 内存上限
        args.push(format!("-Xmn{min_memory}m"));
        args.push(format!("-Xmx{max_memory}m"));
        // 主类
        args.push(self.json.main_class.clone().unwrap_or_default());

        // 游戏参数
        if self.json.uses_arguments() {
            args.extend(self.get_arguments("game"));
        } else {
            args.extend(
                self.json
                    .minecraft_arguments
                    .as_deref()
                    .unwrap_or("")
                    .split_whitespace()
                    .map(str::to_string),
            );
        }

        // 窗口尺寸
        args.push("--width".to_string());
        args.push(self.option.window_width.to_string());
        args.push("--height".to_string());
        args.push(self.option.window_height.to_string());
        // 额外的游戏参数（调用者已经拆成独立参数）。
        args.extend(self.option.additional_game.iter().cloned());

        // 模板替换：这些都是要拼进命令行的值，因此必须是字符串
        let classpath = self.get_libraries(ctx).await?;
        let library_directory = self
            .option
            .root_path
            .join("libraries")
            .to_string_lossy()
            .into_owned();
        let assets_root = self
            .option
            .root_path
            .join("assets")
            .to_string_lossy()
            .into_owned();
        let asset_index_id = self.json.asset_index_id().to_string();
        let game_assets = if asset_index_id == "legacy" {
            self.option
                .root_path
                .join("assets")
                .join("virtual")
                .join("legacy")
        } else {
            self.option.root_path.join("assets")
        }
        .to_string_lossy()
        .into_owned();
        let version_id = self.json.id.clone().unwrap_or_default();
        let version_path = self.version_path.to_string_lossy().into_owned();
        let account = &self.account;
        let custom_info = self.option.custom_info.clone();
        let classpath_separator = classpath_separator().to_string();

        for arg in args.iter_mut() {
            *arg = arg
                .replace("${natives_directory}", &natives_directory)
                .replace("${launcher_name}", LAUNCHER_NAME)
                .replace("${launcher_version}", LAUNCHER_VERSION)
                .replace("${classpath}", &classpath)
                .replace("${library_directory}", &library_directory)
                .replace("${auth_player_name}", &account.username)
                .replace("${auth_uuid}", &account.uuid)
                .replace("${uuid}", &account.uuid)
                .replace("${version_name}", &version_id)
                .replace("${game_directory}", &version_path)
                .replace("${assets_root}", &assets_root)
                .replace("${assets_index_name}", &asset_index_id)
                .replace("${auth_access_token}", &account.access_token)
                .replace("${auth_session}", &account.access_token)
                .replace("${user_type}", &account.user_type)
                .replace("${version_type}", &custom_info)
                .replace("${user_properties}", "{}")
                .replace("${game_assets}", &game_assets)
                .replace("${classpath_separator}", &classpath_separator);
        }
        Ok(args)
    }

    /// 展示 / 可复制执行的命令行（进入工作目录 + 启动参数）。
    pub async fn generate_command(&mut self) -> Result<String> {
        let args = self.gen_args().await?;
        Ok(command_preview(&self.version_path, &args))
    }
}

/// 展示用的命令行：进入工作目录 + 启动参数。可直接复制进当前平台的 shell 执行。
pub(crate) fn command_preview(version_path: &std::path::Path, args: &[String]) -> String {
    #[cfg(windows)]
    {
        let dir = quote_windows(&version_path.to_string_lossy());
        let args = args
            .iter()
            .map(|arg| quote_windows(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("cd /D {dir} && {args}")
    }
    #[cfg(not(windows))]
    {
        let cd = format!("cd {}", quote_posix(&version_path.to_string_lossy()));
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

/// 用给定的工作目录与命令行参数启动游戏子进程，返回**未 wait** 的 [`GameMonitor`]。
///
/// 抽成自由函数是为了让 [`crate::LaunchTask`] 能直接用 [`crate::PreparedLaunch`]
/// 里已有的参数启动，不必再建一个 [`GameLauncher`]、再选一次 java。这里**不打印**
/// 任何东西：命令行由 [`crate::LaunchTask`] 通过进度事件流展示。
pub(crate) fn spawn_game(version_path: &std::path::Path, args: &[String]) -> Result<GameMonitor> {
    if args.is_empty() {
        return Err(anyhow!("命令为空"));
    }
    let mut command = tokio::process::Command::new(&args[0]);
    command.args(&args[1..]).current_dir(version_path);
    let child = command.spawn()?;
    Ok(GameMonitor::new(child))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{AccountSource, OfflineAccountSource};
    use dmcl_mcmeta::os::Arch;
    use std::sync::Arc;

    /// "同名库保留最新版本"这一步现在依赖 `version-compare`。这些是 Minecraft
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

    async fn launcher_with(json: VersionJson, root: PathBuf) -> GameLauncher {
        let source = Arc::new(OfflineAccountSource::new("player"));
        let account = source.login().await.unwrap();
        let option = LaunchOption::new(source, &root, "1.0");
        let version_path = version_dir(&root, "1.0");
        GameLauncher {
            option,
            account,
            version_path,
            java: Some(JavaOption {
                path: PathBuf::from("/usr/bin/java"),
                major: 17,
                arch: Arch::X86_64,
            }),
            json,
        }
    }

    #[tokio::test]
    async fn the_classpath_keeps_only_the_newest_duplicate() {
        let root = std::env::temp_dir().join(format!("dmcl-launch-cp-{}", std::process::id()));
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
                    {"name": "only:other:1", "rules": [{"action": "allow", "os": {"name": "plan9"}}]}
                ]
            }"#,
        )
        .unwrap();

        let launcher = launcher_with(json, root.clone()).await;
        let classpath = launcher
            .get_libraries(&TaskContext::detached("t"))
            .await
            .unwrap();

        assert!(classpath.contains("lwjgl-3.3.2.jar"), "{classpath}");
        assert!(!classpath.contains("lwjgl-3.3.1.jar"), "{classpath}");
        // modloader 库按 maven 坐标推导路径
        assert!(
            classpath.contains("net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0.jar"),
            "{classpath}"
        );
        // 平台不匹配的库不在里面
        assert!(!classpath.contains("only/other/1"), "{classpath}");
        // 条目之间保留占位分隔符（最终由 gen_args 换成平台分隔符），
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
    async fn arguments_apply_rules_and_templates() {
        let root = std::env::temp_dir().join(format!("dmcl-launch-args-{}", std::process::id()));
        let json: VersionJson = serde_json::from_str(
            r#"{
                "id": "1.0",
                "mainClass": "com.example.Main",
                "arguments": {
                    "jvm": ["-XstartOnFirstThread"],
                    "game": [
                        "--username", "${auth_player_name}",
                        {"rules": [{"action": "allow", "os": {"name": "plan9"}}], "value": ["--never"]},
                        {"rules": [{"action": "allow"}, {"action": "disallow", "os": {"name": "plan9"}}], "value": "--always"}
                    ]
                }
            }"#,
        )
        .unwrap();

        let mut launcher = launcher_with(json, root).await;
        launcher.option.min_memory = Some(512);
        launcher.option.max_memory = Some(2048);
        launcher
            .option
            .additional_jvm
            .push("-Ddmcl.test.extra=1.0".to_string());
        let args = launcher.gen_args().await.unwrap();

        // 第一个参数是已选定的 java 可执行文件
        assert!(args[0].contains("java"), "{}", args[0]);
        assert!(args.contains(&"-XstartOnFirstThread".to_string()));
        assert!(args.contains(&"com.example.Main".to_string()));
        assert!(!args.contains(&"--never".to_string()));
        assert!(args.contains(&"--always".to_string()));
        // 模板被替换成账户里的名字
        assert!(args.contains(&"player".to_string()));
        // 内存参数按 MiB 展开
        assert!(args.contains(&"-Xmx2048m".to_string()));
        assert!(args.contains(&"-Xmn512m".to_string()));
        // 窗口尺寸在最前面（走命令行参数）
        assert!(args.contains(&"--width".to_string()));
        // 没有加载器时 additional_jvm 的默认值也会带上
        assert!(args
            .iter()
            .any(|arg| arg.contains("ignoreInvalidMinecraftCertificates")));
        // 调用者追加的额外 JVM 参数原样带上
        assert!(args.contains(&"-Ddmcl.test.extra=1.0".to_string()));
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
