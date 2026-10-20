# gtk4 前端架构

`crates/dmcl/src/gui/gtk4` —— `dmcl` 的 GTK4 + libadwaita 前端，框架用 [relm4]。
面向开发者；整体架构见仓库根 [ARCHITECTURE.md](../../../../../ARCHITECTURE.md)。

## 概览

- 与 CLI 一样是 `dmcl` 的一个前端：把 `Config` 翻译成各业务 crate 的 `*Options`、把
  `TaskChain` 跑起来、把 `TaskEvent` 渲染成界面。业务 crate 一行不改。
- 页面（主页等）是 **relm4 组件**，各自拥有状态、消息与 widget；app 层的共享能力收在
  `Shell` 服务里；根组件只做组装。

## 编译与运行

- cargo feature **`gtk4`，默认关**，用 `--features gtk4` 开启；`relm4` 是可选的依赖
  （Linux / Windows / macOS）。
- 需要系统 **GTK ≥ 4.18 / libadwaita ≥ 1.7**（relm4 开 `gnome_48`；用到了 `AdwWrapBox`、
  `AdwSpinner`、`AdwDialog` 等较新的 Adw 控件）。
- 命令：`dmcl gui [frontend]`，前端默认 `gtk4`；显式的 `gui` 命令在前端不可用时直接报错。
- 无子命令是单独路径：`gui::run_default` 有 `gtk4` 就启动 GUI，否则进交互式菜单。
- **GUI 必须占主线程**，所以 `main.rs` 不用 `#[tokio::main]`：直接匹配 `cli.command`
  （`None` 走默认逻辑，`Some(gui)` 走显式 GUI），`Config::load_sync` 同步读一次配置，
  再决定走 GUI 还是建 tokio runtime 执行命令行。
- `RelmApp` 只把 `argv[0]` 传给 `GApplication`，避免 `gui` 被当成要打开的文件。

## 设计取向：遵循 GNOME HIG，优先 Adw 控件

- **能用 Adw 就不用 Gtk**；Gtk 只在 Adw 没有对应控件、或语义明显更合适时使用。
- 场景 → 首选：

  | 场景 | 首选 |
  | --- | --- |
  | 窗口 / 外壳 | `AdwApplicationWindow` + `AdwToolbarView` + `AdwHeaderBar` |
  | 自定义对话框 | `AdwDialog`（`present` 在主窗口里，自适应）；需要用户拖动改尺寸时用模态 `AdwWindow` |
  | 确认 / 提示对话框 | `AdwAlertDialog`（不要再用 1.6 起弃用的 `AdwMessageDialog`） |
  | 通知 | `AdwToastOverlay` / `AdwToast` |
  | 空状态 | `AdwStatusPage` |
  | 加载 / 运行中 | `AdwSpinner` |
  | 卡片流式布局 | `AdwWrapBox` |
  | 设置页 | `AdwPreferencesPage` / `AdwPreferencesGroup` + `AdwActionRow` / `AdwSwitchRow` / `AdwEntryRow` / `AdwComboRow` |
  | 头像 | `AdwAvatar` |
  | 标签页 | `AdwTabView` / `AdwTabBar` |

- **仍用 Gtk 的地方**：`GtkBox` / `GtkLabel` / `GtkButton` / `GtkScrolledWindow` /
  `GtkMenuButton` / `GtkPopover` / `GtkGestureClick` / `GtkProgressBar` / `GtkImage` /
  `GtkExpander` —— Adw 没有对应控件，或语义更合适。例如任务日志用 `GtkExpander` 而不是
  `AdwExpanderRow`：后者是「偏好设置行」形态（行内边距、hover、分隔线），放进卡片里不合适。
- **图标**：用 Adwaita 主题的 `*-symbolic`，不打包位图。
- **样式**：优先 Adw 的标准 class（`.card` / `.dim-label` / `.success` / `.error` /
  `.monospace`）；需要自定义 CSS 时用 class + display 级 provider，class 带 `dmcl-` 命名空间
  （GTK 4.10 起 per-widget provider 已废弃）。
