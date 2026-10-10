# D → Rust 迁移说明

迁移目标：**逻辑不变**，只换语言与基础设施。本文件记录逐文件映射、成熟库替换审计、
关键 API 对照，以及所有**有意保留的行为**与**为了健壮性而做的偏差**。

---

## 0. 关于"手写代码"的总原则

原 D 版本里有两类"手写"代码，处理方式不同：

1. **因为 Phobos 没有对应能力才手写的**（路径归一化、hex 编码、shell 转义、zip、http、
   sha1-UUID、宽松版本比较）→ **一律换成成熟的 Rust 库**，见第 3 节。
2. **Minecraft 领域逻辑或语义特殊的算法**（规则判定、版本 JSON 合并、maven 坐标字符串处理）→
   **保留**，但在第 3 节逐条说明"查过哪些库、为什么不用"。

另外，所有 JSON 处理从"到处 `Value` 取值"改为**强类型结构体**，见第 2 节。

---

## 1. 逐文件映射

### 入口

| D | Rust |
| --- | --- |
| `src/main.d` | `src/main.rs`（二进制）+ `src/lib.rs`（核心库） |

原 `main` 是「`readConfig()` → `startCli(args, DmclCli(), config.cli_default_command)`」。
现在拆成两层：`main.rs` 负责 `read_config` + `clap` 解析 + 分发，业务逻辑全在 `lib.rs` 之下，
以便未来 GUI 直接复用库。

### config

| D | Rust |
| --- | --- |
| `Config` 结构体（`launch_java_configs`、`account_username` …） | `config::{Config, LaunchConfig, AccountConfig, DownloadConfig, CliConfig}` |
| `getConfigPath()` | `config::get_config_path()`（改用 `dirs`） |
| `readConfig()` | `config::read_config()` |
| `saveConfig!()` | `config::save_config()` |
| `showConfig()` | `config::show_config()` |

原实现用 `__traits(allMembers, Config)` + `name.replaceFirst("_", ".")` 把扁平字段名映射成
点分 JSON 键（`launch_java` → `launch.java`）。Rust 侧直接用**嵌套结构体**表达同一份文档，
`#[serde(default)]` 复现「缺键保留默认值」的语义。**生成/读取的 `dmcl.json` 与原版逐键兼容**。

全局可变变量 `config` → `static OnceLock<RwLock<Config>>`，访问函数
`with_config` / `with_config_mut` / `set_config`。

### mcjson（新增）

| D | Rust |
| --- | --- |
| 散落各处的 `JSONValue` 取值 | `mcjson::*` 强类型结构体 |
| `checkRules`（在 `utils`） | `mcjson::check_rules` / `rules_allow` |
| `mergeJSON`（在 `utils`） | `mcjson::merge_value` / `merge_typed` |

### utils

| D | Rust | 备注 |
| --- | --- | --- |
| `getPath` | `utils::get_path` | `path-clean`；返回原生 `PathBuf`（`get_path_str` 已删除） |
| （新增） | `utils::version_dir` / `version_json_path` / `version_jar_path` | 用 `PathBuf::join` 拼版本路径，替代 `format!` |
| `getOSName` | `utils::get_os_name` | `windows`/`linux`/`osx` |
| `extractZip` | `utils::extract_zip` | `zip` crate，`spawn_blocking` 内执行 |
| `libNameToPath` | `utils::lib_name_to_path` | 保留手写 |
| `libNameGetName` | `utils::lib_name_get_name` | 保留手写 |
| `libNameGetVersion` | `utils::lib_name_get_version` | 保留手写 |
| `compareVersion` | —（删除） | 改用 `version-compare`，调用点在 `GameLauncher::get_libraries` |
| `checkRules` | `mcjson::check_rules` | 已类型化（`&[Rule]`） |
| `mergeJSON` | `mcjson::merge_value` / `merge_typed` | `merge_typed` 用 serde 往返实现同语义 |
| `readVersionJSONSafe` | `utils::read_version_json_safe` | 返回 `VersionJson` |
| `writeSafe` | `utils::write_safe` | 另加 `write_json_safe`（紧凑序列化） |
| `getSafe` | —（已删除） | 全项目零引用（原 D 的 `getSafe` 同样从未被调用）；需要宽松 GET 时写 `get_text(url).await.ok()` |
| （新增） | `utils::get_text` / `get_json_as::<T>` | 带错误上下文的 GET / 类型化解析 |
| `getDownload` | `utils::get_download` | 重试次数与原版完全一致 |
| `downloadSafe` | `utils::download_safe` | |
| `checkFile` | `utils::check_file` | |
| `escapeShellCommand` | `utils::escape_shell_command` | `shell-words` |
| `driveName` | `utils::drive_name` | 保留（仅用于回显） |
| （新增） | `utils::zip_read_entry` / `zip_has_entry` | 对应 `zip.expand(...)` 与 `name in zip.directory` |
| `split(...)` 拆参数 | `str::split_whitespace()` | 见第 4 节 |

