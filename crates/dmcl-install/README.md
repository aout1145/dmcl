# dmcl-install

在磁盘上装出一个可玩的 Minecraft 版本：原版、Fabric、Forge、NeoForge、OptiFine。
它只负责「创建版本目录、写元数据、安装加载器」这些安装器才负责的事；版本文件规划、
完整性检查、补全与 natives 在 `dmcl-game`，Java 解析与 JRE 安装在 `dmcl-java`。

在 workspace 内依赖 `dmcl-game`、`dmcl-java`、`dmcl-mcmeta`、`dmcl-download` 和
`dmcl-task`。配置（根目录、下载策略、Java 解析器、JRE 安装位置）由调用者通过
`InstallOptions` 传入，**这个 crate 不读任何配置文件**。

## 模块布局

```text
vanilla.rs    原版元数据落盘、launcher_profiles.json
loader/       fabric / forge / neoforge / optifine / xforge（Forge 与 NeoForge 共用工具）
spec.rs       顶层任务图 install_task、InstallSpec / InstallOutcome / LoaderKind
options.rs    InstallOptions
archive.rs    读安装器 jar / zip 内的条目
io.rs         安全写盘（先建父目录）
```

## 设计意图

### 版本级原子安装

`install_task` 先校验并占用 `versions/<version_name>`，目录已存在就直接报错，不碰已有版本。
占用成功后才登记回滚，之后任何一步失败或被取消都会删掉本次新建的版本目录，不让半装版本
出现在版本列表里。共享的 `libraries/` / `assets/` 是内容校验过的缓存，不跟着回滚。

### 任务图按阶段组织

安装图按「获取元数据 → 下载游戏文件 → 安装加载器 → 完成」组织，节点之间传递类型明确的
中间值，而不是反复从磁盘猜状态。版本 jar、资源与原版依赖库都只依赖原版版本 JSON，可以并行；
加载器安装也不读写版本 JSON，只产出 patch，最后统一应用并写盘。客户端 jar 必须在
Forge / NeoForge 的 processor 之前就位，processor 节点会显式依赖它。

### 加载器安装统一为 patch

三种加载器各有自己的安装步骤，但都只**返回 patch**，不直接改写版本 JSON。Forge 与 NeoForge
的安装器档案分别建模；Forge 同时兼容现代式与经典式安装器，NeoForge 只有现代式。需要跑
processor 时，由安装图按需选择 Java，JRE 下载复用同一份下载上下文与全局进度。没有加载器
的原版安装也会在写盘前把原版文档记为 `patches[0]`，让识别流程能直接读到版本 id。

### OptiFine

OptiFine 是叠加在原版或 Forge 上的 tweaker，拆成「下载」与「安装」两个节点：下载只依赖
版本列表，可以和其它下载并行；安装生成 patch，负责 tweaker、launchwrapper 与主类调整。
是否兼容由调用方决定，`dmcl-install` 不检查组合，只按调用方给的加载器顺序生成并应用 patch。

### 安装参数

`InstallOptions` 包含根目录、游戏文件选项、Java 解析器与 JRE 自动安装策略。安装器 processor
需要 Java 时先让解析器找现有 Java，找不到再按版本要求下载 JRE；JRE 的组件选择、下载与
展开在 `dmcl-java`，`dmcl-install` 只知道装到哪个 runtime 根目录。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 版本规划、完整性与 natives：[crates/dmcl-game/README.md](../dmcl-game/README.md)
- Java / JRE：[crates/dmcl-java/README.md](../dmcl-java/README.md)
- 元数据、URL 模板与下载源：[crates/dmcl-mcmeta/README.md](../dmcl-mcmeta/README.md)
- 并发下载与校验：[crates/dmcl-download/README.md](../dmcl-download/README.md)
- 启动流程：[crates/dmcl-launch/README.md](../dmcl-launch/README.md)
