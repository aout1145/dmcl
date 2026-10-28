# 设置页

`pages/settings` —— 主菜单里的「首选项」。页面仿 GNOME 设置：左侧分类列表，右侧显示当前
分类的内容。目前「Minecraft 目录」和「Java 管理」可用，其余分类置灰占位。

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
- 右栏是 `GtkStack`，前两个分类分别放 `DirectoryPage` 和 `JavaPage`；选中左栏的分类就切换
  到对应页面。账号与全局游戏选项仍置灰并带「尚未实现」副标题。

分类实现在 `launcher/`，可复用的交互放在 `common/`：

| 文件 | 内容 |
| --- | --- |
| `common/drag_list.rs` | 通用的拖动排序：拖动手柄、落点提示、`DragMove` |
| `launcher/mod.rs` | 分栏、分类列表、右栏切换、`PreferencesMsg` |
| `launcher/directory.rs` | Minecraft 目录分类、`RootRow`、对话框与校验、配置写入 |
| `launcher/java.rs` | Java 分类、`JavaRow`、搜索、探测对话框与配置写入 |

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

添加 / 编辑对话框在输入时实时校验：名称、路径或「路径格式」一变，确认按钮和下方提示就跟着
更新（空字段不报错，只保持按钮不可点击）。路径行在回车或失去焦点时再规范化并改写，
「路径格式」下拉选择存绝对路径还是相对路径：相对路径以启动时的工作目录为基准，不在
其下的路径会保留绝对路径，同时把「路径格式」切回绝对路径并在下方提示。校验要求路径存在
且是目录，保存时一律按所选格式规范化（`canonicalize()`，Windows 下去掉 `\\?\` 前缀），
不会保留输入框里的写法。删除前先弹 `AdwAlertDialog` 确认。对话框、校验和测试都在
`directory.rs` 里。名称或路径为空、重名、路径不存在或不是目录时，右上角的确认按钮保持
不可点击。

配置里保存对话框规范化后的路径（相对或绝对），行副标题显示绝对化后的实际目录。读取配置
不会把相对路径改写成绝对路径；绝对化只在程序内部取用根目录时发生。

目录名同时是标题栏「Minecraft 目录」菜单里的主页名。主页在启动时按名字创建，所以改名、
改路径和删除只写配置，**重启 DMCL 后**才会反映到已有主页；设置页的描述里写明了这一点。

## Java 管理

`JavaPage` 把 `Config::javas` 的每一项映射成 `FactoryVecDeque<JavaRow>`：

- 每行标题是「JDK/JRE + 完整版本」（例如 `JDK 17.0.2`），右侧小字是「供应商, 架构」
  （例如 `Microsoft, x86_64`），副标题是绝对路径；配置里的键只是内部标识，不显示；
- 行首是拖动手柄，行尾常驻「编辑」「删除」两个扁平图标按钮，删除按钮用错误色；
- 分组顶部是「搜索已安装的 Java」，底部是「添加 Java」，两个都是 `AdwButtonRow`，添加固定
  在最后。

Java 页第一次显示时才在后台执行 `JavaDetails::probe`：探测完成前该行显示转圈，成功后按上面的
格式显示。结果只缓存在页面内存里，不写回配置；页面隐藏后不再发起新的探测，再次显示时复用缓存。

页面只管理可用的 Java，不提供选择默认 Java 的入口；自动选择 Java 属于「全局游戏选项」，
该分类尚未实现。

「搜索已安装的 Java」在后台调用 `dmcl_launch::find_java`，结果按路径去重后合并进 `javas`：
已有条目保留名字、只刷新 `major` / `arch`，新条目用扫描生成的名字（`类型-平台-架构-版本-短哈希`）。
合并规则是 `config/java.rs` 的 `merge_javas`，CLI 的 `search-java` 与界面共用。

添加 / 编辑对话框只有可执行文件一行。路径可以用文件选择器，也可以手输；路径一变就清掉旧
结果并立即校验，不必等输入框失去焦点。半截路径只检查文件是否存在，只有路径已经指向文件时
才会在后台执行 `JavaDetails::probe`，所以不会为了无效路径白跑 `java` 子进程。回车、失焦、
选完文件或点确认时同样探测，下方显示探测到的「JDK 17.0.2, Microsoft, x86_64」。路径为空、
不是文件、被其它条目占用或还没探测成功时，确认按钮保持不可点击；路径被另一个 Java 占用时
不会执行 Java，直接在下方说明原因。编辑时路径没改就直接复用页面缓存里的探测结果（页面还没
探测过时才沿用配置里缓存的版本 / 架构），只检查文件仍然存在；路径一改就重新探测。保存时写入
绝对路径和探测结果；新增条目的内部键由探测结果自动生成，不从界面收集名称。

`Config` 上的 Java 方法与根目录那套保持一致：名称只是内部键，非空且不重名；拖动只改顺序；
删掉的正好是 `launch.java` 指向的条目时，`launch.java` 回到 `autoselect`。

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
3. 在左栏启用对应行，把新页加进右栏的 `GtkStack`，并在选中行时切到它。
