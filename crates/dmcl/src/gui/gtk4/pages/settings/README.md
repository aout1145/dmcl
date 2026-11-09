# 设置页

`pages/settings` —— 主菜单里的「首选项」。页面仿 GNOME 设置：左侧分类列表，右侧显示当前
分类的内容。目前「Minecraft 目录」「账号」「Java 管理」「全局游戏选项」可用；「下载」仍是
占位。

## 结构

`PreferencesPage` 用 `AdwNavigationSplitView` 做常驻双栏：左栏按组分三个列表，右栏是
`GtkStack`，选中分类就切换到对应页面。分类实现在 `launcher/` 和 `game/`，可复用的交互放在
`common/`：

```text
common/account_display.rs  账户的 ID、类型文案与占位头像文字
common/drag_list.rs        通用拖动排序
common/java_cache.rs       Java 探测结果缓存
common/java_display.rs     Java 标题 / 供应商 / 架构的展示文案
launcher/mod.rs            分栏、分组侧栏、右栏切换
launcher/account.rs        账号分类
launcher/directory.rs      Minecraft 目录分类
launcher/java.rs           Java 分类
game/mod.rs                游戏设置的目标抽象与字段级改动
game/page.rs               可复用的 GameOptionsPage 表单
game/choice.rs             展开选择项行
game/global.rs             全局落点：把改动写进共享配置
```

## 设计约定

- 配置只从全局 `ConfigStore` 读取，写只走 `update`；分类不持有 `Config` 副本。根组件转发
  配置变更时，各分类重新读取。
- 目录页与账号页也被首次启动向导复用，行为保持一致。
- **目录**：名称非空且不重名，路径必须是存在的目录；改名保持顺序、默认目录跟着改名；拖动
  只改顺序；删除时至少保留一个目录，默认目录被删后回退到当前目录或剩下的第一项。目录名
  同时是主页名，改名 / 改路径 / 删除要重启后才反映到已有主页。
- **账号**：支持离线账户与 Microsoft 账户；微软凭据存系统钥匙串，保存成功后配置才写入。
  没有默认账户时新账户自动成为默认；删除默认账户后回到未选择。
- **Java**：页面只管理可用 Java，默认 Java 在「全局游戏选项」里选。搜索本机后按路径去重
  合并进配置；首次显示时才在后台探测，结果进入共享缓存，不写回配置。
- **全局游戏选项**：`GameOptionsPage` 是可复用表单，只通过 `GameOptionsTarget` 读取当前
  生效的启动配置、提交字段级改动；全局 target 写入共享配置，以后实例管理页可以复用同一个
  表单。控件改动立即更新模型，防抖后只提交本次改动的字段。
- 页面常驻：重复触发只聚焦已有标签，关闭标签只清空标签句柄，组件和页面状态继续保留。
