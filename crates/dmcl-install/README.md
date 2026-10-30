# dmcl-install

在磁盘上装出一个可玩的 Minecraft 版本：原版、Fabric、Forge、NeoForge、OptiFine，以及
版本完整性检查。它只做「把文件铺好」的事（写入 + 校验），不做任何决策。

在 workspace 内依赖 `dmcl-mcmeta`（元数据与下载源）、`dmcl-download`（并发下载）和
`dmcl-task`（任务链与进度）。配置（根目录、镜像、下载策略、是否跳过音乐、java 提供者）
由调用者通过 `InstallOptions` 传入，**这个 crate 不读任何配置文件**。

## 模块布局

```text
plan.rs       纯函数规划：这个版本需要哪些文件
files.rs      按规划构造下载任务并落盘；natives 下载 + 解压
integrity.rs  完整性检查：完整吗？缺什么？
vanilla.rs    原版元数据落盘、launcher_profiles.json
loader/       fabric / forge / neoforge / optifine / common / xforge（共用工具）
spec.rs       顶层任务链 install_task、InstallSpec / LoaderKind
options.rs    InstallOptions、JavaProvider
zip.rs        jar / zip 读取与解压
io.rs         安全写盘（先建父目录）
```

## 设计要点

### 规划与下载分离

「这个版本由哪些文件组成」是**纯函数**（`plan`）：输入版本 JSON + `PlanContext`，输出
`Vec<PlannedFile>`（目标路径 + URL 模板 + 校验值 + 大小）。下载那一半（`files`）只管按
规划落盘。好处：

- 规划可以离线单测（喂真实版本 JSON，断言结果）；
- `dmcl-launch` 的完整性检查**复用同一批纯函数**，两边永远不会对不上。

`PlannedFile` 里的 url 仍然是模板，直到 `into_item` 才按 `DownloadSource` 的镜像策略
展开成候选地址。

### 版本级原子安装

`install_task` 的顺序是：占用 `versions/<version_name>` → 原版元数据 → 客户端 jar →
各加载器 → 游戏文件。第一步目录已存在就直接报错（不会碰已有版本）；只有这一步成功后
才走到 `on_rollback` 登记删除动作，之后任何一步失败或被取消，都会删掉本次新建的版本
目录，不让半装版本出现在版本列表里。共享的 `libraries/` / `assets/` 是内容校验过的缓存，
不跟着回滚（下次安装会跳过已有文件）。

客户端 jar 必须先于加载器就位，因为 Forge / NeoForge 的 processor 要读它来生成
patched client（`{MINECRAFT_JAR}`）。

### natives

natives 的**下载 + 解压**单独放进 `files::natives_task`，不在 `install_task` 里：要下哪个
分类器（版本 JSON 里的 `natives-linux` / `natives-linux-64` 之类）取决于运行游戏的 JVM
架构，只有启动时选好 java 才知道。解压目录由 `dmcl_mcmeta::layout::natives_dir` 决定，
形如 `<临时目录>/dmcl-natives-<os>-<arch>-<版本名哈希>`（例如
`dmcl-natives-linux-x86_64-…`），换平台或换 JVM 架构时不会混用。因此完整性检查也
**不覆盖 natives**，由启动流程自愈。

### 完整性检查

`check_integrity(plan, version_name)` 返回
`IntegrityStatus { complete, missing, checked }`，每个缺失项带目标路径 + URL 模板 +
校验值 + 原因（缺失 / 内容不符）；它**不检查 natives**。

它**不下载任何东西**：修复由调用者跑 `game_files_task`（已存在的文件会被跳过）。
「哪些文件应该有」用 `plan` 的纯函数，「本地这个文件好不好」用 `dmcl_download::file_ok`
（与下载的跳过同一判定），并发调度交给 `dmcl_download::files_ok`。

```rust
// 完整性检查在 dmcl-install：只回答"完整吗？缺什么？"，不下载任何东西
let status = dmcl_install::check_integrity(&install_options.plan(), version).await?;
if !status.is_complete() {
    // 由调用者决定：跑一次 dmcl-install 补全（已存在的文件会被跳过）
    game_files_task(install_options, version)
        .run_with((), |event| printer.handle(event))
        .await?;
}
```

### 加载器安装

三种加载器各有自己的安装步骤（`loader/`），Forge 与 NeoForge 的 `InstallProfile` 也各有
一份（`dmcl-mcmeta/{forge,neoforge}/install_profile.rs`），便于以后各自演化：

- **Fabric** —— 取 loader profile，合并进原版版本 JSON；
- **Forge** —— 安装器分现代式（spec 0/1，包内有 `version.json`，`json` 字段指出位置）
  与经典式（≤1.11.2，profile 内联在 `versionInfo` 里、库在 `install.filePath` 指向的内嵌
  jar 中）两种；跑 processor 生成客户端 jar；
- **NeoForge** —— 只有现代式（spec 1）；**别给它加 Forge 的经典式**（`install` /
  `versionInfo`）分支。

`read_installer_version_json` 的查找顺序：`json` 指向的条目 →（仅 Forge）内联
`versionInfo` →（兜底）`install.filePath` 指向的内嵌 jar。

加载器版本由调用者给定（`LoaderRequest.version`），`dmcl-install` 不查最新；查询版本
列表在 `dmcl-mcmeta`。

### OptiFine

OptiFine 不是独立加载器，而是叠加在原版或 Forge 上的 tweaker（`loader/optifine.rs`）：

1. 按调用者给的 OptiFine 版本，从 bmclapi 的列表里找到 `type` / `patch` 并下载 jar；
2. 把 jar 放到 `libraries/optifine/OptiFine/<mc>_<type>_<patch>/`；
3. **没有 Forge** 时，从 jar 的 `launchwrapper-of.txt`（或 `launchwrapper-of-*.jar` 条目名）
   读出版本，把内嵌的 launchwrapper 解到 `libraries/net/minecraft/launchwrapper/of-<version>/`；
4. 版本 JSON 里加 `optifine:OptiFine:…`（以及没有 Forge 时的 `net.minecraft:launchwrapper:of-…`）；
5. **没有 Forge** 时把 `mainClass` 改成 `net.minecraft.launchwrapper.Launch` 并追加
   `--tweakClass optifine.OptiFineTweaker`；**有 Forge** 时保持 Forge 的 `mainClass` 与
   launchwrapper 不动，只追加 `--tweakClass optifine.OptiFineForgeTweaker`。

是否兼容由调用方决定，`dmcl-install` 不检查组合；它只按 `LoaderRequest` 的顺序安装，
不调整顺序。CLI 与其它加载器一样不做校验（加载器顺序固定为 Forge → NeoForge → Fabric →
OptiFine），GUI 用冲突组限制选择。参数追加到 `arguments.game`；老版本没有 `arguments`
时追加到 `minecraftArguments` 字符串末尾。

### 安装参数

`InstallOptions` 包含根目录、`DownloadSource`、`DownloadOptions`、`skip_music` 和
`Arc<dyn JavaProvider>`。`JavaProvider` 只为 Forge / NeoForge 的安装器 processor 提供
java（默认 `PathJava` 从 `JAVA_HOME` 或 `PATH` 找），与启动时选游戏 java 是两件事。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 元数据、URL 模板与下载源：[crates/dmcl-mcmeta/README.md](../dmcl-mcmeta/README.md)
- 并发下载与校验：[crates/dmcl-download/README.md](../dmcl-download/README.md)
- 启动时如何复用完整性检查：[crates/dmcl-launch/README.md](../dmcl-launch/README.md)
