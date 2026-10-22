# 架构设计

本文说明 dmcl 的工程结构、各 crate 的职责，以及几个关键设计（URL 模板、任务与进度、
下载、安装规划、启动与安装解耦）。使用方式与构建见 [README.md](README.md)。

## 概览

一个 cargo workspace，按职责拆成六个 crate：**通用任务**（`dmcl-task`）、
**Minecraft 元数据**（`dmcl-mcmeta`）、**下载**（`dmcl-download`）、
**安装**（`dmcl-install`）、**启动**（`dmcl-launch`），以及**应用层**（`dmcl`）。

```text
dmcl-rs/
├── Cargo.toml                 # 纯 workspace（members + 统一的依赖版本）
└── crates/
    ├── dmcl-task/             # 通用任务链 + 进度事件流（与 MC 无关）
    ├── dmcl-mcmeta/           # MC 元数据文档、下载源、URL 模板、本地目录布局
    ├── dmcl-download/         # 并发 / 分块下载（实现 dmcl-task 的 Task）
    ├── dmcl-install/          # 原版 + Fabric/Forge/NeoForge 安装、完整性检查
    ├── dmcl-launch/           # 账号、java、命令行生成、启动
    └── dmcl/                  # 应用：config + cli（未来在这里加 gui）
```

## 依赖关系

按 crate 看（箭头表示「依赖」）：

```text
dmcl ─┬─► dmcl-install ─┬─► dmcl-mcmeta   （叶子）
      │                 ├─► dmcl-download ──► dmcl-task   （叶子）
      │                 └─► dmcl-task
      ├─► dmcl-launch ───► dmcl-install / dmcl-mcmeta / dmcl-download / dmcl-task
      ├─► dmcl-download ─► dmcl-task
      ├─► dmcl-mcmeta
      └─► dmcl-task
```

- `dmcl-task` 与 `dmcl-mcmeta` 是最底层的两个 crate，互不依赖；
- `dmcl-download` 只依赖 `dmcl-task`（哈希 / 校验实现都在它自己里面）；
- `dmcl-install` 依赖 mcmeta + download + task；
- `dmcl-launch` 依赖 mcmeta + download + install + task（复用同一套「这个版本由哪些
  文件组成」的纯函数做完整性检查）；
- 只有 `dmcl` 知道配置文件的存在。

不引入反向 / 环形依赖；也不要为了「两个 crate 都要用」随手新建 common crate —— 就近拆分，
或各留一份小副本。

## 一条硬规则：crate 不读配置

除 `dmcl` 外，**没有任何 crate 会去读配置文件**。所有外部输入都由调用者通过结构体传入：

| crate           | 入参                                                                                  |
| --------------- | ------------------------------------------------------------------------------------- |
| `dmcl-task`     | 无（纯任务抽象）                                                                      |
| `dmcl-mcmeta`   | `DownloadSource`（镜像 + http 客户端）、URL 模板字符串                                |
| `dmcl-download` | `DownloadOptions`（并发数、分块阈值 / 大小、单文件重试次数、失败是否忽略、UA）        |
| `dmcl-install`  | `InstallOptions`（根目录、下载源、下载策略、是否跳过音乐、java 提供者）             |
| `dmcl-launch`   | `LaunchOption`（账号、根目录、版本名、窗口 / 内存、java 选择与候选表）                |
| `dmcl`          | `dmcl.json`（唯一读写配置的地方）                                                     |

这样每个业务 crate 都能被 GUI / TUI / 测试用自定义选项直接调用。

## 各 crate 职责

### `dmcl-task` —— 通用任务

与 Minecraft 无关的异步任务抽象：

- `Task`：每个任务**接收一个输入、产出一个输出**；
- `TaskChain`：用 `then(任务, 显示名)` 把上一个任务的输出接到下一个的输入上，
  **任一步失败即中止后续**并把错误原样返回；
- `TaskContext`：任务用它上报进度（`progress(current, total, message, percent)`，百分比由
  任务自己算好、`None` 表示没有）与日志（`log`，一次性文字）；
