# dmcl-mcmeta

集中描述「游戏是什么」：Minecraft 的强类型元数据文档、下载源与 URL 模板、本地目录布局。
它**不把文件写到磁盘上**，也不读配置文件；根目录、镜像、并发度等一律由调用者传入。

`dmcl-mcmeta` 是 workspace 最底层的叶子 crate，不依赖其他成员 crate。外部依赖主要是
`serde` / `serde_json` / `reqwest` / `tokio`。

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

## 设计要点

### 强类型文档

版本 JSON、版本清单、资源索引、安装器档案、模组加载器版本列表都是强类型结构体。
会被写回的文档必须能往返：

- 会被写回的结构体都带 `#[serde(flatten)] extra: JsonMap`，未知字段原样保留；
- 「已知取值 + 兜底」的字符串枚举用 `string_enum!` 宏，且**必须保留 `Other(String)`**
  —— 否则回写时会丢掉不认识的取值；
- 可选字段用 `#[serde(default, skip_serializing_if = "Option::is_none")]`，别写出 `null`。

### 版本合并与 patch

`VersionJson::merge` 把子版本合并进父版本：

- `minecraftArguments`、`mainClass` 等可选字段由子版本的 `Some` 覆盖，`None` 保留父版本；
- `arguments` 按 `jvm` / `game` / `default-user-jvm` 分别**父在前、子在后**追加；
- `libraries` 子版本在前、父版本追加在后；同库键 + 同 `rules` 时版本号高者胜出，
  版本号相同或无法比较时保留子版本；
- `rules` 是库身份的一部分，不拼接数组：同键但 `rules` 不同时两个平台变体都保留；
- 未建模的顶层字段（`extra`）继续按对象递归、数组拼接、标量覆盖的通用规则合并；
- `patches` 不参与合并，结果直接采用子版本的值。

库键是 `groupId:artifactId[:classifier]`，忽略版本和 `@ext`；`None` 与 `Some([])` 视为
同一种 `rules`。

#### patch

`VersionJsonPatch` 描述要改什么：`libraries`、`tweakers`、可选的 `mainClass` /
`arguments`，以及只用于显示的 `type` / `releaseTime`。对当前版本依次应用 patch：

- 库的合并顺序与普通合并相同；和原版库同键 + 同 `rules` 时仍按版本号比较，
  和之前应用过的 patch 同键 + 同 `rules` 时由后应用的 patch **无条件覆盖**；
- `mainClass` 有值就替换；
- `arguments` 按 `jvm` / `game` / `default-user-jvm` 分别追加到当前版本的参数末尾；
  当前版本只有 `minecraftArguments` 时，把 patch 参数转成字符串 token 接到末尾；
- 每个 tweaker 写成 `--tweakClass <tweaker>`：先移除已有的同名参数对，再追加到游戏参数末尾
  （有 `arguments` 时写 `arguments.game`，否则写 `minecraftArguments`）；
- `type` / `releaseTime` 只保留在 patch 历史里供显示，不覆盖版本 JSON 的对应字段；
- 第一次应用 patch 时把未修改的 JSON 放进 `patches[0]`，之后每个 patch 依次追加。

`patches[0]` 是原版快照，`patches[1..]` 是应用过的 patch，可用于显示或回退。

#### 多级 `inheritsFrom`

`read_resolved_version_json` 沿 `inheritsFrom` 解析整条链，顺序是**从根祖先开始，依次
merge 到叶子**，因此 `optifine -> forge -> vanilla` 会保留每一层的库、参数和下载信息。
解析是迭代的，出现循环会报错。完成后清空 `inheritsFrom`；合并结果没有显式 `jar` 时，
把它设成根祖先（没有父版本时保持为空）。

### URL 模板与下载源

从 Minecraft 拿到的每一份文档里，所有 url 都会被**归一成模板**：

```text
https://libraries.minecraft.net/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
        ↓
${MC_MAVEN}/net/sf/jopt-simple/…/jopt-simple-5.0.4.jar
```

`${MC_MAVEN}` 是一个「下载根」（`Root`），真正的地址由 `Mirror` 决定：

```rust
let source = DownloadSource::from_policy(MirrorPolicy::MirrorFirst);
source.candidates("${MC_MAVEN}/a/b.jar");
// → ["https://bmclapi2.bangbang93.com/maven/a/b.jar",
//    "https://libraries.minecraft.net/a/b.jar"]
```

要点：

- 归一化选**最长前缀**匹配（否则 bmclapi 的站点根会把 `/maven` 之类的更具体的根吞掉）；
- 一个根可以有**多个等价官域**（Forge 的 `files.minecraftforge.net/maven` 与
  `maven.minecraftforge.net` 是同一个仓库，都归一成 `${FORGE_MAVEN}`）；
