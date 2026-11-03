# DMCL

非官方 Minecraft 启动器，使用 Rust 编写，提供基于 GTK4 + libadwaita 的图形界面，支持
Linux、Windows 和 macOS。图形界面与命令行共用同一套安装、下载和启动功能。

## 功能

- **多目录主页** —— 每个 Minecraft 目录拥有独立主页，可并行管理多个目录。
- **实例管理** —— 自动识别实例并按分类归组，显示运行状态；启动时自动完成账户登录、Java
  选择、文件检查与补全，并支持取消与停止。
- **创建实例** —— 支持原版与 Forge / Fabric / NeoForge / OptiFine，可指定版本、名称、图标
  和分类，并检查实例名是否已被占用。
- **账户** —— 支持离线账户与 Microsoft 账户、设备代码登录和默认账户。
- **Minecraft 目录** —— 添加、重命名、修改路径、设为默认、调整顺序和删除，路径支持相对与
  绝对形式。
- **Java** —— 搜索本机 Java，或手动维护 Java 列表，显示版本、供应商、架构和路径。
- **全局启动配置** —— 设置默认账户、默认 Java、窗口尺寸、内存、版本隔离和附加启动参数。
- **任务** —— 安装与启动任务的进度、步骤、日志和取消；退出时若有未完成任务会先确认。
- **外观** —— 跟随系统、浅色和深色三种主题。
- **实例管理页** —— 编辑单个实例的启动覆盖（未完成）。
- **删除实例** —— 删除已安装的实例（未完成）。
- **下载设置** —— 下载相关的全局设置（未完成）。
- **异常退出报告** —— 游戏异常退出时展示错误信息（未完成）。
- **Forge 版本筛选** —— 按稳定 / 推荐类别筛选 Forge 版本（未完成）。

## 图形界面

图形界面以主页组织实例：每个 Minecraft 目录对应一个主页，可以在多个目录之间切换。关闭
主页不会中断安装、启动或已运行的游戏，重新打开后仍保持原来的状态。

### 首次启动

配置文件不存在时，启动后会显示设置向导，依次添加账户、选择 Minecraft 目录并设置版本隔离。
添加至少一个账户后才能完成向导；完成后进入所选目录的主页。也可以跳过，跳过时会保存当前
配置，以后不再显示向导，之后可在首选项里继续修改。

实例列表会随目录内容自动更新，并按分类分组；分类按首次出现的顺序排列，未分类的实例排在
最后。启动实例时，DMCL 会完成账户登录、Java 选择、文件检查与补全，并生成启动参数。启动
过程中可以取消，运行中可以停止。若实例的启动配置存在问题，启动前会提示用户选择取消或继续。

### 创建实例

创建实例时可选择 Minecraft 版本，以及可选的 Forge、Fabric、NeoForge 或 OptiFine。Forge
与 OptiFine 可以组合，Fabric、NeoForge 与二者互斥。实例名称和图标会根据选择自动生成，
也可以手动指定；分类可沿用已有分类或新建。DMCL 会检查实例名是否已被占用。安装以任务形式
进行，失败或取消不会留下不完整的实例。

### 任务

安装和启动都以任务形式运行。任务列表展示当前步骤、整体进度和日志，并支持取消。关闭任务
列表不会中断任务；退出 DMCL 时若有未完成的任务，会先要求确认。需要用户决定的警告会以
对话框提示，任务在用户选择后继续。

### 设置

首选项集中管理账户、Minecraft 目录、Java 和全局启动选项。

- **账户**：支持离线账户和 Microsoft 账户。Microsoft 账户通过设备代码登录，凭据保存在
  系统凭据存储中；可以设置默认账户，删除 Microsoft 账户时会同时清理凭据。
- **Minecraft 目录**：可添加、重命名、修改路径、设为默认、调整顺序和删除。路径支持相对与
  绝对两种形式；对已有主页而言，名称、路径的修改和删除要重启 DMCL 后才生效。
- **Java**：可搜索本机 Java，或手动添加、编辑、删除和调整顺序；列表显示 Java 版本、
  供应商、架构和路径。默认 Java 在全局游戏选项中设置。
- **全局游戏选项**：设置默认账户、默认 Java、窗口尺寸、内存、版本隔离和附加启动参数。
  修改会自动保存，并在下次启动实例时生效。

下载设置尚未实现。

### 外观

支持跟随系统、浅色和深色三种外观。`Ctrl+,` 可打开首选项。

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
仓库 secret `DMCL_MICROSOFT_CLIENT_ID`，配置了就用它构建。

CI 的 GTK4 任务会产出带运行时的包：Linux 的 AppImage、Windows 的 zip 和 macOS 的
`DMCL.app`。CI 在 Linux / Windows / macOS 三平台跑默认构建与 GTK4 构建。

## 配置文件

全局配置位于 `<系统配置目录>/dmcl/dmcl.json`（Linux `$XDG_CONFIG_HOME` 或 `~/.config`；
Windows `%APPDATA%`；macOS `~/Library/Application Support`）。图形界面的修改会立即写回，
命令行和手动编辑也维护同一份文件。

保存时先写临时文件再改名覆盖。图形界面启动时发现坏掉或不认识的配置项会回退默认值，把原文件
备份为同目录的 `dmcl.old.json`，再写入修复后的配置并弹窗说明。

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

## 许可

GPL-3.0-or-later。
