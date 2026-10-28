# gtk4 前端架构

`crates/dmcl/src/gui/gtk4` —— `dmcl` 的 GTK4 + libadwaita 前端，框架用 [relm4]。
面向开发者；整体架构见仓库根 [ARCHITECTURE.md](../../../../../ARCHITECTURE.md)。

本文只讲**整体架构**：分层、通用约定、组件索引和测试方式。单个目录的细节放在各目录的
`README.md` 里，避免把页面行为堆进这一篇。

## 概览

- 与 CLI 一样是 `dmcl` 的一个前端：把 `Config` 翻译成各业务 crate 的 `*Options`、把
  `TaskChain` 跑起来、把 `TaskEvent` 渲染成界面。业务 crate 一行不改。
- 页面（主页、新建实例等）是 **relm4 组件**，各自拥有状态、消息与 widget；app 层的共享能力
  收在 `Shell` 服务里；根组件只做组装。
- 整个前端共用一份配置（见下文「共享配置」），组件不各存一份 `Config`，也不会把配置
  逐层往下传。
- 一个主页绑定一个根目录；标题栏的根目录菜单可以打开多个主页。所有 root 的主页
  从启动起常驻，关闭标签只影响显示，页面状态和运行中的游戏监视不会丢失。

## 编译与运行

- cargo feature **`gtk4`，默认关**，用 `--features gtk4` 开启；`relm4` 是可选的依赖
  （Linux / Windows / macOS）。
- 需要系统 **GTK ≥ 4.22 / libadwaita ≥ 1.9**（relm4 开 `gnome_50`；用到了 `AdwWrapBox`、
  `AdwSpinner`、`AdwDialog` 等较新的 Adw 控件）。
- 命令：`dmcl gui [frontend]`，前端默认 `gtk4`；显式的 `gui` 命令在前端不可用时直接报错。
- 无子命令是单独路径：`gui::run_default` 有 `gtk4` 就启动 GUI，否则打印帮助并退出。
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

- **仍用 Gtk 的地方**：`GtkBox` / `GtkLabel` / `GtkTextView` / `GtkButton` /
  `GtkScrolledWindow` / `GtkMenuButton` / `GtkPopover` / `GtkPopoverMenu` / `GMenu` /
  `GtkGestureClick` / `GtkProgressBar` / `GtkImage` / `GtkExpander` —— Adw 没有对应控件，
  或语义更合适。例如任务日志用 `GtkTextView` 增量追加、用 `GtkExpander` 而不是
  `AdwExpanderRow`：后者是「偏好设置行」形态（行内边距、hover、分隔线），放进卡片里不合适。
- **图标**：UI 图标用 Adwaita 主题的 `*-symbolic`，只挑稳定常见的名字，不打包位图；实例图标
  用 `AdwAvatar` 首字母彩色头像，不内置 Mojang 贴图。
- **运行状态**用彩色文字 / 圆点（CSS class）而不是图标。
- **样式**：优先 Adw 的标准 class（`.card` / `.dim-label` / `.success` / `.error` /
  `.monospace`）；需要自定义 CSS 时用 class + display 级 provider，class 带 `dmcl-` 命名空间
  （GTK 4.10 起 per-widget provider 已废弃）。各模块自带 CSS，由 `run` 统一挂上。
- **版本**：用到新的 Adw 控件时，把 relm4 的 `gnome_4x` feature 抬到对应版本（当前
  `gnome_50` → GTK 4.22 / adw 1.9）。

## 状态驱动：状态放模型，界面用 `view!` 刷新

- 页面优先用 **`#[relm4::component]` + `view!`** 声明视图，状态变化交给 `#[watch]` /
  `#[track]`；**不要手写 `render` 重建子控件**。动态列表用 `FactoryVecDeque` /
  `FactoryComponent` 同步增删，不要长期维护「名字 → widget」的旁路表。
- **不要在 `update` / `init` 里命令式循环创建、清空或重建控件树**；列表增删改走
  `FactoryVecDeque`，一次性静态结构写进 `view!`。
- **状态只有一份**：互斥状态用 enum（如 `InstanceState`、任务的 `TaskStatus`），不要用几个
  布尔量 / 集合分别表示，否则容易出现「状态改了但界面没刷新」或两个标志互相矛盾。
