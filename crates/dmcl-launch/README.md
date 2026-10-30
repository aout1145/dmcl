# dmcl-launch

账号、Java 探测 / 扫描 / 自动选择、命令行生成、启动与状态监控。

在 workspace 内依赖 `dmcl-install`（完整性检查与 natives）、`dmcl-mcmeta`（版本文档、
目录布局、平台信息）、`dmcl-download` 和 `dmcl-task`。它**不读全局配置**：账户来源、
根目录、版本名、窗口 / 内存、Java 选择器全部通过 `LaunchOption` 由调用者传入。

## 模块布局

```text
account.rs  Account（登录快照）与 AccountSource（登录 / 刷新）
java.rs     JavaOption / JavaDetails、JavaSelector、扫描与自动选择
memory.rs   堆内存的自动分配与解析
launcher.rs 拼启动命令行
option.rs   LaunchOption
prepare.rs  启动链：登录 → 检查并补全 → 准备 natives → 生成参数 → 启动
monitor.rs  GameMonitor：等待退出 / 停止游戏
```

## 设计要点

### 账户分两层

- `Account` 是登录完成后用来拼命令行的只读快照（名字 / uuid / token / 类型）；
- `AccountSource` 负责 `login` / `refresh` 并产出快照。

账户层不依赖启动链或任务系统；`LaunchOption` 只持有登录源，启动链的 `LoginTask` 只是
它的调用方之一，账号管理页以后也可以直接复用。

### Java 选择

这里只认识 `JavaOption` / `JavaDetails`，不依赖配置表或 `IndexMap`；具体用哪个 Java 由
调用者实现的 `JavaSelector` 决定。

- `default_java_roots()` 按平台给出常见安装目录与 `JAVA_HOME`；
- `find_java(roots)` 只返回扫描结果（扫不到就是空列表）；
- `auto_select_java(required_major, &options)` 只按缓存的 `major` / `arch` 选择、**不执行
  Java**：版本相等优先，其次最接近的更高版本，再其次最高的更低版本；同一档内优先当前
  平台架构，再按候选顺序；
- `ScanJava` 是没有配置表时的默认选择器：扫描常见目录后自动挑选。

### 内存

`min_memory` / `max_memory` 都可以留空：`resolve_memory` 在选好 Java 后按系统可用内存的
3/4（至少 1 GiB）决定 Xmx；32 位 Java 压到 1.5 GiB（显式上限也会被压）；读不到可用内存
时固定 1.5 GiB；Xms 留空时取 Xmx。显式 `min_memory` 大于最终上限时原样保留，由调用方
决定是否告警。

### 启动与安装解耦

`GameLauncher` 只负责拼命令行，起进程由 `LaunchTask` 完成；文件完整性由
`dmcl_install::check_integrity` 负责，**是否修复交给调用者**。想要「缺了就补」的一站式
流程时，用 `launch_chain`。

```rust
// 完整性检查在 dmcl-install：只回答"完整吗？缺什么？"，不下载任何东西
let status = dmcl_install::check_integrity(&install_options.plan(), version).await?;
if !status.is_complete() {
    // 由调用者决定：跑一次 dmcl-install 补全（已存在的文件会被跳过）
    game_files_task(install_options, version)
        .run_with((), |event| printer.handle(event))
        .await?;
}

// dmcl-launch 只负责拼参数，起进程走任务链；账户由登录源产出
let account = option.account_source.login().await?;
let mut launcher = GameLauncher::new(option.clone(), account).await?;
let _command = launcher.generate_command().await?; // 展示 / 复制用

let mut monitor = launch_chain(option, install).run(()).await?;
monitor.wait().await?;
```

### 一站式启动链

`launch_chain(option, install)` 把五步串成 `TaskChain<(), GameMonitor>`：

1. **登录** —— 先让 `AccountSource` 产出可用会话，失败就中止、不开始下载；
2. **检查并补全文件** —— 复用 `dmcl-install` 的完整性检查；缺件时跑一次安装补全；
3. **准备 natives** —— 先选好 java，再按当前平台的 os 与这个 java 的 arch 跑
   `natives_task`（下载 + 解压到 `natives_dir`，即
   `<临时目录>/dmcl-natives-<os>-<arch>-<版本名哈希>`）；
4. **生成启动参数** —— 只在文件齐备、java 选好后才拼，且复用同一个 java，不再选第二次；
5. **启动游戏** —— 起子进程，产出**未 wait** 的 `GameMonitor`，由调用者决定等待退出
   还是中途停掉。

「先登录、再补全文件、最后生成参数」是刻意的顺序：认证失败时立即中止，不会开始下载；
参数只在文件齐备后才拼，避免在缺件时生成一份用不了（或需要重拼）的命令行。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 完整性检查与 natives 下载：[crates/dmcl-install/README.md](../dmcl-install/README.md)
- 任务链与事件流：[crates/dmcl-task/README.md](../dmcl-task/README.md)