- **版本**：用到新的 Adw 控件时，把 relm4 的 `gnome_4x` feature 抬到对应版本（当前
  `gnome_48` → GTK 4.18 / adw 1.7）。

## 状态驱动：状态放模型，界面用 `view!` 刷新

- 页面优先用 **`#[relm4::component]` + `view!`** 声明视图，状态变化交给 `#[watch]` /
  `#[track]`；**不要手写 `render` 重建子控件**。动态列表用 `FactoryVecDeque` /
  `FactoryComponent` 同步增删，不要长期维护「名字 → widget」的旁路表。
- **状态只有一份**：互斥状态用 enum（如 `InstanceState`），不要用几个布尔量 / 集合分别表示，
  否则容易出现「状态改了但界面没刷新」或两个标志互相矛盾。
- `update` 只改模型；只有一次性的构造、Pango 测量 / 运行时尺寸计算等 `view!` 表达不了的
  东西，才放 `init` / `init_widgets`，且只跑一次。
- 命令式 GTK 仍可用于静态组装或 `view!` 覆盖不到的控件，但不要让它和模型形成两套状态。
- 参考实现：`pages/home` 的实例网格、空状态、底部操作栏都由 `InstanceState` + `#[watch]`
  驱动。

## 目录结构

```text
gtk4/
├── mod.rs                      # RelmApp 启动 + 全局 CSS
├── app/                        # ── 界面框架：服务 + 组装 ──
│   ├── mod.rs                  #   根组件：只做组装（Input = ()）
│   ├── page_host.rs            #   页面宿主：PageHost / Page / PageInit / TabTitle
│   ├── shell.rs                #   Shell 服务：run(任务)→TaskHandle（oneshot，不回调）/ toast
│   ├── toast.rs                #   左下角堆叠 toast（新的在下面，4 秒自动消失，最多 4 条）
│   ├── header.rs               #   标题栏（一行：添加实例 + AdwTabBar + 账号/任务/设置 + 窗口按钮）
│   └── task_popup/             #   任务窗口（组件 TaskCenter，Root 是模态 AdwWindow）
│       ├── mod.rs              #     对话框 + 卡片 + 取消 + 确认 + 起止提示 + 关闭清理
│       ├── message.rs          #     TaskMsg
│       ├── confirm.rs          #     模态确认框（AdwAlertDialog）
│       ├── task_view.rs        #     任务状态 + 进度 reducer（纯逻辑，可单测）
│       └── task_card.rs        #     单个任务卡片（AdwSpinner + 进度 + GtkExpander 日志）
└── pages/                      # ── 页面：真组件 ──
    ├── home/                   #   主页组件（实例 + 运行状态 + 游戏进程）
    │   ├── mod.rs              #     组件：view! 声明网格/空状态/操作栏，状态由 #[watch] 刷新
    │   ├── message.rs          #     HomeMsg
    │   └── instance_card.rs    #     单个实例磁贴（FactoryComponent；运行中右上角绿点）
    └── download/               #   下载相关页面
        ├── common/             #     download 域公共组件（不是全 app 通用小部件）
        │   ├── mod.rs          #       公共 CSS（版本列表选中行）
        │   ├── instance_summary.rs  #  实例概要：图标 / 名称 / 分类 / 安装按钮
        │   └── version_select.rs    #  版本下拉：置顶 + 搜索 / chips + 动态取数
        └── custom/mod.rs       #     「自定义」页：装配两个组件、联动与安装
```

## 组件与服务

- **根组件（`app/mod.rs`）**：纯组装。建窗口（`AdwApplicationWindow` + `AdwToolbarView`）、
  标签页、提示层，启动 `TaskCenter` 与 `HomePage` 两个子组件，给 home 注入 `Rc<Shell>`；
  建 `PageHost` 并把「添加实例 → 自定义」接到 `open_page::<CustomPage>` 上；主页标签拦
  `close-page` 拒绝关闭（其余标签放行），标签 `page-detached` 时让宿主丢掉页面保活。
  **没有任何消息**（`Input = ()`）：任务按钮直接把 `TaskMsg::Open` 发给任务组件，其余都在子组件里。
