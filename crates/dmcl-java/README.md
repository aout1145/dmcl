# dmcl-java

Java 探测、扫描、自动选择，以及 Mojang JRE 的安装。

在 workspace 内依赖 `dmcl-mcmeta`、`dmcl-download` 和 `dmcl-task`。它**不读配置文件**：
Java 选择由调用者实现 `JavaResolver`，JRE 下载参数由 `JavaRuntimeOptions` /
`JreInstallRequest` 传入；自动安装成功后的配置写回由调用者实现 `JavaInstalledHook`。

## 模块布局

```text
option.rs    JavaOption / JavaDetails
probe.rs     执行 java、解析版本横幅 / 大版本
roots.rs     default_java_roots / official_runtime_root
scan.rs      find_java / auto_select_java
resolver.rs  JavaResolver / JavaInstalledHook；JavaOption 自身也是解析器
runtime.rs   JavaRuntimeOptions（下载源 + 下载策略）
jre.rs       Mojang JRE 安装（raw / lzma）与组件选择
```

## 设计意图

### 只解析，不产生副作用

`JavaResolver` 只根据要求的大版本返回可用的 `JavaOption`（路径 / 大版本 / 架构），安装流程
只用路径，启动流程用全部三项。扫描与探测各自返回结果，不自动写配置；需要写回时由调用方
通过 `JavaInstalledHook` 决定。这样 CLI、GUI 与安装流程可以按各自的策略使用同一套解析。

### 自动选择

`auto_select_java` 只读缓存的版本与架构，不执行 Java：版本相等优先，其次最接近的更高版本，
再其次最高的更低版本；同一档内优先当前平台架构。

### JRE 安装

`install_jre` 是唯一的公共安装入口，接收调用方共享的 `DownloadContext`，因此 JRE 文件与
安装 / 启动图里的其它下载共用并发预算和全局进度。请求强制要求大版本，可选指定组件；目标
目录固定为官方 runtime 布局。安装按增量缓存处理：已完整的文件跳过，缺失或损坏的重新下载，
目录里的其它文件不动；默认优先 lzma，缺失时回退 raw。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 元数据与平台信息：[crates/dmcl-mcmeta/README.md](../dmcl-mcmeta/README.md)
- 并发下载与校验：[crates/dmcl-download/README.md](../dmcl-download/README.md)
