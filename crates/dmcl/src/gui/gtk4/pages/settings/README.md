# 设置页

`pages/settings` —— 主菜单里的「首选项」。页面仿 GNOME 设置：左侧分类列表，右侧显示当前
分类的内容。目前只有「Minecraft 目录」可用，其余分类置灰占位。

## 页面与入口

| 页面 | 组件 | 菜单位置 | 快捷键 |
| --- | --- | --- | --- |
| 首选项 | `PreferencesPage` | 主菜单「首选项(P)」 | `Ctrl+,` |

- 菜单点击和快捷键都走同一个应用级 action：`app.preferences`；
- 菜单项在打开菜单后也可以用 `Alt+P` 激活。

两条路径最终都落到根组件的 `open_preferences`，行为一致。

## 结构

`PreferencesPage` 用 `AdwNavigationSplitView` 做常驻双栏：

- 左栏是 `GtkListBox`（`.navigation-sidebar`），按顺序列出「Minecraft 目录」「账号」
  「Java 管理」「全局游戏选项」；
- 右栏当前固定是 `DirectoryPage`（`AdwPreferencesPage`）。只有第一个分类可用，其余行
  `sensitive = false` 并带「尚未实现」副标题；等分类多起来再接切换逻辑。

分类实现在 `launcher/`，可复用的交互放在 `common/`：

| 文件 | 内容 |
| --- | --- |
| `common/drag_list.rs` | 通用的拖动排序：拖动手柄、落点提示、`DragMove` |
| `launcher/mod.rs` | 分栏、分类列表、`PreferencesMsg` |
| `launcher/directory.rs` | Minecraft 目录分类、`RootRow`、对话框与校验、配置写入 |

## Minecraft 目录

`DirectoryPage` 把 `Config::roots()` 的每一项映射成 `FactoryVecDeque<RootRow>`：

- 每行是 `AdwActionRow`，标题为目录名、副标题为展开后的路径；只有右侧按钮响应点击；
- 行首是拖动手柄，拖动可调整顺序；
- 行尾常驻「设为默认 / 默认目录」「编辑」「删除」三个扁平图标按钮，默认项用实心星星，删除按钮用错误色；
- 分组最底部是「添加目录」`AdwButtonRow`，重建列表后仍保持在末尾。

拖动排序由 `common/drag_list.rs` 提供：拖动时源行淡化，落点行上方或下方显示提示线；
放下后把源目录插入目标目录之前或之后。顺序即标题栏菜单里的顺序。

所有改动都通过 `config_store::store().update(...)` 写回全局配置并异步落盘，失败时用页面级
toast 提示。`Config` 上的对应方法维护不变式：

- 名称非空且不能与已有目录重名，路径不能为空；
- 改名保持原顺序，默认目录跟着改名；拖动只改顺序，不改默认目录；
- 删除时至少保留一个目录；删掉默认目录后优先回退到「当前目录」，否则取剩下的第一项。

添加 / 编辑对话框在确认前校验名称和路径。路径行在回车或失去焦点时立即校验并改写，
「路径格式」下拉选择存绝对路径还是相对路径：相对路径以启动时的工作目录为基准，不在
其下的路径会保留绝对路径，同时把「路径格式」切回绝对路径并在下方提示。校验要求路径存在
且是目录，保存时一律按所选格式规范化（`canonicalize()`，Windows 下去掉 `\\?\` 前缀），
不会保留输入框里的写法。删除前先弹 `AdwAlertDialog` 确认。对话框、校验和测试都在
`directory.rs` 里。名称或路径为空、重名、路径不存在或不是目录时，右上角的确认按钮保持
不可点击；按钮状态随名称、路径和「路径格式」的每次变化重新计算。

配置里保存对话框规范化后的路径（相对或绝对），行副标题显示绝对化后的实际目录。读取配置
不会把相对路径改写成绝对路径；绝对化只在程序内部取用根目录时发生。

目录名同时是标题栏「Minecraft 目录」菜单里的主页名。主页在启动时按名字创建，所以改名、
改路径和删除只写配置，**重启 DMCL 后**才会反映到已有主页；设置页的描述里写明了这一点。

## 生命周期

首选项页在根组件启动时创建，存放在 `PageSlot<PreferencesPage>` 里：

- 页面最多只有一个标签；重复触发只聚焦已有标签；
- 关闭标签只清空 `PageSlot::tab`，组件和页面状态继续保留；
- 再次触发时把同一个根 widget 重新 append 成标签。

它不实现 `Page` trait；`PageHost` 只负责 `CustomPage` 这类关闭即销毁的一次性页面。

## 配置与刷新

配置来自 `app/config_store.rs` 的全局单例，不通过 `Init` 传入：读用
`config_store::store().config()` 拿 `ConfigRef`，写用
`config_store::store().update(|config| …, |err| { … })`。`update` 自己异步落盘，第二个
参数是保存失败的处理函数（在主线程上执行），必须给出。

根组件在 `ConfigChanged` 时转发 `PreferencesMsg::ConfigChanged`，首选项再把它交给各分类，
由分类重新读取配置。跨页面状态不要放到根组件。

## 扩展

新增分类时：

1. 在 `launcher/` 下加一个 `SimpleComponent`，页面本身用 `AdwPreferencesPage` /
   `AdwPreferencesGroup` + `AdwActionRow` / `AdwSwitchRow` / `AdwEntryRow` / `AdwComboRow`；
2. 在 `PreferencesPage` 里用 `Controller` 保活，并把 `ConfigChanged` 继续转给它；
3. 在左栏启用对应行，同时把右栏换成能按选中行切换内容的容器（例如 `GtkStack`）。
