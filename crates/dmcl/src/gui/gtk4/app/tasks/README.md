# 任务窗口与任务卡片

`crates/dmcl/src/gui/gtk4/app/tasks` —— 任务列表窗口、任务卡片、警告确认。
app 层见 [app/README.md](../README.md)，整体架构见
[gtk4/README.md](../../README.md)。

## 窗口与生命周期

- 任务列表是一个独立的**模态 `AdwWindow`**（`transient_for` 主窗口，用户可以拖动边缘改
  尺寸，默认 640×720）：标题栏按钮发 `TaskMsg::Open` → `present()`。之所以不用
  `AdwDialog`，是因为它不支持用户拖动改尺寸。
- 关闭（`close-request`）时直接隐藏窗口，**不取消任务**，任务继续在后台运行。
- 关闭**主窗口**时由 `TaskCenter` 处理 `TaskMsg::CloseMain`：没有未结束任务就直接回
  `TaskCenterOut::Quit`；有任务则弹确认，确认后发 `TaskMsg::CancelAllAndQuit`，
  取消所有未结束任务再回 `Quit`。任务卡片进入「取消中」。
- 窗口本身只是隐藏（保留尺寸与位置），下次打开沿用；隐藏时只清掉**成功**的任务卡片，
  失败、已取消和取消中的任务保留。
- **任务起止有提示**：`TaskMsg::Start` 时弹「任务「…」开始」，`TaskMsg::Finished` 时弹
  「任务「…」完成 / 失败 / 已取消」。任务属于 app 层，这些提示走 `global_toast`，
  不依赖发起任务的页面是否还开着。
- 没有任务时显示 `AdwStatusPage` 空状态（`GtkStack` 切换，由 `cards.is_empty()` 驱动）。
- 每次处理消息后把**未结束**（运行中 / 取消中）的任务数通过
  `TaskCenterOut::RunningCount` 上报，标题栏据此显示 / 隐藏任务按钮角标。

## 任务卡片

每个任务 = 一条 `TaskChain` 的一次运行。卡片是 **`FactoryComponent`**，视图用 `view!`
声明；任务列表是 `FactoryVecDeque<TaskCard>`，模型一变就由工厂同步控件。

卡片从上到下：

1. 状态图标 + 任务名 + 右上角的叉；
2. 同一行里的「步骤 a/b」和当前步骤名；
3. 进度条上方的说明：百分比 / 已下载与总字节 / 任务自己的计数（下载是「35/120 个文件」）；
4. 进度条（文字全部放在上方，条内不再显示百分比）；
5. 可展开的日志。

状态机：

- **运行中**：左侧 `AdwSpinner`，显示总进度、当前步骤、进度说明与进度条；没有百分比时用
  pulse 当不确定进度条。
- **取消中**：点了叉、已发出取消但任务还没收尾。显示「取消中…（步骤 a/b）」，叉变灰不可再点。
- **已完成**：左侧对勾，只显示「已完成」。
- **已取消**：左侧停止图标，显示「已取消」。
- **失败**：左侧错误图标，直接显示任务链的错误文本（里面已经带了出错的步骤名）。

右上角的叉：运行中 = 取消；取消中 = 禁用；已结束 = 清除这条记录（`CardOut::Close`，
由任务窗口按状态分派）。取消请求由 `Shell` 识别任务返回的 `Cancelled` 错误，最后落到
「已取消」而不是「失败」。

## 进度还原

总进度用**栈**还原层级，只统计**顶层**步骤完成（嵌套子链会把步骤事件原样转发到父链，不能
直接数 `StepFinished`）：`StepStarted` push、`StepFinished` pop，栈由非空变空才 +1。

分项进度取栈顶步骤最近一条 `Progress`，`TaskState::progress_text` 把它拼成一行
（百分比 / 字节 / `message`）；没有百分比时用 `pulse` 当不确定进度条。pulse 定时器由卡片的
`post_view` 按需启停，状态变化后不会留下空转的定时器。

**日志用 `GtkTextView` 增量追加**：下载这类会产生大量文件日志的任务，进度事件很频繁，
如果每条日志都 `join` 全文并重设控件，会让进度条卡顿。日志到达时只把新增行 `insert`
进 `TextBuffer`；展开时等布局完成后滚到最后一行。

`task_view.rs` 是纯逻辑，单测覆盖：顶层步骤计数、当前步骤进度、日志累积、结束状态、
取消中到已取消的转换，以及进度说明的拼法。

## 退出确认

主窗口的关闭策略放在 `TaskCenter`，因为未结束任务数就是它从卡片列表算出来的，不需要在
根组件再缓存一份。`TaskMsg::CloseMain` 到达时直接查 `cards`：确认框的 parent 也按
“任务窗口开着就挂它，否则挂主窗口”选择，避免被模态任务窗口挡住。

## 警告确认

任务通过 `TaskEvent::Warning` 发来需要拍板的警告，且会阻塞等待。`TaskCenter` 收到后直接用
`confirm.rs` 弹 `AdwAlertDialog`：

- parent 取**当前最上层的窗口**：任务窗口开着就挂任务窗口，关着就挂主窗口，不会为了弹确认框
  把任务列表带出来；
- **标题是任务名**；
- 选「继续 / 中止」后调 `WarningDecision::accept()` / `abort()`。

## 消息

| 类型 | 文件 | 说明 |
| --- | --- | --- |
| `TaskMsg` | `message.rs` | `Open` / `Start` / `Event` / `Finished { id, ok, cancelled, error }` / `CloseCard` / `CloseRequested` / `CloseMain` / `CancelAllAndQuit` |
| `TaskCenterOut` | `mod.rs` | `RunningCount(n)`：当前未结束任务数，给标题栏角标；`Quit`：根组件可以销毁主窗口了 |
| `CardMsg` | `task_card.rs` | 日志区展开状态变化 |
| `CardOut` | `task_card.rs` | `Close(DynamicIndex)`：点卡片右上角的叉 |

## CSS

`tasks::CSS` 给任务日志区 `.dmcl-task-log` 一个明确的边框。
