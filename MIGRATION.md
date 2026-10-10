# 重构说明：单 crate → 六 crate workspace

本文记录本轮重构的**逐文件去向**、**`utils.rs` 的拆解结果**、**新设计的理由**，
以及**与原实现（含上一轮 D → Rust 迁移）的所有差异**。

---

## 0. 目标与约束

目标：

1. 拆成多个 crate，边界清晰；
2. **这些 crate 一律不使用"全局读取式"配置**，而是由调用者传入配置选项；
3. 拆掉极度冗杂的 `utils.rs`；
4. 清理遗留的 D 式写法。

约束（实施前已确认）：

| 决策点 | 结论 |
| --- | --- |
| 仓库布局 | 标准 workspace：根 `Cargo.toml` 只放 `[workspace]`，六个成员在 `crates/` 下 |
| 跨 crate 复用的零散工具 | 就近拆分到各 crate，**不新增 common crate**（下文列出每一处的归属） |
| 进度获取方式 | 事件流（`broadcast` channel），而非轮询 |
| 任务链失败语义 | 快速失败：任一步出错即中止后续并返回该错误 |
| 镜像源 | 固定枚举 `official` / `bmclapi`（不做运行时可注册的注册表） |
| 下载能力 | 多文件并发 + 大文件分块；**不做断点续传** |
| 完整性检查 | 启动侧只返回"是否完整 + 缺失清单"，由调用者去调安装 |
| CLI / 配置兼容性 | 命令与配置键名基本保持，允许按需新增字段 |
| 交付方式 | 一次性完成全部 crate，并分阶段提交 |

---

## 1. 逐文件去向

| 原路径 | 新位置 | 说明 |
| --- | --- | --- |
| `src/main.rs` | `crates/dmcl/src/main.rs` | 读配置 → 解析参数 → 分发 |
| `src/lib.rs` | `crates/dmcl/src/lib.rs` | 应用层（config + cli） |
| `src/env.rs` | 拆散 | `LAUNCHER_NAME`/`VERSION` → `dmcl-launch`；`LAUNCHER_PROFILES_JSON` → `dmcl-install::vanilla` |
| `src/config.rs` | `crates/dmcl/src/config.rs` | 仍是 serde 结构体；**去掉了全局 `RwLock<Config>`**，改为普通值传递 |
| `src/mcjson.rs` | `crates/dmcl-mcmeta/src/{rules,version,asset,manifest,install_profile,loader}.rs` | 一个 868 行的文件按文档类型拆开 |
| `src/utils.rs` | 见第 2 节 | 按归属拆散 |
| `src/download/mirrors.rs` | `crates/dmcl-mcmeta/src/source.rs` + `meta.rs` | 镜像表 → `Root` 的 base 表；新增 URL 模板 |
| `src/download/multitasks.rs` | `crates/dmcl-download/src/*` | `TaskPool` 语义 → `DownloadTask`（并发 + 分块 + 进度 + 校验） |
| `src/download/game.rs` | `crates/dmcl-install/src/{plan,files,vanilla}.rs` | 拆成"规划"（纯函数）与"下载 / 落盘" |
| `src/download/forge.rs` | `crates/dmcl-install/src/loader.rs` | |
| `src/download/neoforge.rs` | `crates/dmcl-install/src/loader.rs` | |
| `src/download/fabric.rs` | `crates/dmcl-install/src/loader.rs` | |
| `src/download/processors.rs` | `crates/dmcl-install/src/processors.rs` | processor 改用 `CommandTask` 执行 |
| `src/account/*` | `crates/dmcl-launch/src/account.rs` | |
| `src/launch/java.rs` | `crates/dmcl-launch/src/java.rs` | 不再读写全局配置 |
| `src/launch/option.rs` | `crates/dmcl-launch/src/option.rs` | 增加 java 选择与本地布局字段 |
| `src/launch/launcher.rs` | `crates/dmcl-launch/src/launcher.rs` | **不再解压 natives**；新增 `check_integrity` |
| `src/launch/monitor.rs` | `crates/dmcl-launch/src/monitor.rs` | 增加 `kill` / `try_status` |
| `src/cli/mod.rs` | `crates/dmcl/src/cli/mod.rs` | 新增进度渲染 |
| `src/cli/prompt.rs` | `crates/dmcl/src/cli/prompt.rs` | 仅把 `escape_shell_command` 的来源换掉 |
| `build.rs` | `crates/dmcl/build.rs` | 内容不变 |
| `tests/online.rs` | 每个 crate 的 `tests/online.rs` | 原先一个文件覆盖全项目；现在按 crate 拆分，并新增了更多断言 |