- **`Shell`（`app/shell.rs`，普通服务）**：**通用**的任务/UI 能力，不认识实例与游戏。
  - `run(name, chain, input) -> TaskHandle<O>`：在 tokio 上跑链，把 `TaskEvent` 经
    `TaskMsg` 转发给任务组件，返回结果句柄。
  - `toast(text)`：往主界面**左下角**的提示层加一条；新的排在**下面**，`4` 秒后自动消失，最多同时 4 条。
- **`PageHost` 与 `Page`（`app/page_host.rs`）**：**页面宿主**——把「实现了 `Page` 的 relm4
  组件」挂成标签页，并保活。页面组件的 `Init` 固定是 `PageInit`（`TabTitle` + 页面自己的
  参数）；`PageHost::open_page::<C>(args)` 启动组件、`AdwTabView::append` 出标签页、绑好标题
  句柄、切到新标签页，并把控制器存进保活表（`page-detached` 时丢弃）。`TabTitle` 是页面**直接**
  改标签标题、请求关闭自己的句柄（不经消息）：`open_page` 先启动组件再 append，所以绑定前
  `set_title` 会缓存、绑定后补写；`close()` 内置 `tab_view` + `tab_page` 引用，调
  `tab_view.close_page` 走根组件的 `close-page`（主页仍会被拒），页面可据此「安装成功后关页」。
  新增页面：在 `pages/` 下写一个 `impl Page` + `#[relm4::component]` 的组件，`type Args` 是它的
  实参（多个参数打包成元组，没有就是 `()`），再在根组件里 `open_page` 即可。
- **`TaskCenter`（`app/task_popup`，组件）**：**它本身就是那个模态任务窗口**（`Root = AdwWindow`，用户可拖动改尺寸）。
  自己的 `update` 里处理：卡片列表（空状态用 `AdwStatusPage`，用 `GtkStack` 切换）、取消、
  警告确认（`confirm.rs`）、起止提示、关闭时清掉已结束的任务。它不认识任务是什么，事件由
  `Shell` 转发过来。
- **`HomePage`（`pages/home`，组件）**：实例列表、「运行中」状态、游戏进程都归它。
  **视图用 `view!` 声明**（`GtkOverlay` + `GtkStack` + `AdwWrapBox` + 底部胶囊操作栏），
  `#[watch]` 负责在模型变化后刷新空状态/操作栏文案/图标/可用性；磁贴列表用
  `FactoryVecDeque<InstanceCard>`，不再手写 `render` / `remove`+`append`。
  **实例列表自动跟随 `versions` 目录**：后台每秒对比顶层版本目录快照（名字 + 目录
  mtime），变化就发 `HomeMsg::Reload`——安装成功关页、外部新增/删除版本都能立刻看到。
  这里用轮询而不是事件监听库：版本目录很小、跨平台，且不必处理「目录还不存在 / 被整体
  替换」等监听器边界，也不用新增依赖。
  磁贴是**等宽**的透明 `GtkButton`（类似 GNOME Files：大图标 + 最多三行名称；行高按内容
  收缩，不预留三行）；**运行中**时右上角显示小绿点（`media-record-symbolic` + `.success`）。
  磁贴的单击选中 / 双击启动通过 `CardOut` 上报给主页。
  **单击选中**，底部浮出**胶囊操作栏**（`.card` + `.dmcl-action-bar`，含启动/停止、管理、
  取消选择）；**点空白处**或胶囊栏的 × 取消选中；**双击启动**（双击手势放在捕获阶段，
  否则会被按钮自己的点击吞掉）。**运行中 / 启动中 / 停止中的实例拒绝再次启动**：
  每个实例用 `InstanceState`（`Idle` / `Starting` / `Running` / `Stopping`）记录状态，底部栏的
  文案、图标、按钮动作都由它派生。**启动中可以直接取消**：用 `CancelHandle` 取消启动链，同时
  发停止信号，覆盖「链刚好跑完、进程已经起来」的竞态；按钮变成不可点的「停止中…」，等收尾
  完成再回到 `Idle`。启动链交给 `Shell` 跑，自己等结果并监督进程（见下）。
