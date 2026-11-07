# 任务对话框与任务卡片

`crates/dmcl/src/gui/gtk4/app/tasks` —— 任务对话框、任务卡片、警告确认。
app 层见 [app/README.md](../README.md)，整体架构见
[gtk4/README.md](../../README.md)。

## 对话框与生命周期

- 任务列表是一个**模态 `AdwDialog`**（`present` 在主窗口里，内容尺寸 640×720，
  主窗口更小时会自动缩小）：标题栏按钮发 `TaskMsg::Open` → `present()`。`AdwDialog`
  不能由用户拖动边缘改尺寸。
- 关闭（`closed`）时只是收起对话框，**不取消任务**，任务继续在后台运行。
- 关闭**主窗口**时由 `TaskCenter` 处理 `TaskMsg::CloseMain`：没有未结束任务就直接回
  `TaskCenterOut::Quit`；有任务则弹确认，确认后发 `TaskMsg::CancelAllAndQuit`，
  取消所有未结束任务再回 `Quit`。任务卡片进入「取消中」。
- 对话框本身只是收起（内容保留），下次打开沿用；收起时只清掉**成功**的任务卡片，
  失败、已取消和取消中的任务保留。
- **任务起止有提示**：`TaskMsg::Start` 时弹「任务「…」开始」，`TaskMsg::Finished` 时弹
  「任务「…」完成 / 失败 / 已取消」。任务属于 app 层，这些提示走 `global_toast`，
  不依赖发起任务的页面是否还开着。
- 没有任务时显示 `AdwStatusPage` 空状态（`GtkStack` 切换，由 `cards.is_empty()` 驱动）。
- 每次处理消息后把**未结束**（运行中 / 取消中）的任务数通过
  `TaskCenterOut::RunningCount` 上报，标题栏据此显示 / 隐藏任务按钮角标。

## 任务卡片

每个任务 = 一张 `TaskGraph` 的一次运行。卡片是 **`FactoryComponent`**，视图用 `view!`
声明；任务列表是 `FactoryVecDeque<TaskCard>`，模型一变就由工厂同步控件。

卡片从上到下：

1. 状态图标 + 任务名 + 右上角的叉；
2. 每个 Step 一行，**点击标题展开**：状态图标（`object-select` / `content-loading` /
   `radio` / `dialog-error`）+「步骤 N 名称」+ 右侧计数（`12/12 完成` / `5/8 运行中` /
   `等待中` / `未执行` / 失败数）+ **最右侧的展开箭头**；所有 Step 同时列出。展开后是
   该 Step 下的任务：任务名 + 状态文字（等待中 / 运行中 / 完成 / 失败 / 未执行）。
   展开区最多 240px 高，任务多时内部滚动；
3. 分隔线；
4. 单独一行的全局进度条（占满整行）；
5. 日志展开控件：左侧箭头 + 右侧「已下载 / 总字节」；
6. 展开后的日志区。

Step 默认折叠，展开 / 收起完全由用户点击决定；运行中、失败、取消都不会自动展开。
Step 行用嵌套的 `FactoryVecDeque<StepRow>`，任务明细再用一层
`FactoryVecDeque<TaskRow>`。标题是扁平按钮，展开区是 `GtkRevealer`；`StepRowMsg::Toggle`
切换行模型里的 `expanded`，`StepRowMsg::View` 只更新 Step 快照。因为更新不替换整行，
计数变化不会丢掉用户已经展开的状态；纯进度事件仍然不会重画整块列表。

**行数上限与排序**：下载会为每个文件创建一个节点，整个安装可能产生几千个任务，全部
渲染会直接卡死 UI。因此折叠的 Step **不创建任务行**；展开时最多渲染 50 条
（`task_view::MAX_VISIBLE_TASKS`），排序固定为 **运行中 → 等待中 → 其余（完成 / 失败 /
未执行）**，同一优先级内保持加入顺序；超出上限的任务在滚动内容的最末尾显示
「还有 N 个任务未显示」，滚到最底下才看得到。

状态机：

- **运行中**：左侧 `AdwSpinner`，显示 Step 计数、全局进度与进度条；`total` 未知或
  `downloaded >= total` 且仍有任务时用 pulse 当不确定进度条。