- `TaskEvent`（`broadcast` 事件流）：前端通过 `subscribe()` 实时消费，一个任务可以上报
  任意多次，也可以被多个前端同时消费；链套链时子链的**步骤级**事件会原样转发到父链
  （含 `StepStarted` / `StepFinished`，只有链级的 `Started` / `Finished` 丢弃），前端据此
  用**栈**还原嵌套层级：进入 `StepStarted` push、`StepFinished` pop；
- `CancelHandle`：可从任意线程取消；
- `TaskChain::set_cleanup`：在链的当前位置登记**中止清理**动作；只有它前面的步骤都成功、
  运行真正到达这里时才生效，链随后因取消或某一步失败而中止时按登记逆序执行（成功结束不执行）；
  清理是异步、尽力而为的，用于删掉写了一半的安装目录；
- `CommandTask`：跑外部程序，默认把它的输出**逐行变成日志事件**（Forge 的安装器
  processor 就是这么跑的）；取消时子进程被 `kill_on_drop` 杀掉。
- `WarningDecision`：任务可以发出「警告」事件，让前端决定继续还是中止。

### `dmcl-mcmeta` —— 元数据与地址

集中「描述游戏是什么」的东西，**不涉及把文件写到磁盘上**：

- **强类型文档**：版本 JSON、版本清单、资源索引、安装器档案、模组加载器版本列表。
  约定（文档会被写回，所以必须能往返）：
  - 每个结构体都带 `#[serde(flatten)] extra: JsonMap`，未知字段原样保留；
  - 「已知取值 + 兜底」的字符串枚举用 `string_enum!` 宏，且**必须保留 `Other(String)`**
    —— 否则回写时会丢掉不认识的取值；
  - 可选字段用 `#[serde(default, skip_serializing_if = "Option::is_none")]`，别写出 `null`；
- **下载源与 URL 模板**：见下文；
- **本地目录布局**：`versions/<name>/<name>.json` 之类的路径约定，以及读取版本 JSON
  （含 `inheritsFrom` 合并）。

模块按「域」划分：

```text
vanilla/    version.rs  manifest.rs  asset.rs  rules.rs  urls.rs
fabric/     类型 + 地址 + fetch
forge/      mod.rs + install_profile.rs
neoforge/   mod.rs + install_profile.rs
source / os / maven / merge / layout / json   —— 四个域共用的底座
```

加载器版本列表（Fabric / Forge / NeoForge）的 `fetch_*` 统一按版本**从新到旧**返回：
Fabric / NeoForge 比版本字符串，Forge 比 `build` 号；因此「最新」就是列表第一条，
调用方不再需要单独的 `latest_*` 取数函数。

`rules.rs` 按**标准 Mojang 语义**判定：空规则允许；否则按顺序评估、**最后一条命中的
规则的 action 决定结果**；一条都不命中则不允许。条件涵盖 `os.name` / `os.arch` /
`os.version`(正则) / `os.versionRange`(区间) / `features`。

### `dmcl-download` —— 下载

`DownloadTask` 是一个 `Task`，一次派发多个下载项，见下文「下载」。

### `dmcl-install` —— 安装

只做「把文件铺好」的事（写入 + 完整性检查），不做任何决策。内部把安装拆成：

- `plan`：**纯函数**的「这个版本需要哪些文件」规划（库、资源、客户端 jar；natives
  由 `plan_native_jars` 按 java 架构单独规划）；
- `files`：按规划构造 `DownloadTask` 并落盘；
- `integrity`：完整性检查（「完整吗？缺什么？」），缺失判定与下载共用同一个 `file_ok`；
- `vanilla`：原版元数据落盘、`launcher_profiles.json`；
- `loader/{fabric,forge,neoforge,common}`：各加载器自己的安装步骤（Fabric 直接合并
  profile；Forge / NeoForge 解出 `install_profile.json`、跑 processor）；
- `spec`：顶层任务链（`install_task` / `install_loader_task`）；
- `zip` / `io`：jar / zip 处理与安全写盘。

`install_task` 是**版本级原子**的：第一步先占用 `versions/<version_name>`，目录已存在
就直接报错（不会碰已有版本）；只有这一步成功后才走到 `set_cleanup` 登记删除动作，
之后任何一步失败或被取消，都会删掉本次新建的版本目录，不让半装版本出现在版本列表里。
共享的 `libraries/` / `assets/` 是内容校验过的缓存，不跟着回滚（下次安装会跳过已有文件）。

