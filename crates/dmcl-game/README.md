# dmcl-game

一个已安装 Minecraft 版本的领域对象与文件物化：版本引用 / 解析、文件规划、完整性检查、
补全缺件、natives 下载与解压。

在 workspace 内依赖 `dmcl-mcmeta`（版本文档与目录布局）、`dmcl-download`（并发下载）和
`dmcl-task`（任务上下文）。配置（下载源、并发策略、是否跳过音乐）由调用者通过
`GameFilesOptions` 传入，**这个 crate 不读任何配置文件**。

## 模块布局

```text
version.rs    VersionRef / ResolvedVersion / resolve_version
options.rs    GameFilesOptions、download_planned
plan.rs       纯函数规划：这个版本需要哪些文件
integrity.rs  完整性检查：完整吗？缺什么？
ensure.rs     ensure_files / ensure_client_jar / ensure_asset_index
natives.rs    按 os / arch 下载并解压 natives
extract.rs    薄 extract_zip（zip 9 的安全解压）
```

## 设计要点

### 版本描述不内嵌资源索引

`ResolvedVersion` 只有 `reference` + `json`。资源索引是 `assetIndex` 指向的独立文档，
可能不存在、未下载或已损坏，因此由 `ensure_asset_index` / `check_integrity` 单独读取，
`plan_files` 显式接收 `Option<&AssetIndexFile>`。这样只看 `javaVersion` 的调用方不必为一个
可能几 MB 的索引付 I/O 成本。

`resolve_version` 只解析一层 `inheritsFrom`，与现有行为一致。

### 规划与下载分离

「这个版本由哪些文件组成」是**纯函数**（`plan`）：输入 `VersionRef` / `VersionJson`
（以及可选的资源索引），输出 `Vec<PlannedFile>`。`PlannedFile` 里的 url 仍是模板，
直到 `into_item` 才按 `DownloadSource` 的镜像策略展开成候选地址。

### 完整性检查

`check_integrity` 只回答「完整吗？缺什么？」，不下载任何东西。它单独读取本地资源索引：
存在且可解析才规划资源对象；索引缺失或损坏时只报告索引本身。它**不检查 natives**，
因为 natives 的分类器取决于启动时选中的 JVM 架构。

### 补全

`ensure_files` 先确保资源索引，再按 `plan_files` 规划并下载；已存在的文件会被跳过，
额外文件不会被删除。`ensure_client_jar` 只下客户端 jar，供 Forge / NeoForge 的 processor
在安装时读取。

### natives

`ensure_natives` 按给定 os / arch 下载分类器 jar，再解压到
`dmcl_mcmeta::layout::natives_dir`（`<临时目录>/dmcl-natives-<os>-<arch>-<版本名哈希>`）。
缺失的 jar 跳过并记日志，不把「库还没下」当成失败。解压使用 zip 9 的安全解压，
不创建符号链接。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 元数据、URL 模板与下载源：[crates/dmcl-mcmeta/README.md](../dmcl-mcmeta/README.md)
- 并发下载与校验：[crates/dmcl-download/README.md](../dmcl-download/README.md)
