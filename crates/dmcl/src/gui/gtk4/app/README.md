# app 层架构（外壳与服务）

`crates/dmcl/src/gui/gtk4/app` —— GTK4 前端的界面框架：根组件、通用服务、页面宿主、
标题栏与任务窗口。整体架构见 [gtk4/README.md](../README.md)。

## 根组件

`app/mod.rs` 只做组装，视图用 `view!` 声明：

- `AdwApplicationWindow` + `AdwToolbarView`、标签页、提示层都由 `view!` 搭出来；
- 启动 `Header`、`TaskCenter`；按配置里的全部 `minecraft_roots` 创建常驻 `HomePage`，
  启动时只给默认 root 建标签页；
- 根组件输入 `AppMsg`：`HeaderOut` 负责打开主页、设置默认 root 和任务；主页的
  `HomeOut::CreateCustom` 打开新建实例页；`HomeTabClosed` 只把对应标签状态设为关闭；
  `TaskCenterOut` 负责任务角标与退出；
- 所有主页标签都可以关闭。标签 `page-detached` 时清理 `PageHost` 和标签映射，但
  `HomePage` 继续常驻；重新打开时把同一个根 widget 挂到新标签，运行状态不会丢。

标题栏也是独立组件（`header/mod.rs`，根目录菜单行在 `header/root_row.rs`，外观切换在
`header/theme.rs`，「关于」在 `header/about.rs`），整行由 `view!` 声明，再由根组件的
`view!` 用 `add_top_bar` 挂上去。需要页面宿主 / 任务窗口配合的动作通过 `HeaderOut`
上报给根组件。

## `Shell`

`app/shell.rs` 是**通用**的任务 / UI 服务，不认识实例与游戏：

- `run(name, chain, input) -> TaskHandle<O>`：在 tokio 上跑链，把 `TaskEvent` 经
  `TaskMsg` 转发给任务组件，返回结果句柄。
- `open_tasks()`：主动给任务组件发 `TaskMsg::Open`，弹出任务列表窗口。
- `global_toast(text)`：应用级提示，始终往主界面**左下角**的提示层加一条。
- `page_toast(page, text)`：页面级提示，只在目标页面当前可见（`is_mapped`）时显示；
  页面在后台标签、标签已关闭或已销毁时直接丢弃，不排队。

两种提示共用同一个提示层：新的排在**下面**，`4` 秒后自动消失，最多同时 4 条。每条只占
一行，超长文字用省略号；颜色取主题的 `@card_bg_color` / `@card_fg_color`，跟随深色 /
浅色模式。任务开始 / 结束的提示属于 app 层，始终走 `global_toast`。

`Shell::run` 返回一个**可 `await` 的句柄**，而不是回调，避免回调地狱：

```text
let handle = shell.run(format!("启动 {name}"), chain, ());
// handle.finished().await -> Result<GameMonitor, String>
```

内部：在 tokio 上 `run_with`，事件经 `TaskMsg::Event` 发给任务组件，结束时把结果经
`oneshot` 交给发起者（`TaskHandle`）。任务产出（如 `GameMonitor`）**不经 Shell**，Shell
保持通用。结束消息还会带一个 `cancelled` 标记：Shell 从返回的错误里认出 `Cancelled`，
让任务卡片显示「已取消」而不是「失败」。

## 页面宿主

`app/page_host.rs` 负责把「实现了 `Page` 的 relm4 组件」挂成标签页，并保活：

- 页面组件的 `Init` 固定是 `PageInit`（`TabTitle` + 页面自己的参数）；
- `PageHost::open_page::<C>(args)` 启动组件、`AdwTabView::append` 出标签页、绑好标题句柄、
  切到新标签页，并把控制器存进保活表（`page-detached` 时丢弃）；
- `TabTitle` 是页面**直接**改标签标题、请求关闭自己的句柄（不经消息）：`open_page` 先启动
  组件再 append，所以绑定前 `set_title` 会缓存、绑定后补写；`close()` 内置 `tab_view` +
  `tab_page` 引用，调 `tab_view.close_page`；所有标签（包括主页）都允许关闭。

新增页面：在 `pages/` 下写一个 `impl Page` + `#[relm4::component]` 的组件，`type Args` 是
它的实参（多个参数打包成元组，没有就是 `()`），再在根组件里 `open_page` 即可。

