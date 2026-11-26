# 设置页

`pages/settings` —— 主菜单里的「首选项」与主页每个实例的「实例设置」。两者都是仿 GNOME
设置的左右分栏：左侧分类列表，右侧显示当前分类的内容。

首选项包含「Minecraft 目录」「账户」「Java 管理」「全局游戏选项」和「下载」。
实例设置的侧栏分两组：「实例」「启动选项」与「模组管理」「资源包管理」「存档管理」；「实例」
管理图标、名称、分类、加载器和删除，「启动选项」复用全局游戏选项的表单，每个字段既可以
继承全局值，也可以单独覆盖。

## 结构

`PreferencesPage` 用 `AdwNavigationSplitView` 做常驻双栏：左栏按组分三个列表，右栏是
`GtkStack`，选中分类就切换到对应页面。分类实现在 `launcher/` 和 `game/`，可复用的交互放在
`common/`；实例设置页在 `instance/`：

```text
common/account_display.rs  账户的 ID、类型文案与占位头像文字
common/choice_row.rs       展开式选择行的列表项（ChoiceRow）
common/fixed_row.rs        展开式选择行的固定项（FixedRow）
common/drag_list.rs        通用拖动排序
common/java_cache.rs       Java 探测结果缓存
common/java_display.rs     Java 标题 / 供应商 / 架构的展示文案
launcher/mod.rs            分栏、分组侧栏、右栏切换
launcher/account.rs        账户分类
launcher/directory.rs      Minecraft 目录分类
launcher/download.rs       下载分类
launcher/java.rs           Java 分类
game/mod.rs                游戏设置的目标抽象与字段级改动
game/global.rs             全局落点：把改动写进共享配置
game/target.rs             实例落点：字段级覆盖写进实例 dmcl.json
game/page/mod.rs           可复用的 GameOptionsPage 表单：骨架、继承与写回
game/page/account.rs       账户分组
game/page/java.rs          Java 分组与后台探测
game/page/window.rs        窗口类型分组
game/page/quick_play.rs    快速游玩分组
game/page/game_integrity.rs 游戏完整性分组
game/page/game_args.rs     游戏参数分组
game/page/priority.rs      进程优先级分组
game/page/memory.rs        内存分组
game/page/jvm_args.rs      JVM 参数分组
game/page/jvm_compatibility.rs  JVM 兼容性分组
game/page/version_isolation.rs  版本隔离分组
instance/mod.rs            实例设置页：两组侧栏、内容切换、标签标题与页面输出
instance/general.rs        「实例」分类：图标、名称、分类、加载器与删除
instance/files.rs          实例目录重命名与删除
instance/loaders.rs        从 dmcl-patches 读取当前加载器
```

## 设计约定

- 首选项只从全局 `ConfigStore` 读取，写只走 `update`；分类不持有 `Config` 副本。根组件转发
  配置变更时，各分类重新读取。
- 目录页与账户页也被首次启动向导复用，行为保持一致。
- **目录**：名称非空且不重名，路径必须是存在的目录；改名保持顺序、默认目录跟着改名；拖动
  只改顺序；删除时至少保留一个目录，默认目录被删后回退到当前目录或剩下的第一项。目录名
  同时是主页名，改名 / 改路径 / 删除要重启后才反映到已有主页。
- **账户**：支持离线账户与 Microsoft 账户；微软凭据存系统钥匙串，保存成功后配置才写入。
  没有默认账户时新账户自动成为默认；删除默认账户后回到未选择。
- **Java**：页面只管理可用 Java，默认 Java 在「全局游戏选项」里选。搜索本机后按路径去重
  合并进配置；首次显示时才在后台探测，结果进入共享缓存，不写回配置。
- **下载**：镜像策略、并发、分块、重试和安装器 Java 都直接对应 `download` 段；改动先留在
  页面内，防抖后整段写回共享配置。安装器 Java 与游戏设置共用展开式选择行，只影响安装流程的
  processor，不影响启动。
- **全局游戏选项**：`GameOptionsPage` 是可复用表单，只通过 `GameOptionsTarget` 读取当前
  生效的启动配置、提交字段级改动；全局 target 写入共享配置。控件改动立即更新模型，防抖后
  只提交本次改动的字段。表单按 `GameOptionGroup` 拆在 `game/page/` 下，每组持有自己的
  选择状态、探测缓存与控件，通过 `GroupView` 读取共享的启动配置与继承状态。
- **展开式选择行**：列表项用 `ChoiceRow`，固定项用 `FixedRow`；两者激活后都自动收起所在的
  展开行。需要保留输入框的行显式 `keep_open`，分组里不再手写 `set_expanded(false)`。
- **JVM 参数与兼容性**：`additional_jvm` 为 `null` 表示使用默认参数，空串表示自定义但未填写；
  「JVM 兼容性」决定启动时是否校验所选 Java 的版本与架构，以及「自动选择」找不到合适 Java 时
  是否下载 JRE；已指定具体 Java 时只检查、不替换（选「忽略」则跳过检查）。
- **游戏完整性**：决定启动时是否检查 / 补全版本文件。「仅检查」在不完整时提示并由用户决定
  是否继续；只有「检查并补全」会下载缺失文件与 natives jar。安装流程已按当前平台下载
  natives jar。
- **实例设置**：同一个 root + 实例名只保留一个标签，再次「管理」时聚焦已有标签；页面关闭
  即销毁。实例覆盖是字段级的：只写本次改动过的 `launch` 字段到实例 `dmcl.json`，未覆盖的
  字段继续继承全局；选「使用全局配置」就删掉对应键。是否覆盖以实例文件里出现的键为准，
  坏值也算覆盖，用户仍可用「使用全局配置」清掉。
- **实例分类**：图标写 `versions/<名称>/logo.png`，选「自动」就删掉该文件，由版本类型与
  加载器决定显示素材。分类写入实例 `dmcl.json` 的 `instance.category`；清空分类时删掉该键，
  `instance` 段空了整段删除。名称修改是目录重命名：同步 `<名称>.json` / `<名称>.jar` 的文件名
  和版本 JSON 里等于旧名的 `id` / `jar`；`dmcl.json`、`logo.png` 等文件跟着目录移动，其它
  版本写在 `inheritsFrom` 里的引用不跟着改。重命名、删除、图标与加载器操作同一时间只允许
  一个，运行中的实例也允许删除。
- **加载器**：只有版本 JSON 带 `dmcl-patches` 的实例可调整；以 `dmcl-patches[0]` 为原版快照，
  保留未变化的加载器 patch，新增 / 换版本的加载器重新安装，最后重写版本 JSON。界面从远程
  列表选择加载器版本，未变化时「应用」不可用；同组加载器冲突时自动清掉另一项的选择。
- 实例页的继承展示与写入都在表单这一层完成：每个分组提供「使用全局配置」的行，继承时展开
  行小字标注「使用全局配置」、输入控件变灰，继承行本身仍显示全局当前值；「自动选择」的 Java
  行显示按实例版本要求解析出的实际 Java。
- 首选项常驻：重复触发只聚焦已有标签，关闭标签只清空标签句柄，组件和页面状态继续保留。
