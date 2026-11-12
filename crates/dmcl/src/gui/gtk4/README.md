# gtk4 前端架构

`crates/dmcl/src/gui/gtk4` —— `dmcl` 的 GTK4 + libadwaita 前端，框架用 [relm4]。
面向开发者；整体架构见仓库根 [ARCHITECTURE.md](../../../../../ARCHITECTURE.md)，应用层与配置见
[crates/dmcl/README.md](../../../README.md)。

本文只讲**整体架构与实现约定**。单个目录的职责放在各目录的 `README.md` 里。

## 概览

- 与 CLI 一样是 `dmcl` 的一个前端：把 `Config` 翻译成各业务 crate 的 `*Options`、把
  `TaskGraph` 跑起来、把 `TaskEvent` 渲染成界面。业务 crate 一行不改。
- 页面（主页、新建实例等）是 **relm4 组件**，各自拥有状态、消息与 widget；app 层的共享能力
  收在 `Shell` 服务里；根组件只做组装。
- 整个前端共用一份配置（见下文「共享配置」），组件不各存一份 `Config`，也不把配置
  逐层往下传。
- 一个主页绑定一个根目录；标题栏的根目录菜单可以打开多个主页。所有 root 的主页
  从启动起常驻，关闭标签只影响显示，页面状态和运行中的游戏监视不会丢失。
- 配置文件不存在时，启动后弹出首次设置向导：账户、Minecraft 目录、版本隔离；
  「跳过」需确认并写入当前配置，完成或跳过之后都不再出现。

## 编译与运行

- cargo feature **`gtk4`，默认关**，用 `--features gtk4` 开启；`relm4` 是可选的依赖。
- 需要系统 **GTK ≥ 4.22 / libadwaita ≥ 1.9**。
- 命令：`dmcl gui [frontend]`，前端默认 `gtk4`；显式的 `gui` 命令在前端不可用时直接报错。
- GUI 必须占主线程，所以 `main.rs` 先同步读一次配置，再决定启动 GUI 还是创建 tokio runtime
  执行命令行。

## 设计取向：遵循 GNOME HIG，优先 Adw 控件

- **能用 Adw 就不用 Gtk**；Gtk 只在 Adw 没有对应控件、或语义明显更合适时使用。
- 窗口 / 外壳用 `AdwApplicationWindow` + `AdwToolbarView` + `AdwHeaderBar`；对话框用
  `AdwDialog` / `AdwAlertDialog`；空状态用 `AdwStatusPage`；加载用 `AdwSpinner`；卡片流式
  布局用 `AdwWrapBox`；设置页用 `AdwPreferencesPage` 及其行控件；标签页用
  `AdwTabView` / `AdwTabBar`。
- **图标**：UI 图标用 Adwaita 主题的 `*-symbolic`，只挑稳定常见的名字；实例图标用内置
  方块 / 加载器素材或实例目录的 `logo.png`，按原比例完整显示，不裁成圆形。
- **样式**：优先 Adw 的标准 class；需要自定义 CSS 时用 class + display 级 provider，
  class 带 `dmcl-` 命名空间。各模块自带 CSS，由 `run` 统一挂上。
- 用到新的 Adw 控件时，把 relm4 的 `gnome_4x` feature 抬到对应版本。

## 状态驱动：状态放模型，界面用 `view!` 刷新

- 页面优先用 **`#[relm4::component]` + `view!`** 声明视图，状态变化交给 `#[watch]` /
  `#[track]`；**不要手写 `render` 重建子控件**。动态列表用 `FactoryVecDeque` /
  `FactoryComponent` 同步增删，不要长期维护「名字 → widget」的旁路表。
- **不要在 `update` / `init` 里命令式循环创建、清空或重建控件树**；列表增删改走
  `FactoryVecDeque`，一次性静态结构写进 `view!`。
- **状态只有一份**：互斥状态用 enum，不要用几个布尔量 / 集合分别表示，否则容易出现
  「状态改了但界面没刷新」或两个标志互相矛盾。
- **状态按组件归属拆分**：不要集中放进一个大结构体。
- `update` 只改模型；只有一次性的构造、Pango 测量 / 运行时尺寸计算等 `view!` 表达不了的
  东西，才放 `init` / `init_widgets`，且只跑一次。
- 命令式 GTK 仍可用于静态组装或 `view!` 覆盖不到的控件，但不要让它和模型形成两套状态。

## 共享配置

整个前端只有一份 `Config`，由 `app/config_store.rs` 管理：

- `ConfigStore` 是进程级单例，在 `app::run` 里用启动时读到的配置初始化。
- 组件需要读配置时拿只读的 `ConfigRef`，需要写时走 `ConfigStore::update`；不接收 `Config`
  参数，也不各存副本。`ConfigRef` 指向共享配置，`update` 原地修改，之前拿到的句柄也能读到
  新值。