### `dmcl-launch` —— 启动

账号、Java 选择、命令行生成、启动与状态监控，见下文「启动与安装解耦」。

### `dmcl` —— 应用

唯一知道配置文件的一层：把 `dmcl.json` 读成 `Config`，翻译成各业务 crate 的
`*Options`，再分发命令。它同时是库（`dmcl`）与可执行文件（`bin/dmcl`），
方便将来复用到 GUI / 守护进程。

配置加载按项容错：坏掉或不认识的配置项回退默认值，问题汇总成 `ConfigError`；CLI 把它当
错误、逐条打印后退出，GUI 在启动后弹窗提示（确认写回修复后的配置，忽略则不写）并继续用
默认配置。

## 设计要点

### 1. URL 模板与下载源

从 Minecraft 拿到的每一份文档里，所有 url 都会被**归一成模板**：

```text
https://libraries.minecraft.net/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
        ↓
${MC_MAVEN}/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
```

`${MC_MAVEN}` 是一个「下载根」（`Root`），真正的地址由 `Mirror` 决定：

```rust
let source = DownloadSource::from_policy(MirrorPolicy::MirrorFirst);
source.resolve("${MC_MAVEN}/a/b.jar");
// → https://bmclapi2.bangbang93.com/maven/a/b.jar
```

要点：

- 归一化选**最长前缀**匹配（否则 bmclapi 的站点根会把 `/maven` 之类的更具体的根吞掉）；
- 一个根可以有**多个等价官域**（Forge 的 `files.minecraftforge.net/maven` 与
  `maven.minecraftforge.net` 是同一个仓库，都归一成 `${FORGE_MAVEN}`）；
- 识别不出的 host 原样保留 = 不可镜像；
- `UrlFields` trait 统一遍历文档里的 url 字段，`templatize_urls` / `resolve_urls`
  对任何文档类型都通用，新增字段不会漏改；
- 用哪种镜像由 `MirrorPolicy` 决定：`official-only`（仅官方）、`official-first`
  （官方优先，默认）、`mirror-first`（镜像优先）；各域取 meta 统一走 `fetch_meta(url)`
  —— 内部按镜像优先级展开候选、轮换重试（A B A …）直到用满重试次数，
  `fetch_json` 就是它的 JSON 版：`source.fetch_json("${MC_META}/mc/game/version_manifest.json").await?`。

写回磁盘的版本 JSON 会把模板解析回**官方**地址，因此 `.minecraft` 里的文档与官方启动器
完全通用。

### 2. 任务与进度

```rust
let chain = TaskChain::new("安装 1.20.1")
    .then(vanilla_metadata_task(..), "获取原版元数据")
    .then(install_fabric_task(..), "安装 Fabric")
    .then(game_files_task(..), "下载游戏文件");

// 事件流：一边跑一边打印（`()` 是这条链的初始输入）
chain.run_with((), |event| println!("{event}")).await?;
```

手动订阅可以自己控制消费节奏（GUI 场景）：

```rust
let mut chain = /* … */;
let mut rx = chain.subscribe();
let handle = tokio::spawn(async move { chain.run(()).await });
while let Ok(event) = rx.recv().await { /* 实时进度 */ }
handle.await??;
```

`async fn` 用装箱 future 表达，因此 `Box<dyn Task>` 可以直接当 trait 对象用，
**不需要 `async-trait` 依赖**。

每个操作都是一个 `TaskChain`；进度**只走 `TaskEvent` 事件流**，不用轮询、也不用全局状态来汇报，
**不引入全局可变配置**。

### 3. 下载

