# 下载域（新建实例）

`crates/dmcl/src/gui/gtk4/pages/download` —— 「下载 / 自定义」新建实例页及其公共组件。
整体架构见 [gtk4/README.md](../../README.md)。

当前只有「自定义」一种下载类型：用户选 MC 版本与可选的加载器，填名称后跑安装任务。

## 数据流

```text
CustomPage
├── InstanceSummary       名称 / 分类 / 图标 / 安装按钮
└── VersionSelect × 4     MC / Forge / Fabric / NeoForge
        │
        │ SelectOut::Changed(Option<VersionItem>)
        ▼
  自动命名 / 图标 / 冲突检查 / InstallSpec
        │
        ▼
  Shell::run(install_task) → 打开任务窗口 → 关闭页面
```

页面拥有联动与安装流程，见 [custom/README.md](custom/README.md)。`InstanceSummary` 与
`VersionSelect` 位于 `common/`，是 download 域公共组件（不是全 app 通用小部件），细节见下文。

## 页面布局

整页包在 `GtkScrolledWindow` 里（展开多个版本列表时页面可以滚），顶部 `InstanceSummary`
（图标 + 名称 + 分类 + 安装按钮），下面一个 `.boxed-list` 里排 Minecraft / Forge / Fabric /
NeoForge 四个 `VersionSelect`。

## InstanceSummary

`common/instance_summary.rs` —— 图标 + 名称 + 分类 + 安装按钮。`Init = Vec<String>`（已有
分类建议的拷贝），由页面通过输入消息推状态。

- 名称与图标都有**当前值 + 用户是否改过**两个状态：外部推自动值
  （`SetAutoName` / `SetAutoIcon`）只在用户没改过时生效；用户一改过就**永不**覆盖。
  名称被清空时视为「从未编辑」，自动命名可以重新接管；图标菜单的「自动」会清掉修改标记、
  恢复跟随自动值。
- 自动命名规则（`dmcl_install::default_version_name`）由页面计算后推送，组件自身不知道版本与
  加载器怎么命名。
- 分类是 `AdwExpanderRow`：收起时副标题显示当前分类；展开后先列已有分类，当前项带
  对勾，最下面是「自定义分类」`AdwEntryRow`（可以输入任意新名字，留空表示未分类）。
  选中已有项后自动收起；分类非空时行尾出现 ×，点击回到「未分类」。
- 安装按钮可用性由页面通过 `SetInstallEnabled` 推。页面检查到 `versions/<名称>` 已存在时发
  `SetNameError(Some("实例已经存在"))`：输入框不变色，提示文案用 `.error` 标红，和安装按钮
  排在同一行（提示在按钮左侧）。
- **名称输入框不能写成 `#[watch] set_text` + `connect_changed` 回写模型**：视图每次刷新都会
  重设文本，GTK 可能先发一条中间态 `changed("")`（先清空再写入），被当成用户清空后自动命名
  又写回，形成无限来回（表现为选中版本后整个窗口 100% CPU、无响应）。现在由组件持有
  `EntryRow`，只在 `SetAutoName` 里显式写文本；`NameEdited` 只接受「事件文本 == 输入框当前
  文本」的事件，中间态直接丢掉；用户真正清空时重置修改标记，自动命名重新接管。
- 图标来源 `IconSource`：`Initials`（首字母彩色头像）/ `Loader(LoaderKind)`（按加载器取首字母，
  Forge → F）/ `File(PathBuf)`（用户自选本地图片，载入失败回落首字母）。
- 输出 `Install { name, category }` 与 `NameChanged(String)`；前者用于页面组安装请求，
  后者用于页面同步标签标题。

## VersionSelect

`common/version_select.rs` —— 通用版本选择器。行 = `AdwExpanderRow`，标题 / 副标题显示状态，
展开区里是可选搜索框、可选 chips、类型列、置顶区 + 完整列表（两个
`FactoryVecDeque<VersionRow>`），loading / failed 由 `GtkStack` 切换。取数器、chips、是否显示
搜索框全部由页面在 Init 注入，组件不认识 Minecraft。

### Init 与取数接口

```rust
type FetchVersions =
    Arc<dyn Fn(&str) -> BoxFuture<'_, Result<Vec<VersionItem>>> + Send + Sync>;
```

页面构造闭包时捕获 `DownloadSource`；组件不知道数据从哪来。动态取数带**代际计数**（`gen`）：
换上下文就 +1，回来时代数不匹配的响应直接丢弃。上下文变化、或新列表里不再包含旧选择时，
清空选择并发 `Changed(None)`。

`VersionSelectInit`：

| 字段 | 作用 |
| --- | --- |
| `title` | 行标题，如「Forge 版本」 |
| `fetch` | 按上下文取版本列表的闭包 |
| `chips` | 筛选 chips；空 = 没有类别筛选，也不显示版本类型列；每个 chip 带 `default_selected` |
| `show_search` | 是否显示搜索框（MC = true；加载器 = false） |
| `allow_none` | 是否允许未选择（加载器 = true，空选择即「不安装」；MC = false 必选） |
| `initially_expanded` | 第一次变为可展开时是否自动展开（MC 行 = true；加载器行 = false） |