- **取消中**：点了叉、已发出取消但任务还没收尾。显示「取消中…（已完成 a/b 个 Step）」，叉变灰不可再点。
- **已完成**：左侧对勾，只显示「已完成」。
- **已取消**：左侧停止图标，显示「已取消」以及任务图返回的取消原因。
- **失败**：左侧错误图标，直接显示任务图的错误文本（里面已经带了出错的任务名）。

右上角的叉：运行中 = 取消；取消中 = 禁用；已结束 = 清除这条记录（`CardOut::Close`，
由任务对话框按状态分派）。取消请求由 `Shell` 识别任务返回的 `Cancelled` 错误，最后落到
「已取消」而不是「失败」。

## 进度还原

`GraphStarted` 一次性带出全部静态 Step；之后 `TaskAdded / TaskStarted / TaskFinished /
TaskFailed / TaskSkipped` 分别维护每个 Step 的等待 / 执行 / 完成 / 失败 / 未执行计数，
同时按任务身份记录名字与状态，供展开 Step 时显示。
失败仍计入「完成」并额外记录失败数；`TaskSkipped`（取消 / FailFast 时还没启动的任务）
**不计入完成**，而是记为「未执行」。

Step 的状态与文案：

- 全部完成：绿色 `N/N 完成`；
- 还有任务在跑：`N/M 运行中` 或 `N/M 等待中`；
- 有完成也有未执行：整行变灰，显示 `N/M 完成`；
- 完全没有执行（或整个 Step 没有任务）：灰色 `未执行`；
- 有失败：红色 `N/M 失败`（只显示失败计数，不再追加未执行数）。

全局进度只有一条 `Progress { downloaded, total }`。`TaskState::progress_text` 只拼出
「已下载 / 总字节」，不显示百分比；Step 计数由 `StepRow` 列表展示。没有确定进度时用
`pulse` 当不确定进度条，并且**不会**在每个事件里重设 fraction，避免打断 GTK 的动画。
pulse 定时器由卡片的 `post_view` 按需启停，状态变化后不会留下空转的定时器。

**日志用 `GtkTextView` 增量追加**：下载这类会产生大量文件日志的任务，进度事件很频繁，
如果每条日志都 `join` 全文并重设控件，会让进度条卡顿。日志到达时只把新增行 `insert`
进 `TextBuffer`；展开时等布局完成后滚到最后一行。

`task_view.rs` 是纯逻辑，单测覆盖：真实任务图事件驱动的 Step 计数、取消后未执行任务的
展示、全局进度、日志累积、结束状态与取消中到已取消的转换。

## 退出确认

主窗口的关闭策略放在 `TaskCenter`，因为未结束任务数就是它从卡片列表算出来的，不需要在
根组件再缓存一份。`TaskMsg::CloseMain` 到达时直接查 `cards`：确认框同样 `present` 在主窗口
里，靠对话框宿主叠在任务对话框上层。

## 警告确认

任务通过 `TaskEvent::Warning` 发来需要拍板的警告，且会阻塞等待。`TaskCenter` 收到后直接用
`confirm.rs` 弹 `AdwAlertDialog`：

- 同样 `present` 在主窗口里，显示在当前打开的对话框上层；关着时不会为了弹确认框把任务
  列表带出来；
- **标题是任务名**；
- 选「继续 / 中止」后调 `WarningDecision::accept()` / `abort()`。

## 消息

| 类型 | 文件 | 说明 |
| --- | --- | --- |
| `TaskMsg` | `message.rs` | `Open` / `Start` / `Event` / `Finished { id, ok, cancelled, error }` / `CloseCard` / `Closed` / `CloseMain` / `CancelAllAndQuit` |
| `TaskCenterOut` | `mod.rs` | `RunningCount(n)`：当前未结束任务数，给标题栏角标；`Quit`：根组件可以销毁主窗口了 |
| `StepRow` / `StepRowMsg` | `step_row.rs` | 可展开的 Step 行：状态图标、名称、计数、右侧箭头，展开后是任务明细；`Toggle` 切换展开，`View` 更新 Step 快照 |
| `TaskRow` | `task_row.rs` | 展开后的单个任务行：任务名、状态文字 |
| `CardMsg` | `task_card.rs` | 日志区展开状态变化 |
| `CardOut` | `task_card.rs` | `Close(DynamicIndex)`：点卡片右上角的叉 |

## CSS

`tasks::CSS` 给任务日志区 `.dmcl-task-log` 一个明确的边框。
