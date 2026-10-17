# DMCL

非官方 Minecraft 启动器。

## 特性

- **安装原版** —— 按版本号下载版本 JSON、客户端 jar、库与资源文件，在
  `versions/<名字>/` 下装出一个可玩的版本。
- **一键装模组加载器** —— `--fabric` / `--forge` / `--neoforge`，可指定加载器版本。
- **启动游戏** —— 自动挑选 Java、生成 JVM 与游戏参数、启动子进程并监控（可停止）。
- **启动前完整性检查** —— 报告缺失 / 内容不符的文件；`launch` 默认「缺了就补」，
  也可以只用 `check_integrity` 问「完整吗」再自行决定。
- **镜像源** —— `official`（Mojang）或 `bmclapi`。文档里的地址统一归一成
  `${MC_MAVEN}` 这样的模板，下载时才按镜像解析；写回磁盘的版本 JSON 与官方启动器通用。
- **并发下载** —— 多文件并发 + 大文件按 `Range` 分块；先写 `.part`，**校验通过才改名**，失败自动重试。
- **可配置** —— 镜像、并发度、分块阈值、内存、窗口尺寸、附加 JVM / 游戏参数、Java 选择……
- **图形界面（开发中）** —— GTK4 + libadwaita，`dmcl gui`（用 `--features gtk4` 开启）。
- **跨平台** —— Linux / Windows / macOS 都有 CI 覆盖（默认构建与 GTK4 前端）。

## 构建

需要 stable Rust（1.93 或更新）。

```bash
cd dmcl-rs
cargo build --release
./target/release/dmcl --help          # Windows: .\target\release\dmcl.exe --help
```

图形界面默认**不**编译；开启需要 `--features gtk4`，并装好 GTK4 / libadwaita 开发库
（GTK ≥ 4.18、libadwaita ≥ 1.7）：

```bash
cargo build --release --features gtk4
./target/release/dmcl gui              # 默认前端 gtk4
```

GTK 开发库的装法：Debian / Ubuntu 用 `libgtk-4-dev libadwaita-1-dev`；macOS 用
`brew install gtk4 libadwaita`；Windows 用 MSYS2 的 `mingw-w64-x86_64-gtk4`
`mingw-w64-x86_64-libadwaita`，并以 `--target x86_64-pc-windows-gnu` 构建。
三个平台的 `dmcl gui` 默认前端都是 `gtk4`。

CI 的 GTK4 任务会输出自包含包：Linux AppImage、Windows zip（双击 `dmcl.cmd` 启动）、
macOS `.app`。Linux AppImage 在 Ubuntu 26.04 上构建，需要较新的 glibc；macOS 包是
ad-hoc 签名，首次打开可能需要在 Finder 里右键 → 打开。

## 快速开始

直接运行 `dmcl`（不带子命令）会进入交互式菜单。

或者：

```bash
# 找本机 Java 并写进配置
dmcl java search

# 装原版 1.20.1，版本名也叫 1.20.1
dmcl install 1.20.1 1.20.1

# 再装一个 Fabric 版本，版本名自动生成
dmcl install 1.20.1 --fabric

# 启动
dmcl launch 1.20.1
```

## 命令

| 命令                                                            | 说明                                                          |
| --------------------------------------------------------------- | ------------------------------------------------------------- |
| `launch <version-name>`                                         | 检查并补全文件 → 生成参数 → 启动                              |
| `install <mc-version-id> [version-name]`                        | 安装原版；省略版本名时自动命名                                |
| `install … [--fabric] [--forge] [--neoforge]`                   | 安装加载器                                                    |
| `install … [--fabric-version=…] [--forge-version=…] [--neoforge-version=…]` | 指定加载器版本                                    |
| `list`                                                          | 列出**已安装**的版本                                          |
| `list-mc [--type=release,snapshot,…]`                           | 列出 Minecraft 版本                                           |
| `list-fabric <mc-version-id>`                                   | 列出 Fabric loader 版本                                       |
| `list-forge <mc-version-id>`                                    | 列出 Forge 版本                                               |
| `list-neoforge <mc-version-id>`                                 | 列出 NeoForge 版本                                            |
| `java [list\|search\|add\|remove\|use]`                          | 管理本机 java；不带子命令进交互菜单                           |
| `gui [frontend]`                                                | 启动图形界面（默认前端 gtk4；需 `--features gtk4` 构建）      |
| `prompt`                                                        | 交互式菜单                                                    |
| `config`                                                        | 查看或编辑配置；非终端下只打印                                |
| `version`                                                       | 显示版本与构建时间                                            |

不带子命令时执行配置里的 `cli.default_command`（默认 `prompt`）。

### `java` 子命令

| 子命令                        | 说明                                       |
| ----------------------------- | ------------------------------------------ |
| `java list`                   | 列出已配置的 java 与当前选择               |
| `java search`                 | 搜索本机 java 并**合并**进配置             |
| `java add <path> [name]`      | 手动添加 java                              |
| `java remove <name>`          | 删除 java 配置                             |
| `java use <name\|autoselect>` | 选择使用的 java                            |

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
  },
  "cli": { "default_command": "prompt" }
}
```

`launch.java` 可以是 `"autoselect"`，也可以是 `java_configs` 里的某个名字。

`download.mirror` 是镜像使用策略：`"official-only"`（仅官方源）、
`"official-first"`（官方源优先，失败再试镜像源，**默认**）、`"mirror-first"`（镜像源优先，
失败再回官方源）。元数据取数与文件下载都按这个优先级轮换。

## 测试

```bash
cargo test --workspace                       # 离线单测
cargo test --workspace -- --ignored          # 联网冒烟测试（需要能访问 Mojang / bmclapi）
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

## 架构

本项目是一个 cargo workspace，由六个 crate 组成（配置读取、业务逻辑、任务链、下载、
元数据等各司其职）。目录结构、依赖关系与设计说明见
[ARCHITECTURE.md](ARCHITECTURE.md)。

## 许可

GPL-3.0-or-later。