- 一个镜像可以只覆盖**部分根**：`Root::mirror_bases` 返回 `None` 表示该镜像不镜像这个
  根。例如 ForgeCDN（`neoforged.forgecdn.net/releases`）只映射 `${NEOFORGE_MAVEN}`；
  展开候选地址时会跳过不覆盖该根的镜像，所以部分镜像不会把官方地址提前到其他镜像前面；
- 识别不出的 host 原样保留 = 不可镜像；Fabric loader 列表里 `launcherMeta` 的库地址
  同样参与归一化，Maven Central 之类的第三方仓库保持原样；
- `UrlFields` trait 统一遍历文档里的 url 字段，`templatize_urls` / `resolve_official_urls`
  对任何文档类型都通用，新增字段不会漏改；下载地址统一走 `resolve_url_candidates`，
  写盘时只解析回官方地址；
- 用哪种镜像由 `MirrorPolicy` 决定：`official-only`（仅官方）、`official-first`
  （官方优先，默认）、`mirror-first`（镜像优先，顺序是 bmclapi → ForgeCDN → 官方）；
  各域取 meta 统一走 `fetch_meta(url)`
  —— 内部按镜像优先级展开候选、轮换重试（A B A …）直到用满重试次数；重试前按指数退避，
  429 等限流响应优先遵循 `Retry-After`（bmclapi 的 `x-ratelimit-reset-short` 作为兜底）；
  `fetch_json` 就是它的 JSON 版：
  `source.fetch_json("${MC_META}/mc/game/version_manifest.json").await?`。

写回磁盘的版本 JSON 会把模板解析回**官方**地址，因此 `.minecraft` 里的文档与官方启动器
完全通用。

### rules 判定

`vanilla/rules.rs` 按**标准 Mojang 语义**判定：空规则允许；否则按顺序评估、**最后一条
命中的规则的 action 决定结果**；一条都不命中则不允许。条件涵盖 `os.name` / `os.arch` /
`os.version`（正则）/ `os.versionRange`（区间）/ `features`。

### Java 运行时

`vanilla/jre.rs` 描述 Mojang 的两层 JRE 文档：

- `JreManifest` 是产品索引 `all.json`：`平台 → 组件（java-runtime-gamma …）→ 运行时条目`。
  顶层平台键翻译成 mcmeta 的 `(Os, Arch)`（`windows-x64` → `(Windows, X86_64)`、
  `linux-i386` → `(Linux, X86)` …），`gamecore` 这类非桌面平台直接忽略；Mojang 没有提供的
  组合（如 Linux arm64）自然查不到。`current_jre_platform()` 给出本机对应的键。
- 每个条目指向一份运行时文件清单 `manifest.json`，由 `JreRuntimeManifest` 描述：按相对
  路径列出目录 / 文件 / 链接，文件带 `raw` 与 `lzma` 两份下载信息和可执行位。
- `DownloadSource::fetch_jre_manifest` / `fetch_jre_runtime_manifest` 负责取数，并把清单
  地址与文件下载地址归一成 `${MC_META}` / `${MC_DATA}` 模板。

### 加载器版本列表

Fabric / Forge / NeoForge 的 `fetch_*` 统一按版本**从新到旧**返回：Fabric / NeoForge
比版本字符串，Forge 比 `build` 号；因此「最新」就是列表第一条，调用方不再需要单独的
`latest_*` 取数函数。

OptiFine 的列表来自 bmclapi 的 `/optifine/<mcversion>`（**不可镜像**，直接写 bmclapi
地址）。`fetch_optifine_versions` 把稳定版排在前面、同主版本的预览版按 `preN` 倒序，
并用 `OptiFineVersionEntry::latest` 选最新稳定版（没有稳定版才回退最新预览版）。
OptiFine 下载地址是 `/optifine/<mcversion>/<type>/<patch>`，同样固定走 bmclapi。

### 本地目录布局

`layout.rs` 约定 `versions/<name>/<name>.json`、`libraries/`、`assets/` 等路径，以及
natives 解压目录（`<临时目录>/dmcl-natives-<os>-<arch>-<版本名哈希>`），并提供读取版本
JSON（含递归 `inheritsFrom` 合并）的入口。这里只算路径、读文档，不写盘；真正的落盘在
`dmcl-install`。

## 相关文档

- 总体结构与依赖关系：[ARCHITECTURE.md](../../ARCHITECTURE.md)
- 用这些元数据规划并落盘：[crates/dmcl-install/README.md](../dmcl-install/README.md)
- 下载任务如何消费候选地址：[crates/dmcl-download/README.md](../dmcl-download/README.md)
