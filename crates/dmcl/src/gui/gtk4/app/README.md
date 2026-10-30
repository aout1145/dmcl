# app 层架构（外壳与服务）

`crates/dmcl/src/gui/gtk4/app` —— GTK4 前端的界面框架：根组件、通用服务、页面宿主、
标题栏与任务窗口。整体架构见 [gtk4/README.md](../README.md)。

## 根组件

`app/mod.rs` 只做组装，视图用 `view!` 声明：

- `AdwApplicationWindow` + `AdwToolbarView`、标签页、提示层都由 `view!` 搭出来；
- 启动 `Header`、`TaskCenter`；按配置里的全部 `minecraft_roots` 创建常驻 `HomePage`，
  启动时只给默认 root 建标签页；
- 根组件输入 `AppMsg`：`HeaderOut` 负责打开主页和任务；主页的
  `HomeOut::CreateCustom { categories }` 打开新建实例页；`ConfigChanged` 给配置里新增的 root 补建主页
  并刷新标题栏菜单；`TabClosed` 清理对应页面的标签句柄并同步根目录勾选；主菜单和快捷键
  共用 `app.*` action，分别发 `OpenPreferences` / `About`；
  `TaskCenterOut` 负责任务角标与退出；
- 所有标签都可以关闭。标签 `page-detached` 时发送 `TabClosed`，清理 `PageHost` 和各
  `PageSlot` 的标签句柄，但常驻页面继续存活；重新打开时把同一个根 widget 挂到新标签，
  运行状态不会丢。

标题栏也是独立组件（`header/mod.rs`，根目录菜单行在 `header/root_row.rs`，外观切换在
`header/theme.rs`，「关于」在 `header/about.rs`），整行由 `view!` 声明，再由根组件的
`view!` 用 `add_top_bar` 挂上去。需要页面宿主 / 任务窗口配合的动作通过 `HeaderOut`
上报给根组件。

## 共享配置

`app/config_store.rs` 管理整个前端唯一的一份 `Config`：

- `ConfigStore` 是进程级单例（`static OnceLock`），GUI 入口把 `Config::load_sync` 读到的
  配置经由 `app::run` 放进去。单例里只有配置锁、保存锁和一条 `tokio::sync::watch`
  版本号，都是 `Send + Sync`。
- 读配置：`config_store::store().config()`，返回 `ConfigRef`（只读句柄）；写配置：
  `config_store::store().update(|config| …)`。组件不接收 `Config` 参数，也不再各存副本。
- **只有根组件监听变更**：`init` 里 `spawn_local` 一个 `watch` 循环，收到变化后补建新
  root 的主页、刷新标题栏菜单，并给主页发 `HomeMsg::ConfigChanged`。页面不各自订阅，
  `#[watch]` 的重画由这条消息触发。
- `update` 原地修改共享配置，释放写锁后才推进版本号；等到消息时值已经是最新的。
- `update` 自己异步落盘，并强制传一个失败处理函数（在 GLib 主线程上执行）；保存锁串行化，
  连续修改时最后一次写入包含全部改动。
- 启动时的配置修复走 `save_with_backup`：先把原文件备份为同目录的 `dmcl.old.json`，再写入
  修复后的配置；完成后由配置修复弹窗说明结果。
- 读 guard 要短命：持着 guard 调 `update` 会自锁。

根目录列表只增不减：`AppMsg::ConfigChanged` 时给新 root 补建常驻 `HomePage`，删除 root
不销毁页面、只从标题栏菜单移除；同名 root 改路径不热更新（页面已按旧路径挂了 watcher）。
新 root 也不会自动打开标签页，需要从菜单点开。

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

## 页面生命周期

`app/page_host.rs` 按页面是否需要长期保活分成两种：

- **一次性页面**（`CustomPage`）：实现 `Page`，`Init` 固定是 `PageInit`（`TabTitle` +
  页面自己的参数）。`PageHost::open_page::<C>(args)` 启动组件、`AdwTabView::append` 出
  标签页、绑好标题句柄、切到新标签页，并把控制器存进保活表；`page-detached` 时丢弃。
- **常驻页面**（主页、首选项）：启动时创建组件，由 `PageSlot<C>` 直接
  持有 `Controller<C>` 和 `Option<AdwTabPage>`。`PageSlot::open` 在已打开时只聚焦，关闭后
  重新 append 同一个根 widget；`PageSlot::close` 只清空标签句柄，组件和页面状态继续保留。
  设置页固定一个字段，主页按 root 名字存放在 `Vec<HomeSlot>` 里（`HomeSlot` 是
  名字 + `PageSlot<HomePage>`），不需要字符串 key 或类型擦除注册表。