新增的模块（原实现没有对应物）：

| 新模块 | 作用 |
| --- | --- |
| `dmcl-task` 全部 | 通用任务链、进度事件流、取消、`CommandTask` |
| `dmcl-mcmeta::source` | URL 模板、下载根、镜像优先级 |
| `dmcl-mcmeta::meta` | 根元数据常量 + 各种"取数"方法 |
| `dmcl-mcmeta::layout` | 本地目录布局 + 读版本 JSON（原在 `utils`） |
| `dmcl-install::plan` | 纯函数的"需要哪些文件"规划 |
| `dmcl-launch::integrity` | 完整性检查 |
| `dmcl-install::spec` | 安装计划与顶层任务链 |

---

## 2. `utils.rs` 拆到哪儿去了

原 `utils.rs`（437 行）里的每一样东西现在都有明确归属：

| 原函数 | 新位置 | 备注 |
| --- | --- | --- |
| `http_client` | — | 客户端由 `DownloadSource` / `DownloadTask` 各自持有；下载任务直接复用下载源的客户端 |
| `get_path`（路径归一化） | `dmcl-mcmeta::layout::absolute` | `dmcl-download` 与 `dmcl` 各有一份 6 行的私有副本（避免为它建公共 crate） |
| `get_os_name` | `dmcl-mcmeta::os` | install / launch 都依赖 mcmeta，无重复 |
| `classpath_separator` | `dmcl-mcmeta::os` | 同上 |
| `sha1_hex` | `dmcl-download::hash` | 下载校验、processor 产物校验、完整性检查三处共用；`dmcl-mcmeta` 不需要哈希 |
| `check_file` | `dmcl-download::hash::check_file` | |
| — | `dmcl-download::hash::hash_file` / `file_matches` | 新增（流式哈希 + 带大小的判断） |
| `extract_zip` | `dmcl-install::zip` | 只有安装流程需要解压（natives 解压也移到了这里） |
| `zip_read_entry` / `zip_has_entry` | `dmcl-install::zip` | |
| — | `dmcl-install::zip::main_class_of` | 新增：读 processor jar 的 `Main-Class` |
| `lib_name_to_path` / `lib_name_get_name` / `lib_name_get_version` | `dmcl-mcmeta::maven` | install 与 launch 共用 |
| `version_dir` / `version_json_path` / `version_jar_path` | `dmcl-mcmeta::layout` | 并新增 `libraries_dir` / `assets_dir` / `assets_index_path` / `assets_objects_dir` / `assets_virtual_dir` / `natives_dir` |
| `read_version_json_safe` | `dmcl-mcmeta::layout` | 读出来即归一化成 URL 模板 |
| `write_safe` / `write_json_safe` | `dmcl-install::io` | 只有安装流程写文件 |
| `get_text` / `get_json_as` | `dmcl-mcmeta::source::DownloadSource::{get_text,get_json}` | 支持 URL 模板、支持镜像回退 |
| `get_download`（重试 6 次） | `dmcl-download`（重试 `max_attempts` 次，默认 3） | 详见第 6 节 |
| `download_safe` | — | 由 `DownloadTask` 取代（走 `.part` + 校验 + 改名） |
| `escape_shell_command` | `dmcl-launch::util` | cli 也从这里用 |
| `drive_name` | `dmcl-launch::util` | |
| （原 `dmcl.env` 的常量） | `dmcl-launch::option::{LAUNCHER_NAME,LAUNCHER_VERSION}` / `dmcl-install::vanilla::LAUNCHER_PROFILES_JSON` | |