- **`CustomPage`（`pages/download/custom`，页面组件）**：「下载 / 自定义」= 新建实例页。
  整页包在 `GtkScrolledWindow` 里（展开多个版本列表时页面可以滚），顶部 `InstanceSummary`
  （图标 + 名称 + 分类 + 安装按钮），下面一个 `.boxed-list` 里排 Minecraft / Forge /
  Fabric / NeoForge 四个 `VersionSelect`。负责 MC → 加载器的联动、加载器互斥、自动命名 /
  图标、组装 `InstallSpec` 并跑安装任务（成功后自动关页）。
- **`InstanceSummary`（`pages/download/common`，组件）**：实例概要。名称 / 图标各带
  「当前值 + 用户是否改过」；外部 `SetAutoName` / `SetAutoIcon` 只在没改过时生效。分类下拉
  v1 禁用（`set_sensitive(false)` + TODO），安装按钮可用性由页面推。
  **名称输入框不能写成 `#[watch] set_text` + `connect_changed` 回写模型**：视图每次刷新都会
  重设文本，GTK 可能先发一条中间态 `changed("")`（先清空再写入），被当成用户清空后自动命名
  又写回，形成无限来回（表现为选中版本后整个窗口 100% CPU、无响应）。现在由组件持有
  `EntryRow`，只在 `SetAutoName` 里显式写文本；`NameEdited` 只接受「事件文本 == 输入框当前
  文本」的事件，中间态直接丢掉，用户真正清空时仍按用户修改处理。
- **`VersionSelect`（`pages/download/common`，组件）**：通用版本选择器。行 = `AdwExpanderRow`，
  标题 / 副标题显示状态，展开区里是搜索框、可选 chips、置顶区 + 完整列表（两个
  `FactoryVecDeque<VersionRow>`），loading / failed 由 `GtkStack` 切换。取数器、chips 全部
  由页面在 Init 注入，组件不认识 Minecraft。选中版本后**不自动收起**，由用户再点标题栏收起。
- **`header.rs`**：`AdwHeaderBar`，title widget 是**整行内容**自己排的 `GtkBox`：`+`、
  `AdwTabBar`、账号/任务/设置、窗口按钮。之所以不把标签栏直接当 title widget、其余按钮分头
  `pack_start`/`pack_end`，是因为头栏把 title widget 摆在 `GtkCenterBox` 正中（中间宽度 =
  总宽 − 2×max(左, 右)），左窄右宽时标签栏会被挤到右边、左边空出一大截；都并进 title widget、
  并把 `show-start/end-title-buttons` 关掉（窗口按钮改用 `GtkWindowControls` 加在行尾）之后，
  左右两侧宽度都是 0，标签栏正好铺满加号到窗口按钮之间。`+` 的 popover 里列出下载类型
  （目前只有「自定义」，由 root 接到 `open_page`）；账号、设置是占位 popup；任务按钮发
  `TaskMsg::Open` 打开任务对话框。

## 消息归属

消息按组件拆分，不集中在一个文件：

