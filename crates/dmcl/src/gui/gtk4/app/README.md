# app 层架构（外壳与服务）

`crates/dmcl/src/gui/gtk4/app` —— GTK4 前端的界面框架：根组件、通用服务、页面宿主、
标题栏与任务对话框。整体架构与实现约定见 [gtk4/README.md](../README.md)。

## 模块职责

```text
mod.rs            根组件：只做组装，连接标题栏、任务对话框与各页面
config_store.rs   整个前端唯一的共享配置
config_warning.rs 配置修复结果弹窗
page_host.rs      一次性 / 常驻页面的宿主
shell.rs          通用任务与提示服务
toast.rs          应用级与页面级提示
header/           标题栏、根目录菜单、外观切换、关于
setup/            首次启动设置向导
tasks/            任务对话框与任务卡片
```

## 根组件

`app/mod.rs` 只做组装：视图用 `view!` 声明，启动标题栏与任务对话框，按配置里的全部
`minecraft_roots` 创建常驻 `HomePage`，启动时只给默认 root 建标签页；配置文件不存在时
再启动首次设置向导。

根组件负责应用级消息与动作：打开主页 / 首选项 / 关于、打开新建实例页、配置变更后补建新增
root 的主页、处理标签关闭、退出前把关闭请求交给任务对话框。页面与任务的动作彼此独立，
根组件不缓存页面状态。

## 共享配置

`app/config_store.rs` 管理整个前端唯一的一份 `Config`：

- `ConfigStore` 是进程级单例，GUI 入口把启动时读到的配置放进去。
- 读配置拿到只读的 `ConfigRef`，写配置走 `ConfigStore::update`；组件不接收 `Config`
  参数，也不各存副本。
- **只有根组件监听变更**：收到变化后补建新 root 的主页、刷新标题栏菜单，并通知页面重画；
  页面不各自订阅。
- `update` 原地修改共享配置，释放写锁后才推进版本号；异步落盘由 `update` 自己负责，调用方
  必须提供保存失败的处理函数。后台任务用 `update_async`，返回的 future 只负责落盘。
- 启动时的配置修复走 `save_with_backup`：先备份原文件为同目录的 `dmcl.old.json`，再写入
  修复后的配置。
- 读 guard 要短命，持着 guard 调 `update` 会自锁。
- 根目录列表只增不减：新增 root 补建常驻主页，删除 root 不销毁页面、只从标题栏菜单移除；
  同名 root 改路径不热更新。

## `Shell`

`app/shell.rs` 是**通用**的任务 / UI 服务，不认识实例与游戏：

- `run` 在 tokio 上跑任务图，把 `TaskEvent` 转发给任务组件，返回一个可 `await` 的结果句柄；
  任务产出不经 `Shell`，保持通用。
- `open_tasks` 主动弹出任务对话框。
- `global_toast` 发应用级提示；`page_toast` 只在目标页面可见时显示，页面不可见时直接丢弃。
- 任务开始 / 结束的提示属于 app 层，始终走 `global_toast`。

## 页面生命周期

`app/page_host.rs` 按页面是否需要长期保活分成两种：

- **一次性页面**（如 `CustomPage`）：实现 `Page`，由 `PageHost::open_page` 启动组件、创建
  标签页；标签关闭即销毁。
- **常驻页面**（主页、首选项）：由 `PageSlot<C>` 直接持有组件控制器和标签句柄。重复打开只
  聚焦已有标签；关闭标签只清空句柄，组件和页面状态继续保留，重新打开时把同一个根 widget
  挂到新标签。主页按 root 名字存放在 `Vec<HomeSlot>` 里。

新增一次性页面：在 `pages/` 下写一个 `impl Page` + `#[relm4::component]` 的组件，再在根组件
里 `open_page`。新增常驻页面：在根组件加一个 `PageSlot<C>` 字段，启动时 `launch` 一次，并在
标签关闭时调用它的 `close`。

## 其他部分

- **首次启动向导**：配置文件不存在时打开一个不能直接关闭的模态对话框，复用首选项的账户页
  与目录页；「跳过」需确认并写配置，「完成」要求默认账户和目录都有效。见
  [setup/README.md](setup/README.md)。
- **标题栏**：整行作为 `AdwHeaderBar` 的 title widget，包含根目录菜单、标签栏、任务 / 主菜单
  与窗口按钮；根目录菜单负责打开 / 聚焦主页。主菜单与快捷键共用 `app.*` action。
- **关闭主窗口**：根组件只转发请求，由任务对话框判断是否有未结束任务；有任务时确认后再
  取消并退出。正在运行的游戏不属于任务对话框，关闭窗口不会结束它们。

## 任务对话框

`TaskCenter` 是独立的模态对话框，细节见 [tasks/README.md](tasks/README.md)。app 层只负责在
根组件里启动它，并把 `Shell` 产生的事件转给它。
