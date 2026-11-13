# dmcl

应用层：唯一知道配置文件存在的一层，同时提供命令行（`cli`）与 GTK4 图形界面（`gui`，
由 cargo feature `gtk4` 门控）。它把全局配置与实例覆盖读成 `Config` / `LaunchConfig`，
翻译成各业务 crate 的 `*Options`，再分发命令或驱动界面。

它是库（`dmcl`）与可执行文件（`bin/dmcl`）。在 workspace 内依赖全部业务 crate；
`relm4` / `sysinfo` / `notify` 只在 `gtk4` feature 下编译，`keyring` 只在应用层使用。

## 模块布局

```text
lib.rs         库入口
main.rs        可执行文件入口：先同步读配置，再决定走 GUI 还是 tokio + CLI
config/        mod.rs 整份文档读写与逐项容错
               account.rs / java.rs / root.rs / launch.rs / download.rs 各配置段
               instance.rs 实例 dmcl.json 的 instance 段（分类）
cli/           mod.rs 命令定义、本地化与分发；progress.rs 终端进度
               commands/ install / launch / list / login / search_java
gui/           mod.rs 前端分派；gtk4/ GTK4 + libadwaita 前端（见其 README）
```

## 设计意图

### 唯一的配置读取层

除 `dmcl` 外，没有任何 crate 会去读配置文件。这一层把配置翻译成各业务 crate 的 `*Options`，
业务 crate 因此可以被 GUI / TUI / 测试用自定义选项直接调用。配置的键、默认值与用户可见
含义在 `config/` 各段定义和根 README 里说明。

### 全局配置与实例覆盖

全局配置包含 `launch` / `accounts` / `javas` / `minecraft_roots` / `minecraft_root` /
`download`。保存时先写临时文件再改名覆盖；配置加载按项容错，坏掉或不认识的项回退默认值并
汇总成 `ConfigError`，CLI 当错误处理，GUI 备份原文件、写回修复结果并提示用户。微软凭据
存在系统钥匙串，配置里只写玩家名与 uuid。

实例配置位于 `versions/<version_name>/dmcl.json`，`launch` 以全局配置为底逐字段覆盖，
`instance.category` 记录分类。实例配置由 GUI 在新建实例成功后写入分类，其它字段不会被
自动改写；读取时的账户 / Java 失效项会回退并警告。

### 根目录与 Java 缓存

应用层取用根目录时才把「名字 → 路径」绝对化，读取与整体保存不会改写路径写法。`minecraft_root`
只决定启动时默认打开哪个根目录，CLI 的 `--minecraft-root` 只影响本次运行。

`search-java` 与首选项里的搜索会扫描本机，按路径去重后把路径 / 大版本 / 架构合并进全局
`javas`；之后启动只读这份缓存。CLI 启动只认配置，不会扫描或下载；GUI 没有可用条目或没有
匹配版本时会先扫描本机，仍没有则下载 Mojang JRE 并写回配置。安装器 processor 使用的 Java
与启动 Java 分开，由 `download.java` 决定。

### CLI

CLI 用 `clap` derive 定义命令，命令定义、本地化与分发在 `cli/mod.rs`，各命令实现按文件
拆在 `cli/commands/` 下。界面语言为中文，命令名、参数名、配置键保持英文。不带子命令时，
编译了 `gtk4` 就打开 GUI，否则打印帮助并以非零状态退出；显式的 `dmcl gui` 在前端不可用时
直接报错。

### GUI

GTK4 + libadwaita 前端在 `src/gui/gtk4`，用 relm4，由 cargo feature `gtk4` 门控。与 CLI
共用业务 crate：把 `Config` 翻译成 `*Options`、用 `Shell` 跑 `TaskGraph`、把 `TaskEvent`
渲染成页面与任务窗口；业务 crate 不读配置。每个根目录一个常驻主页，关闭标签只影响显示，
运行状态、启动链与游戏监视继续保留。整个前端共用一份 `ConfigStore`。

前端的整体分层、relm4 约定与组件文档见
[crates/dmcl/src/gui/gtk4/README.md](src/gui/gtk4/README.md)。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 使用方式、命令与配置键：[README.md](../../README.md)
- 业务 crate 的职责与设计：各 `crates/*/README.md`