### account

| D | Rust |
| --- | --- |
| `interface Account` | `account::Account`（trait） |
| `class OfflineAccount` | `account::OfflineAccount` |
| `sha1UUID(name)` | `account::offline::sha1_uuid`（改用 `uuid::Builder::from_sha1_bytes`） |

### download

| D | Rust |
| --- | --- |
| `mirrors.d` | `download/mirrors.rs` |
| `multitasks.d`（`TaskPool`） | `download/multitasks.rs`（`DownloadManager`） |
| `game.d` | `download/game.rs`（改用 `VersionManifest`/`AssetIndexFile`） |
| `forge.d` | `download/forge.rs`（改用 `ForgeVersionEntry`/`InstallProfile`） |
| `neoforge.d` | `download/neoforge.rs`（改用 `NeoForge*`） |
| `fabric.d` | `download/fabric.rs`（改用 `FabricLoaderEntry`） |
| （forge/neoforge 中重复的处理器逻辑） | `download/processors.rs`（抽公共实现，仅此一处） |

镜像表与原版**逐字节相同**。

### launch

| D | Rust |
| --- | --- |
| `option.d` `LaunchOption` | `launch/option.rs` |
| `java.d` `JavaOption` / `findJava` / `selectJava` | `launch/java.rs` |
| `launcher.d` `GameLauncher` | `launch/launcher.rs`（`json` 字段改为 `VersionJson`） |
| `monitor.d` `GameMonitor`（空 stub） | `launch/monitor.rs`（持有子进程句柄） |

### cli

| D | Rust |
| --- | --- |
| `cli.d` `Command`/`Param` UDA + `startCli` | `clap` derive：`cli::Cli` / `cli::Commands` |
| `cli.d` `CLI_HELP` mixin | `clap` 自动生成的 `--help` / `help` |
| `prompt.d` `getch`/`clrscr`/`CLI_PROMPT` | `cli/prompt.rs`（`dialoguer` + 其再导出的 `console`） |
| `dmcl.d` `DmclCli` 各命令 | `cli::dispatch` 转发到 `download::*` / `launch::*` |

### build.rs（新增）

`__DATE__ __TIME__` → 构建脚本注入 `DMCL_BUILD_TIME`，并用
`cargo:rerun-if-changed=src` 让时间戳在源码变化后刷新、而 `cargo build` 无改动时仍然完全缓存。

---

## 2. JSON：从 `Value` 到强类型结构体

原实现里每次取字段都是 `json["a"]["b"].str`，缺键就抛异常/崩。现在每个文档都有类型：

