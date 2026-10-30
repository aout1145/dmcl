# AGENTS.md

给在本仓库里工作的编码代理（以及人）的约定。动手前先读
[README.md](README.md)（功能 / 构建 / 用法）和 [ARCHITECTURE.md](ARCHITECTURE.md)（总体结构 /
依赖 / 文档索引），再按需要看对应 crate 的 `README.md`（职责 / 模块布局 / 设计细节）。
改 GTK4 前端时**必须遵循** [gtk4/README.md](crates/dmcl/src/gui/gtk4/README.md) 里的
relm4 / `view!` 约定，尤其是「状态驱动：状态放模型，界面用 `view!` 刷新」一节；那里规定的
声明式写法、`FactoryComponent` / `FactoryVecDeque` 用法和命令式 GTK 的边界都同样有效。

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

`dmcl`：非官方 Minecraft 启动器，Rust。一个 cargo workspace —— crate 划分、依赖关系、
各 crate 职责见 [ARCHITECTURE.md](ARCHITECTURE.md) 与各 crate 的 `README.md`。

Rust edition 2021，MSRV **1.95**，许可 GPL-3.0-or-later。

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

## 公开 API

不追求公开 API（如 `dmcl-task` 的 `TaskEvent`、`Task`、各 `*Options`）绝对稳定。只要能让设计更
清晰，就改动它 —— 新增 / 调整变体、字段、方法、签名都可以，把调用方一并改掉即可；不要为了
「不动 API」把设计拧成别扭的样子。改动仍按「汇报与确认」办理：讲清现状、可选项、代价与取舍、
以及推荐，等用户拍板。

## 代码约定

模块级的设计约定（`dmcl-mcmeta` 的结构体、加载器目录布局、`rules` 语义等）见对应 crate 的
`README.md`（例如 [crates/dmcl-mcmeta/README.md](crates/dmcl-mcmeta/README.md)）；
下面是通用编码约定。

- 异步 trait 用**装箱 future**（`dmcl_task::BoxFuture`）表达，**不要引入 `async-trait`**。
- **不要引入日志框架**：直接用 `println!` / `eprintln!`。
- 改了函数签名 / 实现 / 字段后，**把所有调用点一并改掉**；不要为了少改几处、或为了向后兼容，给
  接口加**隐式的默认行为**（给新参数一个默认值、漏字段时用默认值补齐、留一个只转发旧签名的包装
  / 重载…… 都不行）。宁可多改几个调用点，也别让接口藏一层没说清的默认语义。
- 设计改过之后（例如某块逻辑挪到了别的 crate / 模块），**不要为了兼容留下别扭的转发 / 包装**：
  职责已经搬走，就让调用方直接用新入口，旧的转发顺手删掉。
- 字段都 `pub` 的数据结构直接用结构体字面量即可。
- **避免只转发单个字段的包装函数 / 方法**：`fn is_running(&self) -> bool { self.state == Running }`
  这类（只读、只写、只做一次比较都算）调用点直接读 / 写字段或写表达式；只有封装了额外语义、
  多处复用或需要维护不变式时才提取。
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
- **不写显而易见的测试**：不要把步骤名、字段默认值、常量映射这类看一眼实现就能确认的
  内容原样抄成断言；测试应覆盖行为、边界和容易回归的逻辑。

## 提交

- message 用中文，格式 `type(scope): 摘要`：
  `feat` / `fix` / `refactor` / `docs` / `chore` …，scope 一般是 crate 名或模块名
  （如 `fix(launch): …`、`refactor(mcmeta): …`）。
- 一次提交只做一件事；重构与行为改动尽量分开提交。
- 只有**单纯的文档更新**才单独用 `docs` 提交；实现新功能 / 重构时同步修改的文档
  （README、ARCHITECTURE、crate 文档等）必须和代码放在同一个提交里，不要拆成单独的
  `docs` 提交。
- **提交前先请用户 review 并得到同意**（见上文「汇报与确认」）；不要自作主张地 commit。
- 提交前确保 fmt / clippy / test 全过。

## 文档分工

- [README.md](README.md)：面向使用者 —— 功能、构建、快速开始、命令、配置文件。
- [ARCHITECTURE.md](ARCHITECTURE.md)：面向开发者 —— workspace 的总体目录结构、依赖关系、
  crate 不读配置的硬规则、各 crate 文档索引、总体测试策略。
- `crates/*/README.md`：面向开发者 —— 各 crate 的职责、依赖、模块布局与设计要点。
- 架构相关的新设计写进对应 crate 的 `README.md`；只有跨 crate 的总体结构 / 依赖变化才改
  `ARCHITECTURE.md`。代码注释解释「为什么」，不要复述显而易见的「是什么」。
- 文档和注释只写**当前**行为；不要提已经被删掉 / 不存在的旧行为（「不再…」「之前…」
  这类历史对比），也不要为了对比去解释旧实现。

## 许可

GPL-3.0-or-later；新增文件沿用同一许可。
