# 「自定义」新建实例页

`crates/dmcl/src/gui/gtk4/pages/download/custom` —— 「下载 / 自定义」页，也就是新建实例页。
下载域与公共组件见 [../README.md](../README.md)，整体架构见
[gtk4/README.md](../../../README.md)。

## 模块

- `mod.rs`：页面组件、状态、`init` / `update` 与安装流程；
- `message.rs`：`CustomMsg`；
- `loaders.rs`：`LoaderDef` / `LOADER_DEFS`、运行时 `LoaderSlot` 与 `loader_init`；
- `versions.rs`：各数据源的取数器与 `VersionItem` 适配。

## 装配

`CustomPage` 的 `Init = PageInit<(Rc<Shell>, PathBuf)>`。页面由
`InstanceSummary` 与四个 `VersionSelect` 组成：

- `InstanceSummary` 推名称 / 图标 / 安装按钮状态；
- MC 一行 + Forge / Fabric / NeoForge 三个加载器行；
- 打开时只给 MC 下拉发一次 `SetContext(Some(""))`，让清单取数跑起来（MC 取数器忽略上下文）；
- 主页右下角「新建实例」→「自定义」把 `Rc<Shell>` 和该主页的根目录路径传进来；安装目标、
  实例是否已存在都只用这个 root；`download::common::CSS` 注册到全局样式。
- 下载源与安装参数从全局共享配置现读（`ConfigRef`），页面不持有 `Config` 副本。

整页包在 `GtkScrolledWindow` 里；展开多个版本列表时页面可以滚。

## 自动命名与图标

MC 或任一加载器选择变化后，页面用
`dmcl_install::default_version_name(mc, [(LoaderKind, Some(label))…])`（CLI 与 GUI 共用）算
默认名，推 `SetAutoName`。

- 加载器这里传**展示版本**（Forge 是 `version` 字段，不含 mcversion 前缀），安装仍用条目 id
  （Forge `full_name`）。
- 有选中的加载器就推 `SetAutoIcon(Loader(kind))`，否则 `Initials`。
- `InstanceSummary` 用 `name_user_modified` / `icon_user_modified` 守住用户输入：改过就
  **永不**再应用自动值；名称清空则视为「从未编辑」、恢复跟随自动命名；图标菜单的「自动」
  会清掉修改标记、恢复跟随。
- 标签标题随名称显示为 `安装 <名称>`（还没名字时 `安装 新实例`）。
- MC 上下文**真正变化**时才清空加载器选择并重发 `SetContext`；重复选同一个 MC 版本保留
  加载器选择。

## 名称冲突检查

页面用 `dmcl_mcmeta::layout::version_dir(root, name).exists()` 判断 `versions/<名称>` 是否
已被占用；名称变化（自动命名或用户编辑）时重查。

- 同名时提示「实例已经存在」并禁用安装；
- 未选 MC 时安装按钮不可用，但不显示名称提示（输入框也不标红）；
- 空名称不算冲突，但安装按钮也不可用；
- 最终的同名冲突由 `install_task` 把关（它创建版本目录时会失败）。

## 加载器互斥

`LOADER_DEFS` 是 `LoaderDef { kind, mutex_group }` 常量表，页面持 `Vec<LoaderSlot>`
（定义 + 控制器 + 当前选择）。

同组里选中一个就对其它项发 `SetConflict(Some(选中项名字))`，组件禁用该行并显示
「与 xx 冲突」；取消选择后发 `None` 恢复。现阶段 Forge / Fabric / NeoForge 同属 `"loader"`；
将来允许叠加（双加载器整合包）或加新加载器只改表，`mutex_group: None` 即不参与互斥。

## 版本适配层

`versions.rs` 里的纯函数把各数据源转成 `VersionItem`：

- **MC**：`latest.release` → 「最新正式版」，`latest.snapshot` → 「最新预览版」；两者都可进
  置顶区。`old_alpha` / `old_beta`（以及部分镜像的 `alpha` / `beta`）合并成 `old` 类别，
  供「远古版」chip 过滤。
- **Forge**：第一条 → 「最新版本」，`branch == "recommended"` → 「推荐版本」；类别 chips
  暂缺（TODO）。列表展示与自动命名用 `version` 字段（如 `47.2.0`），安装 id 仍是
  `full_name`（如 `1.20.1-47.2.0`）。
- **Fabric**：不区分版本类型（和 Forge / NeoForge 一样没有 chips）；第一条 → 「最新版本」，
  第一条 `stable` → 「推荐版本」。两者可以不是同一条。
- **NeoForge**：第一条 → 「最新版本」，没有 recommended / stable，也没有 chips。
- 空列表是常态（Forge 对不少 MC 版本没有构建、NeoForge 只覆盖 1.20.1+），走 `Empty`
  而不是错误。

## 安装

`SummaryOut::Install { name }` → 校验 MC 已选、名称非空 → 按装配态组 `InstallSpec`
（加载器按 `LOADER_DEFS` 顺序）→ `Shell::run(dmcl_install::install_task(...))` →
`Shell::open_tasks()` 主动弹出任务列表 → 下一轮 idle 里 `tab.close()` 立即关页。

任务本身由 `Shell` / 任务窗口独立跑，结果（完成 / 失败 / 取消）都在任务窗口里看，页面不再
等结果。先弹任务窗口再关页，避免任务窗口被主窗口压到下面。

## 消息

| 类型 | 说明 |
| --- | --- |
| `CustomMsg` | `McChanged` / `LoaderChanged { slot, item }` / `NameChanged` / `Install` |

## 测试

`mod.rs` 的单测覆盖标签标题（空名称回退到「安装 新实例」）；`versions.rs` 覆盖：

- MC 类型合并与最新徽标；
- Forge 展示 `version` / 安装 `full_name`、第一条与 recommended 徽标；
- Fabric 第一条 + 第一条 stable（可以重合，也可以没有 stable）；
- NeoForge 只有第一条带徽标。
