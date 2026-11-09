# 「自定义」新建实例页

`crates/dmcl/src/gui/gtk4/pages/download/custom` —— 「下载 / 自定义」页，也就是新建实例页。
下载域与公共组件见 [../README.md](../README.md)，整体架构见
[gtk4/README.md](../../../README.md)。

## 模块

```text
mod.rs        页面组件、状态、init / update 与安装流程
message.rs    CustomMsg
loaders.rs    LoaderDef / LOADER_DEFS、运行时 LoaderSlot 与 loader_init
versions.rs   各数据源的取数器与 VersionItem 适配
```

## 装配

`CustomPage` 由 `InstanceSummary` 与五个 `VersionSelect`（MC + 四种加载器）组成。打开页面时
从主页传入 `Shell`、该主页的根目录和已有分类拷贝；安装目标、实例是否已存在都只用这个 root。
下载源与安装参数从全局共享配置现读，页面不持有 `Config` 副本。

## 设计约定

- **自动命名与图标**：MC 或加载器选择变化后，用 CLI 与 GUI 共用的命名规则算默认名，推给
  `InstanceSummary`；有加载器时用对应图标，否则用首字母头像。用户改过名称 / 图标后不再
  自动覆盖。
- **名称校验**：格式校验、`versions/<名称>` 冲突检查与按钮可用性都在 `InstanceSummary`
  内完成；最终的同名 / 非法名称仍由 `install_task` 把关。
- **加载器互斥**：`LOADER_DEFS` 用互斥组描述组合关系，两行共享任意组就冲突。Forge 可与
  OptiFine 叠加，Fabric / NeoForge 与两者都冲突；调整组合只改表。
- **版本适配**：`versions.rs` 里的纯函数把各数据源转成统一的 `VersionItem`，负责最新 /
  推荐徽标、MC 类型合并、Forge 展示版本与安装 id 的区分等。空列表是常态，走「无可用版本」
  而不是错误。
- **安装**：点击安装后按装配态组 `InstallSpec`，用 `Shell` 跑 `install_task` 并弹出任务
  对话框，页面立即关闭；任务结果在任务对话框里看。安装成功且分类非空时，再把分类写进该
  实例的 `dmcl.json`；失败 / 取消时安装链已回滚，不写分类。
