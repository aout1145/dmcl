# DMCL

非官方 Minecraft 启动器。

## 特性

- **安装原版** —— 按版本号下载版本 JSON、客户端 jar、库与资源文件，在
  `versions/<名字>/` 下装出一个可玩的版本。
- **一键装模组加载器** —— `--fabric` / `--forge` / `--neoforge`，可指定加载器版本
  （默认取最新）。
- **启动游戏** —— 自动挑选 Java、生成 JVM 与游戏参数、启动子进程并监控（可停止）。
- **启动前完整性检查** —— 报告缺失 / 校验不符的文件；`launch` 默认「缺了就补」，
  也可以只用 `check_integrity` 问「完整吗」再自行决定。
- **镜像源** —— `official`（Mojang）或 `bmclapi`。文档里的地址统一归一成
  `${MC_MAVEN}` 这样的模板，下载时才按镜像解析；写回磁盘的版本 JSON 与官方启动器通用。
- **并发下载** —— 多文件并发 + 大文件按 `Range` 分块；先写 `.part`，**校验通过才改名**，失败自动重试。
- **可配置** —— 镜像、并发度、分块阈值、内存、窗口尺寸、附加 JVM / 游戏参数、Java 选择……
- **跨平台** —— Linux / Windows 有 CI 覆盖，macOS 也可用。

## 构建

需要 stable Rust（1.75 或更新）。

```bash
cd dmcl-rs
cargo build --release
./target/release/dmcl --help          # Windows: .\target\release\dmcl.exe --help
```

调试构建会打印一行 `!!! DMCL SNAPSHOT !!!`。

## 快速开始

直接运行 `dmcl`（不带子命令）会进入交互式菜单。

或者：

```bash
# 找本机 Java 并写进配置（可选，也可以之后在交互菜单里选）
dmcl search-java

# 装原版 1.20.1，本地版本目录名也叫 1.20.1
dmcl install 1.20.1 1.20.1

# 再装一个 Fabric 版本
dmcl install 1.20.1 1.20.1-fabric --fabric

# 启动
dmcl launch 1.20.1-fabric
```

## 命令

| 命令                                                                        | 说明                                  |
| --------------------------------------------------------------------------- | ------------------------------------- |
| `launch <version-name>`                                                     | 检查完整性 → 缺文件则补全 → 启动      |
| `install <mc-version-id> <version-name>`                                    | 安装原版                              |
| `install … [--fabric] [--forge] [--neoforge]`                               | 顺便装加载器                          |
| `install … [--fabric-version=…] [--forge-version=…] [--neoforge-version=…]` | 指定加载器版本（留空取最新）          |
| `list-mc [--type=release,snapshot,…]`                                       | 列出 Minecraft 版本（默认 `release`） |
| `list-fabric <mc-version-id>`                                               | 列出 Fabric loader 版本               |
| `list-forge <mc-version-id>`                                                | 列出 Forge 版本                       |
| `list-neoforge <mc-version-id>`                                             | 列出 NeoForge 版本                    |
| `install-forge <version-name> <forge-version-id>`                           | 在已有版本上装 Forge                  |
| `install-neoforge <version-name> <neoforge-version-id>`                     | 在已有版本上装 NeoForge               |
| `search-java`                                                               | 搜索本机 Java 并写入配置              |
| `prompt`                                                                    | 交互式菜单                            |
| `version`                                                                   | 显示版本与构建时间                    |
| `config`                                                                    | 显示当前配置                          |
| `edit-config`                                                               | 打印配置文件位置                      |

全局选项：

- `--java <PATH>` —— 用这个 java，并把它记进命名 java 配置；
- `--mirror <official|bmclapi>` —— 覆盖下载镜像（仅本次运行）。

不带子命令时执行配置里的 `cli.default_command`（默认 `prompt`）。

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
    "mirror": "official",
    "chunk_threshold_mib": 8,
    "chunk_size_mib": 4,
    "max_chunks_per_file": 8,
    "max_attempts": 3
  },
  "cli": { "default_command": "prompt" }
}
```

`launch.java` 可以是 `"autoselect"`，也可以是 `java_configs` 里的某个名字。
`launch.natives_arch`（可省略）用来覆盖 natives 分类器里 `${arch}` 的取值，
默认按本机位宽。

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