| 文档 | 类型 |
| --- | --- |
| 版本清单 `version_manifest.json` | `VersionManifest` / `VersionManifestEntry` |
| 版本 JSON `versions/<n>/<n>.json` | `VersionJson` |
| 库条目 | `Library` / `LibraryDownloads` / `Artifact` |
| 规则 / 参数 | `Rule` / `OsRule` / `Arguments` / `Argument` / `RuledArgument` / `StringOrVec` |
| 资源索引 `assets/indexes/<id>.json` | `AssetIndexFile` / `AssetObject` |
| 安装器 `install_profile.json` | `InstallProfile` / `InstallEntry` / `Processor` / `DataEntry` / `DataPair` |
| Forge / NeoForge / Fabric 版本列表 | `ForgeVersionEntry` / `NeoForgeVersionListing` / `NeoForgeListEntry` / `FabricLoaderEntry` |

两条关键设计：

* **未知字段一律保留**：每个结构体尾部都有 `#[serde(flatten)] extra: Map<String, Value>`，
  所以 Forge/Fabric 安装后回写版本 JSON 时，本项目没建模的字段（`time`、`releaseTime`、
  `type`、`minimumLauncherVersion`、`complianceLevel`、`logging`…）**一个都不会丢**。
  已有单测 `unknown_version_fields_round_trip` 锁定；联网实测 `download_vanilla` 落盘的
  1.7.10 JSON 里这些字段齐全。
* **缺省即缺省**：可选字段用 `Option<T>` + `skip_serializing_if`，回写时不会凭空多出 `null`。

> `flatten` 字段必须是结构体的**最后一个字段**：serde 按声明顺序把剩余键灌进它，
> 放在中间会把后面的字段吞掉。

### 合并语义如何保持

`mergeJSON` 的语义是「对象递归合并、数组拼接、其余覆盖」。为了既用强类型又保证语义逐字一致，
`VersionJson::merge` 用 `merge_typed` 实现：把两个结构体序列化成 `Value` → 调用同一个
`merge_value`（原算法）→ 反序列化回结构体。这样合并规则只有一个来源，不会随结构体演进走样。

`read_version_json_safe` 的 `inheritsFrom` 合并、Forge/NeoForge/Fabric 的安装器合并都走这条路径。

---

## 3. 成熟库替换审计

### 3.1 utils：换成成熟库的

| 原手写工具 | 为什么原来是手写 | 现在用什么 |
| --- | --- | --- |
| `getPath` 的 `..` 归一化 | Phobos 的 `buildNormalizedPath` 手写 | [`path-clean`] 的 `PathClean::clean` |
| sha1 结果转 hex | Phobos 无 hex 编码器 | [`sha1`] + [`hex`] |
| `escapeShellCommand` 加引号 | Phobos 无 shell 转义 | [`shell-words`] 的 `join` |
| `extractZip` / `zip.expand` | `std.zip` API 原始 | [`zip`] crate + 封装 `zip_read_entry` |
| `getDownload` 的 http 与重试 | 无异步 http 客户端 | [`reqwest`]（重试次数原样保留） |
| `sha1UUID` 手动改 version/variant 位 + 手动拼 UUID 字符串 | 有 `std.uuid`，但迁移时先手写了 | [`uuid`] 的 `Builder::from_sha1_bytes`（黄金用例锁定字节一致） |
| `compareVersion` 宽松版本比较 | Phobos 无版本比较 | [`version-compare`] 的 `compare_to(a, b, Cmp::Gt)` |
| `split(" ")` 拆参数串 | D 的 `split` 会产生空元素，所以需要一个包装 | `str::split_whitespace()`（见第 4 节） |
| 主动清理屏幕的 ANSI 转义 | 无终端库 | [`dialoguer`] 再导出的 [`console`] `Term::clear_screen` |

`compareVersion` 的替换说明：原 D 实现先删掉所有非 `[0-9.]` 字符再按 `.` 数字比较，
把 `1.20.1-47.2.0` 变成 `[1, 20, 147, 2, 0]`（`1` 与 `47` 粘连）。`version-compare` 会正确切分
`-`/`+`，并把 `1.0` 与 `1.0.0` 判为相等——后者与原实现一致，前者是修正。
依赖它的语义已被 `launch::launcher::tests::library_versions_are_ordered_by_version_compare` 锁定。

