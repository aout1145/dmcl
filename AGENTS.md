# AGENTS.md

给在本仓库里工作的编码代理（以及人）。动手前先读
[README.md](README.md)（功能 / 构建 / 用法）、[ARCHITECTURE.md](ARCHITECTURE.md)（总体结构 /
依赖 / 文档索引），再按需看对应 crate 的 `README.md`（职责 / 文件布局 / 设计意图）。
改 GTK4 前端时**必须遵循** [gtk4/README.md](crates/dmcl/src/gui/gtk4/README.md) 的 relm4 /
`view!` 约定，尤其是「状态驱动：状态放模型，界面用 `view!` 刷新」一节。

`dmcl`：非官方 Minecraft 启动器，Rust workspace。edition 2021，MSRV **1.95**，
GPL-3.0-or-later（新增文件沿用同一许可）。

## 汇报与确认（最重要）

- **需求含糊时立即提问，不要猜测意图或过度思考。**
- **重要设计抉择先汇报、由用户拍板**：crate / 依赖方向、公开 API 与数据结构、跨 crate 约定、
  行为或兼容性变化、新依赖、多模块重构。汇报时讲清现状、可选项、代价与取舍、推荐及理由。
- 不擅自扩大范围，也不顺手重构无关代码；发现问题只报告，不自行决定。
- 每完成一处改动（或一个阶段）就把 diff / 结果交给用户 review，得到同意后再 commit；
  **不要自行提交**。

## 交流与文案

- 用户文案用中文，标识符用英文；注释、文档、commit message 和回复用中文。
- 表达简洁、准确、正式，不堆砌辞藻。

## 常用命令

```bash
cargo build --release
cargo test --workspace
cargo test --workspace -- --ignored   # 联网冒烟测试
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo doc --no-deps --workspace
```

改完至少跑 fmt、clippy、test。默认 CI 在 **Linux / Windows / macOS** 上跑 fmt、clippy、test
和 `cargo build --workspace --release`；三边都要过，且 clippy 零警告。

## 工具使用

- 调用工具时只取需要的片段，用过滤 / 限量，避免整文件、全量日志和大段无关内容。

## 代码约定

- 异步 trait 用 `#[async_trait]`，不要手写装箱 future。
- 不引入日志框架，直接用 `println!` / `eprintln!`。
- 改签名、实现或字段，或职责搬走时，改掉所有调用点；不留隐式默认值、兼容包装或旧转发入口。
- 字段全 `pub` 的结构体用结构体字面量。
- 只转发单个字段的包装函数不要提取，只有封装额外语义、多处复用或维护不变式时才提取。
- 依赖版本写在 workspace 根 `[workspace.dependencies]`，成员用 `xxx.workspace = true`。
- 不要用 `.clone()`、`Rc<RefCell<T>>` 等方式绕过借用检查器。
- 不要为逻辑上不可能出现的情况添加防御性检查。
- 代码保持精简：死代码、无用抽象、临时调试代码和多余包装该删就删。
- 公开 API 不追求绝对稳定，能简化设计就改，调用方一起改。

## 测试

- 离线单测为主；联网测试标 `#[ignore]`，用 `-- --ignored` 单独跑。
- 必须同时在 **Linux / Windows / macOS** 成立：平台相关代码用 `#[cfg]` 门控，整个测试模块
  只适用某平台时门控模块本身（否则其它平台会留下未使用的 `use`，而 CI 是 `-D warnings`）；
  不写死 `sleep` / `sh -c` 等 unix 命令，不假设信号语义，断言平台无关的性质。
- 路径比较注意 components 与 Windows 盘符：别拿字面量 `/tmp/...` 比较，用
  `std::env::temp_dir()` 或与 `absolute(...)` 比。
- 不写显而易见的测试：不把步骤名、默认值、常量映射原样抄成断言；覆盖行为、边界和易回归
  逻辑。

## 文档

- 根 README 面向使用者；ARCHITECTURE 写总体结构与依赖；各 crate README 写职责、文件布局与
  设计意图。跨 crate 的结构 / 依赖变化才改 ARCHITECTURE。
- 只写**当前**行为，不提已删除的旧行为。
- 代码注释解释「为什么」，不复述显而易见的「是什么」。
- 只写设计：文件 / 模块功能、总体意图和必须遵守的约定；不写控件尺寸、CSS、消息字段清单、
  算法步骤、测试清单、调试手法。实现约定要保留；根 README 的使用说明除外。

## 提交

- message 用中文，格式 `type(scope): 摘要`；scope 一般是 crate 名或模块名。
- 一次提交只做一件事，重构与行为改动尽量分开。
- 实现 / 重构同步修改的文档必须和代码同一提交；只有单纯文档更新才单独用 `docs`。
