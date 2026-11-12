# DMCL

非官方 Minecraft 启动器，使用 Rust 编写，提供基于 GTK4 + libadwaita 的图形界面，支持
Linux、Windows 和 macOS。图形界面与命令行共用同一套安装、下载和启动功能。

## 功能

- **多目录主页** —— 每个 Minecraft 目录拥有独立主页，可并行管理多个目录。
- **实例管理** —— 自动识别实例并按分类归组，显示运行状态；启动时自动完成账户登录、Java
  选择、文件检查与补全，并支持取消与停止。
- **创建实例** —— 支持原版与 Forge / Fabric / NeoForge / OptiFine，可指定版本、名称、图标
  和分类，并检查实例名是否已被占用；图标会按版本类型和加载器自动选择，也可以指定本地图片。
- **账户** —— 支持离线账户与 Microsoft 账户、设备代码登录和默认账户。
- **Minecraft 目录** —— 添加、重命名、修改路径、设为默认、调整顺序和删除，路径支持相对与
  绝对形式。
- **Java** —— 搜索本机 Java，或手动维护 Java 列表，显示版本、供应商、架构和路径。
- **全局启动配置** —— 设置默认账户、默认 Java、窗口尺寸、内存、版本隔离和附加启动参数。
- **任务** —— 安装与启动任务的进度、步骤、日志和取消；退出时若有未完成任务会先确认。
- **外观** —— 跟随系统、浅色和深色三种主题。

尚未实现：实例管理页、删除实例、下载设置、游戏异常退出报告、Forge 版本筛选。

## 图形界面

图形界面以主页组织实例：每个 Minecraft 目录对应一个主页，可以在多个目录之间切换。关闭
主页不会中断安装、启动或已运行的游戏，重新打开后仍保持原来的状态。

### 首次启动

配置文件不存在时，启动后会显示设置向导，依次添加账户、选择 Minecraft 目录并设置版本隔离。
添加至少一个账户后才能完成向导；完成后进入所选目录的主页。也可以跳过，跳过时会保存当前
配置，以后不再显示向导。

### 创建实例

创建实例时可选择 Minecraft 版本，以及可选的 Forge、Fabric、NeoForge 或 OptiFine。
Forge 与 OptiFine 可以组合，Fabric、NeoForge 与二者互斥。实例名称和图标会根据选择自动
生成，也可以手动指定；分类可沿用已有分类或新建。安装以任务形式进行，失败或取消不会留下
不完整的实例。

自动图标按加载器与版本类型选择：Forge / NeoForge / Fabric 优先于 OptiFine，OptiFine
优先于原版类型；原版正式版、快照版、远古版分别使用草方块、命令方块、远古圆石。主页上
版本 JSON 读不出来的实例使用红石块。用户选择的自定义图标会转成 PNG 保存为
`versions/<实例名>/logo.png`；自动图标不写文件。

### 任务与设置

安装和启动都以任务形式运行。任务列表同时展示全部步骤的计数、整体进度和日志，并支持取消；
关闭任务列表不会中断任务。退出 DMCL 时若有未完成任务，会先要求确认。

首选项集中管理账户、Minecraft 目录、Java 和全局启动选项。修改会自动保存，并在下次启动
实例时生效。`Ctrl+,` 可打开首选项；外观支持跟随系统、浅色和深色三种主题。

## 命令行

不启用图形界面时，`dmcl` 是一个普通命令行工具；启用图形界面后也可以继续使用这些子命令。
常用操作：

```bash
dmcl search-java                       # 扫描本机 Java 并写入全局配置
dmcl login                             # 微软设备代码登录
dmcl install 1.20.1                    # 安装原版
dmcl install 1.20.1 --fabric           # 安装 Fabric
dmcl install 1.20.1 --forge --forge-version 47.3.0 --optifine
dmcl launch 1.20.1                     # 启动
dmcl list                              # 查看已安装版本
```

`dmcl install 1.20.1 --neoforge` 安装 NeoForge，`--optifine` 单独安装 OptiFine，
`--fabric-version` / `--neoforge-version` / `--optifine-version` 指定加载器版本。
`--minecraft-root=<名字>` 可以在本次运行中切换根目录，不写回全局配置。完整命令和参数用
`dmcl --help` 查看。