### 3.2 utils / 领域逻辑：查过但**故意保留**手写的

| 工具 | 候选库 | 为什么不用 |
| --- | --- | --- |
| `lib_name_to_path` / `lib_name_get_name` / `lib_name_get_version` | 无 | crates.io 上没有维护良好、且能处理 `group:artifact:version[:classifier][@ext]` 这种自由形式的库；这是约 20 行纯字符串处理 |
| `classpath_separator` | 无 | 这是 Java classpath 约定（`;`/`:`），不是路径分隔符，`std::path` 不提供 |
| `get_os_name` | 无 | 需要返回 Minecraft 规则里的名字（macOS 必须叫 `osx`），是领域映射 |
| `check_rules` | 无 | Minecraft 规则语义 |
| `drive_name` | 无 | 两行，且只用于回显 `cd <drive>` |

### 3.3 config

| 关注点 | 决策 |
| --- | --- |
| 解析 / 序列化 | **`serde` + `serde_json`**。`config` / `figment` 这类 crate 面向"多来源分层合并"（文件 + 环境变量 + 远程 + 覆盖优先级），本项目只需要"读一个 JSON 文件、缺键取默认值"，引入它们只增加依赖和语义风险 |
| 配置文件位置 | **改用 [`dirs`]**：Windows `%APPDATA%`、Linux `$XDG_CONFIG_HOME`/`~/.config` 与原版一致；**macOS 由 `~/.config` 变为 `~/Library/Application Support`**（见第 4 节） |
| 反射式键名映射 | 已由 `serde` 派生结构体取代 |

### 3.4 cli / tui

| 关注点 | 决策 |
| --- | --- |
| 参数解析 + `--help` | **[`clap`] derive**（替代 UDA + `__traits` 解析器与手写 USAGE）。`argh`/`bpaf` 功能更少，无收益 |
| 交互式菜单 / 输入 | **[`dialoguer`]**（替代 `getch()` + 手写 ANSI 菜单），清屏用其再导出的 [`console`] |
| 全屏 TUI | 若将来要做真正的面板式界面，选 [`ratatui`]；当前的"列表选择 + 逐参数提问"用 `dialoguer` 已足够，换成 `ratatui` 属于过度设计 |

### 3.5 删掉的无效代码

`pub` 条目在 lib crate 里不会被 `dead_code` 报告，所以这一轮是人工扫描（按名字统计全仓引用数）：
凡"零引用"且**不是**对某个 D 函数的有意 API 映射者，一律删除。

| 删除项 | 原因 |
| --- | --- |
| `utils::get_safe` | 全项目零引用；原 D 的 `getSafe` 同样从未被调用。等价能力 = `get_text(url).await.ok()` |
| `mirrors::MIRRORS` | 零引用的镜像名数组；D 里那个同名全局变量也被 `mirrorUrl` 的同名参数遮蔽、实际没用上 |
| `mirrors::mc_assets()` | 零引用；`MC_ASSETS` 常量本身在用，直接用常量即可 |
| `mirrors::fabric_maven()` | 零引用（此前靠 `#[allow(dead_code)]` 压住警告）；`FABRIC_MAVEN` 常量本身在用 |
| `lib.rs` 的 `#![allow(dead_code)]` | 移除后 `cargo check --all-targets` 与 `--release` 都是零警告，说明它当前只是冗余；留着反而会掩盖将来真正未使用的私有函数 |

保留的零引用项只有一处，且是有意的：

| 保留项 | 原因 |
| --- | --- |
| `launch::GameLauncher::generate_command` | 对应 D 的 `generateCommand`，是"只生成/预览启动命令、不真正 spawn"的能力，供未来 GUI 使用；CLI 走 `launch_game`。doc 注释已写明 |

