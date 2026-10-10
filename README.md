# dmcl (Rust)

非官方 Minecraft 启动器 —— 由原 D 语言版本 `../dmcl` 迁移而来，并按职责拆成了
**一个 cargo workspace + 六个 crate**。

逻辑与指令语义保持与原版一致（在 MIGRATION.md 里逐条列出有意保留的怪癖与少量修正），
但内部结构做了彻底重构：不再有全局配置、不再有万能 `utils.rs`、启动与安装解耦。

```text
dmcl-rs/
├── Cargo.toml                 # 纯 workspace（members + 统一的依赖版本）
└── crates/
    ├── dmcl-task/             # 通用任务链 + 进度事件流（与 MC 无关）
    ├── dmcl-mcmeta/           # MC 元数据文档、下载源、URL 模板、本地目录布局
    ├── dmcl-download/         # 并发 / 分块下载（实现 dmcl-task 的 Task）
    ├── dmcl-install/          # 原版 + Fabric/Forge/NeoForge 安装
    ├── dmcl-launch/           # 账号、java、命令行生成、启动、完整性检查
    └── dmcl/                  # 应用：config + cli（未来在这里加 gui）
```

## 依赖关系

```text
        dmcl  ──────────────┬──────────────┬───────────────┐
          │                 │              │               │
          ▼                 ▼              │               │
    dmcl-install        dmcl-launch        │               │
      │      │            │      │        │               │
      │      └────► dmcl-download ◄────────┘              │
      │                    │                              │
      ▼                    ▼                              ▼
  dmcl-mcmeta  ◄──────── dmcl-task  ◄─────────────────────┘
```

* `dmcl-task` / `dmcl-mcmeta` 是最底层的两个 crate，互不依赖；
* `dmcl-download` 依赖 task（以及 mcmeta 的 sha1 助手）；
* `dmcl-install` 依赖 mcmeta + task + download；
* `dmcl-launch` 依赖 mcmeta + task + download + install（复用同一套"这个版本由哪些文件
  组成"的纯函数做完整性检查）；
* 只有 `dmcl` 知道配置文件的存在。

## 一条硬规则：crate 不读配置

除 `dmcl` 本身外，**没有任何 crate 会去读配置文件**。所有外部输入都由调用者通过
结构体传入：

| crate | 入参 |
| --- | --- |
| `dmcl-task` | 无（纯任务抽象） |
| `dmcl-mcmeta` | `DownloadSource`（镜像 + http 客户端）、URL 模板字符串 |
| `dmcl-download` | `DownloadOptions`（并发数、分块阈值 / 大小、重试次数、UA） |
| `dmcl-install` | `InstallOptions`（根目录、下载源、下载策略、是否跳过音乐、natives 位宽、java 提供者） |
| `dmcl-launch` | `LaunchOption`（账号、根目录、版本名、窗口 / 内存、java 选择与候选表） |
| `dmcl` | `dmcl.json`（唯一读写配置的地方） |

## 主要设计

### 1. 下载源与 URL 模板（`dmcl-mcmeta`）

从 Minecraft 拿到的每一份文档里，所有 url 都会被**归一成模板**：

```text
https://libraries.minecraft.net/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
        ↓
${MC_MAVEN}/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
```

`${MC_MAVEN}` 是一个「下载根」（`Root`），真正的地址由 `Mirror` 决定：

```rust
let source = DownloadSource::new(Mirror::Bmclapi);
source.resolve("${MC_MAVEN}/a/b.jar");
// → https://bmclapi2.bangbang93.com/maven/a/b.jar
```

识别不出来的第三方仓库地址原样保留（即"不可镜像"，与原实现的镜像表行为一致）。
`DownloadSource` 还可以带**镜像优先级列表**，前一个失败自动试下一个，并且能直接按模板
发请求：`source.get_json(VERSION_MANIFEST).await?`。

写回磁盘的版本 JSON 会把模板解析回**官方**地址，因此 `.minecraft` 里的文档与官方
启动器完全通用。

### 2. 任务与进度（`dmcl-task`）

```rust
let chain = TaskChain::new("安装 1.20.1")
    .then(vanilla_metadata_task(..), "获取原版元数据")
    .then(install_fabric_task(..), "安装 Fabric")
    .then(game_files_task(..), "下载游戏文件");

// 事件流：一个任务可以上报任意多次进度，也可以被多个前端同时消费
chain.run_with(|event| println!("{event}")).await?;
```

* `Task` trait：`fn run(&self, ctx: TaskContext) -> BoxFuture<Result<()>>`；
* `TaskChain`：`then(任务, 显示名)` 串联，**任一步失败即中止后续**并把错误原样返回；
* 进度用 `broadcast` 事件流（`TaskEvent`：`Started` / `StepStarted` / `Progress` /
  `StepFinished` / `Failed` / `Finished`），可以边跑边消费；链套链时子链的进度会转发到父链；
* `CancelHandle` 可以从任意线程取消（GUI 的"取消"按钮）；
* 附带 `CommandTask`：跑外部程序，并把它的输出**逐行变成进度事件**；
  Forge 的安装器 processor 就是这么跑的。

### 3. 下载（`dmcl-download`）