**没有新增 crate**：凡是只有一处用的，就近放；只有 `sha1` / `get_path` 这两个
被两个以上 crate 真正需要，前者收了唯一一份到 `dmcl-download`，后者各留 6 行副本。

---

## 3. URL 模板与下载源

原实现是"用一个镜像枚举去改写 url 前缀"，模板化之后变成两步：

1. **归一化**：从网络 / 磁盘读到的文档里，所有 url 都尽量换成 `${ROOT}/相对路径`；
2. **解析**：真正下载前，按当前镜像把 `${ROOT}` 换成 base url。

实现要点：

* `Root` 有九个：`MC_META` / `MC_DATA` / `MC_LAUNCHER` / `MC_MAVEN` / `MC_ASSETS` /
  `FORGE_MAVEN` / `NEOFORGE_MAVEN` / `FABRIC_META` / `FABRIC_MAVEN`；
* 一个根可以有**多个等价官域**（Forge 的 `files.minecraftforge.net/maven` 与
  `maven.minecraftforge.net` 是同一个仓库，都归一成 `${FORGE_MAVEN}`）；
* 归一化时选**最长前缀**，否则 bmclapi 的站点根会把 `/maven` 之类的更具体的根吞掉；
* 识别不出的 host 原样保留 = 不可镜像，与原实现"镜像表里没有的 host 原样返回"一致；
* `UrlFields` trait 统一遍历文档里的 url 字段，`templatize_urls` / `resolve_urls`
  对任何文档类型都通用，新增字段不会漏改；
* bmclapi **不镜像** `piston-data` / `launcher.mojang.com`（与原镜像表一致），
  这两个根在两个镜像下解析结果相同；
* `DownloadSource` 支持镜像**优先级列表**（前一个失败自动试下一个）。默认只有一个镜像，
  与原行为一致。

Forge 的版本列表接口（`bmclapi …/forge/minecraft/<mc>`）**只有 bmclapi 提供**，
NeoForge 则是官方 / bmclapi 两套接口 —— 这两处按原实现写死，并在 `dmcl-mcmeta::meta`
里逐条注明。

---

## 4. 任务模型（`dmcl-task`）

```rust
pub trait Task: Send + Sync {
    fn name(&self) -> &str;
    fn run<'a>(&'a self, ctx: TaskContext) -> BoxFuture<'a, Result<()>>;
}
```

* 用装箱 future 表达 `async fn`，因此 `Box<dyn Task>` 可以直接当 trait 对象用，
  也**不需要引入 `async-trait` 依赖**；
* `TaskChain::then(任务, 显示名)` 串联；`run()` 顺序执行，失败即中止并给错误加上
  出错步骤的名字；
* 进度是 `broadcast` 事件流，所以**一个任务可以上报任意多次**、
  **多个前端可以同时消费**；`run_with(回调)` 是"边跑边打印"的便捷写法；
* 链套链时，`impl Task for TaskChain` 会把子链的进度**转发**到父上下文，
  嵌套不会丢事件（有单测锁定）；
* `TaskContext::detached("名字")` 建立独立上下文：只想跑一次、不要界面时用；
* `CommandTask` 跑外部程序：默认逐行上报输出，失败时把输出结尾拼进错误信息；
  取消时子进程被 `kill_on_drop` 杀掉（有单测）。

对应你给的伪代码：

