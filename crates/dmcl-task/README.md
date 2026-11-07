# dmcl-task

与 Minecraft 无关的通用异步任务图：节点之间只有依赖边，下游通过依赖注入直接拿到上游
返回值；Step 只用于显示，全局进度只有 `downloaded / total` 字节数。

`dmcl-task` 是 workspace 最底层的叶子 crate，不依赖其他成员 crate（只用 `tokio`、
`anyhow` 与 `async-trait`）。它不读配置，也不持有全局状态。

## 模块布局

```text
task.rs        Task trait、闭包适配器 task_fn
deps.rs        Deps、Handle / DynamicHandle、TaskHandle / WaitableTaskHandle、Dependency
graph.rs       TaskGraphBuilder / TaskGraph、StepId / StepDef、构图检查
runtime.rs     调度器、节点状态、FailFast、取消、动态节点、清理
context.rs     TaskContext、CancelHandle、AddTaskError、cleanup 注册
event.rs       TaskEvent / TaskRef / TaskStatus
bus.rs         可增长事件总线
progress.rs    ProgressHandle / ProgressSnapshot / human_bytes
command.rs     CommandTask：运行外部程序
warning.rs     WarningDecision：需要前端决策的警告
```

## 设计要点

### 只有 DAG，没有任务树

- `TaskGraphBuilder::add(step, name, deps, task)` 原地追加静态节点，只返回该节点的
  `Handle<O>`；
- 依赖只能指向已经创建的静态节点，因此构图天然无环；
- `build(sink)` 显式指定唯一最终节点，并检查：
  - sink 属于这张图；
  - sink 没有 dependent；
  - 除 sink 外每个静态节点都有 dependent；
- `run(|event| …)` 返回最终节点的 `Arc<O>`；不关心事件时传 `|_| {}`。

### Task 与依赖注入

```rust
#[async_trait::async_trait]
pub trait Task<I, O>: Send + Sync {
    async fn run(&self, input: I, ctx: TaskContext) -> anyhow::Result<O>;
}
```

- 没有依赖：`I = ()`；
- 一个依赖：`I = Arc<T>`；
- 多个依赖：`I = (Arc<A>, Arc<B>, …)`；
- `task_fn(|input, ctx| async move { … })` 统一接收输入与上下文；
- 依赖值不需要时写 `_` 或解构时忽略。

### 两种句柄

- `Handle<O>`：静态句柄，**没有 `wait`**，只能作为下游节点的 deps 被注入；
- `DynamicHandle<O>`：运行期动态节点，额外提供 `wait()`（等待 + 强制取值）；
- `DynamicHandle` **不实现 `Deps`**，不能写进别的节点的 deps；
- 两种句柄都能通过 `.task_ref()` 变成事件 / UI 用的 `TaskRef`。

### 动态节点

- `TaskContext::add_task(step, name, deps, task)` 在运行期追加节点；
- 动态节点自动成为创建者的依赖，创建者结束时它们必然已经结束；
- 需要返回值时才保留 `DynamicHandle` 并 `wait()`；
- 动态节点不能作为最终输出；最终输出是 `build(sink)` 指定的静态节点；
- 动态任务使用已有 `StepId`；第一版不支持运行期新增 Step。

### Step 与事件

- `GraphStarted` 一次性带出全部静态 `StepDef`；
- Step 计数由 `TaskAdded / TaskStarted / TaskFinished / TaskFailed / TaskSkipped` 推导；
- 失败 / 跳过计入「已完成」，Step 额外记录 failed；
- `TaskEvent::Progress` 是全局快照，不带任务；
- 事件总线按订阅者 cursor 保留事件，慢订阅者不会丢事件；没有订阅者时不保留。

### 失败、取消与回滚

- 固定 `FailFast`：任一节点失败即全局取消；等待中节点 `Skipped`，运行中节点取消后
  `Failed`；
- `CancelHandle` 可从任意线程触发取消；
- `TaskContext::on_rollback` 在**当前节点**登记清理动作：
  - 节点失败 / 取消：立即按注册逆序执行；
  - 节点成功：追加到图级清理栈；图最终失败时按完成逆序执行，成功时不执行；
- 动态子节点也算创建者的一部分：任一动态子节点失败，创建者节点失败。

### 全局进度

- `ProgressHandle::add_total(i64)` / `add_downloaded(i64)` 支持正负调整；
- 快照把负值 clamp 到 0；
- 图每 100ms 左右发一条 `Progress`，结束时发最终快照；
- 非下载任务不写进度，只改变 Step 计数。

### 运行外部程序

`CommandTask` 跑外部程序，默认把 stdout / stderr 逐行变成日志事件，并保留末尾若干行用于
失败信息；`OutputMode::Inherit` 可让子进程直接继承终端。取消时子进程被 `kill_on_drop`
杀掉。

### 警告确认

`TaskContext::confirm` 发出警告并阻塞**当前节点**等待答复；其他节点继续调度。没有订阅者
时直接继续；`abort` 会全局取消整张图。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 下载上下文如何创建单文件任务：[crates/dmcl-download/README.md](../dmcl-download/README.md)