其余所有 `utils` 函数都至少有一个调用点（`get_text` 由 `get_json_as` 使用、
`extract_zip` 由 `GameLauncher::get_libraries` 使用，等等）。

### 3.6 类型审计：路径 / 整数 / 字符串枚举

**(a) 路径全部改用 Rust 原生类型**

`get_path_str` 被删除，凡是"路径"的字段与参数都从 `String` 改成 `PathBuf` / `&Path`：
`LaunchOption.root_path`、`GameLauncher.version_path` / `natives_path`、`JavaOption.path`、
`DownloadFileMeta.save_to`、`LaunchConfig.minecraft_root_path`、`LaunchConfig.java_configs` 的值、
`config::get_config_path()`，以及 `download::*` / `launch::*` 各函数的 `root_path` 参数。
路径拼接从 `format!("{root}/versions/{name}/{name}.json")` 改为 `PathBuf::join`，
并新增 `utils::version_dir` / `version_json_path` / `version_jar_path` 三个小助手，
把散落在 game/forge/neoforge/fabric/processors/launcher 里的重复拼接收敛成一处。

仍然出现 `String` 的地方只有两类，都是**必须**的边界转换（全仓 20 处 `to_string_lossy`）：

* 命令行参数与 classpath 模板的替换（`launcher`、`processors`）——它们本来就是命令行值；
* 打印用的文件名（`base_name`、`drive_name`）。

**(b) 有符号整数改成无符号**

| 字段 | 原 | 现 | 理由 |
| --- | --- | --- | --- |
| `LaunchConfig.window_width` / `window_height` | `i32` | `u32` | 窗口尺寸不可能为负 |
| `LaunchConfig.min_memory` / `max_memory` | `i32` | `u32` | 堆上限不可能为负 |
| `JavaOption.arch` | `i32` | `u32` | 只有 32 / 64 |
| `JavaOption.major_version` | `i64` | `u32` | 版本号为正 |
| `JavaVersion.major_version` | `Option<i64>` | `Option<u32>` | 同上 |
| `ForgeVersionEntry.build` | `i64` | `u64` | build 号为正 |

顺带的正向副作用：`selectJava` 的自动选择里不再需要 `abs()`（无符号相减天然非负）；
配置里写负数会被 serde 直接拒绝（单测 `negative_sizes_are_rejected_instead_of_wrapping`），
而不是像 D 的 `int` 那样把负数一路传给 JVM。

**(c) 字符串改成枚举**

| 原 | 现 | 说明 |
| --- | --- | --- |
| `download.mirror: String`（各处 `match` 比较 `"official"`/`"bmclapi"`） | `Mirror`（`download::mirrors`） | 字符串比较消失；`--mirror` 交给 clap 校验并生成候选值；`neoforge::list_for_mirror` 变成穷尽 `match`，D 里"未知镜像抛 SwitchError"那条分支不再可能存在。**配置文件里的未知取值仍回退 `official`**（保持原行为，单测 `mirror_names_round_trip_and_typos_are_tolerated`） |
| `Rule.action: String` | `Option<RuleAction>`（`Allow` / `Disallow` / `Other(String)`） | 去掉魔法字符串，`check_rules` 变穷尽匹配；`Other` 保证回写版本 JSON 时不丢未知取值（单测 `string_enums_keep_unknown_values`） |
| `Processor.sides: Vec<String>` | `Vec<Side>`（`Client` / `Server` / `Other(String)`） | `is_client_side()` 不再比较字符串 |
| `launch.java` 里的哨兵值 `"autoselect"` | `JavaChoice`（`Auto` / `Named(String)`） | 序列化仍是原来的裸字符串，配置文件格式不变（单测 `java_choice_round_trips_as_a_plain_string`） |

两个"若干已知值 + `Other(String)`"的枚举由 `mcjson.rs` 里的 `string_enum!` 宏声明，
省掉重复的 serde 样板；`Mirror`（带 clap `ValueEnum` 与有损解析）和
`JavaChoice`（带哨兵语义）各自手写实现。