### 状态与交互

- `Idle`：还没给上下文，行不可展开。
- `Loading`：取数中；行可展开看加载页。
- `Ready`：列表非空；副标题显示已选 `label`。加载器未选显示「不安装」，MC 未选显示
  「请选择版本…」。
- `Empty`：列表为空，行不可展开，副标题「无可用版本」。
- `Failed`：行尾显示刷新按钮，展开区只显示左对齐的错误信息；刷新按钮与加载器的 × 同一位置，
  失败时替代 ×。点击刷新发 `Reload`。
- 加载器行已选时出现 ×（`Deselect`，发 `Changed(None)`）。选中版本后立即收起。
- 同组已有别的加载器选中时，页面发 `SetConflict(Some("Forge"))`：该行禁用、副标题显示
  「与 Forge 冲突」，清空选择后发 `None` 恢复。

### 置顶与筛选

- `VersionItem.badges` 非空即进置顶区（可多条，不再跳过列表第一条）。`Badge { key, label }`
  是开放词汇（不给枚举；`key` 留给以后做样式钩子，v1 只显示 `label`）。
- 置顶行把徽标并进主文案（`1.20.1（最新、推荐）`），下方完整列表仍包含这些条目，但通过
  `VersionRow::show_badges = false` 不显示标签，避免重复；没有置顶项时「全部版本」标题隐藏
  （置顶区本身不加标题），没有药丸样式。
- 搜索框非空时置顶区整体隐藏（连同「全部版本」标题），只显示过滤后的完整列表，避免不匹配的
  置顶版本混在结果里。
- 搜索对 `id` / `label` 做大小写不敏感子串匹配；是否显示搜索框由 Init 的 `show_search`
  控制（MC 显示，加载器不显示）。
- chips（`KindChip`）由页面给，每个 chip 带 `default_selected`，默认按版本类型选，例如 MC
  只开「正式版」，而不是全选。全开 = 只显示列出的类别，全关 = 不按类别筛；chips 用
  `caption` 字号 + `.dmcl-version-chip` 收紧 `.pill` 的默认 padding。
- 类型列直接取 `VersionItem.kind` 对应的 chip 文案，所以 MC 行右侧显示「正式版 / 快照版 /
  远古版」，没有 chips 的加载器不显示类型列。
- 过滤是纯函数 `filter_items`（保持数据源的新→旧顺序），有单测。选中标记只更新真正变化的
  两行（`iter_mut` 会把整张表标脏，900 行的 MC 列表会因此重绘整表）。

### 展开承载

- 行 = `AdwExpanderRow`；展开区随内容变长，列表区 220–480 高（超过 480 在内部滚动）。页面
  外层有 `GtkScrolledWindow`，展开多个列表时可以滚。
- 不用 `AdwComboRow`：它的弹出层是单层平铺列表，放不下分节标题与 chips。
- 版本行外层 `GtkListBox` 会再套一层可悬停的隐式 row，所以列表加 `.dmcl-version-list`、在
  CSS 里清掉外层 row 的 hover 背景，否则会和外层 Button 的 hover 叠成「外浅内深」两圈强调色。
  展开内容本身也被包成一行，该行设为不可激活（`.dmcl-version-panel-row`），否则鼠标进入整块
  展开区会变深。

### 展开时机

- 加载器行默认收起；MC 行在新建页面时首次可展开即自动展开
  （`VersionSelectInit::initially_expanded`）。选中版本后立即收起。
- 自动展开只消费一次，用户手动收起后不会被后续状态同步重新展开。
- libadwaita 的 `set_enable_expansion(false → true)` 会**顺手把行展开**，所以 `sync_view` 在
  启用后按 `expand_on_first_show` 显式设置展开态（只有首次自动展开的行会展开一次），并且
  不能把 `set_enable_expansion` 写成 `#[watch]`（每次视图刷新都会重新触发、把用户刚收起的
  行再弹开）。

## 消息

| 类型 | 说明 |
| --- | --- |
| `SummaryIn` / `SummaryOut` | 概要：`SetAutoName` / `SetAutoIcon` / `SetInstallEnabled` / `SetNameError(Option<String>)` / 图标菜单 / 名称与分类编辑；输出 `Install { name, category }` / `NameChanged` |
| `SelectIn` / `SelectOut` | 版本下拉：`SetContext` / `SetConflict` / `Reload` / `Deselect` / `SearchChanged` / `ToggleChip` / `RowClicked` / `Loaded`；输出 `Changed(Option<VersionItem>)` |

## CSS

`common/mod.rs` 注册本域全局 CSS：

- `.dmcl-version-selected`：选中行高亮；
- `.dmcl-version-list`：清掉 `GtkListBox` 隐式行的 hover 背景；
- `.dmcl-version-panel-row`：展开面板行不可激活；
- `.dmcl-version-chip`：收紧 chip 的 padding；
- `.dmcl-category-choice`：分类选择器里的可选项行高更紧凑。

## 测试

`common/version_select.rs` 的单测覆盖 `filter_items`：类别全开 / 全关、搜索大小写不敏感、
保持数据源顺序、未列在 chips 里的类别在筛选时被隐藏。
