# dmcl-java

Java 探测、扫描、自动选择，以及 Mojang JRE 的安装。

在 workspace 内依赖 `dmcl-mcmeta`（平台信息、JRE 索引类型、目录键）、`dmcl-download`
（并发下载）和 `dmcl-task`（任务上下文）。它**不读配置文件**：Java 选择由调用者实现
`JavaResolver`，JRE 下载参数由 `JavaRuntimeOptions` / `JreInstallRequest` 传入；
自动安装成功后的配置写回由调用者实现 `JavaInstalledHook`。

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

## 设计要点

### 只解析，不产生副作用

`JavaResolver::resolve(major)` 只返回 `JavaOption`（路径 / 大版本 / 架构）。安装流程只
需要其中的 `path`，启动流程需要全部三项。固定 Java 就是 `Arc<JavaOption>`，不再需要额外
的包装类型。

自动下载 JRE 并探测成功后，由 `JavaInstalledHook` 交给调用方决定是否写回配置；GUI 实现
这个 hook，CLI 与安装流程不设置它。

### 自动选择

`auto_select_java` 只读缓存的 `major` / `arch`，不执行 Java：版本相等优先，其次最接近的
更高版本，再其次最高的更低版本；同一档内优先当前平台架构。

`find_java` 只返回扫描结果；是否写回配置由调用方决定。

### JRE 安装

`install_jre` 是唯一的公共安装入口；它接收调用方共享的 `DownloadContext`，因此 JRE 文件
与安装 / 启动图里的其它下载共用连接、文件、worker 预算与全局进度。`JreInstallRequest`
强制要求 `major`，`component` 可选：

- `None`：按 `major` 在所有组件里选，优先非 snapshot；没有匹配直接报错；
- `Some(name)`：只检查该组件，它必须实际提供 `major`，否则直接报错。

两种情况都没有 `jre-legacy` 兜底。目标目录固定为
`<runtime_root>/<组件名>/<平台键>/<组件名>`。

安装按增量缓存处理：已经完整（sha1 / 大小一致）的文件跳过，缺的、坏的重新下载，目录里
其它文件不动。默认用 lzma，没有 lzma 版的文件回退 `raw`；失败时只删除入口
（`bin/java` / `bin/javaw`，macOS 在 `jre.bundle/Contents/Home/bin/` 下）。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 元数据与平台信息：[crates/dmcl-mcmeta/README.md](../dmcl-mcmeta/README.md)
- 并发下载与校验：[crates/dmcl-download/README.md](../dmcl-download/README.md)