**明确保留为字符串的**

| 字段 | 为什么 |
| --- | --- |
| `VersionManifestEntry.kind`（`release`/`snapshot`/…） | 取值由 Mojang 的远程文档定义，且原版支持 `--type=<任意值>` 过滤；改成枚举会让"未来新增的版本类型"解析失败、也让任意取值过滤失效 |
| `JavaOption.type_`（`openjdk`/`java`/…） | 来自 `java -version` 的自由文本 |
| `CliConfig.default_command` | 指向一个 clap 子命令；做成枚举会让 `config` 依赖 `cli`（层次倒置），而当前语义是"不认识就打印 unknown command" |
| `Account::get_type()` | 直接替换进 `${user_type}` 模板，D 的接口签名本来就是字符串 |

---

## 4. 有意保留的行为 / 与原版的差异

### 保留（含原版怪癖）

1. **`mirrorUrl(mirror, null, url)` 对官方 URL 是空操作**（判断的是"URL 是否已含目标镜像前缀"），
   单测 `table_less_mode_reproduces_the_original_no_op` 锁定。
2. **`checkRules`**：带 `features` 的规则一律拒绝；`allow` 规则里出现 `os.version` 也拒绝。
3. **`download.max_tasks`**：原为线程池线程数，现为信号量许可数；入队不阻塞。
4. **`getDownload` 最多尝试 6 次**；失败不再 `exit(-1)`，改为 `Err` 向上传播，CLI 打印
   `error: ...` 并以退出码 1 结束。
5. **`default_command`**：无子命令时按配置执行，默认 `prompt`；`unknown command` 分支保留。
6. **`saveConfig` 全量序列化**（原版只合并指定字段，等价）。
7. **`launcher_profiles.json` 内容**与原版字符串完全一致。
8. **`account.online_mode`** 仍是未使用配置项，账号只有 offline 实现。
9. **诊断快照**：`debug` 构建打印 `!!! DMCL SNAPSHOT !!!`。
10. **`selectJava` 自动选择算法**；`java_configs` 由无序 AA 变为 `BTreeMap`（有序，更确定）。
11. **`getLatestNeoForge` 取列表最后一个元素**；**Forge 列表只走 bmclapi**。
12. **不带 `rules` 的参数对象会被跳过**（`("rules" in arg && checkRules(...))`）。
13. **资源索引的 `virtual` 只有"存在且为 true"才算虚拟索引**。
14. **版本 JSON 以紧凑格式回写**（对应 D 的 `json.toString()`，非 pretty）。
15. **`Argument` 对象全部由 `RuledArgument` 承接**（其字段全可选），因此对象里的未知键会进
    `extra` 而不会丢；只有非字符串非对象的元素才落到 `Argument::Other`。

### 与原版不同的（需你确认）

