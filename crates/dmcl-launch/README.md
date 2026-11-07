# dmcl-launch

账号、命令行生成、启动与状态监控。

在 workspace 内依赖 `dmcl-game`（版本解析、完整性 / 补全、natives）、`dmcl-java`（Java
解析与 JRE 自动安装）、`dmcl-mcmeta`（版本文档、目录布局、平台信息）和 `dmcl-task`。
它**不依赖 `dmcl-install`**，也**不读全局配置**：账户来源、版本引用、窗口 / 内存、
版本隔离、Java 解析器、JRE 安装位置全部通过 `LaunchOption` / `launch_chain` 由调用者传入。

## 模块布局

```text
account.rs   Account（登录快照）与 AccountSource（登录 / 刷新）
microsoft.rs 微软账户：设备代码登录、refresh token 刷新、Xbox / Minecraft 会话
memory.rs    堆内存的自动分配与解析
launcher.rs  拼启动命令行
option.rs    LaunchOption
prepare.rs   启动链：读取版本 → 检查并补全 / 选择 Java → 解压 natives → 登录 → 生成参数 → 启动
monitor.rs   GameMonitor：等待退出 / 停止游戏
```

## 设计要点

### 账户分两层

- `Account` 是登录完成后用来拼命令行的只读快照（名字 / uuid / token / 类型）；
- `AccountSource` 负责 `login` / `refresh` 并产出快照。

账户层不依赖启动链或任务系统；`LaunchOption` 只持有登录源，启动链的 `LoginTask` 只是
它的调用方之一，账号管理页以后也可以直接复用。

### 微软账户

- `device_code_login` 是交互式授权，只在交互式登录场景里调用：请求设备代码 → 通过
  `DeviceCodePresenter` 展示给用户 → 轮询令牌端点 → 换 Xbox Live / XSTS / Minecraft
  会话；返回账户快照与 refresh token；
- `MicrosoftAccountSource` 只做静默刷新：从 `RefreshTokenStore` 读凭据换新会话。凭据里
  记着它属于哪个档案；uuid 与配置不一致或凭据缺失时按「尚未登录」返回错误，已失效时
  报「登录已过期」，不会在启动游戏时突然要求浏览器授权；
- 凭据怎么存由调用方实现 `RefreshTokenStore`（`dmcl` 用系统钥匙串），这个 crate 不依赖
  具体凭据库；
- 编译时通过 `DMCL_MICROSOFT_CLIENT_ID` 嵌入 Azure 公共客户端 ID；未设置时登录返回
  一条说明怎么重新构建的错误。

### Java 选择

这里只使用 `dmcl_java::JavaResolver` 与 `JavaOption`，不依赖配置表或 `IndexMap`；
具体用哪个 Java 由调用者实现解析器决定。

启动链在选择 Java 时：

- 先调用 `JavaResolver::resolve(required_major)`；找不到或版本 / 架构不匹配，且传入了
  `JreAutoInstall`（即允许自动下载）时，用版本 JSON 的 `javaVersion.component` 与
  `required_major` 调 `dmcl_java::install_jre`；
- 下载并探测成功后调用 `JavaInstalledHook::on_installed`（GUI 用它写回配置）；
  CLI 与安装流程不设置 hook；
- 仍然不匹配时通过任务事件询问用户是否继续。

### 内存

`min_memory` / `max_memory` 都可以留空：`resolve_memory` 在选好 Java 后按系统可用内存的
3/4（至少 1 GiB）决定 Xmx；32 位 Java 压到 1.5 GiB（显式上限也会被压）；读不到可用内存
时固定 1.5 GiB；Xms 留空时取 Xmx。显式 `min_memory` 大于最终上限时原样保留，由调用方
决定是否告警。

### 版本隔离

`LaunchOption::version_isolation` 决定游戏进程的工作目录与 `${game_directory}`：

- `true`（默认）：使用版本目录 `versions/<version_name>`，每个版本的存档、mods 和配置互不影响；
- `false`：使用 Minecraft 根目录，所有版本共用同一套游戏数据。

安装文件、库和资源的路径不受影响，仍然来自根目录与版本目录。

### 启动与文件物化解耦

`GameLauncher` 只负责拼命令行，起进程由 `LaunchTask` 完成；文件完整性由
`dmcl_game::check_integrity` 负责，**是否修复交给调用者**。想要「缺了就补」的一站式
流程时，用 `launch_chain`。

```rust
// 文件物化在 dmcl-game：只回答"完整吗？缺什么？"，不下载任何东西
let resolved = dmcl_game::resolve_version(&option.version).await?;
let status = dmcl_game::check_integrity(&resolved, &game).await?;
if !status.is_complete() {
    // 由调用者决定：补全（已存在的文件会被跳过）
    let downloads = game.download_context(ctx.progress_handle());
    dmcl_game::ensure_files(&resolved, &game, &downloads, &ctx).await?;
}

// dmcl-launch 只负责拼参数，起进程走任务图；账户由登录源产出
let account = option.account_source.login().await?;
let mut launcher = GameLauncher::new(option.clone(), account).await?;
// ctx 来自任务图节点里的 TaskContext
let _command = launcher.generate_command(&ctx).await?; // 展示 / 复制用

let monitor = launch_chain(option, game, None).run(|_| {}).await?;
let mut monitor = std::sync::Arc::try_unwrap(monitor)
    .map_err(|_| anyhow::anyhow!("游戏监控句柄仍被共享"))?;
monitor.wait().await?;
```

### 一站式启动链

`launch_chain(option, game, jre)` 组成 `TaskGraph<GameMonitor>`：

1. **读取版本** —— `resolve_version` 解析版本 JSON（含一层 `inheritsFrom`）；
2. **检查并补全文件** —— `dmcl_game::check_integrity`；缺件时 `ensure_files`，产出
   `ResolvedVersion`；
3. **选择 Java** —— 读取版本要求的 Java 大版本，通过 `JavaResolver` 选好 Java；允许自动
   安装时下载 Mojang 的 JRE 并探测；版本 / 架构仍不匹配时先等用户确认；
4. **解压 natives** —— 依赖已解析的版本与已选 Java，按当前平台的 os 与 Java 的 arch 跑
   `dmcl_game::ensure_natives`（下载 + 解压到 `natives_dir`，即
   `<临时目录>/dmcl-natives-<os>-<arch>-<版本名哈希>`）；
5. **登录** —— 等文件补全和 natives 都完成后，让 `AccountSource` 产出可用会话，失败就中止；
6. **生成启动参数** —— 同时依赖账户与选好的 Java，只在两者都就绪后才拼；
7. **启动游戏** —— 起子进程，产出**未 wait** 的 `GameMonitor`，由调用者决定等待退出
   还是中途停掉。

读取版本后，检查文件与选 Java 并行；natives 同时需要版本与 Java，因此接在两者之后。登录
刻意等到文件补全与 natives 都结束，避免在线会话在长时间下载中过期；参数只在文件齐备、
Java 选好、账户可用后才拼，避免在缺件时生成一份用不了（或需要重拼）的命令行。整张图共享
一个 `DownloadContext`，连接 / 文件 / worker 预算与全局进度都只有一份。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 版本规划、完整性检查与 natives：[crates/dmcl-game/README.md](../dmcl-game/README.md)
- Java 探测与 JRE 安装：[crates/dmcl-java/README.md](../dmcl-java/README.md)
- 任务图、Step 与事件：[crates/dmcl-task/README.md](../dmcl-task/README.md)
