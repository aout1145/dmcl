# 设置页

`pages/settings` —— 主菜单里的「启动器选项」和「全局游戏选项」。两个页面目前都只有空的
`AdwPreferencesPage` 骨架，正式分组后续补。

## 页面与入口

| 页面 | 组件 | 菜单位置 | 快捷键 |
| --- | --- | --- | --- |
| 启动器选项 | `LauncherOptionsPage` | 主菜单「启动器选项(L)」 | `Ctrl+,` |
| 全局游戏选项 | `GameOptionsPage` | 主菜单「全局游戏选项(G)」 | `Ctrl+Shift+,` |

- 菜单点击和快捷键都走同一组应用级 action：`app.launcher-options` /
  `app.game-options`；
- 菜单项在打开菜单后也可以用 `Alt+L` / `Alt+G` 激活。

两条路径最终都落到根组件的 `open_launcher_options` / `open_game_options`，行为一致。

## 生命周期

两个设置页在根组件启动时创建，分别存放在 `PageSlot<LauncherOptionsPage>` 和
`PageSlot<GameOptionsPage>` 里：

- 页面最多只有一个标签；重复触发只聚焦已有标签；
- 关闭标签只清空 `PageSlot::tab`，组件和页面状态继续保留；
- 再次触发时把同一个根 widget 重新 append 成标签。

它们不实现 `Page` trait；`PageHost` 只负责 `CustomPage` 这类关闭即销毁的一次性页面。

## 扩展

具体设置项直接加在各自的 `view!` 里，用 `AdwPreferencesGroup` + `AdwActionRow` /
`AdwSwitchRow` / `AdwEntryRow` / `AdwComboRow`。需要跨组件状态时给组件加 `Input` / `Output`
消息，不要把状态放到根组件。