- 动态的任务卡片也是 `FactoryComponent`（`app/tasks/task_card.rs`），任务列表由
  `FactoryVecDeque<TaskCard>` 同步增删。
- `update` 只改模型；只有一次性的构造、Pango 测量 / 运行时尺寸计算等 `view!` 表达不了的
  东西，才放 `init` / `init_widgets`，且只跑一次。
- 命令式 GTK 仍可用于静态组装或 `view!` 覆盖不到的控件，但不要让它和模型形成两套状态。
- 参考实现：`pages/home` 的实例网格、空状态、底部操作栏都由 `InstanceState` + `#[watch]`
  驱动。

## 共享配置

整个前端只有一份 `Config`，由 `app/config_store.rs` 管理：

- `ConfigStore` 是进程级单例（`static OnceLock`），在 `app::run` 里用启动时读到的配置
  初始化。单例里只有配置锁、保存锁和一条 `tokio::sync::watch` 版本号；**唯一的订阅者是
  根组件**，它在 GLib 主线程上把版本变化转成 `AppMsg::ConfigChanged`。
- 组件需要读配置时用 `config_store::store().config()` 拿一个 `ConfigRef`（只读句柄），
  需要写时用 `config_store::store().update(...)`。不接收 `Config` 参数，也不各存副本。
- `ConfigRef` 指向的就是共享的那份配置：`update` 在原地改，之前拿到的句柄也能读到新值。
- 跨页面自动刷新是「`update` → 根组件消息 → 页面消息 → `#[watch]` 重画」：`#[watch]`
  本身不会监听外部状态，页面必须先收到一条消息。
- `update` 自己负责异步落盘，并强制调用方传一个失败处理函数（在 GLib 主线程上执行）；
  保存内部串行化，连续修改时最后一次写入包含全部改动。`update` 必须在 GTK 主线程调用。
- 启动时加载配置发现的问题由 `save_with_backup` 修复：先备份原文件为同目录的
  `dmcl.old.json`，再写入修复后的配置。
- 读 guard 要短命：`std::sync::RwLock` 不可重入，持着 guard 调 `update` 会自锁。
- `minecraft_roots` 只增不减：配置新增 root 时根组件补建常驻主页；删除 root 不销毁页面，
  只从标题栏菜单移除；同名 root 改路径不热更新。

## 目录结构

```text
gtk4/
├── README.md                    # 本文：整体架构 + GTK 测试
├── mod.rs                       # RelmApp 启动 + 全局 CSS
├── app/                         # 界面框架：服务 + 组装
│   ├── README.md                # app 层：根组件 / Shell / 共享配置 / PageHost / 标题栏 / toast
│   ├── mod.rs
│   ├── config_store.rs          # 全局共享配置：config() / update()
│   ├── config_warning.rs        # 配置修复结果弹窗
│   ├── page_host.rs
│   ├── shell.rs
│   ├── toast.rs
│   ├── header/                  # 标题栏 + 根目录行 + 外观切换 / 关于
│   │   ├── mod.rs
│   │   ├── root_row.rs
│   │   ├── theme.rs
│   │   └── about.rs
│   └── tasks/                   # 任务窗口
│       ├── README.md            # 任务窗口 / 卡片 / 警告确认 / 进度还原
│       ├── mod.rs
│       ├── message.rs
│       ├── confirm.rs
│       ├── task_view.rs
│       └── task_card.rs
└── pages/                       # 页面：真组件
    ├── home/
    │   ├── README.md            # 主页 / 实例磁贴 / 启动停止
    │   ├── mod.rs
    │   ├── message.rs
    │   └── instance_card.rs
    ├── settings/
    │   ├── README.md            # 首选项
    │   ├── mod.rs
    │   ├── common/
    │   │   ├── mod.rs
    │   │   ├── drag_list.rs     # 可拖动排序的列表
    │   │   ├── java_cache.rs    # Java 探测结果缓存
    │   │   └── java_display.rs  # Java 展示文案
    │   ├── game/                # 可复用的游戏设置表单
    │   │   ├── mod.rs           # 目标抽象、字段级改动与选择项映射
    │   │   ├── page.rs          # GameOptionsPage 表单
    │   │   ├── choice.rs        # 展开选择项行
    │   │   └── global.rs        # 全局配置落点
    │   └── launcher/            # 首选项的分类
    │       ├── mod.rs           # 左右分栏、分类列表与右栏切换
    │       ├── directory.rs     # Minecraft 目录管理、对话框与校验
    │       └── java.rs          # Java 管理、搜索、探测对话框与校验
    └── download/
        ├── README.md            # 下载域 / 公共组件 / 数据流
        ├── common/              # download 域公共组件（不是全 app 通用小部件）
        │   ├── mod.rs
        │   ├── instance_summary.rs
        │   └── version_select.rs
        └── custom/
            ├── README.md        # 「自定义」新建实例页
            ├── mod.rs           # 页面组件、状态与安装
            ├── message.rs       # CustomMsg
            ├── loaders.rs       # 加载器定义 / 槽位 / Init
            └── versions.rs      # 版本取数器与清单适配
```

