# 任务窗口与任务卡片

`crates/dmcl/src/gui/gtk4/app/task_popup` —— 任务列表窗口、任务卡片、警告确认。
app 层见 [app/README.md](../README.md)，整体架构见
[gtk4/README.md](../../README.md)。

## 窗口与生命周期

- 任务列表是一个独立的**模态 `AdwWindow`**（`transient_for` 主窗口，用户可以拖动边缘改
  尺寸）：标题栏按钮发 `TaskMsg::Open` → `present()`。之所以不用 `AdwDialog`，是因为它不
  支持用户拖动改尺寸。
- 关闭（`close-request`）时发 `TaskMsg::ClearFinished` 并**隐藏**窗口（保留尺寸与内容，
  下次打开沿用），把**已结束**的任务卡片清掉，仍在运行的保留。
- **任务起止有提示**：`TaskMsg::Start` 时弹「任务「…」开始」，`TaskMsg::Finished` 时弹
  「任务「…」完成 / 失败」。
- 没有任务时显示 `AdwStatusPage` 空状态（`GtkStack` 切换）。

## 任务卡片

每个任务 = 一条 `TaskChain` 的一次运行。卡片从上到下：状态图标 + 名称 + 右上角的叉 /
状态行 / 分项进度 / 可展开日志。

- **运行中**：左侧是 `AdwSpinner`，显示总进度「步骤 x/y」、当前步骤名与进度条（没有百分比时
  用 pulse 当不确定进度条）。
- **已完成**：左侧对勾，不显示任何进度，只显示「已完成」。
- **失败**：左侧错误图标，不显示进度，直接显示任务链的错误文本（里面已经带了出错的步骤名）。
- 日志用等宽字体（`caption` 字号），放在带边框的 `.dmcl-task-log` 区域里，用 `GtkExpander`
  展开 / 收起（`AdwExpanderRow` 是偏好设置行，不适合卡片）。
- 右上角的叉：运行中 = 取消；已结束 = 清除这条记录（`TaskMsg::Close`，由任务组件按状态分派）。

## 进度还原

总进度用**栈**还原层级，只统计**顶层**步骤完成（嵌套子链会把步骤事件原样转发到父链，不能
直接数 `StepFinished`）：`StepStarted` push、`StepFinished` pop，栈由非空变空才 +1。

分项进度取栈顶步骤最近一条 `Progress`；没有百分比时用 `pulse` 当不确定进度条。

这段是纯逻辑，在 `task_view.rs` 里单测：顶层步骤计数、当前步骤进度、日志累积、结束状态，
以及「链自己的名字不能覆盖前端给的展示名」。

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
| `TaskMsg` | `message.rs` | `Open` / `Start` / `Event` / `Finished { id, ok, error }` / `Close` / `ClearFinished` |

## CSS

`task_popup::CSS` 给任务日志区 `.dmcl-task-log` 一个明确的边框。
