# app 层架构（外壳与服务）

`crates/dmcl/src/gui/gtk4/app` —— GTK4 前端的界面框架：根组件、通用服务、页面宿主、
标题栏与任务窗口。整体架构见 [gtk4/README.md](../README.md)。

## 根组件

`app/mod.rs` 只做组装：

- 建 `AdwApplicationWindow` + `AdwToolbarView`、标签页、提示层；
- 启动 `TaskCenter` 与 `HomePage` 两个子组件，给 home 注入 `Rc<Shell>`；
- 建 `PageHost`，把「添加实例 → 自定义」接到 `open_page::<CustomPage>` 上；
- 主页标签拦 `close-page` 拒绝关闭（其余标签放行）；标签 `page-detached` 时让宿主丢掉
  页面保活。

根组件**没有任何消息**（`Input = ()`）：任务按钮直接把 `TaskMsg::Open` 发给任务组件，
其余都在子组件里。

## `Shell`

`app/shell.rs` 是**通用**的任务 / UI 服务，不认识实例与游戏：

- `run(name, chain, input) -> TaskHandle<O>`：在 tokio 上跑链，把 `TaskEvent` 经
  `TaskMsg` 转发给任务组件，返回结果句柄。
- `open_tasks()`：主动给任务组件发 `TaskMsg::Open`，弹出任务列表窗口。
- `toast(text)`：往主界面**左下角**的提示层加一条；新的排在**下面**，`4` 秒后自动消失，
  最多同时 4 条。

`Shell::run` 返回一个**可 `await` 的句柄**，而不是回调，避免回调地狱：

```text
let handle = shell.run(format!("启动 {name}"), chain, ());
// handle.finished().await -> Result<GameMonitor, String>
```

内部：在 tokio 上 `run_with`，事件经 `TaskMsg::Event` 发给任务组件，结束时把结果经
`oneshot` 交给发起者（`TaskHandle`）。任务产出（如 `GameMonitor`）**不经 Shell**，Shell
保持通用。

## 页面宿主

`app/page_host.rs` 负责把「实现了 `Page` 的 relm4 组件」挂成标签页，并保活：

- 页面组件的 `Init` 固定是 `PageInit`（`TabTitle` + 页面自己的参数）；
- `PageHost::open_page::<C>(args)` 启动组件、`AdwTabView::append` 出标签页、绑好标题句柄、
  切到新标签页，并把控制器存进保活表（`page-detached` 时丢弃）；
- `TabTitle` 是页面**直接**改标签标题、请求关闭自己的句柄（不经消息）：`open_page` 先启动
  组件再 append，所以绑定前 `set_title` 会缓存、绑定后补写；`close()` 内置 `tab_view` +
  `tab_page` 引用，调 `tab_view.close_page` 走根组件的 `close-page`（主页仍会被拒），页面
  可据此「安装发起后关页」。

新增页面：在 `pages/` 下写一个 `impl Page` + `#[relm4::component]` 的组件，`type Args` 是
它的实参（多个参数打包成元组，没有就是 `()`），再在根组件里 `open_page` 即可。

## 标题栏与标签页

- `AdwHeaderBar` 的 title widget 是**一整行** `GtkBox`：`+`、`AdwTabBar`、账号 / 任务 /
  设置、`GtkWindowControls`。头栏把 title widget 摆在 `GtkCenterBox` 正中（中间宽度 =
  总宽 − 2×max(左, 右)），若把标签栏单独当 title widget、其余按钮分头 `pack_start` /
  `pack_end`，左窄右宽时标签栏会被推到右边、左边空出一大截；所以整行都并进 title widget，
  并把 `show-start-title-buttons` / `show-end-title-buttons` 关掉、窗口按钮自己加在行尾——
  左右两侧宽度都变成 0，标签栏正好铺满「加号 → 窗口按钮」之间。
- 标签栏 `autohide(false)`：只有一个页面（主页）时也一直显示，避免「从 2 个标签关到 1 个」
  时整条标签栏跟着 `GtkRevealer` 滑走、和标签自身的收缩动画叠在一起。
- 标签页是 `AdwTabView`，由 `PageHost` 管。**主页不可关闭**，其余页面都可关闭：主页标签
  **不能用 `pinned`**（pinned 只显示图标、不显示标题），改为在 `AdwTabView::close-page` 里
  对主页调 `close_page_finish(page, false)` 并返回 `Stop`；标签移除时 `page-detached` 通知
  宿主丢掉页面保活。
- `+` 的 popover 里列出下载类型（目前只有「自定义」，由 root 接到 `open_page`）；账号、
  设置是占位 popup；任务按钮发 `TaskMsg::Open` 打开任务对话框。

## 标题栏 CSS

`header::CSS` 只打在 `headerbar.dmcl-header` 上：

- 收紧竖向内边距，让标题栏保持 47px 的正常高度（否则 tabbar 47px + 标题栏内层 13px 内边距
  会顶到 60px）；
- 隐藏**主页**标签的关闭按钮——libadwaita 没有 per-page 的 closable，也没有公开句柄拿到
  某个标签的关闭按钮，只能按 CSS 节点链（`tabbar` → `tab`）把 `.dmcl-home-tab` 打在主页
  那个标签上（`header::mark_home_tab`，等标签栏建好标签后调用），选择器写成
  `tab.dmcl-home-tab`；其余标签保持可见的 `×`。

其余模块的 CSS 在各自目录的文档里：下载域见
[download](../pages/download/README.md#css)，任务窗口见
[task_popup](task_popup/README.md#css)。

## 任务窗口

`TaskCenter` 是独立的模态 `AdwWindow`，细节见
[task_popup/README.md](task_popup/README.md)。app 层只负责在根组件里启动它，
并把 `Shell` 产生的事件转给它。