## 组件索引

| 范围 | 主要组件 / 服务 | 文档 |
| --- | --- | --- |
| app 外壳 | 根组件、`Shell`、`PageHost` / `PageSlot`、标题栏、toast | [app/README.md](app/README.md) |
| 任务窗口 | `TaskCenter`、任务卡片、警告确认 | [app/tasks/README.md](app/tasks/README.md) |
| 主页 | `HomePage`、`InstanceCard`、启动 / 停止 | [pages/home/README.md](pages/home/README.md) |
| 设置页 | `PreferencesPage`、`DirectoryPage`、`JavaPage`、`GameOptionsPage`、`RootRow`、`JavaRow`、`drag_list` | [pages/settings/README.md](pages/settings/README.md) |
| 下载域与公共组件 | 数据流、`InstanceSummary`、`VersionSelect` | [pages/download/README.md](pages/download/README.md) |
| 新建实例页 | `CustomPage` 的装配、联动与安装 | [pages/download/custom/README.md](pages/download/custom/README.md) |

## 消息与边界

消息按组件拆分，不集中在一个文件里。根组件接收标题栏的 `HeaderOut`（打开主页、
任务窗口）、应用级快捷键的打开首选项消息和任务窗口的 `TaskCenterOut`（运行中数量角标），
页面与任务的动作彼此独立。页面直接调注入的 `Shell`；任务窗口自己处理确认与提示。各组件
完整的消息表见对应目录的文档。

跨组件的边界只有四条：

- **页面 → `Shell`**：传 `TaskChain`，拿回可 `await` 的结果句柄；任务事件由 `Shell` 转发给
  任务窗口。
- **页面 → 子组件**：通过组件输入消息推自动值、冲突与安装按钮状态。
- **子组件 → 页面**：通过组件输出消息上报选择、名称变化和安装请求。
- **页面 ↔ 共享配置**：直接向全局 `ConfigStore` 订阅 / 修改；写完后由订阅消息通知需要
  重画的组件。

## GTK 测试

分三层：**编译 / 静态检查**、**纯逻辑单测**、**真实界面验证**。前两层进 CI；第三层在本地
用 Broadway 做。

### 1. 编译与静态检查

GTK4 前端默认不编译，所以除了 workspace 检查，还要单独开 feature 跑一遍：

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p dmcl --features gtk4 --all-targets -- -D warnings
cargo doc --no-deps --workspace
```

CI 在 **Linux / Windows / macOS** 三个平台都跑 GTK4 的 clippy、测试和 release 构建；
Linux 需要 GTK ≥ 4.22 / libadwaita ≥ 1.9。

### 2. 纯逻辑单测

```bash
cargo test --workspace
cargo test -p dmcl --features gtk4 --lib
```

GTK 这层的测试**不创建 widget**，因此不需要 `DISPLAY` / Broadway。relm4 组件里能抽出来的
纯逻辑都放在同文件的 `#[cfg(test)]` 模块里：

- 任务进度 reducer（`app/tasks/task_view.rs`）：顶层步骤计数、当前步骤进度、日志累积、
  结束状态、取消中 / 已取消的转换、进度说明拼法；