`TabTitle` 是页面**直接**改标签标题、请求关闭自己的句柄（不经消息）：`open_page` 先启动
组件再 append，所以绑定前 `set_title` 会缓存、绑定后补写；`close()` 内置 `tab_view` +
`tab_page` 引用，调 `tab_view.close_page`；所有标签（包括主页）都允许关闭。

新增一次性页面：在 `pages/` 下写一个 `impl Page` + `#[relm4::component]` 的组件，
`type Args` 是它的实参（多个参数打包成元组，没有就是 `()`），再在根组件里 `open_page`。
新增常驻页面：在根组件加一个 `PageSlot<C>` 字段，启动时 `launch` 一次，并在 `TabClosed`
里调用它的 `close`。

## 标题栏与标签页

- `AdwHeaderBar` 的 title widget 是**一整行** `GtkBox`：目录菜单按钮、`AdwTabBar`、任务 /
  主菜单、`GtkWindowControls`。头栏把 title widget 摆在 `GtkCenterBox` 正中（中间宽度 =
  总宽 − 2×max(左, 右)），若把标签栏单独当 title widget、其余按钮分头 `pack_start` /
  `pack_end`，左窄右宽时标签栏会被推到右边、左边空出一大截；所以整行都并进 title widget，
  并把 `show-start-title-buttons` / `show-end-title-buttons` 关掉、窗口按钮自己加在行尾——
  左右两侧宽度都变成 0，标签栏正好铺满「目录菜单 → 窗口按钮」之间。
- 标签栏 `autohide(false)`：页面少时也一直显示，避免关闭标签时整条标签栏跟着
  `GtkRevealer` 滑走、和标签自身的收缩动画叠在一起。
- 标签页是 `AdwTabView`。所有页面都可关闭：关闭主页或设置页标签只移除 `AdwTabPage`，
  `PageSlot` 里的组件继续存活，重新打开时把同一个根 widget 加到标签末尾。`PageHost`
  只负责 `CustomPage` 这类关闭即销毁的一次性页面，标签移除时清理保活。
- 标题栏最左侧是「Minecraft 目录」菜单，列出全部 `minecraft_roots`：打开 / 聚焦主页、
  标记已打开。新建实例由主页右下角的悬浮按钮打开；`CustomPage` 使用主页
  捕获的 root，App 不维护「当前选中的主页是哪个 root」。任务按钮通过
  `HeaderOut::Tasks` 打开任务对话框。
- 最右侧的三点主菜单是 `GtkMenuButton` + `GtkPopoverMenu`，菜单结构由 `GMenu` 生成，
  和 GNOME 应用的主菜单同一条路径：普通项是 GTK 生成的 `GtkModelButton`，统一处理
  行高、分隔线、助记符和右侧快捷键；首选项直接调用 `app.preferences`，
  关于调用 `app.about`。外观切换是 `header/theme.rs` 里的独立子组件
  `ThemeSwitcher`，作为 `custom` child 插进 `GtkPopoverMenu` 的第一段。它照 GNOME Text
  Editor 的 `EditorThemeSelector` 实现：横向 `GtkBox` 间距 12、外边距 9，三个互斥
  `GtkCheckButton` 都 `hexpand` 并居中，色板本身是 44px 圆形，`radio` 指示器透明后挪到
  右下角，选中时显示对勾；分别对应 `AdwStyleManager` 的 `Default` / `ForceLight` /
  `ForceDark`。「关于」由 `header/about.rs` 打开 `AdwAboutDialog`。
- 任务按钮右上角有运行中数量角标；任务窗口在任务开始 / 结束 / 清除时把未结束任务数上报给
  根组件，再转给标题栏，角标为 0 时隐藏。
- 标题栏布局用 `view!` 声明。标签栏仍占满中间宽度；在它和右侧按钮之间留一段 28px 的固定
  `GtkWindowHandle`，拖这段空白可以移动窗口，避免拖拽被标签栏内部的滚动区吃掉。

## 快捷键与助记符

- 应用级 action：`app.preferences`（`Ctrl+,`）、`app.about`，在根组件 `init` 里注册。
  `app.preferences` 发 `AppMsg::OpenPreferences`，`app.about` 发
  `AppMsg::About` 再转给标题栏。
- 菜单项由 `GMenu` 生成，标签里的 `_P` / `_A` 就是 `GtkModelButton` 的助记符：
  菜单打开时 `Alt+P` / `Alt+A` 直接激活对应 action，不需要手写 mnemonic widget。
- 菜单右侧的快捷键来自 `GMenu` 项的 `accel` 属性，由 `GtkPopoverMenu` 按平台渲染成
  `Ctrl+,` 这类纯文本。

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
