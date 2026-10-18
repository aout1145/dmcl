# DMCL

非官方 Minecraft 启动器，Rust 编写。提供 GTK4 + libadwaita 图形界面和命令行两种前端，
业务逻辑在各 `dmcl-*` crate 里，两个前端共用同一套安装、下载、启动实现。

## 特性

- **图形界面** —— 基于 GTK4 + libadwaita（relm4），遵循 GNOME 设计风格。
- **安装原版** —— 按版本号下载版本 JSON、客户端 jar、库与资源文件，在
  `versions/<名字>/` 下装出一个可玩的版本。
- **一键装模组加载器** —— Fabric / Forge / NeoForge，可指定加载器版本。
- **启动游戏** —— 自动挑选 Java、生成 JVM 与游戏参数、启动子进程并监控（可停止）。
- **启动前完整性检查** —— 报告缺失 / 内容不符的文件；`launch` 默认「缺了就补」，
  需要自己决定时也可以直接调用库里的 `dmcl_install::check_integrity`。
- **镜像源** —— `official`（Mojang）或 `bmclapi`。文档里的地址统一归一成
  `${MC_MAVEN}` 这样的模板，下载时才按镜像解析；写回磁盘的版本 JSON 与官方启动器通用。
- **并发下载** —— 多文件并发 + 大文件按 `Range` 分块；先写 `.part`，校验通过才改名，
  失败自动重试。
- **可配置** —— 镜像、并发度、分块阈值、内存、窗口尺寸、附加 JVM / 游戏参数、Java 选择……
- **跨平台** —— Linux / Windows / macOS 都有 CI 覆盖（默认构建与 GTK4 前端）。

## 构建

需要 stable Rust **1.93** 或更新。

### 带图形界面（推荐）

图形前端由 cargo feature `gtk4` 门控，默认**不**编译；开启需要 GTK ≥ 4.18、
libadwaita ≥ 1.7：

```bash
cargo build --release --features gtk4
./target/release/dmcl        # 无子命令：打开图形界面
```

各平台装 GTK 开发库：

| 平台            | 安装方式                                                                                                                                                               |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Debian / Ubuntu | `sudo apt install libgtk-4-dev libadwaita-1-dev`（Ubuntu 24.04 的 GTK 4.14 太旧，建议 26.04+ 或更新发行版）                                                            |
| macOS           | `brew install pkgconf gtk4 libadwaita`                                                                                                                                 |
| Windows         | MSYS2 MINGW64：`pacman -S mingw-w64-x86_64-gcc mingw-w64-x86_64-pkgconf mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita`，再用 rustup 的 `stable-x86_64-pc-windows-gnu` 工具链构建（见下） |

Windows 示例（MSYS2 MINGW64 里用 GNU host 工具链，避免 `pkg-config` 把它当成交叉编译）：

```bash
rustup toolchain install stable-x86_64-pc-windows-gnu --force-non-host
cargo +stable-x86_64-pc-windows-gnu build --release --features gtk4
# 运行 target/release/dmcl.exe（需要 mingw64/bin 在 PATH 里）
```

### 只构建命令行

```bash
cargo build --release
./target/release/dmcl --help
```

不带子命令时默认尝试打开图形界面；没有编译 `gtk4` 时会回退到交互式菜单。显式的
`dmcl gui` 在前端不可用时直接报错，不会回退；也可以显式运行 `dmcl prompt`。

### 自包含包

CI 的 GTK4 任务会产出已经带上 GTK 运行时的包：

- **Linux**：`dmcl-gtk4-linux.AppImage`（在 Ubuntu 26.04 上构建，需要较新的 glibc）；
- **Windows**：`dmcl-gtk4-windows.zip`，解压后双击 `dmcl.cmd` 启动（x64，自带 GTK DLL）；
- **macOS**：`dmcl-gtk4-macos.zip`，解压得到 `DMCL.app`（ad-hoc 签名，首次打开可能需要在
  Finder 里右键 → 打开）。

## 快速开始

不带子命令时默认打开图形界面（没有 `gtk4` 时回退到交互菜单），所以：

```bash
dmcl                 # 打开图形界面
dmcl prompt          # 交互式命令行菜单
```

命令行常用流程：

```bash
# 找本机 Java 并写进配置（也可以 dmcl java add <path> [name]）
dmcl java search

# 装原版 1.20.1，本地版本名也叫 1.20.1
dmcl install 1.20.1

# 装 Fabric / Forge / NeoForge
dmcl install 1.20.1 --fabric
dmcl install 1.20.1 --forge --forge-version 47.3.0

# 启动
dmcl launch 1.20.1

# 看看装了什么 / 远端有哪些版本
dmcl list
dmcl list-mc --type=release,snapshot
```