| 类型 | 文件 | 说明 |
| --- | --- | --- |
| `HomeMsg` | `pages/home/message.rs` | 主页内部：`Reload` / `Loaded` / `Select(DynamicIndex)` / `Deselect` / `Launch(DynamicIndex)` / `Toggle` / `GameStarted` / `GameExited { name, abnormal }` / `LaunchFailed` / `Manage` |
| `CustomMsg` | `pages/download/custom/mod.rs` | 新实例页：`McChanged` / `LoaderChanged { slot, item }` / `NameChanged` / `Install` / `Installed` |
| `SummaryIn` / `SummaryOut` | `pages/download/common/instance_summary.rs` | 概要：`SetAutoName` / `SetAutoIcon` / `SetInstallEnabled` / 图标菜单与名称编辑；输出 `Install { name }` / `NameChanged` |
| `SelectIn` / `SelectOut` | `pages/download/common/version_select.rs` | 版本下拉：`SetContext` / `Reload` / `Clear` / `Deselect` / `SearchChanged` / `ToggleChip` / `RowClicked` / `Loaded`；输出 `Changed(Option<VersionItem>)` |
| `TaskMsg` | `app/task_popup/message.rs` | 任务对话框：`Open` / `Start` / `Event` / `Finished { id, ok, error }` / `Close` / `ClearFinished` |
| `AppMsg` | 无 | 根没有消息（`Input = ()`） |

页面/任务的动作互相独立：页面直接调注入的 `Shell`；任务对话框自己处理确认与提示，不再往
根组件上报。

## 任务：Shell 与结果句柄

`Shell::run` 返回一个 **可 `await` 的句柄**，而不是回调，避免回调地狱：

```text
let handle = shell.run(format!("启动 {name}"), chain, ());
// handle.finished().await -> Result<GameMonitor, String>
```

内部：在 tokio 上 `run_with`，事件经 `TaskMsg::Event` 发给任务组件，结束时把结果经
`oneshot` 交给发起者（`TaskHandle`）。任务产出（如 `GameMonitor`）**不经 Shell**，Shell
保持通用。

## 启动 / 停止 / 运行状态

- 启动：home 用实例名拼出 `dmcl_launch::launch_chain`，交给 `Shell::run`；一个扁平的
  `relm4::spawn` 等 `handle.finished()`，成功就发 `HomeMsg::GameStarted`，失败发 `LaunchFailed`。
- **运行状态不轮询**：把 `GameMonitor` 交给一个 supervisor 任务：
  ```text
  let abnormal = tokio::select! {
      status = monitor.wait() => !matches!(status, Ok(s) if s.success()),
      _ = &mut stop_rx => {
          let _ = monitor.kill().await;
          let _ = monitor.wait().await;
          false   // 用户主动停止不算异常
      }
  };
  sender.input(HomeMsg::GameExited { name, abnormal });
  ```
  （`select!` 的两个分支都不在处理器里碰 monitor，避免双重可变借用；要 kill 就先退出 select。）
- **异常退出**：非 0 退出码（或拿不到退出状态）算异常，home 弹「xxx 异常退出」；用户主动
  停止不算异常。更详细的错误报告窗口是远期规划。
- 启动/停止：操作栏只发 `HomeMsg::Toggle`，home 按选中实例的状态决定启动、取消还是停止。
  停止时给 supervisor 发信号；启动中先用 `CancelHandle` 取消启动链，同时把停止信号也发出去，
  覆盖「链刚好跑完、进程已经起来」的竞态。
- 取消任务：任务组件持有 `CancelHandle`；卡片右上角的叉在运行中会取消（`TaskMsg::Close`）。

## 新建实例页（download）

- **装配**：`CustomPage` 的 `Init = PageInit<(Rc<Shell>, Config)>`。页面由 `InstanceSummary`
  与四个 `VersionSelect` 组成；打开时只给 MC 下拉发一次 `SetContext(Some(""))`，让清单
  取数跑起来（MC 取数器忽略上下文）。根组件「添加实例 → 自定义」把 `Rc<Shell>` 与 `Config`
  的克隆传进来，并把 `download::common::CSS` 注册到全局样式。