命令行启动只使用全局配置 `javas` 里已有的 Java，不会自动扫描本机，也不会自动下载；第一次
使用或配置里的路径都失效时，先执行 `dmcl search-java`。

安装流程用 `download.java` 指定安装器 processor 使用的 Java，默认 `autoselect`：先从
`javas` 里选择，没有匹配版本时扫描本机，仍然没有就下载 Mojang 的 JRE 到官方启动器的
runtime 目录。它不影响启动使用的 Java。图形界面启动实例时若没有可用 Java，也会先扫描
本机并写回配置，仍没有匹配版本时自动下载；写配置失败不阻断本次启动。

不带子命令时：启用图形界面则打开图形界面，否则打印帮助并以非零状态退出。

## 构建与运行

需要 stable Rust **1.95** 或更新。图形界面由 cargo feature `gtk4` 门控，默认不编译：

```bash
cargo build --release --features gtk4
./target/release/dmcl
```

需要 GTK ≥ 4.22、libadwaita ≥ 1.9：

- **Debian / Ubuntu**：`sudo apt install libgtk-4-dev libadwaita-1-dev`
- **macOS**：`brew install pkgconf gtk4 libadwaita`
- **Windows**：用 MSYS2 MINGW64 安装 `mingw-w64-x86_64-gtk4` 与
  `mingw-w64-x86_64-libadwaita` 后，用 `stable-x86_64-pc-windows-gnu` 工具链构建。

只构建命令行：

```bash
cargo build --release
./target/release/dmcl --help
```

微软登录需要一份 Azure 公共客户端应用的 client id，编译时传入：

```bash
DMCL_MICROSOFT_CLIENT_ID=<client-id> cargo build --release --features gtk4
```

不设置也能编译，只是登录 Microsoft 账户时会提示缺少应用 ID；离线账户不受影响。CI 会读取
仓库 secret `DMCL_MICROSOFT_CLIENT_ID`。CI 在 Linux / Windows / macOS 三平台跑默认构建与
GTK4 构建，GTK4 任务会产出带运行时的包。

## 配置文件

全局配置位于 `<系统配置目录>/dmcl/dmcl.json`（Linux `$XDG_CONFIG_HOME` 或 `~/.config`；
Windows `%APPDATA%`；macOS `~/Library/Application Support`）。图形界面的修改会立即写回，
命令行和手动编辑也维护同一份文件。保存时先写临时文件再改名覆盖。

图形界面启动时发现坏掉或不认识的配置项会回退默认值，把原文件备份为同目录的
`dmcl.old.json`，再写入修复后的配置并弹窗说明。

`download` 段可调整下载策略：`max_connections`、`max_active_files`、`max_workers`、
`chunk_threshold_mib`、`chunk_size_mib`、`max_retries`、`retry_base_delay_ms`、
`retry_max_delay_ms`、`mirror`、`skip_music`、`java`。

## 开发与测试

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo clippy -p dmcl --features gtk4 --all-targets -- -D warnings
cargo test -p dmcl --features gtk4
```

需要联网的测试标了 `#[ignore]`，用 `cargo test --workspace -- --ignored` 单独运行。

总体结构、依赖关系与文档索引见 [ARCHITECTURE.md](ARCHITECTURE.md)；各 crate 的职责与设计
见各自的 `README.md`，GTK4 前端从
[crates/dmcl/src/gui/gtk4/README.md](crates/dmcl/src/gui/gtk4/README.md) 开始。

## 第三方素材

`crates/dmcl/src/gui/gtk4/assets/icons` 里的方块与加载器图标取自
[Minecraft Wiki](https://minecraft.wiki)（草方块、命令方块、圆石、铁砧、红石块、Fabric）、
[NeoForged](https://neoforged.net)（NeoForge logo）和 [OptiFine 官网](https://optifine.net)
（OF 图标）。Minecraft 相关材质的版权归
Mojang / Microsoft 所有；这些图片不适用本项目的 GPL 许可，仅作为界面图标使用。

## 许可

GPL-3.0-or-later。
