# 主页

`crates/dmcl/src/gui/gtk4/pages/home` —— 一个 Minecraft 根目录下的实例列表、运行状态与
游戏进程。整体架构见 [gtk4/README.md](../../README.md)。

## 一个主页 = 一个 root

`HomePage` 的初始化参数里带该 root 已展开的 `PathBuf`。版本目录监听、实例配置读取、
安装参数与启动参数都只用这个 root，不再读取全局 `minecraft_root`。`Config` 不随参数传入：
页面持有全局 `ConfigStore` 的只读句柄，启动 / 安装时现读，运行时改配置立即生效。

主页标签可以随时关闭，但关闭只影响显示：`HomePage` 从程序启动起就常驻，标签只是它的
一个挂载点。

- 关闭标签不会中止版本目录 watcher；
- 关闭标签不会中止 monitor task，也不会 drop `GameMonitor`；
- 游戏进程继续独立运行；
- 重新打开同一 root 时把同一个根 widget 挂回新标签，选中项、运行状态和启动链都保留。

最后一个主页标签也可以关闭，此时内容区为空，仍可从标题栏的根目录菜单重新打开。

## 视图

`HomePage` 是 relm4 组件，**视图用 `view!` 声明**（`GtkOverlay` + `GtkStack` +
`AdwWrapBox` + 底部胶囊操作栏），`#[watch]` 负责在模型变化后刷新空状态 / 操作栏文案 / 图标 /
可用性；磁贴列表用 `FactoryVecDeque<InstanceCard>`。

- 磁贴单击选中、双击启动；底部胶囊操作栏显示启动 / 取消 / 停止、管理、取消选择。
- 右下角有一个悬浮的圆形 `+` 按钮：点开模态「创建实例」对话框。对话框有标题栏和关闭
  按钮；目前只有「自定义」一项，副标题是「创建自定义的 Minecraft 实例」。选择后主页发出
  `HomeOut::CreateCustom`，App 使用这个主页创建时捕获的 root 打开 `CustomPage`，不依赖
  当前选中的标签。
- **运行中**时磁贴右上角显示小绿点；启动链阶段不显示。
- 选中项按**版本名**保存；列表重建后按名字恢复。

## 实例列表

`versions/` 目录用 `notify` watch；`versions` 还不存在时先 watch 最近存在的祖先目录，
它创建后再重新挂到 `versions`。事件先做 200ms 合并，再发 `HomeMsg::Reload`，不做固定轮询。

显示列表是：

```text
installed（目录里的版本，保持顺序）
  ∪
runtimes（Starting / Running / Stopping 的版本）
```

活动但目录里暂时不存在的版本会追加成幽灵卡片，直到进程退出；这样不会因为目录被删就
失去停止入口。

`Reload` 重建卡片时状态从 `runtimes` 取，不会把 Running 重置成 Idle。

## 运行状态

每个活动实例用：

```text
Idle / Starting / Running / Stopping
```

`Idle` 不进入 `runtimes`；没有条目就是 Idle。

运行状态只有一个来源：本页启动成功后拿到的 `GameMonitor`。

启动链成功拿到 `GameMonitor` 后，主页启动一个 monitor task：

```text
select! {
    status = monitor.wait() => 自然退出
    _ = stop_rx => kill + wait
}
  ↓
HomeMsg::MonitorExited { name, stopped, abnormal }
```

它能精确判断正常退出、异常退出、用户主动停止。关闭标签页后 monitor task 和 `GameMonitor`
继续工作；重新打开页面时看到的就是最新状态。只有程序退出时才会中止这些任务。

## 启动 / 停止

- **启动**：同步读取实例配置；有警告先弹确认框，确认后用「全局 `launch` + 实例覆盖」
  和本主页 root 拼出 `launch_chain`，交给 `Shell::run`。全局 `launch` 从共享配置的
  `ConfigRef` 现读，不缓存启动那一刻的副本。
- 启动链的完成任务不挂在 `HomePage` 的 `TaskGuard` 上：关闭标签后启动链继续跑，
  结果仍会回到常驻的 `HomePage`。
- **Starting 阶段取消**：取消启动链；如果链刚好成功，`Launched` 会看到 `Stopping`，
  立即给 monitor task 发停止信号。
- **Running 阶段停止**：给 monitor task 发 stop。
- **取消任务**：任务窗口自己持有 `CancelHandle`，与主页停止是两条路径。

## 提示

`HomePage` 持有自己的 `PageToast`：

- 游戏正常退出 / 异常退出 / 用户停止、启动失败、版本目录 watcher 失败走
  `Shell::global_toast`，标签关闭时也能看到。
- 页面内反馈走 `Shell::page_toast`，页面不可见时直接丢弃。包括“正在启动 / 已在运行 /
  正在停止”、读取已安装版本失败和管理页占位提示。

## 消息

`HomeMsg`（`message.rs`）包括：

```text
Reload / Loaded
Select / Deselect / Launch / Toggle
LaunchConfirmed { name, launch }
Launched { name, monitor }
LaunchFailed { name }
MonitorExited { name, stopped, abnormal }
WatchFailed(String)
Manage
CreateInstance
ConfigChanged
```

对外只输出 `HomeOut::CreateCustom`；App 用创建主页时捕获的 root 打开新建实例页。

迟到消息按 runtime 是否存在、来源是否匹配来忽略；不使用 run id。

## 实例配置警告

实例目录里的 `dmcl.json` 只覆盖全局 `launch`，且只读。读取后若有坏字段、未知键或失效的
账户 / Java 选择，先弹 `AdwAlertDialog` 逐条列出警告；「取消」是默认响应，实例保持
`Idle`；「继续启动」才用内存里修复过的 `LaunchConfig` 开始启动链。不写回实例文件。

## 测试

- `watch_target`：`versions` 存在时监听它，否则回退到最近存在的祖先目录。
- `visible_versions`：保留 installed 顺序，活动幽灵版本追加并去重。
- `event_changes_versions`：只接受直接子项的创建 / 删除 / 重命名，忽略目录内部文件写入和 metadata 变化。
