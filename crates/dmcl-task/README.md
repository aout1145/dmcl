# dmcl-task

与 Minecraft 无关的通用异步任务图：节点之间只有依赖边，下游通过依赖注入直接拿到上游
返回值；Step 只用于显示，全局进度只有 `downloaded / total` 字节数。

`dmcl-task` 是 workspace 最底层的叶子 crate，不依赖其他成员 crate。它不读配置，也不持有
全局状态。

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

## 设计意图

### 只有 DAG，没有任务树

任务图由调用者用 `TaskGraphBuilder::add` 原地追加静态节点，依赖只能指向已经创建的节点，
因此构图天然无环。`build(sink)` 显式指定唯一最终节点并检查图的完整性；`run` 返回最终节点
的输出，事件通过回调或事件总线交给前端。

### Task 与依赖注入

`Task<I, O>` 统一描述「输入 → 输出 + 任务上下文」。没有依赖时 `I = ()`，一个依赖时
`I = Arc<T>`，多个依赖用 tuple，数量不定的同类型依赖用 `Vec<Handle<T>>`。这样节点之间
不需要共享中间状态，也不需要从磁盘反推上游结果。

### 静态与动态句柄

静态 `Handle<O>` 只能作为下游节点的依赖；运行期动态创建的节点用 `DynamicHandle<O>`，
额外提供等待能力，但不能写进其它节点的依赖。两种句柄都能转成事件 / UI 使用的 `TaskRef`。

### 动态节点

`TaskContext::add_task` 允许在运行期按需追加节点，动态节点自动成为创建者的依赖并随创建者
一起结束。最终输出仍由构图时指定的静态节点决定。

### Step 与事件

Step 只是显示分组：`GraphStarted` 一次性带出全部静态 Step，之后由任务事件推导各 Step 的
等待 / 运行 / 完成 / 失败 / 跳过计数。全局进度独立于 Step，只表示字节数；事件总线按订阅者
游标保留事件，慢订阅者不会丢事件。

### 失败、取消与回滚

采用固定 FailFast：任一节点失败即全局取消。取消可由任意线程触发。`TaskContext::on_rollback`
用于登记当前节点的清理动作：节点失败 / 取消时立即执行，节点成功则转入图级清理栈，整张图
失败时按完成逆序执行。动态子节点失败也算创建者失败。

### 外部程序与警告

`CommandTask` 负责运行外部程序，可把输出逐行变成日志事件，或在继承终端模式下直接透传。
取消时子进程随 drop 被杀掉。`TaskContext::confirm` 让节点阻塞等待前端对警告做决定，其它
节点继续调度；没有订阅者时直接继续。
