# dmcl-launch

账户、命令行生成、启动与状态监控。

在 workspace 内依赖 `dmcl-game`、`dmcl-java`、`dmcl-mcmeta` 和 `dmcl-task`。它**不依赖
`dmcl-install`**，也**不读全局配置**：账户来源、版本引用、Java 解析器、JVM 兼容性、文件
完整性、内存偏好、窗口 / 快速游玩 / 附加参数、进程工作目录与优先级全部由调用者传入。

## 模块布局

```text
account.rs    Account（登录快照）与 AccountSource（登录 / 刷新）
microsoft.rs  微软账户：设备代码登录、refresh token 刷新、Xbox / Minecraft 会话
memory.rs     MemoryOptions（用户偏好）、MemorySize（确定值）与自动分配
arguments.rs  GameLaunchOptions、generate_arguments、GenerateArgsTask
java.rs       JvmCompatibility、SelectJavaTask
files.rs      GameIntegrity、文件 / natives 准备任务
chain.rs      LaunchChainOptions、读取版本 / 登录任务、launch_chain
monitor.rs    ProcessOptions、游戏进程启动、等待退出 / 停止
```

## 设计意图

### 三层选项

- `LaunchChainOptions` 只控制启动链：账户来源、版本、Java 解析器、JVM 兼容性、文件完整性、
  内存偏好。
- `GameLaunchOptions` 只控制生成参数：窗口、快速游玩、客户端 ID、自定义信息、附加 JVM /
  游戏参数。
- `ProcessOptions` 只控制游戏进程：工作目录与优先级。

`version_isolation` 不是启动参数，也不再作为布尔值向下传：配置层按它算出 `game_dir` 与
`ProcessOptions.working_dir`。`generate_arguments` 用 `game_dir` 替换 `${game_directory}`，
`spawn_game` 用 `working_dir` 作为子进程的 `current_dir`；两者互不推导。`launch_chain`
要求两者一致，参数错误会在构图时立刻暴露。

### 账户分两层

`Account` 是登录完成后用于拼命令行的只读快照，`AccountSource` 负责登录 / 刷新并产出快照。
账户层不依赖启动链或任务系统，启动链只是它的调用方之一，账户管理页也可以直接复用。

### 微软账户

设备代码登录只负责交互式授权：请求设备代码、展示给用户、轮询令牌并换取 Xbox / Minecraft
会话。`MicrosoftAccountSource` 只做静默刷新，凭据由调用方实现 `RefreshTokenStore` 保存，
这个 crate 不依赖具体凭据库。凭据缺失或失效时按「尚未登录 / 登录已过期」返回错误，不会在
启动游戏时突然要求浏览器授权。Azure 客户端 ID 通过编译期环境变量嵌入。

### Java 与内存

启动链只使用 `dmcl_java::JavaResolver` 与 `JavaOption`：`SelectJavaTask` 负责选 Java 并产出
`JavaOption`，`generate_arguments` 只接收已选好的 Java，不再自己解析。找不到 Java 且
`JvmCompatibility::Complete` 时才下载 JRE；`Ignore` 直接使用，`Check` 在不匹配时等用户确认。
`Complete` 内嵌 `java_installed_hook`，安装成功后交给调用方决定是否写回配置；`Ignore` /
`Check` 不需要 hook，也不会触发自动安装。

内存偏好放在 `LaunchChainOptions.memory`（`MemoryOptions`）。`GenerateArgsTask` 在拿到已选
Java 后按它的架构调用 `resolve_memory`，把确定下来的 `MemorySize` 传给
`generate_arguments`；后者只展开 `-Xms` / `-Xmx`，不做自动分配。

### 生成启动参数

`GameLauncher` 已删除，启动参数由自由函数 `generate_arguments` 生成，`GenerateArgsTask` 只是
任务适配器。它拼 classpath、主类、JVM / 游戏参数、快速游玩与窗口参数，并替换模板；需要用户
确认的警告（库版本无法比较、Xms 大于 Xmx）通过 `TaskContext` 上报。

### 进程与快速游玩

`GameLaunchOptions.quick_play` 对应 Minecraft 1.20+ 的 `--quickPlaySingleplayer` /
`--quickPlayMultiplayer` / `--quickPlayRealms`，由 `generate_arguments` 拼进游戏参数。
`ProcessOptions.priority` 只影响起进程：Windows 用进程创建标志，Unix 用 `nice`；Unix 上负
nice 需要权限，没有权限时保持当前优先级启动。

### 启动与文件物化解耦

`generate_arguments` 只负责拼命令行，起进程由 `LaunchTask` 完成；文件完整性由
`dmcl_game::check_integrity` 负责，**是否修复交给调用者**。需要「缺了就补」的一站式流程时，
用 `launch_chain`：它按「检查游戏完整性 → 登录 → 启动」组织任务图，文件检查与 Java 选择
可以并行，natives 同时依赖二者；登录等到文件与 natives 都完成后再进行，避免在线会话在
长时间下载中过期。整张图共享一个 `DownloadContext`。

`LaunchChainOptions.game_integrity` 决定这条链对文件缺失的处理：忽略直接跳过检查；仅检查
列出缺失项、等用户确认后继续，不补全；检查并补全在缺失时下载修复。只有检查并补全会下载
缺失的 natives jar，其余模式只解压本地已有的 jar；安装流程已按当前平台下载好 natives。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 版本规划、完整性检查与 natives：[crates/dmcl-game/README.md](../dmcl-game/README.md)
- Java 探测与 JRE 安装：[crates/dmcl-java/README.md](../dmcl-java/README.md)
- 任务图、Step 与事件：[crates/dmcl-task/README.md](../dmcl-task/README.md)