## 标题栏与标签页

- `AdwHeaderBar` 的 title widget 是**一整行** `GtkBox`：`+`、`AdwTabBar`、账号 / 任务 /
  设置、`GtkWindowControls`。头栏把 title widget 摆在 `GtkCenterBox` 正中（中间宽度 =
  总宽 − 2×max(左, 右)），若把标签栏单独当 title widget、其余按钮分头 `pack_start` /
  `pack_end`，左窄右宽时标签栏会被推到右边、左边空出一大截；所以整行都并进 title widget，
  并把 `show-start-title-buttons` / `show-end-title-buttons` 关掉、窗口按钮自己加在行尾——
  左右两侧宽度都变成 0，标签栏正好铺满「目录菜单 → 窗口按钮」之间。
- 标签栏 `autohide(false)`：页面少时也一直显示，避免关闭标签时整条标签栏跟着
  `GtkRevealer` 滑走、和标签自身的收缩动画叠在一起。
- 标签页是 `AdwTabView`。所有页面都可关闭：关闭主页标签只移除 `AdwTabPage`，`HomePage`
  继续存活，游戏进程和监视任务都不受影响；重新打开时把同一个根 widget 加到标签末尾。
  `PageHost` 只管非主页页面，标签移除时清理保活。
- 标题栏最左侧是「Minecraft 目录」菜单，列出全部 `minecraft_roots`：打开 / 聚焦主页、
  标记已打开。新建实例由主页右下角的悬浮按钮打开；`CustomPage` 使用主页
  捕获的 root，App 不维护「当前选中的主页是哪个 root」。账号、设置是占位 popup；任务按钮
  通过 `HeaderOut::OpenTasks` 打开任务对话框。
- 最右侧的三点主菜单提供外观切换和「关于」。外观切换是 `header/theme.rs` 里的独立子组件
  `ThemeSwitcher`，用 GNOME 应用里常见的「跟随系统 / 浅色 / 深色」圆形色板：一组互斥的
  `GtkCheckButton`，把 `radio` 指示器画成圆形，再叠一个选中对勾；分别对应
  `AdwStyleManager` 的 `Default` / `ForceLight` / `ForceDark`。「关于」由
  `header/about.rs` 打开 `AdwAboutDialog`。
- 任务按钮右上角有运行中数量角标；任务窗口在任务开始 / 结束 / 清除时把未结束任务数上报给
  根组件，再转给标题栏，角标为 0 时隐藏。
- 标题栏布局用 `view!` 声明。标签栏仍占满中间宽度；在它和右侧按钮之间留一段 28px 的固定
  `GtkWindowHandle`，拖这段空白可以移动窗口，避免拖拽被标签栏内部的滚动区吃掉。

## 标题栏 CSS

`header::CSS` 负责头栏行高、任务按钮角标和根目录行内边距：

- 收紧竖向内边距，让标题栏保持 47px 的正常高度（否则 tabbar 47px + 标题栏内层 13px 内边距
  会顶到 60px）。

`header/theme.rs` 里的外观色板 CSS 由根组件以 `header::THEME_CSS` 一并挂上。

其余模块的 CSS 在各自目录的文档里：下载域见
[download](../pages/download/README.md#css)，任务窗口见
[tasks](tasks/README.md#css)。

## 关闭主窗口

根组件收到主窗口的 `close-request` 后只转发 `TaskMsg::CloseMain`，由任务组件自己判断：

- 没有未结束的任务：回 `TaskCenterOut::Quit`，根组件销毁主窗口；
- 还有运行中 / 取消中的任务：弹 `AdwAlertDialog` 确认；确认后取消所有未结束任务，
  再回 `TaskCenterOut::Quit`。

未结束任务数由任务组件自己维护，根组件不再缓存一份。关闭任务窗口本身只隐藏窗口，
不取消任务、不弹确认；用户可以在任务继续跑的时候关掉任务列表。

正在运行的游戏不属于任务窗口：关闭主窗口不会结束它们，也不会为它们弹额外确认。

## 任务窗口

`TaskCenter` 是独立的模态 `AdwWindow`，细节见
[tasks/README.md](tasks/README.md)。app 层只负责在根组件里启动它，
并把 `Shell` 产生的事件转给它。