- **自动命名 / 图标**：MC 或任一加载器选择变化后，页面用
  `dmcl_install::default_version_name(mc, [(LoaderKind, Some(id))…])`（CLI 与 GUI 共用）
  算默认名，推 `SetAutoName`；有选中的加载器就推 `SetAutoIcon(Loader(kind))`，否则
  `Initials`。`InstanceSummary` 用 `name_user_modified` / `icon_user_modified` 守住用户输入：
  改过就**永不**再应用自动值；图标菜单的「自动」会清掉修改标记、恢复跟随。
  输入框的 `#[watch] set_text` 每次刷新都会重设文本，所以 `NameEdited` 只有「文本 != 模型值」
  才算用户修改（避免程序同步被误判成用户输入）。
- **版本下拉的取数接口**：

  ```rust
  type FetchVersions =
      Arc<dyn Fn(&str) -> BoxFuture<'_, Result<Vec<VersionItem>>> + Send + Sync>;
  ```

  页面构造闭包时捕获 `DownloadSource`；组件不知道数据从哪来。动态取数带**代际计数**
  （`gen`）：换上下文就 +1，回来时代数不匹配的响应直接丢弃。上下文变化、或新列表里不再
  包含旧选择时，清空选择并发 `Changed(None)`。
- **状态与交互**：`Idle`（还没给上下文，行不可展开）/ `Loading` / `Ready` / `Empty`
  （列表为空，行不可展开、副标题「无可用版本」）/ `Failed`（展开区里显示错误 + 重试）。
  `Ready` 时副标题显示已选 `label`；加载器未选显示「不安装」，MC 未选显示「请选择版本…」。
  加载器行已选时出现 ×（`Deselect`，发 `Changed(None)`）；`Clear` 是给页面编排互斥用的
  静默清空（不发输出）。
- **置顶与筛选**：`VersionItem.badges` 非空即进置顶区，`Badge { key, label }` 是开放词汇
  （不给枚举；`key` 留给以后做样式钩子，v1 只显示 `label`）。列表头与置顶同一条去重；没有
  置顶项时「全部版本」标题隐藏（置顶区本身不加标题）。v1 行文案直接把徽标并进主文案
  （`1.20.1（最新、推荐）`），没有药丸样式。
  搜索对 `id` / `label` 做大小写不敏感子串匹配；chips（`KindChip`）由页面给，全开 =
  只显示列出的类别，全关 = 不按类别筛；chips 用 `caption` 字号 + `.dmcl-version-chip`
  收紧 `.pill` 的默认 padding。过滤是纯函数 `filter_items`（保持数据源的新→旧
  顺序），有单测。选中标记只更新真正变化的两行（`iter_mut` 会把整张表标脏，900 行的 MC
  列表会因此重绘整表）。
- **展开承载**：行 = `AdwExpanderRow`；展开区随内容变长，列表区 220–480 高（超过 480
  在内部滚动）。页面外层有 `GtkScrolledWindow`，展开多个列表时可以滚。不用 `AdwComboRow`：
  它的弹出层是单层平铺列表，放不下分节标题与 chips。版本行外层 `GtkListBox` 会再套一层
  可悬停的隐式 row，所以列表加 `.dmcl-version-list`、在 CSS 里清掉外层 row 的 hover 背景，
  否则会和外层 Button 的 hover 叠成「外浅内深」两圈强调色。展开内容本身也被包成一行，
  该行设为不可激活（`.dmcl-version-panel-row`），否则鼠标进入整块展开区会变深。
- **展开时机**：所有行默认收起，只有用户点标题才展开；选中版本后也不自动收起。
  libadwaita 的 `set_enable_expansion(false → true)` 会**顺手把行展开**，所以 `sync_view`
  在启用后显式 `set_expanded(false)`，并且不能把 `set_enable_expansion` 写成 `#[watch]`
  （每次视图刷新都会重新触发、把用户刚收起的行再弹开）。
- **加载器互斥按组配置**：`LOADER_DEFS` 是
  `LoaderDef { kind, mutex_group }` 常量表，页面持 `Vec<LoaderSlot>`（定义 + 控制器 +
  当前选择），同组里选中一个就对其它项发 `Clear`。现阶段 Forge / Fabric / NeoForge 同属
  `"loader"`；将来允许叠加（双加载器整合包）或加新加载器只改表，`mutex_group: None` 即
  不参与互斥。