| 项目 | 原版 | 现在 | 影响 |
| --- | --- | --- | --- |
| **参数串切分** | `split(" ")`：只按空格切，且**会产生空元素**（`"a  b"` → `["a","","b"]`；空串 → `[]`） | `split_whitespace()`：按任意空白切、**丢弃空元素**（`"a  b"` → `["a","b"]`；空串 → `[]`） | 关键的空串情形与原版一致（`launch.additional_game` 默认就是 `""`）；差异仅在于连续空格/制表符不再产生空参数——那显然是原实现的意外行为。单测 `argument_strings_split_like_d_split` 锁定 |
| **版本比较** | 删掉非 `[0-9.]` 再逐段数字比较（`1.20.1-47.2.0` → `[1,20,147,2,0]`） | `version-compare` 正确按 `.`/`-`/`+` 切分 | 只在"同名库出现多个版本、需要挑最新"时生效；`1.0` 与 `1.0.0` 两者都判相等 |
| **配置里的整数取值** | `int`，负数被接受并一路传给 JVM | `u32` / `u64`，负数在加载配置时报错退出 | 更早暴露错误配置 |
| **`download.mirror` 取值** | 任意字符串，未知值静默按 `official` | 配置文件里仍接受任意字符串（未知 → `official`），但 `--mirror` 由 clap 校验 | 只影响命令行上的错误提示 |
| macOS 配置路径 | `$HOME/.config/dmcl.json` | `~/Library/Application Support/dmcl.json` | 仅 macOS；原 CI 矩阵只有 ubuntu/windows。若要保持旧路径，把 `get_config_path` 换回 `std::env` 版本即可 |
| 版本 JSON 键顺序 | 保持原始插入顺序 | 先已建模字段，再 `extra`（`BTreeMap` 字典序） | JSON 对象无序，语义无影响；仅文件内容顺序不同 |
| 缺键取值 | 多数位置抛异常/崩溃 | `Option` 兜底为 `None`/`""`，仅真正必要时（如找不到 `version_id`）报错 | 更健壮，正常路径无差异 |
| 远程文档解析失败 | 通常表现为深层 panic | 返回描述性错误（含 URL / 文件路径） | 更健壮 |

### 其他健壮性偏差（不影响正常路径）

| 位置 | 原行为 | 现行为 |
| --- | --- | --- |
| `findJava` | 某个候选无法解析则整体中断 | 跳过该候选并继续 |
| `--java` 取值 | 只支持 `--java=value` | 支持 `--java=value` 与 `--java value` |
| `JavaOption` 版本探测 | 只读 stdout | 读 stderr + stdout（`java -version` 输出在 stderr） |
| `getLibraries` | 每替换一个参数就重算 classpath | 只算一次（结果不变，natives 解压副作用只发生一次） |
| 游戏进程 stdio | 建管道但从不读取；`GameMonitor` 是空 stub | stdio 继承，玩家可见日志；`GameMonitor` 持有句柄，CLI 等待其退出 |

---

## 5. 尚未验证的部分

* **完整 `install` / `launch` 未端到端跑通**：需要下载数百 MB 资源并启动真实 JVM。
  已验证：`download_vanilla` 全链路（清单查询 → 镜像改写 → 类型化解析 → 落盘，
  且落盘文档保留 `time`/`releaseTime`/`type`/`minimumLauncherVersion`/`complianceLevel`/`logging`）、
  `show_version_list`、`show_fabric_version_list`、`search_java`、`list-mc`、`config`、`version`。
* **Forge / NeoForge 安装**：`installer.jar` 的 zip 解析与处理器执行路径未在真实安装包上跑过
  （`InstallProfile`/`Processor`/`DataEntry` 已有单测覆盖，含 `_comment_` 字符串这种情况）。
* **Windows**：`findJava` 按原逻辑抛「不支持」；classpath/drive 分支已实现但未在 Windows 编译验证。
* **`mirror=bmclapi`** 的元数据下载在当前网络下不可用（其 302 目标主机被代理拦截），属环境限制。

---

## 6. 如何自测

```bash
cargo test                              # 24 个离线单测
cargo test --test online -- --ignored   # 3 个联网测试
cargo clippy --all-targets              # 无警告
cargo fmt --check
```

[`path-clean`]: https://crates.io/crates/path-clean
[`sha1`]: https://crates.io/crates/sha1
[`hex`]: https://crates.io/crates/hex
[`shell-words`]: https://crates.io/crates/shell-words
[`zip`]: https://crates.io/crates/zip
[`reqwest`]: https://crates.io/crates/reqwest
[`uuid`]: https://crates.io/crates/uuid
[`version-compare`]: https://crates.io/crates/version-compare
[`dirs`]: https://crates.io/crates/dirs
[`clap`]: https://crates.io/crates/clap
[`dialoguer`]: https://crates.io/crates/dialoguer
[`console`]: https://crates.io/crates/console
[`ratatui`]: https://crates.io/crates/ratatui
