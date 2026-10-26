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
- **多账户 / 多根目录 / 多 Java** —— 在全局配置里维护账户、Minecraft 根目录与 Java 表。
- **多主页** —— 每个 `minecraft_roots` 条目对应一个常驻主页；标签可以随时关闭，
  关闭只影响显示，游戏进程和主页运行状态继续保留。
- **全局默认 + 实例覆盖** —— 全局 `launch` 是所有实例的默认启动配置；每个实例可以
  在 `versions/<version_name>/dmcl.json` 里只写要覆盖的字段。
- **镜像源** —— `official`（Mojang）或 `bmclapi`。文档里的地址统一归一成
  `${MC_MAVEN}` 这样的模板，下载时才按镜像解析；写回磁盘的版本 JSON 与官方启动器通用。
- **并发下载** —— 多文件并发 + 大文件按 `Range` 分块；先写 `.part`，校验通过才改名，
  失败自动重试。
- **可配置** —— 镜像、并发度、分块阈值、内存、窗口尺寸、附加 JVM / 游戏参数、Java 选择……
- **跨平台** —— Linux / Windows / macOS 都有 CI 覆盖（默认构建与 GTK4 前端）。

## 构建

需要 stable Rust **1.93** 或更新。

### 带图形界面（推荐）

图形前端由 cargo feature `gtk4` 门控，默认**不**编译；开启需要 GTK ≥ 4.22、
libadwaita ≥ 1.9：

```bash
cargo build --release --features gtk4
./target/release/dmcl        # 无子命令：打开图形界面
```

各平台装 GTK 开发库：

| 平台            | 安装方式                                                                                                                                                                                     |
| --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Debian / Ubuntu | `sudo apt install libgtk-4-dev libadwaita-1-dev`（Ubuntu 24.04 的 GTK 4.14 太旧，建议 26.04+ 或更新发行版）                                                                                  |
| macOS           | `brew install pkgconf gtk4 libadwaita`                                                                                                                                                       |
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

不带子命令时默认尝试打开图形界面；没有编译 `gtk4` 时会打印帮助并以非零状态退出。
显式的 `dmcl gui` 在前端不可用时直接报错，不会回退。

### 自包含包

CI 的 GTK4 任务会产出已经带上 GTK 运行时的包：

- **Linux**：`dmcl-gtk4-linux.AppImage`（在 Ubuntu 26.04 上构建，需要较新的 glibc）；
- **Windows**：`dmcl-gtk4-windows.zip`，解压后双击 `dmcl.cmd` 启动（x64，自带 GTK DLL）；
- **macOS**：`dmcl-gtk4-macos.zip`，解压得到 `DMCL.app`（ad-hoc 签名，首次打开可能需要在
  Finder 里右键 → 打开）。

## 快速开始

带 GTK4 的构建不带子命令时会打开图形界面；只构建命令行时用子命令操作：

