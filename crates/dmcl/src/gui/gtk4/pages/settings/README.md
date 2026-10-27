# 设置页

`pages/settings` —— 主菜单里的「首选项」。页面目前只有空的 `AdwPreferencesPage` 骨架，
正式分组后续补。

## 页面与入口

| 页面 | 组件 | 菜单位置 | 快捷键 |
| --- | --- | --- | --- |
| 首选项 | `PreferencesPage` | 主菜单「首选项(P)」 | `Ctrl+,` |

- 菜单点击和快捷键都走同一个应用级 action：`app.preferences`；
- 菜单项在打开菜单后也可以用 `Alt+P` 激活。

两条路径最终都落到根组件的 `open_preferences`，行为一致。

## 生命周期

首选项页在根组件启动时创建，存放在 `PageSlot<PreferencesPage>` 里：

- 页面最多只有一个标签；重复触发只聚焦已有标签；
- 关闭标签只清空 `PageSlot::tab`，组件和页面状态继续保留；
- 再次触发时把同一个根 widget 重新 append 成标签。

它不实现 `Page` trait；`PageHost` 只负责 `CustomPage` 这类关闭即销毁的一次性页面。

## 扩展

具体设置项直接加在 `view!` 里，用 `AdwPreferencesGroup` + `AdwActionRow` /
`AdwSwitchRow` / `AdwEntryRow` / `AdwComboRow`。

配置来自 `app/config_store.rs` 的全局单例，不通过 `Init` 传入：读用
`config_store::store().config()` 拿 `ConfigRef`，写用
`config_store::store().update(|config| …, |err| { … })`。`update` 自己异步落盘，第二个
参数是保存失败的处理函数（在主线程上执行），必须给出。设置项在 `update` 里改配置，
界面用 `#[watch]` 读 `ConfigRef`；需要重画时由根组件转发的 `ConfigChanged` 消息触发。
跨页面状态不要放到根组件。