```rust
let task = TaskChain::new("下载游戏")
    .then(command_task1, "执行程序任务")
    .then(download_task1, "下载任务");
while let Ok(event) = task.subscribe().recv().await { /* 实时进度 */ }
task.run().await?;
```

---

## 5. 下载模型（`dmcl-download`）

| 关注点 | 做法 |
| --- | --- |
| 多文件并发 | `JoinSet` + 全局 `Semaphore(max_connections)` |
| 大文件分块 | 已知大小且超过 `chunk_threshold` 时，先发一个 `bytes=0-0` 探针确认服务器支持 `Range`（返回 206 + `Content-Range`），再按 `chunk_size` 切块并发；单文件并发块数由 `max_chunks_per_file` 限制 |
| 不支持 Range | 自动退回单连接下载 |
| 完整性 | 先写 `<文件>.part`，sha1（没有 sha1 时用大小）校验通过才 `rename`；失败删掉半成品并重试 `max_attempts` 次 |
| 进度 | 跨文件累计字节数 + 「N/M 个文件」；100 ms 节流；结束前强制上报一次 |
| 进度分母 | 只统计**真正要下载、且知道大小**的文件（跳过项、未知大小的项不计入） |
| 重试与进度 | 每次尝试单独记账，失败重试时把这一轮已上报的字节数**回滚**，因此 `done` 永远不会超过 `total`（修复"100% 了还在涨"） |
| 超时 | `connect_timeout(15s)` + `read_timeout(30s)`，等待响应头也有 30s 上限；连接僵死不再无限等待 |
| 失败处理 | 所有已排队的项目都会跑完，失败项进 `DownloadReport`；`Task` 实现默认"有失败就返回错误" |
| 断点续传 | **不做**（按确认的决策） |

`dmcl-install` 与 `dmcl-download` 共用同一个 `reqwest::Client`（复用连接池）。

> 进度行不再把全局的字节数挂到某个恰好触发上报的文件名下（旧实现用
> `ctx.child(文件名)` 上报，于是出现 `intermediary-1.21.1.jar: 100.0% (85.7 MiB / 84.7 MiB) intermediary-1.21.1.jar`
> 这种文件名字重复两遍、百分比被 `clamp` 死锁在 100% 的乱象）。

---

## 6. 启动与安装解耦（`dmcl-launch`）

* `GameLauncher::check_integrity()` 返回 `IntegrityStatus { complete, missing, checked }`，
  每个缺失项带**目标路径 + URL 模板 + sha1 + 原因**（缺失 / sha1 不符 / 大小不符 / 尚未生成）；
* 它**不下载任何东西**：修复由调用者执行 `dmcl-install`（`game_files_task`），
  已存在的文件会被自动跳过；
* "这个版本由哪些文件组成"直接复用 `dmcl-install::plan` 的纯函数（`PlanContext`），
  两边永远不会对不上；
* natives 的解压从启动流程移到了安装流程（它本质就是"把文件铺好"），
  于是 `dmcl-launch` 不再需要 zip 代码；
* java 选择不再读写全局配置：`find_java(roots)` 只返回结果，
  `select_java(required_major, &choice, &configs)` 接收配置表；
  配置表为空且自动选择时，会现场扫一次 `/lib/jvm/`（**不写回配置**）。

---

## 7. 与原实现的差异

### 7.1 有意保留的行为（含怪癖）

1. `checkRules`：带 `features` 的规则一律让整组失效；`allow` 规则里出现 `os.version` 也失效。
2. 库条目里 `net.minecraftforge:forge:` 开头的"困难库"直接跳过（原实现的注释是"这个文件就是下不下来"）。
3. 无地址的 natives 分类器按 `natives[os]` 找，找不到就不下。
4. 资源索引的 `virtual` 只有"存在且为 true"才算虚拟索引。
5. Forge 的版本列表只走 bmclapi；NeoForge 官方接口的结果仍按版本前缀过滤。
6. `getLatestNeoForge` 取列表**最后一个**元素；`getLatestForge` 取 build 最大者；
   `getLatestFabric` 取最新的 **stable**，没有 stable 时取第一个。
