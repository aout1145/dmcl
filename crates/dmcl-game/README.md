# dmcl-game

一个已安装 Minecraft 版本的领域对象与文件物化：版本引用 / 解析、文件规划、完整性检查、
补全缺件、natives 下载与解压。

在 workspace 内依赖 `dmcl-mcmeta`、`dmcl-download` 和 `dmcl-task`。配置（下载源、并发
策略、是否跳过音乐）由调用者通过 `GameFilesOptions` 传入，**这个 crate 不读任何配置文件**。

## 模块布局

```text
version.rs    VersionRef / ResolvedVersion / resolve_version
options.rs    GameFilesOptions、download_planned
plan.rs       纯函数规划：这个版本需要哪些文件
integrity.rs  完整性检查：完整吗？缺什么？
ensure.rs     ensure_files / ensure_client_jar / ensure_libraries / ensure_assets / ensure_asset_index
natives.rs    按 os / arch 下载并解压 natives
extract.rs    薄 extract_zip（zip 9 的安全解压）
```

## 设计意图

### 版本描述不内嵌资源索引

`ResolvedVersion` 只保存版本引用与版本 JSON；资源索引是独立文档，可能不存在、未下载或已
损坏，因此由完整性检查 / 补全流程按需读取。只看 Java 版本等信息的调用方不必为几 MB 的
索引付 I/O 成本。

### 规划与下载分离

「这个版本由哪些文件组成」是纯函数，输入版本字段，输出待检查 / 下载的文件列表。规划阶段
的 URL 仍是模板，真正下载时才按 `DownloadSource` 展开候选地址。这样安装、启动与加载器
安装可以复用同一套规划，也可以只取其中一部分。

### 完整性与补全

`check_integrity` 只回答「完整吗？缺什么？」，不下载任何东西，也不检查 natives（natives
取决于启动时选中的 JVM 架构）。`ensure_files` 一次补齐整个版本，已存在文件跳过，不删除
额外文件；安装流程需要拆分步骤时，可以分别使用客户端 jar、依赖库、资源等入口，它们共享
同一套规划与下载逻辑。

### natives

`ensure_natives` 按给定 os / arch 下载分类器 jar 并解压到 `dmcl_mcmeta::layout::natives_dir`
约定的目录。缺失的 jar 跳过并记日志，不把「库还没下」当成失败；解压使用安全实现，不创建
符号链接。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 元数据、URL 模板与下载源：[crates/dmcl-mcmeta/README.md](../dmcl-mcmeta/README.md)
- 并发下载与校验：[crates/dmcl-download/README.md](../dmcl-download/README.md)
