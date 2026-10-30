# dmcl

应用层：唯一知道配置文件存在的一层，同时提供命令行（`cli`）与 GTK4 图形界面（`gui`，
由 cargo feature `gtk4` 门控）。它把全局配置与实例覆盖读成 `Config` / `LaunchConfig`，
翻译成各业务 crate 的 `*Options`，再分发命令或驱动界面。

它同时是库（`dmcl`）与可执行文件（`bin/dmcl`），方便将来复用到 GUI / 守护进程。
在 workspace 内依赖全部业务 crate（`dmcl-task` / `dmcl-mcmeta` / `dmcl-download` /
`dmcl-install` / `dmcl-launch`）；`relm4` / `sysinfo` / `notify` 只在 `gtk4` feature 下
编译。

## 模块布局

```text
lib.rs         库入口
main.rs        可执行文件入口：先同步读配置，再决定走 GUI 还是 tokio + CLI
build_info.rs  编译期信息（构建时间等）
config/        mod.rs 整份文档读写与逐项容错
               account.rs / java.rs / root.rs / launch.rs / download.rs 各配置段
cli/           mod.rs 命令定义、本地化与分发；progress.rs 终端进度
               commands/ install / launch / list / search_java / version
gui/           mod.rs 前端分派；gtk4/ GTK4 + libadwaita 前端（见其 README）
```

## 设计要点

### 唯一的配置读取层

除 `dmcl` 外，没有任何 crate 会去读配置文件。这一层把配置翻译成各业务 crate 的
`*Options`：

| crate           | 入参                                                                                  |
| --------------- | ------------------------------------------------------------------------------------- |
| `dmcl-task`     | 无（纯任务抽象）                                                                      |
| `dmcl-mcmeta`   | `DownloadSource`（镜像 + http 客户端）、URL 模板字符串                                |
| `dmcl-download` | `DownloadOptions`（并发数、分块阈值 / 大小、单文件重试次数、失败是否忽略、UA）        |
| `dmcl-install`  | `InstallOptions`（根目录、下载源、下载策略、是否跳过音乐、java 提供者）             |
| `dmcl-launch`   | `LaunchOption`（账户登录源、根目录、版本名、窗口 / 内存（可自动）、`JavaSelector`）   |
| `dmcl`          | `dmcl.json`（唯一读写配置的地方）                                                     |