首次启动前需要一份可用的 Java；`dmcl java search` 会扫描本机并写入配置。账号目前只有
离线模式（`account.username`），联机登录还没实现。图形界面的账号、设置、实例管理页面
仍在开发中。

## 命令

| 命令                                                                        | 说明                                                       |
| --------------------------------------------------------------------------- | ---------------------------------------------------------- |
| `launch <version-name>`                                                     | 检查并补全文件 → 生成参数 → 启动                           |
| `install <mc-version-id> [version-name]`                                    | 安装原版；省略版本名时自动命名                             |
| `install … [--fabric] [--forge] [--neoforge]`                               | 安装模组加载器                                             |
| `install … [--fabric-version=…] [--forge-version=…] [--neoforge-version=…]` | 指定加载器版本                                             |
| `list`                                                                      | 列出**已安装**的版本                                       |
| `list-mc [--type=release,snapshot,…]`                                       | 列出 Minecraft 版本（默认 `release`）                      |
| `list-fabric <mc-version-id>`                                               | 列出 Fabric loader 版本                                    |
| `list-forge <mc-version-id>`                                                | 列出 Forge 版本                                            |
| `list-neoforge <mc-version-id>`                                             | 列出 NeoForge 版本                                         |
| `java [list\|search\|add\|remove\|use]`                                     | 管理本机 Java；不带子命令进交互菜单                        |
| `gui [frontend]`                                                            | 启动图形界面（默认前端 `gtk4`；前端不可用时直接报错）      |
| `prompt`                                                                    | 交互式菜单                                                 |
| `config`                                                                    | 查看或编辑配置；非终端下只打印                             |
| `version`                                                                   | 显示版本与构建时间                                         |

不带子命令时默认走 GUI；没有编译 `gtk4` 时回退到交互菜单。

### `java` 子命令

| 子命令                        | 说明                           |
| ----------------------------- | ------------------------------ |
| `java list`                   | 列出已配置的 Java 与当前选择   |
| `java search`                 | 搜索本机 Java 并**合并**进配置 |
| `java add <path> [name]`      | 手动添加 Java                  |
| `java remove <name>`          | 删除 Java 配置                 |
| `java use <name\|autoselect>` | 选择使用的 Java                |

`dmcl java`（无子命令）进交互菜单。

## 配置文件

位置：`<系统配置目录>/dmcl.json`
（Linux `$XDG_CONFIG_HOME` 或 `~/.config`；Windows `%APPDATA%`；
macOS `~/Library/Application Support`）。

所有键都是可选的，缺失的键沿用默认值：

```json
{
  "launch": {
    "java_configs": { "jdk-17": "/usr/lib/jvm/java-17/bin/java" },
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
  "account": { "online_mode": false, "username": "player" },
  "download": {
    "max_tasks": 64,
    "i_dont_need_music": false,
    "mirror": "official-first",
    "chunk_threshold_mib": 8,
    "chunk_size_mib": 4,
    "max_chunks_per_file": 8,
    "max_attempts": 3
  }
}
```

- `launch.java` 可以是 `"autoselect"`，也可以是 `java_configs` 里的某个名字。
- `launch.minecraft_root_path` 是游戏根目录，相对路径会按当前工作目录补成绝对路径。
- `download.mirror` 是镜像使用策略：`"official-only"`（仅官方源）、
  `"official-first"`（官方源优先，失败再试镜像源，**默认**）、`"mirror-first"`（镜像源优先，
  失败再回官方源）。元数据取数与文件下载都按这个优先级轮换。
- `account.online_mode` 目前未使用；只有离线账号（`account.username`）。

## 开发与测试

```bash
cargo test --workspace                       # 离线单测
cargo test --workspace -- --ignored          # 联网冒烟测试（要能访问 Mojang / bmclapi）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

GTK4 前端额外检查：

```bash
cargo clippy -p dmcl --features gtk4 --all-targets -- -D warnings
cargo test -p dmcl --features gtk4
```

CI 分两个 workflow：`build.yaml` 跑 Linux / Windows / macOS 的默认构建；`build-gtk4.yaml`
装 GTK 后跑三个平台的 GTK4 检查、测试、构建，并上传自包含包。

## 架构

本项目是一个 cargo workspace，由六个 crate 组成（配置读取、业务逻辑、任务链、下载、
元数据等各司其职）。目录结构、依赖关系与设计说明见 [ARCHITECTURE.md](ARCHITECTURE.md)；
GTK4 前端的组件划分见
[crates/dmcl/src/gui/gtk4/ARCHITECTURE.md](crates/dmcl/src/gui/gtk4/ARCHITECTURE.md)。

## 许可

GPL-3.0-or-later。
