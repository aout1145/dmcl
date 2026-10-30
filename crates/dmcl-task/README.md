# dmcl-task

与 Minecraft 无关的通用异步任务抽象：每个任务**接收一个输入、产出一个输出**，用
`TaskChain` 按顺序串起来；进度和日志只通过 `TaskEvent` 事件流上报。

`dmcl-task` 是 workspace 最底层的叶子 crate，不依赖其他成员 crate（只用 `tokio` 与
`anyhow`）。它不读配置，也不持有全局状态：所有输入由调用者通过任务本身传入。

## 模块布局

```text
task.rs      Task trait、BoxFuture、闭包适配器 task_fn / task_fn_io
chain.rs     TaskChain：串联、运行、事件订阅、中止回滚；自身也实现 Task，可嵌套
context.rs   TaskContext（进度 / 日志 / 取消）与 CancelHandle
event.rs     TaskEvent 事件流
progress.rs  Progress 快照与 human_bytes
command.rs   CommandTask：运行外部程序
warning.rs   WarningDecision：需要前端决策的警告
```

## 设计要点

### Task 与 TaskChain

- `Task` 的 `run` 接收输入 `I`、产出输出 `O`，用装箱 future（`BoxFuture`）表达，
  因此 `Box<dyn Task>` 可以直接当 trait 对象用，**不需要 `async-trait` 依赖**；
- `TaskChain::then(任务, 显示名)` 把上一个任务的输出接到下一个任务的输入上，
  **任一步失败即中止后续**并把错误原样返回；步骤的显示名由链传入，同一个任务可以以
  不同名字出现在不同链里；
- `run` 传入初始输入并返回最终输出；`run_with` 是「一边跑一边把事件交给回调」的便捷
  写法；`subscribe` 拿到一条事件流，适合自己控制消费节奏的前端；
- `TaskChain` 本身也实现 `Task`，所以子链可以当成父链的一个步骤；子链的事件会**原样
  转发**到父上下文（链级的开始 / 结束除外），父链订阅者能看到嵌套的每一步。

```rust
let chain = TaskChain::new("安装 1.20.1")
    .then(vanilla_metadata_task(..), "获取原版元数据")
    .then(install_fabric_task(..), "安装 Fabric")
    .then(game_files_task(..), "下载游戏文件");

// 事件流：一边跑一边打印（`()` 是这条链的初始输入）
chain.run_with((), |event| println!("{event}")).await?;
```

手动订阅（GUI 场景）：

```rust
let mut chain = /* … */;
let mut rx = chain.subscribe();
let handle = tokio::spawn(async move { chain.run(()).await });
while let Ok(event) = rx.recv().await { /* 实时进度 */ }
handle.await??;
```

### 进度、日志与事件

- `TaskContext::progress(current, total, message, percent)` 上报**可原地刷新**的量化进度；
  百分比由任务自己算好，`None` 表示没有有意义的百分比；
- `TaskContext::log` 上报**一次性文字**（命令行输出、缺件清单等），前端逐行打印，
  不要像进度那样覆盖；
- `TaskEvent` 走 `broadcast` 通道，一个任务可以上报任意多次，也可以被多个前端同时消费；
  链结束（成功或失败）后发送方被丢弃，订阅者的 `recv()` 返回 `Err`，循环自然结束；
- 链套链时，子链的**步骤级**事件会原样转发到父链（含 `StepStarted` / `StepFinished`），
  只有链级的 `Started` / `Finished` 丢弃。前端据此用**栈**还原嵌套层级：进入
  `StepStarted` push、`StepFinished` pop；
- 失败时先发一条 `Failed`（只带「哪一步失败」），错误详情在 `run` / `run_with` 的返回值里，
  避免事件流和返回值把同一条错误各打一遍。

每个操作都是一个 `TaskChain`；进度**只走 `TaskEvent` 事件流**，不用轮询、也不用全局状态
汇报，**不引入全局可变配置**。

### 取消与回滚

- `CancelHandle` 可从任意线程（含 GUI 事件循环）触发取消；任务用
  `check_cancelled` / `cancelled` 感知，取消时返回 `Cancelled` 错误；
- `TaskChain::on_rollback` 在链的当前位置登记**中止回滚**动作：只有它前面的步骤都成功、
  运行真正到达这里时才生效；链随后因取消或某一步失败而中止时按登记逆序执行
  （成功结束不执行）。回滚是异步、尽力而为的，用于删掉写了一半的安装目录。

### 运行外部程序

`CommandTask` 跑外部程序，默认把 stdout / stderr **逐行变成日志事件**（Forge 的安装器
processor 就是这么跑的），并保留末尾若干行用于失败时构造错误信息；`OutputMode::Inherit`
可让子进程直接继承终端。取消时子进程被 `kill_on_drop` 杀掉。

### 警告确认

`WarningDecision` 让任务发出「警告」事件并**阻塞等待**前端决定继续还是中止；中止会让
任务被取消。若没有订阅者，任务不会阻塞、直接继续。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 下载任务如何实现 `Task`：[crates/dmcl-download/README.md](../dmcl-download/README.md)