| 关注点       | 做法                                                                                                                                                                                         |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 多文件并发   | `JoinSet` + 全局 `Semaphore(max_connections)`                                                                                                                                                |
| 大文件分块   | 已知大小且超过阈值时，先发 `bytes=0-0` 探针确认服务器支持 `Range`（返回 206 + `Content-Range`），再按块并发；单文件并发块数另有限制                                                          |
| 不支持 Range | 自动退回单连接                                                                                                                                                                               |
| 完整性       | 先写 `<文件>.part`，校验通过才 `rename`；失败删掉半成品并重试                                                                                                                                |
| 校验算法     | `Checksum` 支持 **md5 / sha1 / sha256 / sha512**（Minecraft 文档给的是 sha1、Fabric 给的是 sha512；也接受 `sha256:…` 前缀或按摘要长度推断） |
| 进度         | 已知大小的项按字节、未知大小的项按数量（字节「下载多少算多少」），整体百分比按文件数占比加权；总量按规划大小预填，拿到服务器实际大小（`Content-Length` / `Content-Range`）后修订、失败项回滚；带节流 |
| 换源重试     | 每个下载项带一串**候选地址**（由 `dmcl-install` 按镜像策略展开）；第 `n` 次尝试轮换一个（A B A…），全失败才记为失败                                                                        |
| 断点续传     | 不做；失败即整份重来（最多 `max_attempts` 次）                                                                                                                                               |
| 失败处理     | 所有已排队项目都会跑完，失败项进 `DownloadReport`；默认「有失败就报错」，可用 `DownloadOptions::ignore_failures` 改成「只记录不报错」                                                        |

`dmcl-install` 与 `dmcl-download` 共用同一个 `reqwest::Client`（复用连接池）。

### 4. 安装：规划与下载分离

「这个版本由哪些文件组成」是**纯函数**（`dmcl-install::plan`）：输入版本 JSON + 上下文，
输出 `Vec<PlannedFile>`（目标路径 + URL 模板 + 校验值 + 大小）。下载那一半只管按规划
落盘。好处：

- 规划可以离线单测（喂真实版本 JSON，断言结果）；
- `dmcl-launch` 的完整性检查**复用同一批纯函数**，两边永远不会对不上。

natives 的**下载 + 解压**单独放进 `dmcl-install::natives_task`，不在 `install_task` 里：
它的分类器（`natives-linux-64` 之类）取决于运行游戏的 JVM 架构，只有启动时选好 java
才知道要下哪一个、解到哪个目录。所以完整性检查也**不覆盖 natives**，由启动流程自愈。

### 5. 启动与安装解耦

```rust
// 完整性检查在 dmcl-install：只回答"完整吗？缺什么？"，不下载任何东西
let status = dmcl_install::check_integrity(&install_options.plan(), version).await?;
if !status.is_complete() {
    // 由调用者决定：跑一次 dmcl-install 补全（已存在的文件会被跳过）
    game_files_task(install_options, version)
        .run_with((), |event| printer.handle(event))
        .await?;
}

// dmcl-launch 只负责拼参数、起进程
let mut launcher = GameLauncher::new(option).await?;
let mut monitor = launcher.launch_game().await?;
monitor.wait().await?;
```

- `check_integrity(plan, version)`（`dmcl-install`）返回 `IntegrityStatus { complete, missing, checked }`，
  每个缺失项带目标路径 + URL 模板 + 校验值 + 原因（缺失 / 内容不符）；它**不检查 natives**；
- 它**不下载任何东西**：修复由调用者跑 `dmcl-install` 的 `game_files_task`（已存在的文件会被跳过）；
- Java 选择不读全局配置：`find_java(roots)` 只返回结果，`select_java(required_major,
&choice, &configs)` 接收配置表；
- 一站式流程 `launch_chain(option, install)` 把「检查并补全 → 准备 natives → 生成参数 →
  启动」串成一条 `TaskChain<(), GameMonitor>`：**准备 natives** 先选好 java，再按这个 java 的
  os / arch 跑 `natives_task`（下载 + 解压到 `natives_dir`，目录名带 os / arch）；参数只在
  文件齐备、java 选好后才生成，且生成参数复用同一个 java，不再选第二次。

### 6. 加载器安装

三种加载器各有自己的安装步骤（`dmcl-install/src/loader/`），Forge 与 NeoForge 的
`InstallProfile` 也各有一份（`dmcl-mcmeta/{forge,neoforge}/install_profile.rs`），
便于以后各自演化：

- **Fabric** —— 取 loader profile，合并进原版版本 JSON；
- **Forge** —— 安装器分现代式（spec 0/1，包内有 `version.json`，`json` 字段指出位置）
  与经典式（≤1.11.2，profile 内联在 `versionInfo` 里、库在 `install.filePath` 指向的内嵌
  jar 中）两种；跑 processor 生成客户端 jar；