配置的**键、默认值与用户可见含义**见仓库根 [README.md](../../README.md#配置文件)；
这里只讲应用层的组织方式。

### 全局配置

全局配置位于 `<平台配置目录>/dmcl/dmcl.json`（Linux `$XDG_CONFIG_HOME` 或 `~/.config`；
Windows `%APPDATA%`；macOS `~/Library/Application Support`），包含 `launch` / `accounts` /
`javas` / `minecraft_roots` / `minecraft_root` / `download`。

- 保存时先写同目录的 `dmcl.<pid>.json`，再 rename 覆盖目标，读者不会看到半截 JSON；
- Unix 下配置目录收紧为 `0700`、配置文件与 `dmcl.old.json` 备份为 `0600`；Windows 依赖
  `%APPDATA%` 用户目录自带的 ACL；
- 动态表（`accounts` / `javas` / `minecraft_roots`）都保持配置文件里的书写顺序；
- 配置加载按项容错：坏掉或不认识的配置项回退默认值，问题汇总成 `ConfigError`；CLI 把它
  当错误、逐条打印后退出。GUI 在启动后自动把修复后的全局配置写回文件，先把原文件备份为
  同目录的 `dmcl.old.json`，再弹窗说明修复结果并继续使用修复后的配置；
- 各配置段的默认值、解析与校验按段放在 `config/` 下的
  `account.rs` / `java.rs` / `root.rs` / `launch.rs` / `download.rs`；`mod.rs` 只负责整份
  文档的读写、逐项容错与跨段编排。

### 实例覆盖

实例配置位于 `<minecraft_root>/versions/<version_name>/dmcl.json`，只写 `launch`，以全局
`launch` 为底逐字段覆盖；文件不存在时完全使用全局 `launch`。实例配置只读：不自动创建、
不自动写回。加载时账户 / Java 选择项失效会回退并警告，其中 `launch.account` 失效时回退为
`null`，启动时报错。GUI 在启动实例前若读到实例配置警告，会先弹确认框；确认后使用内存里
修复过的配置继续，取消则不启动。

安装阶段使用全局 `LaunchConfig`，因为此时版本目录还不存在；安装不读取、也不创建实例
`dmcl.json`。

### 根目录

应用层在取用根目录时把配置里的「名字 → 路径」映射绝对化，再传给 `load_instance` /
`install_options` / `launch_option`。相对路径按启动时的工作目录解析，绝对化只发生在取用
根目录时；读取配置和整体保存不会改写路径写法，图形界面保存目录时则写入规范化后的路径。

`minecraft_root` 只决定启动时默认打开哪个根目录的主页；CLI 的全局选项
`--minecraft-root=<name>` 只影响本次运行，两者都不会把「当前页面」和业务接口继续绑在一起。

### Java 缓存

`search-java` 和首选项里的「搜索已安装的 Java」都会扫描本机，按路径去重后把 `path` /
`major` / `arch` 合并进全局 `javas` 并保存；手动添加 / 编辑条目时由首选项探测一次并写入
同样的缓存。之后启动只读这份缓存，不再探测。

### CLI

CLI 用 `clap` derive 定义命令，命令定义、本地化与分发在 `cli/mod.rs`，各命令实现按文件
拆在 `cli/commands/` 下。任务事件里的「是否继续？」警告确认用 `dialoguer`；没有交互式
菜单或配置编辑器。

界面语言全部为中文，包括 clap 自动生成的小节标题（`用法：` / `位置参数：` / `选项：` /
`命令：`）与常见报错（`错误：缺少必填参数：…`）。命令名、参数名、配置键名保持英文
（它们是标识符，不是文案）。

不带子命令时：编译了 `gtk4` 就打开 GUI；没有 GUI 时打印帮助并以非零状态退出。显式的
`dmcl gui` 在前端不可用时直接报错，不会回退。

### GUI

GTK4 + libadwaita 前端在 `src/gui/gtk4`，用 relm4，由 cargo feature `gtk4` 门控（默认关）。
与 CLI 共用业务 crate：把 `Config` 翻译成 `*Options`、用 `Shell` 跑 `TaskChain`、把
`TaskEvent` 渲染成页面与任务窗口；业务 crate 不读配置。

- GUI 里每个根目录的主页在启动时全部创建并常驻；标签只是主页的显示入口，关闭标签不会
  销毁 `HomePage`，选中项、运行状态、启动链和 `GameMonitor` 监视任务都继续保留；
- 本页启动的实例由 `GameMonitor` 精确监视退出状态。GUI 退出后游戏进程继续运行；
- GUI 共用一份运行时配置：`gtk4/app/config_store.rs` 里的 `ConfigStore` 是 GTK 主线程上的
  全局单例，GUI 入口把 `Config::load_sync` 读到的配置放进去。页面拿 `ConfigRef` 只读
  句柄，写只能走 `ConfigStore::update`：原地修改、通知订阅者，并异步落盘（保存失败时在
  主线程回调调用方给的错误处理函数）；
- 配置里新增的根目录会补建常驻主页；删除根目录不销毁已有页面，只从标题栏菜单移除；
- 主页按根目录分标签：标题栏最左侧的根目录菜单负责打开 / 聚焦主页；每个主页右下角的
  悬浮 `+` 打开「创建实例」对话框，`CustomPage` 使用该主页捕获的 root，App 不需要跟踪
  当前选中的主页；
- 版本目录用 `notify` 监听，目录不存在时监听最近存在的祖先。

前端的整体分层、relm4 约定、组件索引与 GTK 测试方法见
[crates/dmcl/src/gui/gtk4/README.md](src/gui/gtk4/README.md)；新增页面或组件时，细节写进
对应目录的 `README.md`。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 使用方式、命令与配置键：[README.md](../../README.md)
- 业务 crate 的职责与设计：各 `crates/*/README.md`
