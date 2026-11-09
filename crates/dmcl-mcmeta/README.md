# dmcl-mcmeta

集中描述「游戏是什么」：Minecraft 的强类型元数据文档、下载源与 URL 模板、本地目录布局。
它**不把文件写到磁盘上**，也不读配置文件；根目录、镜像、并发度等一律由调用者传入。

`dmcl-mcmeta` 是 workspace 最底层的叶子 crate，不依赖其他成员 crate。

## 模块布局

```text
vanilla/     version.rs  merge.rs  patch.rs  manifest.rs  asset.rs  rules.rs  urls.rs  jre.rs
fabric/      类型 + 地址 + fetch
forge/       mod.rs + install_profile.rs
neoforge/    mod.rs + install_profile.rs
optifine/    版本列表、稳定 / 预览排序、bmclapi 下载地址
source.rs    镜像、URL 模板、DownloadSource
os.rs        Os / Arch 与平台约定
maven.rs     maven 坐标 → 仓库相对路径
layout.rs    .minecraft 目录布局、读取版本 JSON（含递归 inheritsFrom 合并）
json.rs      JsonMap 别名与 extra 字段的递归合并
macros.rs    string_enum! 内部宏
```

## 设计意图

### 强类型文档

版本 JSON、版本清单、资源索引、安装器档案、模组加载器版本列表都映射成强类型结构体。
会被写回的文档必须能往返：未知字段原样保留，未知枚举值有兜底分支，可选字段不写出 `null`。
这样 DMCL 不会因为 Mojang 或加载器新增字段而丢数据。

### 版本合并与 patch

版本 JSON 通过 `inheritsFrom` 沿祖先链逐层合并；`VersionJson::merge` 负责父子版本的字段、
参数、库与未建模字段的合并。加载器安装不直接改写版本 JSON，而是产出 `VersionJsonPatch`，
由上层按顺序统一应用。`patches` 保留原版快照与已应用的 patch，便于显示或回退。

### URL 模板与下载源

从 Minecraft 拿到的文档里的 URL 会被归一成模板（如 `${MC_MAVEN}`、`${MC_META}`），真正的
下载地址由 `DownloadSource` 按镜像策略展开成候选列表。写回磁盘的文档再解析回官方地址，
保证 `.minecraft` 里的内容与官方启动器通用。镜像只覆盖部分根时按根分别处理；取数统一走
`fetch_meta` / `fetch_json`，由下载源负责候选轮换、重试与退避。

### rules、JRE 与加载器列表

`rules` 按 Mojang 标准语义判定当前平台是否适用。`vanilla/jre.rs` 描述 Mojang 的 JRE 产品
索引与运行时文件清单，并提供本机平台键与取数入口。Fabric / Forge / NeoForge 的版本列表
统一按从新到旧返回，调用方不需要单独的「取最新」接口；OptiFine 列表来自 bmclapi，稳定版
排在预览版之前。

### 本地目录布局

`layout.rs` 约定 `versions/<name>/<name>.json`、`libraries/`、`assets/` 以及 natives 解压
目录的位置，并提供读取版本 JSON 的入口。这里只算路径、读文档，不写盘；真正的落盘在
`dmcl-install`。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 用这些元数据规划并落盘：[crates/dmcl-install/README.md](../dmcl-install/README.md)
- 下载任务如何消费候选地址：[crates/dmcl-download/README.md](../dmcl-download/README.md)