- **NeoForge** —— 只有现代式（spec 1）；**别给它加 Forge 的经典式**（`install` / `versionInfo`）分支。

`read_installer_version_json` 的查找顺序：`json` 指向的条目 →（仅 Forge）内联
`versionInfo` →（兜底）`install.filePath` 指向的内嵌 jar。

## 应用层与前端

`dmcl` 的职责只有三件：把配置翻译成各 crate 的 `*Options`、打印事件流、显示结果。
CLI 用 `clap` derive 定义命令，交互式菜单用 `dialoguer`。命令定义、本地化与分发在
`cli/mod.rs`，各命令实现按文件拆在 `cli/commands/` 下；跟终端打交道的部件
（`Prompter` trait + 中文主题）放在 `cli/interact.rs`，供 `prompt` / `config` / `java`
三处复用（测试里换成脚本化回答）。

配置相关的两处都走 serde 派生、不手写字段清单：

- `config` 命令的编辑器：把 `Config` 序列化成 `serde_json::Value`、按「点分键」逐项展示，
  标量单行编辑、对象 / 数组（`launch.java_configs`）走外部编辑器按 JSON 编辑；写回后
  整体反序列化校验；
- `show()` 也用同一份展开结果打印。

界面语言全部为中文，包括 clap 自动生成的小节标题（`用法：` / `位置参数：` / `选项：` /
`命令：`）与常见报错（`错误：缺少必填参数：…`）。命令名、参数名、配置键名保持英文
（它们是标识符，不是文案）。

## GTK4 前端

GTK4 + libadwaita 前端在 `crates/dmcl/src/gui/gtk4`，用 relm4，由 cargo feature `gtk4`
门控（默认关）。与 CLI 共用业务 crate：把 `Config` 翻译成 `*Options`、用 `Shell` 跑
`TaskChain`、把 `TaskEvent` 渲染成页面与任务窗口；业务 crate 不读配置。

文档按目录拆分，入口是
[crates/dmcl/src/gui/gtk4/README.md](crates/dmcl/src/gui/gtk4/README.md)
（整体架构、编译运行、GNOME HIG 约定与 GTK 测试方法）：

- [app/README.md](crates/dmcl/src/gui/gtk4/app/README.md)：根组件、`Shell`、`PageHost`、
  标题栏、toast；
- [app/tasks/README.md](crates/dmcl/src/gui/gtk4/app/tasks/README.md)：任务窗口、
  任务卡片、警告确认；
- [pages/home/README.md](crates/dmcl/src/gui/gtk4/pages/home/README.md)：主页、实例磁贴、
  启动 / 停止；
- [pages/download/README.md](crates/dmcl/src/gui/gtk4/pages/download/README.md)：下载域数据流、
  `InstanceSummary`、`VersionSelect`；
- [pages/download/custom/README.md](crates/dmcl/src/gui/gtk4/pages/download/custom/README.md)：
  「自定义」新建实例页的装配、联动与安装。

新增页面或组件时，细节写进对应目录的 `README.md`；本文件只保留整个前端的分层与入口。

## 测试策略

- **离线单测**（`cargo test --workspace`）：规划纯函数、URL 模板化 / 解析、规则判定、
  哈希与校验、任务链 / 事件转发、CLI 参数与交互流程（把「跟终端打交道」抽成接口后用
  脚本化回答测试）；
- **GTK4 前端**（`cargo test -p dmcl --features gtk4 --lib`）：不创建控件的纯逻辑单测
  （任务进度、版本过滤 / 适配等），不需要显示；界面验证用 Broadway + Chromium CDP，
  具体见 GTK4 前端入口文档。CI 在 Linux / Windows / macOS 跑 clippy / test / build；
- **联网冒烟测试**（`cargo test --workspace -- --ignored`）：真实版本清单 / 版本 JSON /
  资源索引 / 加载器列表的取数与模板化、真下载几个文件并验证「第二次全跳过」、
  完整性检查与命令行生成（断言不含未替换的模板）。需要能访问 Mojang / bmclapi。