`DownloadTask` 一次派发多个下载项：

* 多文件并发，连接数由 `max_connections` 通过信号量限制；
* 已知大小且超过阈值的文件先探测服务器是否支持 `Range`，支持就**分块并发**下载
  （单文件块数另有限制），不支持就退回单连接；
* 下载完先写 `<文件>.part`，**sha1 校验通过才改名**，所以中断 / 失败不会留下坏文件；
* 进度是跨文件累计字节数，带节流（终端上原地刷新，日志里每 5% 一行）；
* 失败项汇总在 `DownloadReport` 里，安装流程会因此报错（不会"少文件也算成功"）。

### 4. 启动与安装解耦（`dmcl-launch` ↔ `dmcl-install`）

```rust
let mut launcher = GameLauncher::new(option).await?;
let status = launcher.check_integrity().await?;   // 只回答"完整吗？缺什么？"
if !status.is_complete() {
    // 由调用者决定：跑一次 dmcl-install 补全（已存在的文件会被跳过）
    game_files_task(install_options, version).run_with(printer).await?;
}
let mut monitor = launcher.launch_game().await?;
monitor.wait().await?;
```

完整性检查复用 `dmcl-install` 的规划函数（纯函数），因此两边永远不会对不上；
natives 的解压也归安装流程（启动只引用目录），`dmcl-launch` 因此不需要 zip 代码。

## 构建与运行

```bash
cargo build --release
./target/release/dmcl --help
```

调试构建会打印 `!!! DMCL SNAPSHOT !!!`（与原 `debug` 块一致）。

## 命令

| 命令 | 说明 |
| --- | --- |
| `launch <version-name>` | 检查完整性 → 缺文件则补全 → 启动 |
| `install <mc-version-id> <version-name> [--forge] [--neoforge] [--fabric]` | 安装原版（可选装模组加载器） |
| `install … [--forge-version=…] [--neoforge-version=…] [--fabric-version=…]` | 指定加载器版本（留空取最新） |
| `list-mc [--type=release,snapshot,…]` | 列出 Minecraft 版本 |
| `list-forge <mc-version-id>` | 列出 Forge 版本 |
| `list-neoforge <mc-version-id>` | 列出 NeoForge 版本 |
| `list-fabric <mc-version-id>` | 列出 Fabric 版本 |
| `install-forge <version-name> <forge-version-id>` | 在指定版本上安装 Forge |
| `install-neoforge <version-name> <neoforge-version-id>` | 在指定版本上安装 NeoForge |
| `search-java` | 自动搜索本机 Java 并写入配置 |
| `prompt` | 交互式菜单（默认命令） |
| `version` / `config` / `edit-config` | 版本 / 查看配置 / 配置文件位置 |

全局选项：`--java <PATH>`（同时写入命名 java 配置）、`--mirror <official|bmclapi>`。
不带子命令时执行配置里的 `cli.default_command`（默认 `prompt`）。

## 配置文件

位置：`dirs::config_dir()/dmcl.json`
（Linux `$XDG_CONFIG_HOME` 或 `~/.config`，Windows `%APPDATA%`，macOS `~/Library/Application Support`）。

键名与原 D 版本兼容（原来的 `launch_java` 就是现在的 `launch.java`）：

```json
{
  "launch": {
    "java_configs": { "openjdk-64-bit-17": "/usr/lib/jvm/java-17/bin/java" },
    "java": "autoselect",
    "minecraft_root_path": "/home/me/.minecraft",
    "window_width": 854,
    "window_height": 480,
    "min_memory": 512,
    "max_memory": 2048,
    "custom_info": "dmcl",
    "additional_jvm": "-Dfml.ignoreInvalidMinecraftCertificates=True -Dfml.ignorePatchDiscrepancies=True",
    "additional_game": ""
  },
  "account":  { "online_mode": false, "username": "player" },
  "download": {
    "max_tasks": 64,
    "i_dont_need_music": false,
    "mirror": "official",
    "chunk_threshold_mib": 8,
    "chunk_size_mib": 4,
    "max_chunks_per_file": 8,
    "max_attempts": 3
  },
  "cli": { "default_command": "prompt" }
}
```

缺失的键沿用默认值。`download` 段里的分块参数是本轮新加的（见 MIGRATION.md）。

## 测试

```bash
cargo test --workspace                          # 离线单测
cargo test --workspace -- --ignored             # 联网冒烟测试（需要能访问 Mojang / bmclapi）
cargo clippy --workspace --all-targets          # 零警告
cargo fmt --all --check
```

## 为 GUI 预留的扩展点

* 业务逻辑全在 `dmcl-*` 库里，`main.rs` 只做"读配置 → 解析参数 → 分发"；
* 每个操作都是 `TaskChain`，GUI 可以 `subscribe()` 事件流自己渲染进度，
  也可以用 `CancelHandle` 取消；
* `dmcl-mcmeta` / `dmcl-download` / `dmcl-install` / `dmcl-launch` 都不读配置，
  可以直接用自定义的选项结构调用；
* `GameMonitor` 支持 `kill()` / `try_status()`，可实现"停止游戏"。

## 许可

GPL-3.0-or-later，与原项目一致。
