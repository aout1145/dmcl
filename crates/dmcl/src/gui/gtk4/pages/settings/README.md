# 设置页

`pages/settings` —— 主菜单里的「首选项」。页面仿 GNOME 设置：左侧分类列表，右侧显示当前
分类的内容。目前「Minecraft 目录」「Java 管理」「全局游戏选项」可用；「账号」和「下载」仍是
置灰占位。

## 页面与入口

| 页面 | 组件 | 菜单位置 | 快捷键 |
| --- | --- | --- | --- |
| 首选项 | `PreferencesPage` | 主菜单「首选项(P)」 | `Ctrl+,` |

- 菜单点击和快捷键都走同一个应用级 action：`app.preferences`；
- 菜单项在打开菜单后也可以用 `Alt+P` 激活。

两条路径最终都落到根组件的 `open_preferences`，行为一致。

## 结构

`PreferencesPage` 用 `AdwNavigationSplitView` 做常驻双栏：

- 左栏按组分三个 `GtkListBox`（`.navigation-sidebar`），组间用 `GtkSeparator` 隔开：
  1. 「Minecraft 目录」「账号」
  2. 「全局游戏选项」「Java 管理」
  3. 「下载」
- 三个列表各自单选，选中一组时会清掉其它组的选择；账号和下载尚未实现，默认停在
  「Minecraft 目录」；
- 右栏是 `GtkStack`，可用的分类分别放 `DirectoryPage`、`JavaPage` 和 `GameOptionsPage`；
  选中左栏的分类就切换到对应页面。

分类实现在 `launcher/` 和 `game/`，可复用的交互放在 `common/`：

| 文件 | 内容 |
| --- | --- |
| `common/drag_list.rs` | 通用的拖动排序：拖动手柄、落点提示、`DragMove` |
| `common/java_cache.rs` | Java 探测结果缓存，Java 管理页与游戏设置页共用 |
| `common/java_display.rs` | Java 标题 / 供应商 / 架构的展示文案 |
| `launcher/mod.rs` | 分栏、分组侧栏、右栏切换、`PreferencesMsg` |
| `launcher/directory.rs` | Minecraft 目录分类、`RootRow`、对话框与校验、配置写入 |
| `launcher/java.rs` | Java 分类、`JavaRow`、搜索、探测对话框与配置写入 |
| `game/mod.rs` | 游戏设置的目标抽象、字段级改动与选择项映射 |
| `game/page.rs` | 可复用的 `GameOptionsPage` 表单 |
| `game/choice.rs` | `AdwExpanderRow` 里的可选项行 |
| `game/global.rs` | 全局落点：把改动写进共享配置 |

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
格式显示。结果记在 `common/java_cache.rs` 的共享缓存里，不写回配置；页面隐藏后不再发起新的
探测，再次显示或切到「全局游戏选项」时复用缓存。

页面只管理可用的 Java，不提供选择默认 Java 的入口；默认 Java 在「全局游戏选项」里选。

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

## 全局游戏选项

`GameOptionsPage` 是**可复用的表单**，不直接改全局配置：它只通过 `GameOptionsTarget`
读取当前生效的 `LaunchConfig`、提交字段级改动。全局分类由 `GlobalGameOptionsTarget`
把改动写进共享配置；以后实例管理页实现自己的 target，就能把同样的表单复用到单个实例。

页面只有一个「游戏」分组，行从上到下是：

| 行 | 内容 |
| --- | --- |
| 账户 | `AdwExpanderRow`「默认账户」，展开后是 `未选择` + 现有账户 |
| Java | `AdwExpanderRow`「默认 Java」，展开后是 `自动选择` + 现有 Java |
| 窗口大小 | `AdwExpanderRow`；内置默认 / 720p / 1080p，最后一项「自定义」直接输入宽高 |
| 内存 | `AdwExpanderRow`「游戏内存」；可选自动分配 / 手动分配，手动行右侧直接放 Xms / Xmx 输入框 |
| 高级 | `AdwExpanderRow`，默认折叠；里面是启动器信息、额外 JVM 参数、额外游戏参数三行 `AdwEntryRow` |

- 账户表和 Java 表是全局资源，表单直接从只读 `ConfigRef` 读取；写仍只走 target。
- 两个选择器都用 `AdwExpanderRow`：收起时副标题显示当前选择，展开后每项是一行
  `AdwActionRow`，右侧对勾表示当前项；选中后自动收起。
- 窗口大小用和账户 / Java 相同的展开选择器：收起时副标题显示当前宽高；展开后
  「默认」「720p」「1080p」三项上方是大字描述、下方是具体像素，最后一项「自定义」右侧
  用两个 `GtkSpinButton` 直接输入宽高。选中预设后自动收起。
- Java 选择项上方是「JDK/JRE + 完整版本」（如 `JDK 17.0.2`），下方是绝对路径，右侧是
  供应商与架构（如 `Microsoft, x86_64`）；还没探测到时回退为 `Java 17` / `x86_64`。
  探测结果与 Java 管理页共享，切换分类不会重复执行 Java。
- 内存用展开选择器：收起时副标题显示当前模式与数值；展开后「自动分配内存」下方小字显示
  当前自动分配值（系统可用内存的 3/4，至少 1 GiB；32 位 Java 再压到 1.5 GiB；读不到系统
  可用内存时固定 1.5 GiB），设置页可见时每秒刷新一次。「手动分配内存」行右侧直接放两个
  `GtkSpinButton` 编辑 Xms / Xmx。切到手动时以当前自动值为起点；Xms 不能超过 Xmx，输入上限
  取 `sysinfo` 读到的系统总内存，配置里已有更大的值时以上限为准不截断。
- 控件改动立即更新模型，400ms 防抖后把「完整值 + 本次改动的字段」合并成一次提交；
  页面隐藏时立即提交。target 只写列出的字段，实例 target 因此可以只写覆盖层。
- 根组件转发 `ConfigChanged` 时，表单重新读取 target 和全局账户 / Java 表。

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

1. 在 `launcher/`（或合适的子目录）下加一个 `SimpleComponent`，页面本身用
   `AdwPreferencesPage` / `AdwPreferencesGroup` + `AdwActionRow` / `AdwSwitchRow` /
   `AdwEntryRow` / `AdwComboRow`；
2. 在 `PreferencesPage` 里用 `Controller` 保活，并把 `ConfigChanged` 继续转给它；
3. 在左栏启用对应行，把新页加进右栏的 `GtkStack`，并在选中行时切到它。
