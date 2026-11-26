# dmcl-launch

账户、命令行生成、启动与状态监控。

在 workspace 内依赖 `dmcl-game`、`dmcl-java`、`dmcl-mcmeta` 和 `dmcl-task`。它**不依赖
`dmcl-install`**，也**不读全局配置**：账户来源、版本引用、窗口模式 / 快速游玩 / 进程优先级 /
内存、自定义信息、版本隔离、客户端 ID、Java 解析器、JRE 安装位置全部通过 `LaunchOption` /
`launch_chain` 由调用者传入。

## 模块布局

```text
account.rs   Account（登录快照）与 AccountSource（登录 / 刷新）
microsoft.rs 微软账户：设备代码登录、refresh token 刷新、Xbox / Minecraft 会话
memory.rs    堆内存的自动分配与解析
launcher.rs  拼启动命令行
option.rs    LaunchOption、窗口 / 快速游玩 / 进程优先级 / 自定义信息 / JVM 兼容性 / 游戏完整性选项
prepare.rs   启动链：读取版本 → 检查 / 补全文件、选择 Java → 解压 natives → 登录 → 生成参数 → 启动
monitor.rs   游戏进程：启动、等待退出 / 停止
```

## 设计意图

### 账户分两层

`Account` 是登录完成后用于拼命令行的只读快照，`AccountSource` 负责登录 / 刷新并产出快照。
账户层不依赖启动链或任务系统，启动链只是它的调用方之一，账户管理页也可以直接复用。

### 微软账户

设备代码登录只负责交互式授权：请求设备代码、展示给用户、轮询令牌并换取 Xbox / Minecraft
会话。`MicrosoftAccountSource` 只做静默刷新，凭据由调用方实现 `RefreshTokenStore` 保存，
这个 crate 不依赖具体凭据库。凭据缺失或失效时按「尚未登录 / 登录已过期」返回错误，不会在
启动游戏时突然要求浏览器授权。Azure 客户端 ID 通过编译期环境变量嵌入。

### Java 与内存

启动链只使用 `dmcl_java::JavaResolver` 与 `JavaOption`，选哪个 Java 由调用者决定；找不到
且允许自动安装时，按版本要求下载 JRE，并通过 hook 把结果交给调用方决定是否写回配置。
`LaunchOption::jvm_compatibility` 决定是否检查所选 Java 的版本与架构：忽略直接使用，仅检查
在不匹配时等用户确认，检查并补全只对允许自动安装的解析器下载 JRE。
内存留空时按系统可用内存自动分配，手动值不合法时仍交给调用方决定如何处理。

### 版本隔离

`LaunchOption::version_isolation` 决定游戏进程的工作目录与 `${game_directory}`：开启时用
版本目录，每个版本的存档、mods 和配置互不影响；关闭时用 Minecraft 根目录。安装文件、库
和资源的路径不受影响。

### 快速游玩与进程优先级

`LaunchOption::quick_play` 对应 Minecraft 1.20+ 的 `--quickPlaySingleplayer` /
`--quickPlayMultiplayer` / `--quickPlayRealms`，由 `GameLauncher` 拼进游戏参数。
`LaunchOption::process_priority` 只影响起进程：Windows 用进程创建标志，Unix 用 `nice`；
Unix 上负 nice 需要权限，没有权限时保持当前优先级启动。

### 启动与文件物化解耦

`GameLauncher` 只负责拼命令行，起进程由 `LaunchTask` 完成；文件完整性由
`dmcl_game::check_integrity` 负责，**是否修复交给调用者**。需要「缺了就补」的一站式流程时，
用 `launch_chain`：它按「检查游戏完整性 → 登录 → 启动」组织任务图，文件检查与 Java 选择
可以并行，natives 同时依赖二者；登录等到文件与 natives 都完成后再进行，避免在线会话在
长时间下载中过期。整张图共享一个 `DownloadContext`。

`LaunchOption::game_integrity` 决定这条链对文件缺失的处理：忽略直接跳过检查；仅检查列出
缺失项、等用户确认后继续，不补全；检查并补全在缺失时下载修复。只有检查并补全会下载缺失的
natives jar，其余模式只解压本地已有的 jar；安装流程已按当前平台下载好 natives。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 版本规划、完整性检查与 natives：[crates/dmcl-game/README.md](../dmcl-game/README.md)
- Java 探测与 JRE 安装：[crates/dmcl-java/README.md](../dmcl-java/README.md)
- 任务图、Step 与事件：[crates/dmcl-task/README.md](../dmcl-task/README.md)