7. Forge / NeoForge 安装后：`inheritsFrom` 置空、`id` / `jar` 设为版本目录名、
   去掉 `extra` 里的 `_comment_`。
8. 版本 JSON 以**紧凑格式**写回（对应 D 的 `json.toString()`）。
9. `launcher_profiles.json` 的内容与原版字符串一致，且只在文件不存在时创建。
10. 不带 `rules` 的参数对象会被跳过；`--mirror` 之外，未知镜像值仍退化为 `official`。
11. 诊断快照（debug 构建打印 `!!! DMCL SNAPSHOT !!!`）、`unknown command` 分支、
    `cli.default_command` 的默认值都保留。
12. 日志仍然是直接打印（`println!`/`eprintln!`），没有引入日志框架。

### 7.2 本轮的有意改动

| 项目 | 原来 | 现在 | 原因 |
| --- | --- | --- | --- |
| **配置的持有方式** | 全局 `static RwLock<Config>` + `with_config()` | 普通值，由 `cli::run` 传入 | 明确要求；也让各 crate 与配置彻底解耦 |
| **URL 的处理** | 按镜像枚举替换 url 前缀 | 文档内统一归一到 `${ROOT}` 模板，下载前再解析 | 明确要求；可读性与可测试性都更好 |
| **natives 解压位置** | 启动时（构建 classpath 的副作用） | 安装时 | 让启动与安装解耦；启动侧因此不需要 zip |
| **启动前的完整性检查** | 没有：缺什么就现场下什么 | 返回 `IntegrityStatus`，修复交给调用者 | 明确要求 |
| **`install` 命令的加载器版本** | 只能"取最新" | 新增 `--forge-version` / `--neoforge-version` / `--fabric-version` | 便于固定版本；不传仍取最新 |
| **Fabric 安装用的 mc 版本号** | CLI 把**版本目录名**当作 mc 版本号传给 fabric profile 接口 | 用真正的 mc 版本 id | 修复原先 `install <id> <name> --fabric` 在 `id != name` 时会取错 profile 的问题 |
| **下载重试次数** | 硬编码最多 6 次 | `download.max_attempts`（默认 3） | 可配置；元数据请求不再重试（原 `get_text` 也不重试） |
| **校验算法** | 只支持 sha1 | `Checksum` 支持 **md5 / sha1 / sha256** | 明确要求；`Checksum::parse` 接受 `sha256:…` 或按长度推断，写错的校验值会在解析时就报错 |
| **界面语言** | 帮助/提示中英混用（clap 的 `Usage:`/`Print help` + 手写的英文提示） | 全中文 | 明确要求；只保留命令名/参数名/配置键这些标识符为英文 |
| **`prompt` 的可选参数** | 直接回车会拼出 `--forge-version=` 空值，clap 随后报错 | 留空即跳过；必填参数重问；开关选"否"后不再追问其细节参数 | 明确要求（"可选选项现在并不能留空"）。注意 dialoguer 的 `Input` 默认**不允许空输入**，且它的处理方式是静默 `continue`（把提问原样重刷一遍、不给任何提示），必须显式 `allow_empty(true)`，见第 8 节 |
| **单个 artifact 缺 `path` 时** | 目标路径会退化成 `libraries/` 目录本身 | 按 maven 坐标推导 | 原行为会写出一个目录路径，无法工作 |
| **natives 分类器缺 url 时** | 拿空 url 去请求，然后报错 | 跳过该分类器 | 更稳；正常文档里不会出现 |
| **困难库的仓库地址拼接** | `{url}/{path}`（url 带尾斜杠时会出双斜杠） | 去掉尾斜杠再拼 | 纯规范化 |
| **`api` 默认 UA** | `dmcl/<版本>`（`LAUNCHER_NAME`） | 各 crate 自带 `dmcl/<CARGO_PKG_VERSION>` | 避免为一行常量的依赖关系 |
| **配置文件位置（macOS）** | `$HOME/.config/dmcl.json` | `~/Library/Application Support/dmcl.json` | 沿用上一轮的改动（CI 只跑 ubuntu/windows） |