- **适配层（页面里的纯函数）**：
  - MC：`latest.release` → 「最新」，`latest.snapshot` → 「最新快照」；`old_alpha` /
    `old_beta`（以及部分镜像的 `alpha` / `beta`）合并成 `old` 类别，供「远古」chip 过滤。
  - Forge：第一条 → 「最新」，`branch == "recommended"` → 「推荐」；类别 chips 暂缺
    （TODO），目前只有搜索。
  - Fabric：第一条 `stable` → 「最新」+「稳定」（CLI 的 latest 语义：优先稳定版；没有
    稳定版才退回第一条）；按 `stable` 分「稳定 / 不稳定」两个 chip。
  - NeoForge：第一条 → 「最新」，没有 recommended / stable，也没有 chips。
  - 空列表是常态（Forge 对不少 MC 版本没有构建、NeoForge 只覆盖 1.20.1+），走 `Empty`
    而不是错误。
- **安装**：`SummaryOut::Install { name }` → 校验 MC 已选、名称非空 → 按装配态组
  `InstallSpec`（加载器按 `LOADER_DEFS` 顺序）→
  `Shell::run(dmcl_install::install_task(...))`；安装期间按钮禁用。成功 toast
  「… 安装完成」并 `tab.close()` 自动关页；失败 toast 错误、页面留着。

## 任务对话框与任务卡片

- 任务列表是一个独立的**模态 `AdwWindow`**（`transient_for` 主窗口，用户可以拖动边缘改尺寸）：
  标题栏按钮发 `TaskMsg::Open` → `present()`。之所以不用 `AdwDialog`，是因为它不支持
  用户拖动改尺寸。关闭（`close-request`）时发 `TaskMsg::ClearFinished` 并**隐藏**窗口
  （保留尺寸与内容，下次打开沿用），把**已结束**的任务卡片清掉，仍在运行的保留。
- **任务起止有提示**：`TaskMsg::Start` 时弹「任务「…」开始」，`TaskMsg::Finished` 时弹
  「任务「…」完成 / 失败」。
- 没有任务时显示 `AdwStatusPage` 空状态（`GtkStack` 切换）。
- 每个任务 = 一条 `TaskChain` 的一次运行。卡片从上到下：状态图标 + 名称 + 右上角的叉 /
  状态行 / 分项进度 / 可展开日志。
- **运行中**：左侧是 `AdwSpinner`，显示总进度「步骤 x/y」、当前步骤名与进度条（没有百分比时
  用 pulse 当不确定进度条）。
- **已完成**：左侧对勾，不显示任何进度，只显示「已完成」。
- **失败**：左侧错误图标，不显示进度，直接显示任务链的错误文本（里面已经带了出错的步骤名）。
- 日志用等宽字体（`caption` 字号），放在带边框的 `.dmcl-task-log` 区域里，用 `GtkExpander`
  展开/收起（`AdwExpanderRow` 是偏好设置行，不适合卡片）。
- 右上角的叉：运行中 = 取消；已结束 = 清除这条记录（`TaskMsg::Close`，由任务组件按状态分派）。
- 总进度用**栈**还原层级，只统计**顶层**步骤完成（嵌套子链会把步骤事件原样转发到父链，
  不能直接数 `StepFinished`）：`StepStarted` push、`StepFinished` pop，栈由非空变空才 +1。
- 分项进度取栈顶步骤最近一条 `Progress`；没有百分比时用 `pulse` 当不确定进度条。
- 这段是纯逻辑，在 `task_view.rs` 里单测。

## 警告确认

任务通过 `TaskEvent::Warning` 发来需要拍板的警告，且会阻塞等待。`TaskCenter` 收到后直接用
`task_popup/confirm.rs` 弹 `AdwAlertDialog`（parent 取**当前最上层的窗口**：任务窗口开着就挂
任务窗口，关着就挂主窗口，不会为了弹确认框把任务列表带出来；**标题是任务名**），选
「继续 / 中止」后调 `WarningDecision::accept()` / `abort()`。

