# 主页

`crates/dmcl/src/gui/gtk4/pages/home` —— 实例列表、运行状态与游戏进程。整体架构见
[gtk4/README.md](../../README.md)。

## 组件与视图

`HomePage` 是 relm4 组件，**视图用 `view!` 声明**（`GtkOverlay` + `GtkStack` +
`AdwWrapBox` + 底部胶囊操作栏），`#[watch]` 负责在模型变化后刷新空状态 / 操作栏文案 / 图标 /
可用性；磁贴列表用 `FactoryVecDeque<InstanceCard>`，不再手写 `render` / `remove` + `append`。

实例列表**自动跟随 `versions` 目录**：后台每秒对比顶层版本目录快照（名字 + 目录 mtime），
变化就发 `HomeMsg::Reload`——安装关页后创建实例目录、外部新增 / 删除版本都能立刻看到。

这里用轮询而不是事件监听库：版本目录很小、跨平台，且不必处理「目录还不存在 / 被整体替换」
等监听器边界，也不用新增依赖。

## 实例磁贴

- 磁贴是**等宽**的透明 `GtkButton`（类似 GNOME Files：大图标 + 最多三行名称；行高按内容
  收缩，不预留三行）。
- **运行中**时右上角显示小绿点（`media-record-symbolic` + `.success`）。
- 磁贴的单击选中 / 双击启动通过 `CardOut` 上报给主页。
- **单击选中**，底部浮出**胶囊操作栏**（`.card` + `.dmcl-action-bar`，含启动 / 停止、管理、
  取消选择）；**点空白处**或胶囊栏的 × 取消选中；**双击启动**（双击手势放在捕获阶段，否则
  会被按钮自己的点击吞掉）。

## 启动 / 停止 / 运行状态

**运行中 / 启动中 / 停止中的实例拒绝再次启动**：每个实例用 `InstanceState`
（`Idle` / `Starting` / `Running` / `Stopping`）记录状态，底部栏的文案、图标、按钮动作都由它
派生。

- **启动**：home 用实例名拼出 `dmcl_launch::launch_chain`，交给 `Shell::run`；一个扁平的
  `relm4::spawn` 等 `handle.finished()`，成功就发 `HomeMsg::GameStarted`，失败发
  `LaunchFailed`。
- **运行状态不轮询**：把 `GameMonitor` 交给一个 supervisor 任务：

  ```text
  let abnormal = tokio::select! {
      status = monitor.wait() => !matches!(status, Ok(s) if s.success()),
      _ = &mut stop_rx => {
          let _ = monitor.kill().await;
          let _ = monitor.wait().await;
          false   // 用户主动停止不算异常
      }
  };
  sender.input(HomeMsg::GameExited { name, abnormal });
  ```

  （`select!` 的两个分支都不在处理器里碰 monitor，避免双重可变借用；要 kill 就先退出 select。）

- **异常退出**：非 0 退出码（或拿不到退出状态）算异常，home 弹「xxx 异常退出」；用户主动
  停止不算异常。更详细的错误报告窗口是远期规划。
- **启动中可以直接取消**：用 `CancelHandle` 取消启动链，同时发停止信号，覆盖「链刚好跑完、
  进程已经起来」的竞态；按钮变成不可点的「停止中…」，等收尾完成再回到 `Idle`。
- **停止**：操作栏只发 `HomeMsg::Toggle`，home 按选中实例的状态决定启动、取消还是停止。
- **取消任务**：任务窗口自己持有 `CancelHandle`；卡片右上角的叉在运行中会取消
  （`TaskMsg::Close`），与主页的停止是两条独立路径。

## 消息

`HomeMsg`（`message.rs`）覆盖主页内部动作：`Reload` / `Loaded` / `Select(DynamicIndex)` /
`Deselect` / `Launch(DynamicIndex)` / `Toggle` / `GameStarted` / `GameExited { name, abnormal }` /
`LaunchFailed` / `Manage`。

## 测试

`versions_snapshot` 的离线单测覆盖：只列直接子目录、按名字排序、目录不存在时返回空快照
（全新 root 也能起监听）。
