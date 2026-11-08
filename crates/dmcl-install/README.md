# dmcl-install

在磁盘上装出一个可玩的 Minecraft 版本：原版、Fabric、Forge、NeoForge、OptiFine。
它只做「创建版本目录、写元数据、安装加载器」这些安装器才负责的事；版本文件规划、
完整性检查、补全与 natives 在 `dmcl-game`，Java 解析与 JRE 安装在 `dmcl-java`。

在 workspace 内依赖 `dmcl-game`（文件物化）、`dmcl-java`（Java / JRE）、`dmcl-mcmeta`
（元数据与下载源）、`dmcl-download`（单文件下载与并发预算）和 `dmcl-task`（任务图与
Step / 全局进度）。配置
（根目录、下载策略、Java 解析器、JRE 安装位置）由调用者通过 `InstallOptions` 传入，
**这个 crate 不读任何配置文件**。

## 模块布局

```text
vanilla.rs    原版元数据落盘、launcher_profiles.json
loader/       fabric / forge / neoforge / optifine / xforge（Forge 与 NeoForge 共用工具）
spec.rs       顶层任务图 install_task、InstallSpec / InstallOutcome / LoaderKind
options.rs    InstallOptions
archive.rs    读安装器 jar / zip 内的条目
io.rs         安全写盘（先建父目录）
```

## 设计要点

### 版本级原子安装

`install_task` 先占用 `versions/<version_name>`；目录已存在就直接报错（不会碰已有版本）。
名称在这一步先经过 `validate_version_name` 校验：基于 `sanitize-filename` 按 Windows
规则检查非法字符、控制字符、保留设备名、结尾的点 / 空格与长度，不合法就直接失败，避免
拼接路径时逃出 `versions/`。只有占用成功后才在节点里用 `ctx.on_rollback` 登记删除动作，
之后任何一步失败或被取消，都会删掉本次新建的版本目录，不让半装版本出现在版本列表里。
共享的 `libraries/` / `assets/` 是内容校验过的缓存，不跟着回滚（下次安装会跳过已有文件）。

显示步骤按「用户的直觉」分三段，加载器各自再占一个 `安装 X`，最后统一收尾：

```text
获取版本元数据   创建版本目录、获取版本 json
下载游戏文件     下载版本 jar、下载资源、下载原版依赖库
安装 X           下载安装器、解析安装器、解出安装器库、下载加载器库 / 安装器依赖、运行安装器、生成 patch
完成            应用所有 patch、写入版本 JSON
```

安装图的节点传递类型明确的中间值，而不是反复从磁盘猜状态：

```text
创建版本目录          Task<(), VersionRef>
获取并解析原版元数据   Task<Arc<VersionRef>, ResolvedVersion>
下载版本 jar / 资源 / 原版库  Task<Arc<ResolvedVersion>, ()>
下载 OptiFine         Task<(), OptifineFiles>
每个加载器            Task<…, VersionJsonPatch>
完成                  Task<(Arc<ResolvedVersion>, Vec<Arc<VersionJsonPatch>>, …), InstallOutcome>
```

版本 jar、资源与原版依赖库都只依赖原版版本 JSON，所以能在「下载游戏文件」里并行开始；
OptiFine 的 jar 同样只依赖版本列表，单独成一个节点先下。加载器步骤下载自己的依赖库，
并把改动描述成 `VersionJsonPatch`，**不读写版本 JSON**；所有 patch 由最后的 `完成`
节点按调用方给定的顺序统一应用并写盘。

加载器的 patch 数量由调用方决定，`完成` 节点用 `dmcl-task` 的 `Vec<Handle<_>>` 依赖
收集这一组 patch，不需要为了拼固定长度的 tuple 再加中间步骤。

客户端 jar 必须先于 Forge / NeoForge 的 processor 就位，因为 processor 要读它来生成
patched client（`{MINECRAFT_JAR}`）；因此 processor 节点会额外依赖「下载版本 jar」。
加载器库和原版库同时下载；不同步骤规划到同一个目标路径时，由下载层串行化并让后到者
跳过。Forge / NeoForge 的 processor 还要用原版库，所以「运行安装器」额外依赖
「下载依赖库」。

### 规划、完整性与 natives 在 dmcl-game

「这个版本由哪些文件组成」「本地缺了什么」「怎么补齐」「natives 按 os / arch 下载并
解压」都由 `dmcl-game` 提供。`dmcl-install` 与 `dmcl-launch` 使用同一套实现，不会对不上。

### 加载器安装

三种加载器各有自己的安装步骤（`loader/`），Forge 与 NeoForge 的 `InstallProfile` 也各有
一份（`dmcl-mcmeta/{forge,neoforge}/install_profile.rs`），便于以后各自演化：

所有加载器都只**返回 patch**，不读写版本 JSON：

- **Fabric** —— 拆成「获取 Fabric 元数据 → 下载 Fabric 依赖库 → 生成 patch」三步；
  patch 带主类 / 启动参数；
- **Forge** —— 安装器分现代式（spec 0/1，包内有 `version.json`，`json` 字段指出位置）
  与经典式（≤1.11.2，profile 内联在 `versionInfo` 里、库在 `install.filePath` 指向的内嵌
  jar 中）两种；跑 processor 生成客户端 jar；