### 7.3 新增的配置字段

`download` 段新增（都有默认值，旧配置文件照常工作）：

| 键 | 默认 | 含义 |
| --- | --- | --- |
| `chunk_threshold_mib` | 8 | 超过这个大小（MiB）就分块下载 |
| `chunk_size_mib` | 4 | 每块大小（MiB） |
| `max_chunks_per_file` | 8 | 单个文件最多并发几块 |
| `max_attempts` | 3 | 单个文件最多尝试几次 |

`launch.natives_arch`（可省略）：natives 分类器里 `${arch}` 的取值。
原实现用的是"所选 java"的位宽，但安装流程并不负责选 java，因此默认按本机位宽，
需要时用这个键覆盖。

---

## 8. 尚未验证的部分

* **完整的 `install` / `launch` 仍未端到端跑通**：需要下载数百 MB 资源并启动真实 JVM。
  已验证的联网路径见下一节。
* **Forge / NeoForge 安装器**：`installer.jar` 的 zip 解析、`maven/` 抽取、
  processor 执行路径没有在真实安装包上跑过（`InstallProfile` / `Processor` /
  `DataEntry` 有单测，`main_class_of` / `extract_zip` 也有单测）。
* **Windows**：`find_java` 按原逻辑报"不支持"；classpath / drive / `find_on_path`
  的分支已实现但未在 Windows 上编译验证。
* **bmclapi 的部分接口**在当前网络环境下不可用（其 302 目标被代理拦截），属环境限制。
* **交互式菜单没有在真实终端里驱动过**：本机沙箱（内外都一样）没有 `/dev/pts`，
  分配不了伪终端，`script` / `python -c "import pty"` / `console::Term::read_write_pair`
  都走不通，因此 `dmcl prompt` 的按键流程**无法在本机端到端跑**。
  已经做到的替代验证：把"跟终端打交道"抽成 `Prompter` 接口（真实实现是 dialoguer，
  测试里换成脚本化回答），于是整条提问流程 —— "可选参数留空 ⇒ 命令行里没有该参数"、
  "必填参数留空 ⇒ 重问"、"重问次数上限"、"关掉的开关不再问细节参数" ——
  都有真实单测；渲染部分另有中文主题的输出断言。
  **仍然需要人肉确认的只剩一件事**：dialoguer 在 `allow_empty(true)` 下，
  直接回车确实返回空字符串（依据是它的源码 `dialoguer-0.11.0/src/prompts/input.rs`
  中 `permit_empty` 那一段，以及 `allow_empty` 的文档注释；本机没法运行验证）。

## 9. 如何自测

```bash
cargo test --workspace                          # 离线单测
cargo test --workspace -- --ignored             # 联网冒烟测试
cargo clippy --workspace --all-targets          # 零警告
cargo fmt --all --check
```

联网测试覆盖：

* `dmcl-mcmeta`：版本列表 / 版本 JSON 的取数与模板化、Fabric loader 列表；
* `dmcl-download`：单连接下载、**分块结果与单连接逐字节一致**（分块那一份用 **sha256** 校验，
  顺带验证非 sha1 算法在真实下载里也能走通）、进度事件、失败项汇总；
* `dmcl-install`：原版元数据落盘（且**落盘文档里不能残留模板**）、真实版本 JSON 的规划结果、
  真下载几个库并验证"第二次全跳过"；
* `dmcl-launch`：真实版本的完整性检查 + 命令行生成（不含未替换的模板）；
* `dmcl`：`Config` → 各 crate 选项 → 安装 / 启动的完整链路。
