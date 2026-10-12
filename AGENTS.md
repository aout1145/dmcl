# AGENTS.md

给在本仓库里工作的编码代理（以及人）的约定。动手前先读
[README.md](README.md)（功能 / 构建 / 用法）和 [ARCHITECTURE.md](ARCHITECTURE.md)（结构 / 依赖 / 设计）。

## 汇报与确认（最重要）

- **需求不确定或含糊时，先提问，不要凭猜测开工。** 宁可多问一句，也不要按自己的理解实现
  一个可能不对的东西。
- **重要的设计抉择必须先汇报、由用户拍板**，例如：新增或调整 crate / 依赖方向、公开 API
  与数据结构、跨 crate 的约定、会改变行为或降低兼容性的实现、引入新依赖、牵涉多个模块的
  重构。汇报时讲清楚：**现状、可选项、每个选项的代价与取舍、以及你的推荐和理由**，
  等用户确认后再动手。
- 不要擅自扩大范围，也不要顺手重构与本次需求无关的代码；发现相关问题就**报告**，不要自己决定。
- **提交前先请用户审阅（review）。** 每完成一处改动（或一个阶段）就把 diff / 结果摆出来请用户
  过目，得到同意后再 `git commit`；**不要自行提交**。

## 这是什么

`dmcl`：非官方 Minecraft 启动器，Rust。一个 cargo workspace，`crates/` 下六个 crate：

| crate           | 职责                                          |
| --------------- | --------------------------------------------- |
| `dmcl-task`     | 通用任务链 + 进度事件流（与 MC 无关）         |
| `dmcl-mcmeta`   | MC 元数据文档、下载源、URL 模板、本地目录布局 |
| `dmcl-download` | 并发 / 分块下载（实现 `dmcl-task` 的 `Task`） |
| `dmcl-install`  | 原版 + Fabric / Forge / NeoForge 安装         |
| `dmcl-launch`   | 账号、java、命令行生成、启动、完整性检查      |
| `dmcl`          | 应用层：config + cli（唯一读配置的地方）      |

Rust edition 2021，MSRV **1.75**，许可 GPL-3.0-or-later。

## 交流与文案

- 面向用户的文案（帮助、报错、进度、交互菜单）一律**中文**；命令名 / 参数名 / 配置键
  等**标识符保持英文**。
- 代码注释、文档、commit message 用**中文**；回复也用中文。
- 标识符（类型、函数、字段、模块）用英文。

## 常用命令

```bash
cargo build --release                              # 构建
cargo test --workspace                             # 离线单测
cargo test --workspace -- --ignored                # 联网冒烟测试（要能访问 Mojang / bmclapi）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo doc --no-deps --workspace
```

改完代码至少跑：`cargo fmt --all --check`、上面的 clippy、`cargo test --workspace`。
CI（GitHub Actions）在 **ubuntu 和 windows** 上都跑这四项，外加 `cargo build --release`，
所以**两边都得过、且 clippy 零警告**。

## 架构约束（硬性，别破坏）

1. **除 `dmcl` 外，任何 crate 都不读配置文件**。所有外部输入由调用者通过各自的
   `*Options` 传入（`DownloadSource` / `DownloadOptions` / `InstallOptions` / `LaunchOption`）。
2. **依赖方向**：`dmcl` → {install, launch, download, mcmeta, task}；
   `dmcl-launch` → install / download / mcmeta / task；`dmcl-install` → download / mcmeta / task；
   `dmcl-download` → task；`dmcl-task` 与 `dmcl-mcmeta` 是叶子、互不依赖。
   不要引入反向或环形依赖，也不要为了「两个 crate 都要用」随手新建 common crate
   （就近拆分，或各留一份小副本）。
3. 业务逻辑都在库里；`dmcl` 只做「读配置 → 解析参数 → 分发」。
4. 每个操作是一个 `dmcl_task::TaskChain`，进度走 `TaskEvent` 事件流。
   别用轮询或全局状态来汇报进度；别引入全局可变配置（原实现有过 `static RwLock<Config>`，已移除）。

## 代码约定

- **强类型文档可往返**：`dmcl-mcmeta` 里凡是会被写回的文档结构体，都要带
  `#[serde(flatten)] extra: JsonMap`，未知字段原样保留。
- 「已知取值 + 兜底」的字符串枚举用 `string_enum!` 宏，且**必须保留 `Other(String)`**
  —— 否则回写时会丢掉不认识的取值。
- 可选字段用 `#[serde(default, skip_serializing_if = "Option::is_none")]`，别写出 `null`。
- `rules` 按**标准 Mojang 语义**判定（空规则允许；按顺序评估、最后一条命中者的 action 胜；
  一条都不命中则不允许）。改之前先看 `vanilla/rules.rs`。
- 加载器按域分文件（`mcmeta/{vanilla,fabric,forge,neoforge}`、
  `install/loader/{fabric,forge,neoforge,common}`）。Forge 与 NeoForge 的 `install_profile`
  **各一份、允许分化**：NeoForge 只有现代式（spec 1）安装器，别给它加 Forge 的经典式
  （`install` / `versionInfo`）分支。
- 异步 trait 用**装箱 future**（`dmcl_task::BoxFuture`）表达，**不要引入 `async-trait`**。
- **不要引入日志框架**：直接用 `println!` / `eprintln!`。
- 依赖版本统一写在 workspace 根 `Cargo.toml` 的 `[workspace.dependencies]`，成员用
  `xxx.workspace = true`，别在成员里另写版本号。

## 测试

- 离线单测是主体；需要联网的测试标 `#[ignore]`，用 `-- --ignored` 单独跑。
- **必须同时在 Linux 和 Windows 上成立**：
  - 平台相关的命令或路径用 `#[cfg(unix)]` / `#[cfg(windows)]` / `target_os` 门控。
    若**整个测试模块**都只适用于某平台，就门控**模块本身**（`#[cfg(all(test, unix))]`），
    别只门控单个测试 —— 否则另一边会留下未使用的 `use`，而 CI 是 `-D warnings`。
  - 别写死 unix 命令（`sleep` / `sh -c`）或假设信号语义（如 `ExitStatus::code() == None`）。
    用平台等价物，并断言平台无关的性质（例如 `!status.success()`）。
  - 路径比较注意：`PathBuf` 相等是**按 components** 的；而走 `absolute()` / `Config::load`
    的路径在 Windows 会被补上盘符，所以别拿字面量 `/tmp/...` 去比，用 `std::env::temp_dir()`
    或与 `absolute(...)` 比。

## 提交

- message 用中文，格式 `type(scope): 摘要`：
  `feat` / `fix` / `refactor` / `docs` / `chore` …，scope 一般是 crate 名或模块名
  （如 `fix(launch): …`、`refactor(mcmeta): …`）。
- 一次提交只做一件事；重构与行为改动尽量分开提交。
- **提交前先请用户 review 并得到同意**（见上文「汇报与确认」）；不要自作主张地 commit。
- 提交前确保 fmt / clippy / test 全过。

## 文档分工

- [README.md](README.md)：面向使用者 —— 功能、构建、快速开始、命令、配置文件。
- [ARCHITECTURE.md](ARCHITECTURE.md)：面向开发者 —— 目录结构、依赖关系、各 crate 职责、
  设计要点（URL 模板、任务 / 下载 / 安装规划 / 启动解耦）、GUI 扩展点、测试策略。
- 架构相关的新设计写进 ARCHITECTURE.md，别堆进 README。代码注释解释「为什么」，
  不要复述显而易见的「是什么」。

## 许可

GPL-3.0-or-later；新增文件沿用同一许可。