- **NeoForge** —— 只有现代式（spec 1）；**别给它加 Forge 的经典式**（`install` /
  `versionInfo`）分支。

Forge / NeoForge 的安装拆成「下载安装器 → 解析安装器 → 解出安装器库 / 下载加载器依赖 /
下载安装器依赖 → 运行安装器 → 生成 patch」几个节点：

- 「下载安装器」只依赖版本目录，可以和元数据 / 资源下载并行；
- 「下载加载器依赖」下载安装器版本 JSON 里的库（Forge / NeoForge 运行时库）；
- 「下载安装器依赖」下载安装器档案声明的库与可被下载顶替的 `DOWNLOAD_MOJMAPS`；
- 两组依赖分开下载，并都可以和原版库同时下载；同一个目标路径由下载层串行化，不会并发
  写同一个 `.part`；
- 「运行安装器」只等原版库、安装器依赖与客户端 jar；加载器运行时库只要在「生成 patch」
  之前下好，不影响 processor；
- 经典式 Forge 安装器没有 processor，仍会下载两组库，只是跳过跑 Java；
- 确实需要跑 processor 时，「下载安装器依赖」在当前 `安装 X` Step 下动态创建
  `选择 Java` 节点，JRE 下载也归在这个 Step；
- 其中 `DOWNLOAD_MOJMAPS` 只是把官方映射文件下载到指定位置，可以直接用自己的下载器
  完成并跳过这个 processor；其余 processor 仍交给 Java 执行。

经典式 Forge 的 `minecraftArguments` **不会**原样沿用：只解析其中的
`--tweakClass <class>` 放进 patch 的 `tweakers`，其余启动参数仍来自原版版本 JSON。
现代式的 `arguments` 会作为 patch 的一部分追加到最终版本参数末尾。

`read_installer_version_json` 的查找顺序：`json` 指向的条目 →（仅 Forge）内联
`versionInfo` →（兜底）`install.filePath` 指向的内嵌 jar。

加载器版本由调用者给定（`LoaderRequest.version`），`dmcl-install` 不查最新；查询版本
列表在 `dmcl-mcmeta`。

### OptiFine

OptiFine 不是独立加载器，而是叠加在原版或 Forge 上的 tweaker（`loader/optifine.rs`），
拆成「下载 OptiFine」与「安装 OptiFine」两个节点：

1. 「下载 OptiFine」按调用者给的 OptiFine 版本，从 bmclapi 的列表里找到 `type` / `patch`
   并下载 jar；这一步只依赖版本列表，可以和元数据 / 版本 jar / 资源 / Forge 安装并行；
2. 把 jar 放到 `libraries/optifine/OptiFine/<mc>_<type>_<patch>/`；
3. **没有 Forge** 时，从 jar 的 `launchwrapper-of.txt`（或 `launchwrapper-of-*.jar` 条目名）
   读出版本，把内嵌的 launchwrapper 解到 `libraries/net/minecraft/launchwrapper/of-<version>/`；
4. 「安装 OptiFine」生成 patch：加 `optifine:OptiFine:…`（以及没有 Forge 时的
   `net.minecraft:launchwrapper:of-…`）；
5. **没有 Forge** 时 patch 把 `mainClass` 改成 `net.minecraft.launchwrapper.Launch` 并
   追加 `--tweakClass optifine.OptiFineTweaker`；**有 Forge** 时 patch 不设 `mainClass`，
   只追加 `--tweakClass optifine.OptiFineForgeTweaker`。

是否兼容由调用方决定，`dmcl-install` 不检查组合；它只按 `LoaderRequest` 的顺序生成并
应用 patch，不调整顺序。CLI 与其它加载器一样不做校验（加载器顺序固定为 Forge →
NeoForge → Fabric → OptiFine），GUI 用冲突组限制选择。tweaker 追加到最终版本的
`arguments.game`；老版本没有 `arguments` 时追加到 `minecraftArguments` 字符串末尾。

### 安装参数

`InstallOptions` 包含根目录、`GameFilesOptions`、`Arc<dyn JavaResolver>` 和恒有的
`JreAutoInstall`。安装器 processor 需要 Java 时调用
`InstallOptions::java_for_or_install`：先让解析器找现有 Java，找不到再按版本要求下载
JRE；下载复用安装图共享的 `DownloadContext` 与全局进度。

JRE 的组件选择、下载与展开在 `dmcl-java`；`dmcl-install` 只知道「装到哪个 runtime 根目录」。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 版本规划、完整性与 natives：[crates/dmcl-game/README.md](../dmcl-game/README.md)
- Java / JRE：[crates/dmcl-java/README.md](../dmcl-java/README.md)
- 元数据、URL 模板与下载源：[crates/dmcl-mcmeta/README.md](../dmcl-mcmeta/README.md)
- 并发下载与校验：[crates/dmcl-download/README.md](../dmcl-download/README.md)
- 启动流程：[crates/dmcl-launch/README.md](../dmcl-launch/README.md)