- 跨页面自动刷新是「`update` → 根组件消息 → 页面消息 → `#[watch]` 重画」：`#[watch]`
  本身不会监听外部状态，页面必须先收到一条消息。
- `update` 自己负责异步落盘，并强制调用方传一个失败处理函数（在 GLib 主线程上执行）；
  保存内部串行化。`update` 必须在 GTK 主线程调用。
- 后台任务用 `update_async`：同步改内存并通知页面，返回的 future 只负责落盘。
- 读 guard 要短命：`std::sync::RwLock` 不可重入，持着 guard 调 `update` 会自锁。
- 启动时加载配置发现的问题先备份原文件为同目录的 `dmcl.old.json`，再写入修复后的配置。
- `minecraft_roots` 只增不减：新增 root 时补建常驻主页；删除 root 不销毁页面，只从标题栏
  菜单移除；同名 root 改路径不热更新。

## 目录结构

```text
gtk4/
├── README.md                    # 本文：整体架构 + 实现约定
├── mod.rs                       # RelmApp 启动 + 全局 CSS
├── app/                         # 界面框架：服务 + 组装 + 首次向导 + 任务对话框
│   ├── README.md
│   ├── config_store.rs          # 全局共享配置
│   ├── config_warning.rs        # 配置修复结果弹窗
│   ├── page_host.rs             # 一次性 / 常驻页面宿主
│   ├── shell.rs                 # 任务与提示服务
│   ├── setup/                   # 首次启动设置向导
│   ├── toast.rs
│   ├── header/                  # 标题栏、根目录菜单、外观切换、关于
│   └── tasks/                   # 任务对话框
├── pages/                       # 页面
│   ├── home/                    # 主页 / 实例磁贴 / 启动停止
│   ├── settings/                # 首选项
│   └── download/                # 新建实例页与公共组件
│       └── custom/              # 「自定义」新建实例页
├── widgets/                     # 跨页面复用的部件与逻辑
│   └── instance_icon.rs         # 实例图标来源、自动规则与 PNG 编码
└── assets/icons/                # 实例图标内置素材
```

## 组件索引

| 范围 | 主要组件 / 服务 | 文档 |
| --- | --- | --- |
| app 外壳 | 根组件、`Shell`、`PageHost` / `PageSlot`、标题栏、toast | [app/README.md](app/README.md) |
| 首次启动向导 | `SetupWizard`、完成条件、跳过与保存 | [app/setup/README.md](app/setup/README.md) |
| 任务对话框 | `TaskCenter`、任务卡片、警告确认 | [app/tasks/README.md](app/tasks/README.md) |
| 主页 | `HomePage`、`InstanceCard`、启动 / 停止 | [pages/home/README.md](pages/home/README.md) |
| 设置页 | `PreferencesPage` 及目录 / 账户 / Java / 游戏选项分类 | [pages/settings/README.md](pages/settings/README.md) |
| 下载域与公共组件 | 数据流、`InstanceSummary`、`VersionSelect` | [pages/download/README.md](pages/download/README.md) |
| 新建实例页 | `CustomPage` 的装配、联动与安装 | [pages/download/custom/README.md](pages/download/custom/README.md) |

## 消息与边界

消息按组件拆分，不集中在一个文件里。页面直接调注入的 `Shell`；任务对话框自己处理确认与
提示。跨组件的边界只有四条：

- **页面 → `Shell`**：传 `TaskGraph`，拿回可 `await` 的结果句柄；任务事件由 `Shell`
  转发给任务对话框。
- **页面 → 子组件**：通过组件输入消息推自动值、冲突与安装按钮状态。
- **子组件 → 页面**：通过组件输出消息上报选择、名称变化和安装请求。
- **页面 ↔ 共享配置**：直接向全局 `ConfigStore` 订阅 / 修改；写完后由订阅消息通知需要
  重画的组件。

## GTK 测试

分三层：编译 / 静态检查、纯逻辑单测、真实界面验证。前两层进 CI；界面验证先由用户在自己的
桌面环境完成。

```bash
cargo clippy -p dmcl --features gtk4 --all-targets -- -D warnings
cargo test -p dmcl --features gtk4 --lib
```

GTK 这层的单测不创建 widget，也不依赖 `DISPLAY`；relm4 组件里能抽出来的纯逻辑放在同文件的
`#[cfg(test)]` 模块里，重点覆盖行为、边界和容易回归的逻辑。

控件行为（展开、hover、布局、焦点、动画）无法用单测覆盖，改完后**先请用户在自己的桌面环境
实际测试**，以用户看到的真实 GTK 效果为准。用户无法测试、且确实需要自动检查时，才用
Broadway + Chromium 做临时渲染验证；它只是浏览器里的软件渲染，**不等于真实 GTK 后端**。
需要联网的测试标 `#[ignore]`，单独运行。

[relm4]: https://relm4.org