- 版本目录监听（`pages/home/mod.rs`）：`notify` watch `versions`；目录不存在时 watch 最近祖先，
  事件合并后重新扫描；
- 版本适配（`pages/download/custom/versions.rs`）：MC 类型合并、Forge 展示 / 安装 id 的区别、
  Fabric 最新 / 推荐规则、NeoForge 置顶；标签标题在 `custom/mod.rs`；
- 过滤（`pages/download/common/version_select.rs`）：类别、搜索、保持顺序、未列出类别；
- 游戏设置（`pages/settings/game/mod.rs`）：选择项下标映射、缺失项回退、字段级写回；
- 前端选择（`gui/mod.rs`）：有 / 无 `gtk4` feature 时的回退。

写测试时优先覆盖**行为、边界和容易回归的逻辑**（例如 Fabric 没有 stable、Forge 的
`version` 与 `full_name` 不能混用），不要把常量映射或默认值原样抄成断言。需要联网的测试标
`#[ignore]`，用 `cargo test --workspace -- --ignored` 单独跑。

### 3. 界面验证：Broadway + Chromium

GTK 控件的行为（展开、hover、布局、焦点、动画）单测覆盖不到，改动页面后要实际渲染检查。
没有桌面时用 Broadway 把 GTK 画到浏览器里；也可以请用户在自己的桌面环境里自行验证，
真实 GTK 后端的观感以用户看到的结果为准：

```bash
cargo build -p dmcl --features gtk4

# 1. Broadway 服务；:5 是 Broadway display，8085 是 HTTP 端口
gtk4-broadwayd -p 8085 -a 127.0.0.1 :5 &

# 2. 应用连到 Broadway
GDK_BACKEND=broadway BROADWAY_DISPLAY=:5 ./target/debug/dmcl &

# 3. 用 headless Chromium 打开并暴露 CDP
chromium --headless=new --no-sandbox --disable-gpu --hide-scrollbars \
  --window-size=1200,1000 --remote-debugging-port=9223 \
  http://127.0.0.1:8085/ &
```

Debian / Ubuntu 的 `gtk4-broadwayd` 在 `libgtk-4-bin` 包里。等应用窗口画出来后，用
`curl -s http://127.0.0.1:9223/json` 找到页面的 `webSocketDebuggerUrl`，再用任意 CDP 客户端：

- `Page.captureScreenshot` 截图，检查布局 / 文案 / 对齐；
- `Input.dispatchMouseEvent` 点击、悬停、滚动，验证展开、搜索、置顶隐藏、冲突禁用等交互。

截图放 `/tmp`，不要提交到仓库。验证完记得把 Broadway、应用和 Chromium 都关掉。

注意事项：

- Broadway 是浏览器里的软件渲染，**不等于真实 GTK 后端**；最终观感仍以本机桌面为准。
- 大列表（例如 900 条 MC 版本）在 Broadway 里快速展开 / 收起时，可能触发
  `Gdk:ERROR ... decode_nodes`；这是 Broadway 自身的问题，不代表应用 panic。需要看某个
  特定状态时，可以临时缩小列表或注入测试上下文，**验证完必须删掉临时代码**。
- 不要提交 `DMCL_REPRO*` 之类的调试开关；提交前用 `grep -R DMCL_REPRO crates/` 确认。

### 4. 联网与平台

- 真实下载 / 清单取数用 `cargo test --workspace -- --ignored`，需要能访问 Mojang / bmclapi。
- 平台相关的路径、命令和信号语义按 `#[cfg(unix)]` / `#[cfg(windows)]` 门控；GTK4 的构建与
  单测必须在 Linux / Windows / macOS 都成立。

## 待办

- 实例设置页（`Manage` 打开新标签）的 `Page` 组件。
- Forge 版本下拉的类别 chips（稳定 / 推荐）。
- 实例分类下拉的启用与持久化。
- 任务按钮的「运行中数量」角标。
- 账号功能；首选项里的账号分类。
- 实例设置页复用 `GameOptionsPage`，实现实例 target 与逐项覆盖。
- 游戏异常退出的错误报告窗口（远期）。
- 删除实例等破坏性操作。

[relm4]: https://relm4.org