```bash
# 找本机 Java 并写进全局配置
dmcl search-java

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

首次启动前需要一份可用的 Java；`dmcl search-java` 会扫描常见安装目录
（Linux `/usr/lib/jvm`、Windows 各 JDK 厂商目录、macOS `/Library/Java/JavaVirtualMachines`
与 Homebrew）以及 `JAVA_HOME` 并写入全局配置。账户、根目录和 Java 表也可以通过直接编辑
全局 `dmcl.json` 维护。账号目前只有离线模式，联机登录还没实现。图形界面的账号、实例管理
页面仍在开发中；主菜单的「启动器选项」「全局游戏选项」目前是空白设置页。

图形界面里，标题栏左侧的「Minecraft 目录」菜单可以打开其它 `minecraft_roots` 的主页。
所有 root 的主页在程序启动时就已创建，关闭标签只是隐藏
页面；重新打开后选中项、运行状态和正在监视的游戏都还在。新建实例使用主页右下角的悬浮
`+` 按钮；创建目标始终是该主页绑定的 root。

最右侧主菜单里的「启动器选项」「全局游戏选项」也可以分别用 `Ctrl+,`、`Ctrl+Shift+,`
打开；每个设置页最多只有一个标签。菜单打开时，`Alt+L` / `Alt+G` / `Alt+A` 分别触发
启动器选项、全局游戏选项和关于。

## 命令

| 命令                                                                        | 说明                                                  |
| --------------------------------------------------------------------------- | ----------------------------------------------------- |
| `launch <version-name>`                                                     | 检查并补全文件 → 生成参数 → 启动                      |
| `install <mc-version-id> [version-name]`                                    | 安装原版；省略版本名时自动命名                        |
| `install … [--fabric] [--forge] [--neoforge]`                               | 安装模组加载器                                        |
| `install … [--fabric-version=…] [--forge-version=…] [--neoforge-version=…]` | 指定加载器版本                                        |
| `list`                                                                      | 列出**已安装**的版本                                  |
| `list-mc [--type=release,snapshot,…]`                                       | 列出 Minecraft 版本（默认 `release`）                 |
| `list-fabric <mc-version-id>`                                               | 列出 Fabric loader 版本                               |
| `list-forge <mc-version-id>`                                                | 列出 Forge 版本                                       |
| `list-neoforge <mc-version-id>`                                             | 列出 NeoForge 版本                                    |
| `search-java`                                                               | 搜索本机 Java（常见安装目录 / `JAVA_HOME`），合并进全局配置并保存 |
| `gui [frontend]`                                                            | 启动图形界面（默认前端 `gtk4`；前端不可用时直接报错） |
| `version`                                                                   | 显示版本与构建时间                                    |

不带子命令时：有 GTK4 前端就打开 GUI；没有就打印帮助并以非零状态退出。

全局选项 `--minecraft-root=<name>` 用来在本次运行中切换 `minecraft_roots` 里的根目录，
可以放在子命令前或后，不会写回全局配置：

```bash
dmcl --minecraft-root=test launch 1.20.1
dmcl list --minecraft-root=test
```

## 配置文件

全局配置位置：`<系统配置目录>/dmcl.json`
（Linux `$XDG_CONFIG_HOME` 或 `~/.config`；Windows `%APPDATA%`；
macOS `~/Library/Application Support`）。

所有键都是可选的，缺失的键沿用默认值：

```json
{
  "launch": {
    "java": "autoselect",
    "account": "player",
    "window_width": 854,
    "window_height": 480,
    "min_memory": 512,
    "max_memory": 2048,
    "custom_info": "dmcl",
    "additional_jvm": "-Dfml.ignoreInvalidMinecraftCertificates=True -Dfml.ignorePatchDiscrepancies=True",
    "additional_game": ""
  },
  "accounts": {
    "player": { "type": "offline", "username": "player" }
  },
  "javas": {
    "jdk-17": "/usr/lib/jvm/java-17/bin/java"
  },
  "minecraft_roots": [
    {
      "name": "当前目录",
      "path": "${CWD}/.minecraft"
    },
    {
      "name": "官方启动器",
      "path": "/home/me/.minecraft"
    }
  ],
  "minecraft_root": "当前目录",
  "download": {
    "max_tasks": 64,
    "skip_music": false,
    "mirror": "official-first",
    "chunk_threshold_mib": 8,
    "chunk_size_mib": 4,
    "max_chunks_per_file": 8,
    "max_attempts": 3
  }
}
```

- `launch.java` 可以是 `"autoselect"`，也可以是 `javas` 里的某个名字。
- `search-java` 自动生成的 `javas` 名字形如
  `openjdk-linux-x86_64-17.0.2-1a2b3c4d`，末尾是安装路径的短哈希；同一路径不会重复加入。
- `launch.account` 是 `accounts` 里的某个名字，也可以写成 `null`；为 `null` 时启动会报错。
- `minecraft_roots` 是具名根目录的数组，条目为 `{ "name": "...", "path": "..." }`；
  `name` 直接作为界面上的显示名，必须唯一。默认有两个：「当前目录」
  （`${CWD}/.minecraft`）和「官方启动器」（Windows 为 `%APPDATA%\.minecraft`、
  macOS 为 `~/Library/Application Support/minecraft`、其它平台为 `~/.minecraft`）。
- `path` 里的 `${CWD}` 表示当前工作目录，相对路径也按当前工作目录解析；两者都在使用根
  目录时展开，配置里保留原文，保存配置不会把它们写死。
- `minecraft_root` 指向 `minecraft_roots` 里的某个名字，默认「当前目录」。
- `download.skip_music` 为 `true` 时跳过 `*.ogg` 资源（音乐与音效）。
- `download.mirror` 是镜像使用策略：`"official-only"`（仅官方源）、
  `"official-first"`（官方源优先，失败再试镜像源，**默认**）、`"mirror-first"`（镜像源优先，
  失败再回官方源）。元数据取数与文件下载都按这个优先级轮换。
- 账户条目目前只支持离线账户，格式为
  `{ "type": "offline", "username": "..." }`。

### 实例覆盖

每个实例可以在自己的版本目录里写一份只包含 `launch` 的配置：

```text
<minecraft_root>/versions/<version_name>/dmcl.json
```

```json
{
  "launch": {
    "account": "steve",
    "max_memory": 4096
  }
}
```

- 文件不存在：完全使用全局 `launch`，不会自动创建文件。
- 文件存在：以全局 `launch` 为底，实例里出现的字段逐项覆盖，没写的字段继续继承全局。
- 实例配置只读：不会自动创建，也不会自动写回；坏字段的修复只在内存里生效。
- 实例配置里有警告时，GUI 会在启动链开始前弹出确认框；点「取消」不启动，点
  「继续启动」才使用修复后的配置继续。
- 安装阶段还没有实例目录，使用全局 `launch`，不读取、不创建实例 `dmcl.json`。


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
元数据等各司其职）。目录结构、依赖关系、GTK4 前端文档与设计说明见
[ARCHITECTURE.md](ARCHITECTURE.md)。

## 许可

GPL-3.0-or-later。
