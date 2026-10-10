# dmcl (Rust)

非官方 Minecraft 启动器 —— 由原 D 语言版本 `../dmcl` 迁移而来。

保留原有全部逻辑与指令语义，仅替换语言与基础设施：

| 关注点 | 原 D 实现 | 现 Rust 实现 |
| --- | --- | --- |
| 异步 / IO | 线程池 + 阻塞 `std.net.curl` | `tokio`（fs / process / sync 全异步） |
| JSON | `std.json` 手写遍历、到处 `Value` 取值 | `serde` 强类型结构体（`mcjson`），未知字段通过 `flatten` 原样保留 |
| 配置 | `__traits` 反射 + 手搓键名拼接 | `serde` 派生嵌套结构体，`#[serde(default)]`；路径用 `dirs` |
| 并发下载 | `std.parallelism.TaskPool` | `tokio::spawn` + `tokio::sync::Semaphore` |
| 命令行解析 | UDA + `__traits` 手搓解析器 | `clap` derive |
| 交互式菜单 | 手写 `getch()` + ANSI TUI | `dialoguer`（+ 其再导出的 `console`） |
| 路径归一化 / hex / shell 转义 / UUID / 版本比较 | 手写 | `path-clean` / `hex` / `shell-words` / `uuid` / `version-compare` |
| 类型 | 路径用 `String`、尺寸/内存用 `int`、镜像与动作是裸字符串 | `PathBuf` / `&Path`、无符号整数、`Mirror`/`RuleAction`/`Side`/`JavaChoice` 枚举 |
| CLI 与界面 | CLI 与逻辑耦合 | 逻辑全部在 lib，CLI 只是其中一个前端 |

逐文件映射、成熟库替换审计（含"查过但故意保留"的清单）见 **`MIGRATION.md`**。

---

## 目录结构

```
dmcl-rs/
├── src/
│   ├── main.rs            # 二进制入口（原 dmcl.main）
│   ├── lib.rs             # 核心库根，供未来 GUI 复用
│   ├── env.rs             # 常量（原 dmcl.env）
│   ├── mcjson.rs          # Minecraft 各 JSON 文档的强类型定义（新增）
│   ├── utils.rs           # 通用工具（原 dmcl.utils）
│   ├── config.rs          # 配置（原 dmcl.config，改用 serde + dirs）
│   ├── account/           # 账号（原 dmcl.account）
│   ├── download/          # 下载（原 dmcl.download）
│   ├── launch/            # 启动（原 dmcl.launch）
│   └── cli/               # 命令行前端（原 dmcl.cli）
├── tests/online.rs        # 联网冒烟测试（默认 #[ignore]）
├── build.rs               # 编译期写入 build time
└── Cargo.toml
```

## 构建与运行

```bash
cargo build --release
./target/release/dmcl --help
```

调试构建会打印 `!!! DMCL SNAPSHOT !!!`（与原 `debug` 块一致）。

## 命令

| 命令 | 说明 |
| --- | --- |
| `launch <version-name>` | 下载并启动指定版本 |
| `install <mc-version-id> <version-name> [--forge] [--neoforge] [--fabric]` | 安装原版（可选装模组加载器） |
| `list-mc [--type=release,snapshot,...]` | 列出 Minecraft 版本 |
| `list-forge <mc-version-id>` | 列出 Forge 版本 |
| `list-neoforge <mc-version-id>` | 列出 NeoForge 版本 |
| `list-fabric <mc-version-id>` | 列出 Fabric 版本 |
| `install-forge <version-name> <forge-version-id>` | 在指定版本上安装 Forge |
| `install-neoforge <version-name> <neoforge-version-id>` | 在指定版本上安装 NeoForge |
| `search-java` | 自动搜索本机 Java 并写入配置 |
| `prompt` | 交互式菜单（默认命令） |
| `version` / `config` / `edit-config` | 版本 / 查看配置 / 提示配置文件位置 |

全局选项：`--java <PATH>`（同时写入命名 java 配置）、`--mirror <official|bmclapi>`。
不带子命令时执行配置里的 `cli.default_command`（默认 `prompt`）。

## 配置文件

位置：`dirs::config_dir()/dmcl.json`
（Linux `$XDG_CONFIG_HOME` 或 `~/.config`，Windows `%APPDATA%`，macOS `~/Library/Application Support`）。

键名与原 D 版本完全兼容（原来的 `launch_java` 就是现在的 `launch.java`）：

```json
{
  "launch": {
    "java_configs": { "openjdk-64-bit-17": "/usr/lib/jvm/java-17/bin/java" },
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
  "account":  { "online_mode": false, "username": "player" },
  "download": { "max_tasks": 64, "i_dont_need_music": false, "mirror": "official" },
  "cli":      { "default_command": "prompt" }
}
```

缺失的键沿用默认值（与原读取逻辑一致）。

## 为 GUI 预留的扩展点

* 全部业务逻辑位于库 crate，`main.rs` 只做「读配置 → 解析参数 → 分发」。
* 核心函数均为 `async fn`，可直接在任意 GUI 的异步运行时中调用。
* 配置通过 `config::with_config` / `with_config_mut` / `set_config` 访问，GUI 可整体替换。
* 下载并发由 `DownloadManager` 封装，可脱离全局单例自建实例；每个下载任务的进度点集中在
  `download_one`，便于接进度条。
* `GameMonitor` 持有子进程句柄，GUI 可实现「停止游戏」。
* `cli` 只依赖 `clap`/`dialoguer`，删除该模块即可换成其他前端。

## 测试

```bash
cargo test                                  # 24 个离线单测（算法等价性 + 类型化解析/合并/枚举）
cargo test --test online -- --ignored       # 3 个联网冒烟测试（需可访问 Mojang）
```

## 许可

GPL-3.0-or-later，与原项目一致。
