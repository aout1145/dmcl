# 架构设计

本文只讲 workspace 的总体结构：有哪些 crate、彼此如何依赖、共同遵守的硬规则，以及
文档入口。各 crate 的职责、模块布局与设计细节在对应目录的 `README.md` 里；使用方式与
构建见 [README.md](README.md)。

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
    ├── dmcl-install/          # 原版 + Fabric/Forge/NeoForge/OptiFine 安装、完整性检查
    ├── dmcl-launch/           # 账号、java、命令行生成、启动
    └── dmcl/                  # 应用：config + cli + gui（GTK4）
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
| `dmcl-launch`   | `LaunchOption`（账户登录源、根目录、版本名、窗口 / 内存（可自动）、`JavaSelector`）   |
| `dmcl`          | `dmcl.json`（唯一读写配置的地方）                                                     |

这样每个业务 crate 都能被 GUI / TUI / 测试用自定义选项直接调用。

## crate 文档

各 crate 的 README 说明它的职责、依赖、模块布局和设计要点：

| crate | 一句话职责 | 文档 |
| --- | --- | --- |
| `dmcl-task` | 通用任务链、进度 / 日志事件、取消与回滚 | [crates/dmcl-task/README.md](crates/dmcl-task/README.md) |
| `dmcl-mcmeta` | 强类型元数据文档、URL 模板与下载源、目录布局 | [crates/dmcl-mcmeta/README.md](crates/dmcl-mcmeta/README.md) |
| `dmcl-download` | 多文件并发 + 大文件分块的下载任务 | [crates/dmcl-download/README.md](crates/dmcl-download/README.md) |
| `dmcl-install` | 版本规划、落盘、完整性检查、加载器 / OptiFine 安装 | [crates/dmcl-install/README.md](crates/dmcl-install/README.md) |
| `dmcl-launch` | 账户、Java 选择、命令行生成、启动与监控 | [crates/dmcl-launch/README.md](crates/dmcl-launch/README.md) |
| `dmcl` | 配置、CLI 与 GTK4 前端 | [crates/dmcl/README.md](crates/dmcl/README.md) |

GTK4 前端文档按目录拆分，入口是
[crates/dmcl/src/gui/gtk4/README.md](crates/dmcl/src/gui/gtk4/README.md)（整体架构、编译
运行、GNOME HIG 约定与 GTK 测试方法）。

新增或调整设计时，细节写进对应 crate 的 README；只有跨 crate 的总体结构 / 依赖变化才改
本文件。

## 测试策略

- **离线单测**（`cargo test --workspace`）：规划纯函数、URL 模板化 / 解析、规则判定、
  哈希与校验、任务链 / 事件转发、CLI 参数与配置加载 / 实例覆盖；
- **GTK4 前端**（`cargo test -p dmcl --features gtk4 --lib`）：不创建控件的纯逻辑单测
  （任务进度、版本过滤 / 适配等），不需要显示；界面验证用 Broadway + Chromium CDP，
  具体见 GTK4 前端入口文档。CI 在 Linux / Windows / macOS 跑 clippy / test / build；
- **联网冒烟测试**（`cargo test --workspace -- --ignored`）：真实版本清单 / 版本 JSON /
  资源索引 / 加载器列表的取数与模板化、真下载几个文件并验证「第二次全跳过」、
  完整性检查与命令行生成（断言不含未替换的模板）。需要能访问 Mojang / bmclapi。