## 标题栏与标签页

- `AdwHeaderBar` 的 title widget 是**一整行** `GtkBox`：`+`、`AdwTabBar`、账号/任务/设置、
  `GtkWindowControls`。头栏把 title widget 摆在 `GtkCenterBox` 正中（中间宽度 = 总宽 −
  2×max(左, 右)），若把标签栏单独当 title widget、其余按钮分头 `pack_start`/`pack_end`，
  左窄右宽时标签栏会被推到右边、左边空出一大截；所以整行都并进 title widget，并把
  `show-start-title-buttons` / `show-end-title-buttons` 关掉、窗口按钮自己加在行尾——左右两侧
  宽度都变成 0，标签栏正好铺满「加号 → 窗口按钮」之间。
- 标签栏 `autohide(false)`：只有一个页面（主页）时也一直显示，避免「从 2 个标签关到 1 个」
  时整条标签栏跟着 `GtkRevealer` 滑走、和标签自身的收缩动画叠在一起。
- 标签页是 `AdwTabView`，由 `PageHost` 管。**主页不可关闭**，其余页面都可关闭：主页标签
  **不能用 `pinned`**（pinned 只显示图标、不显示标题），改为在 `AdwTabView::close-page` 里对
  主页调 `close_page_finish(page, false)` 并返回 `Stop`；标签移除时 `page-detached` 通知宿主
  丢掉页面保活。
- **CSS 补丁**：各模块自带自己的全局 CSS，由 `run` 统一挂上。GTK4 的 per-widget provider
  自 4.10 起已废弃，官方做法是「class + display 级 provider」——选择器带 `dmcl-` 命名空间，
  不会影响别的控件：
  - `header::CSS`（`headerbar.dmcl-header`，标题栏内嵌标签栏）：收紧竖向内边距，让标题栏
    保持 47px 的正常高度（否则 tabbar 47px + 标题栏内层 13px 内边距会顶到 60px）；隐藏**主页**
    标签的关闭按钮——libadwaita 没有 per-page 的 closable，也没有公开句柄拿到某个标签的关
    闭按钮，只能按 CSS 节点链（`tabbar` → `tab`）把 `.dmcl-home-tab` 打在主页那个标签上
    （`header::mark_home_tab`，等标签栏建好标签后调用），选择器写成 `tab.dmcl-home-tab`，
    其余标签保持可见的 `×`；
  - `download::common::CSS`：版本列表里选中行 `.dmcl-version-selected` 的高亮，以及清掉
    `GtkListBox` 隐式行 hover 背景、避免内外两圈强调色；
  - `task_popup::CSS`：给任务日志区 `.dmcl-task-log` 一个明确的边框。

## 图标

- **UI 图标**用 Adwaita 主题的 `*-symbolic`（按名字引用、不打包；随主题换色、随缩放）。
  只挑稳定常见的名字（个别名字在旧版 adwaita-icon-theme 里缺失）。
- **实例图标**：默认用 `AdwAvatar` 首字母彩色头像（零素材、零版权风险）。新建实例页支持
  自动按加载器显示首字母（Forge → F）与用户自选本地图片（`GdkTexture`，载入失败回落首字母）。
  **不内置 Mojang 贴图**。
- **运行状态**用彩色文字/圆点（CSS class）而不是图标。

## 待办

- 实例设置页（`Manage` 打开新标签）：`PageHost::open_page` 已就绪，写个 `Page` 组件接上即可。
- Forge 版本下拉的类别 chips（稳定 / 推荐）——目前只有搜索与置顶徽标。
- 实例分类下拉启用与持久化（v1 禁用于 `InstanceSummary`）。
- 任务按钮的「运行中数量」角标。
- 账号 / 设置 popup 的实际功能。
- 游戏异常退出的错误报告窗口（远期）。
- 删除实例等破坏性操作。

[relm4]: https://relm4.org
