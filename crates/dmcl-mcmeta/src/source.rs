//! 下载源：镜像源枚举、URL 模板、以及"按模板发 http 请求"的客户端。
//!
//! # URL 模板
//!
//! 所有从 Minecraft 文档里读出来的链接都会被**归一成模板**，例如：
//!
//! ```text
//! https://libraries.minecraft.net/net/sf/jopt-simple/jopt-simple/5.0.4/jopt-simple-5.0.4.jar
//! ${MC_MAVEN}/net/sf/jopt-simple/jopt-simple/5.0.4/jopt-simple-5.0.4.jar
//! ```
//!
//! `${MC_MAVEN}` 叫一个 [`Root`]（下载根）。真正下载时由 [`Root::base`] 按当前
//! [`Mirror`] 决定换成官方地址还是镜像地址，于是"下什么"和"从哪儿下"彻底分开：
//! 安装流程只跟模板打交道，换镜像不需要改一行安装逻辑。
//!
//! 识别不出来的地址（第三方仓库等）保持原样，也就是**不可镜像**。

use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 默认 User-Agent，未显式设置时使用。
pub const DEFAULT_USER_AGENT: &str = concat!("dmcl/", env!("CARGO_PKG_VERSION"));

// ---------------------------------------------------------------------------
// 镜像源
// ---------------------------------------------------------------------------

/// 使用哪个镜像源。
///
/// 类型化之后，`--mirror` 的候选值由 clap 生成，代码里不再到处拿字符串比较。
///
/// `dmcl.json` 里写错的值会**直接报错**（见 `FromStr`），不会静默退化。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Mirror {
    #[default]
    Official,
    Bmclapi,
}

impl Mirror {
    /// 全部镜像，按"官方优先"排列。
    pub const ALL: [Mirror; 2] = [Mirror::Official, Mirror::Bmclapi];

    /// 配置文件与命令行里使用的名字。
    pub fn as_str(self) -> &'static str {
        match self {
            Mirror::Official => "official",
            Mirror::Bmclapi => "bmclapi",
        }
    }
}

impl fmt::Display for Mirror {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Mirror {
    type Err = anyhow::Error;

    /// 不认识的值直接报错（配置文件写错时能立刻发现）。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "official" => Ok(Mirror::Official),
            "bmclapi" => Ok(Mirror::Bmclapi),
            other => Err(anyhow!(
                "不认识的镜像源：{other}（可选 official / bmclapi）"
            )),
        }
    }
}

impl Serialize for Mirror {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Mirror {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// 下载根（URL 模板里的占位符）
// ---------------------------------------------------------------------------

/// URL 模板里的一个"根"。每个根在各镜像下对应一个（或几个等价的）base url。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Root {
    /// Mojang 的版本元数据（`piston-meta`）。
    McMeta,
    /// Mojang 的版本文件（`piston-data`，新版客户端 jar 在这里）。
    McData,
    /// 旧的 `launcher.mojang.com` 资源。
    McLauncher,
    /// 官方库仓库（libraries）。
    McMaven,
    /// 资源文件（assets objects）。
    McAssets,
    /// Forge 的 maven 仓库。
    ForgeMaven,
    /// NeoForge 的 maven 仓库。
    NeoForgeMaven,
    /// Fabric 的元数据接口。
    FabricMeta,
    /// Fabric 的 maven 仓库。
    FabricMaven,
}

impl Root {
    /// 全部下载根。
    pub const ALL: [Root; 9] = [
        Root::McMeta,
        Root::McData,
        Root::McLauncher,
        Root::McMaven,
        Root::McAssets,
        Root::ForgeMaven,
        Root::NeoForgeMaven,
        Root::FabricMeta,
        Root::FabricMaven,
    ];

    /// 模板里的占位符，例如 `${MC_MAVEN}`。
    pub fn placeholder(self) -> &'static str {
        match self {
            Root::McMeta => "${MC_META}",
            Root::McData => "${MC_DATA}",
            Root::McLauncher => "${MC_LAUNCHER}",
            Root::McMaven => "${MC_MAVEN}",
            Root::McAssets => "${MC_ASSETS}",
            Root::ForgeMaven => "${FORGE_MAVEN}",
            Root::NeoForgeMaven => "${NEOFORGE_MAVEN}",
            Root::FabricMeta => "${FABRIC_META}",
            Root::FabricMaven => "${FABRIC_MAVEN}",
        }
    }

    /// 官方镜像下的 base url。
    ///
    /// 返回切片是因为同一个仓库可能有多个等价域名（Forge 的
    /// `files.minecraftforge.net/maven` 与 `maven.minecraftforge.net` 就是同一个仓库）。
    pub fn official_bases(self) -> &'static [&'static str] {
        match self {
            Root::McMeta => &["https://piston-meta.mojang.com"],
            Root::McData => &["https://piston-data.mojang.com"],
            Root::McLauncher => &["https://launcher.mojang.com"],
            Root::McMaven => &["https://libraries.minecraft.net"],
            Root::McAssets => &["https://resources.download.minecraft.net"],
            Root::ForgeMaven => &[
                "https://files.minecraftforge.net/maven",
                "https://maven.minecraftforge.net",
            ],
            Root::NeoForgeMaven => &["https://maven.neoforged.net/releases"],
            Root::FabricMeta => &["https://meta.fabricmc.net"],
            Root::FabricMaven => &["https://maven.fabricmc.net"],
        }
    }

    /// `mirror` 下该根的 base url；该镜像没有对应地址时回落到官方地址
    /// （也就是"这个根不被镜像"）。
    pub fn bases(self, mirror: Mirror) -> &'static [&'static str] {
        match mirror {
            Mirror::Official => self.official_bases(),
            Mirror::Bmclapi => match self {
                Root::McMeta => &["https://bmclapi2.bangbang93.com"],
                Root::McMaven => &["https://bmclapi2.bangbang93.com/maven"],
                Root::McAssets => &["https://bmclapi2.bangbang93.com/assets"],
                Root::ForgeMaven => &["https://bmclapi2.bangbang93.com/maven"],
                Root::NeoForgeMaven => &["https://bmclapi2.bangbang93.com/maven"],
                Root::FabricMeta => &["https://bmclapi2.bangbang93.com/fabric-meta"],
                Root::FabricMaven => &["https://bmclapi2.bangbang93.com/maven"],
                // bmclapi 不镜像这两个根（不改写）
                Root::McData | Root::McLauncher => self.official_bases(),
            },
        }
    }

    /// `mirror` 下的主 base url。
    pub fn base(self, mirror: Mirror) -> &'static str {
        self.bases(mirror).first().copied().unwrap_or_default()
    }

    /// 由占位符反查根，例如 `"${MC_MAVEN}"` → [`Root::McMaven`]。
    pub fn from_placeholder(text: &str) -> Option<Root> {
        Root::ALL
            .into_iter()
            .find(|root| root.placeholder() == text)
    }
}

// ---------------------------------------------------------------------------
// 模板 <-> 真实 url
// ---------------------------------------------------------------------------

/// `url` 是否含有模板占位符。
pub fn is_templated(url: &str) -> bool {
    url.contains("${")
}

/// 把真实 url 归一成模板；识别不出 host 时原样返回（即"不可镜像"）。
///
/// 选择**最长**的匹配前缀，否则 `${MC_META}`（bmclapi 下就是站点根）会把
/// `${MC_MAVEN}` 之类的更具体的根吞掉。
pub fn templatize_url(url: &str) -> String {
    let mut best: Option<(usize, &'static str)> = None;
    for root in Root::ALL {
        for mirror in Mirror::ALL {
            for base in root.bases(mirror) {
                if !base.is_empty() && url.starts_with(base) {
                    let better = match best {
                        None => true,
                        Some((len, _)) => base.len() > len,
                    };
                    if better {
                        best = Some((base.len(), root.placeholder()));
                    }
                }
            }
        }
    }
    match best {
        Some((len, placeholder)) => format!("{placeholder}{}", &url[len..]),
        None => url.to_string(),
    }
}

/// 把模板里的占位符换成 `mirror` 下的真实 base。
pub fn resolve_url(url: &str, mirror: Mirror) -> String {
    let mut out = url.to_string();
    if !is_templated(&out) {
        return out;
    }
    for root in Root::ALL {
        let placeholder = root.placeholder();
        if out.contains(placeholder) {
            out = out.replace(placeholder, root.base(mirror));
        }
    }
    out
}

/// 按镜像优先级把模板展开成候选 url（保序去重）。
pub fn resolve_url_candidates(url: &str, mirrors: &[Mirror]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for mirror in mirrors {
        let resolved = resolve_url(url, *mirror);
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    if out.is_empty() {
        out.push(url.to_string());
    }
    out
}

/// 一个下载地址：既可能是不含占位符的普通 url，也可能是 `${MC_MAVEN}/…`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UrlTemplate(String);

impl UrlTemplate {
    /// 直接包一层，不校验。普通 url 也允许（表示这个地址无法镜像）。
    pub fn new(template: impl Into<String>) -> Self {
        Self(template.into())
    }

    /// 由根与相对路径拼出模板。
    pub fn from_parts(root: Root, path: &str) -> Self {
        let path = path.strip_prefix('/').unwrap_or(path);
        Self(format!("{}/{}", root.placeholder(), path))
    }

    /// 校验模板：每个 `${…}` 都必须是已知的下载根。
    pub fn parse(template: &str) -> Result<Self> {
        let mut rest = template;
        while let Some(start) = rest.find("${") {
            let after = &rest[start..];
            let end = after
                .find('}')
                .ok_or_else(|| anyhow!("URL 模板缺少右花括号: {template}"))?;
            let placeholder = &after[..=end];
            if Root::from_placeholder(placeholder).is_none() {
                return Err(anyhow!("URL 模板里有未知的下载根: {placeholder}"));
            }
            rest = &after[end + 1..];
        }
        Ok(Self(template.to_string()))
    }

    /// 原始模板文本。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 是否含占位符（即是否可被镜像）。
    pub fn is_templated(&self) -> bool {
        is_templated(&self.0)
    }

    /// 模板里出现的下载根。
    pub fn roots(&self) -> Vec<Root> {
        Root::ALL
            .into_iter()
            .filter(|root| self.0.contains(root.placeholder()))
            .collect()
    }

    /// 解析成 `mirror` 下的真实链接。
    pub fn resolve(&self, mirror: Mirror) -> String {
        resolve_url(&self.0, mirror)
    }

    /// 按镜像优先级展开成候选链接。
    pub fn candidates(&self, mirrors: &[Mirror]) -> Vec<String> {
        resolve_url_candidates(&self.0, mirrors)
    }

    /// 由真实 url 反推模板；地址无法识别时返回 `None`。
    pub fn from_url(url: &str) -> Option<Self> {
        let templated = templatize_url(url);
        if templated == url {
            None
        } else {
            Some(Self(templated))
        }
    }
}

impl fmt::Display for UrlTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for UrlTemplate {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for UrlTemplate {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl Serialize for UrlTemplate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for UrlTemplate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(deserializer)?))
    }
}

// ---------------------------------------------------------------------------
// 文档里的 url 字段
// ---------------------------------------------------------------------------

/// 一个文档结构体里所有"下载地址"字段的遍历。
///
/// 有了它，[`templatize_urls`] / [`resolve_urls`] 就能对所有文档类型通用，
/// 新增字段时只需要在 `for_each_url_mut` 里补一行，不会漏。
pub trait UrlFields {
    /// 对每个 url 字段调用一次 `f`（可以就地改写）。
    fn for_each_url_mut(&mut self, f: &mut dyn FnMut(&mut String));
}

/// 读出文档里所有 url（先克隆再遍历，因此要求 `Clone`）。
pub fn urls_of<T: UrlFields + Clone>(value: &T) -> Vec<String> {
    let mut owned = value.clone();
    let mut out = Vec::new();
    owned.for_each_url_mut(&mut |url| out.push(url.clone()));
    out
}

/// 把文档里所有 url 归一成模板形式，返回改写的个数。
pub fn templatize_urls<T: UrlFields>(value: &mut T) -> usize {
    let mut changed = 0;
    value.for_each_url_mut(&mut |url| {
        let templated = templatize_url(url);
        if templated != *url {
            *url = templated;
            changed += 1;
        }
    });
    changed
}

/// 把文档里所有 url 模板解析成 `mirror` 下的真实链接，返回改写的个数。
pub fn resolve_urls<T: UrlFields>(value: &mut T, mirror: Mirror) -> usize {
    let mut changed = 0;
    value.for_each_url_mut(&mut |url| {
        if !is_templated(url) {
            return;
        }
        let resolved = resolve_url(url, mirror);
        if resolved != *url {
            *url = resolved;
            changed += 1;
        }
    });
    changed
}

// ---------------------------------------------------------------------------
// 下载源
// ---------------------------------------------------------------------------

/// 一个下载源：**镜像优先级列表** + http 客户端。
///
/// * 列表里第一个是首选镜像，请求失败（网络错误 / 404）时自动尝试下一个；
/// * 所有接受 url 的方法都同时接受"模板"与"普通 url"，
///   因此可以直接 `source.get_json("${MC_META}/mc/game/version_manifest.json")`。
#[derive(Debug, Clone)]
pub struct DownloadSource {
    mirrors: Vec<Mirror>,
    client: reqwest::Client,
    user_agent: String,
}

impl DownloadSource {
    /// 单一镜像。
    pub fn new(mirror: Mirror) -> Self {
        Self::build(vec![mirror], DEFAULT_USER_AGENT)
    }

    /// 官方源。
    pub fn official() -> Self {
        Self::new(Mirror::Official)
    }

    /// 按优先级列表尝试多个镜像。列表不能为空。
    pub fn with_priority(mirrors: impl IntoIterator<Item = Mirror>) -> Result<Self> {
        let mirrors: Vec<Mirror> = {
            let mut out: Vec<Mirror> = Vec::new();
            for mirror in mirrors {
                if !out.contains(&mirror) {
                    out.push(mirror);
                }
            }
            out
        };
        if mirrors.is_empty() {
            return Err(anyhow!("下载源的镜像列表不能为空"));
        }
        Ok(Self::build(mirrors, DEFAULT_USER_AGENT))
    }

    /// 覆盖 User-Agent。
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self.client = build_client(&self.user_agent);
        self
    }

    fn build(mirrors: Vec<Mirror>, user_agent: &str) -> Self {
        Self {
            mirrors,
            client: build_client(user_agent),
            user_agent: user_agent.to_string(),
        }
    }

    /// 优先级最高的镜像。
    pub fn mirror(&self) -> Mirror {
        self.mirrors.first().copied().unwrap_or_default()
    }

    /// 镜像优先级列表。
    pub fn mirrors(&self) -> &[Mirror] {
        &self.mirrors
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// 复用的 http 客户端（连接池 / keep-alive）。
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 用首选镜像解析模板。
    pub fn resolve(&self, url: impl AsRef<str>) -> String {
        resolve_url(url.as_ref(), self.mirror())
    }

    /// 按优先级展开成候选链接。
    pub fn candidates(&self, url: impl AsRef<str>) -> Vec<String> {
        resolve_url_candidates(url.as_ref(), &self.mirrors)
    }

    /// GET 一个 url（可以是模板），依次尝试各镜像，返回响应体。
    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let candidates = self.candidates(url);
        let mut last: Option<anyhow::Error> = None;
        for candidate in &candidates {
            match self.fetch_bytes(candidate).await {
                Ok(bytes) => return Ok(bytes),
                Err(err) => {
                    last = Some(err.context(format!("GET {candidate} 失败")));
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("没有可用的下载地址: {url}")))
    }

    /// GET 一个 url 并当作 UTF-8 文本返回。
    pub async fn get_text(&self, url: &str) -> Result<String> {
        let origin = self.candidates(url).into_iter().next().unwrap_or_default();
        let bytes = self.get_bytes(url).await?;
        String::from_utf8(bytes).with_context(|| format!("{origin} 的响应不是合法 UTF-8"))
    }

    /// GET 一个 url 并按强类型反序列化 JSON。
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let text = self.get_text(url).await?;
        serde_json::from_str(&text).with_context(|| format!("解析 {url} 的 JSON 失败"))
    }

    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self.client.get(url).send().await?;
        let response = response.error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }
}

fn build_client(user_agent: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(user_agent)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_names_round_trip_and_typos_are_rejected() {
        assert_eq!(Mirror::Official.as_str(), "official");
        assert_eq!(Mirror::Bmclapi.to_string(), "bmclapi");
        assert_eq!("bmclapi".parse::<Mirror>().unwrap(), Mirror::Bmclapi);
        // 配置里写错的值直接报错
        assert!("typo".parse::<Mirror>().is_err());
        assert!(serde_json::from_str::<Mirror>("\"typo\"").is_err());
        assert_eq!(
            serde_json::to_string(&Mirror::Bmclapi).unwrap(),
            "\"bmclapi\""
        );
        assert_eq!(
            serde_json::from_str::<Mirror>("\"bmclapi\"").unwrap(),
            Mirror::Bmclapi
        );
        assert_eq!(Mirror::default(), Mirror::Official);
    }

    #[test]
    fn official_bases_are_rewritten_into_templates() {
        assert_eq!(
            templatize_url("https://libraries.minecraft.net/a/b.jar"),
            "${MC_MAVEN}/a/b.jar"
        );
        assert_eq!(
            templatize_url("https://piston-meta.mojang.com/mc/game/version_manifest.json"),
            "${MC_META}/mc/game/version_manifest.json"
        );
        assert_eq!(
            templatize_url("https://resources.download.minecraft.net/ab/cdef"),
            "${MC_ASSETS}/ab/cdef"
        );
        assert_eq!(
            templatize_url("https://meta.fabricmc.net/v2/versions/loader/1.20.1"),
            "${FABRIC_META}/v2/versions/loader/1.20.1"
        );
        // Forge 的两个等价域名归一到同一个根
        assert_eq!(
            templatize_url("https://files.minecraftforge.net/maven/x.jar"),
            "${FORGE_MAVEN}/x.jar"
        );
        assert_eq!(
            templatize_url("https://maven.minecraftforge.net/x.jar"),
            "${FORGE_MAVEN}/x.jar"
        );
    }

    #[test]
    fn already_mirrored_urls_normalise_back_to_templates() {
        // 最长前缀优先：bmclapi 的 `/maven` 必须先于站点根被匹配
        assert_eq!(
            templatize_url("https://bmclapi2.bangbang93.com/maven/a/b.jar"),
            "${MC_MAVEN}/a/b.jar"
        );
        assert_eq!(
            templatize_url("https://bmclapi2.bangbang93.com/assets/ab/cdef"),
            "${MC_ASSETS}/ab/cdef"
        );
        assert_eq!(
            templatize_url("https://bmclapi2.bangbang93.com/mc/game/version_manifest.json"),
            "${MC_META}/mc/game/version_manifest.json"
        );
    }

    #[test]
    fn unknown_hosts_are_left_alone() {
        let url = "https://example.com/a.jar";
        assert_eq!(templatize_url(url), url);
        assert!(UrlTemplate::from_url(url).is_none());
    }

    #[test]
    fn templates_resolve_per_mirror() {
        let template = "${MC_MAVEN}/a/b.jar";
        assert_eq!(
            resolve_url(template, Mirror::Official),
            "https://libraries.minecraft.net/a/b.jar"
        );
        assert_eq!(
            resolve_url(template, Mirror::Bmclapi),
            "https://bmclapi2.bangbang93.com/maven/a/b.jar"
        );
        // 不被镜像的根在两个镜像下相同
        assert_eq!(
            resolve_url("${MC_DATA}/v1/objects/x/client.jar", Mirror::Bmclapi),
            "https://piston-data.mojang.com/v1/objects/x/client.jar"
        );
        // 不含占位符时原样返回
        assert_eq!(
            resolve_url("https://example.com/a.jar", Mirror::Bmclapi),
            "https://example.com/a.jar"
        );
    }

    #[test]
    fn candidates_are_ordered_and_deduplicated() {
        let candidates = resolve_url_candidates(
            "${MC_META}/mc/game/version_manifest.json",
            &[Mirror::Bmclapi, Mirror::Official],
        );
        assert_eq!(
            candidates,
            vec![
                "https://bmclapi2.bangbang93.com/mc/game/version_manifest.json".to_string(),
                "https://piston-meta.mojang.com/mc/game/version_manifest.json".to_string(),
            ]
        );

        // 同一个地址只出现一次
        let candidates =
            resolve_url_candidates("${MC_DATA}/x", &[Mirror::Bmclapi, Mirror::Official]);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn url_template_validates_its_roots() {
        assert!(UrlTemplate::parse("${MC_MAVEN}/a.jar").is_ok());
        assert!(UrlTemplate::parse("https://example.com/a.jar").is_ok());
        assert!(UrlTemplate::parse("${NOPE}/a.jar").is_err());
        assert!(UrlTemplate::parse("${MC_MAVEN/a.jar").is_err());

        let template = UrlTemplate::from_parts(Root::McAssets, "/ab/cdef");
        assert_eq!(template.as_str(), "${MC_ASSETS}/ab/cdef");
        assert_eq!(template.roots(), vec![Root::McAssets]);
        assert_eq!(
            template.resolve(Mirror::Bmclapi),
            "https://bmclapi2.bangbang93.com/assets/ab/cdef"
        );
        assert!(template.is_templated());
        assert!(!UrlTemplate::new("https://example.com/a.jar").is_templated());
    }

    #[test]
    fn a_source_needs_at_least_one_mirror() {
        assert!(DownloadSource::with_priority([]).is_err());
        let source = DownloadSource::with_priority([Mirror::Bmclapi, Mirror::Official]).unwrap();
        assert_eq!(source.mirror(), Mirror::Bmclapi);
        assert_eq!(source.mirrors().len(), 2);
        // 重复的镜像会被去掉
        let source = DownloadSource::with_priority([Mirror::Bmclapi, Mirror::Bmclapi]).unwrap();
        assert_eq!(source.mirrors().len(), 1);
        assert_eq!(source.mirrors(), &[Mirror::Bmclapi]);
    }

    #[test]
    fn generic_rewriting_visits_every_url_field() {
        use crate::vanilla::version::VersionJson;

        let json = serde_json::json!({
            "downloads": {
                "client": {"url": "https://piston-data.mojang.com/v1/objects/x/client.jar"}
            },
            "assetIndex": {"url": "https://piston-meta.mojang.com/v1/packages/x/1.json"},
            "libraries": [
                {"name": "a:a:1", "url": "https://maven.minecraftforge.net/",
                 "downloads": {"artifact": {"url": "https://libraries.minecraft.net/a.jar"}}}
            ]
        });
        let mut version: VersionJson = serde_json::from_value(json).unwrap();

        // 4 处：client jar、资源索引、库的仓库根、库的 artifact
        assert_eq!(templatize_urls(&mut version), 4);
        let urls = urls_of(&version);
        assert!(urls.contains(&"${MC_MAVEN}/a.jar".to_string()));
        assert!(urls.contains(&"${FORGE_MAVEN}/".to_string()));
        assert!(urls.contains(&"${MC_DATA}/v1/objects/x/client.jar".to_string()));
        // 未建模字段里的字符串不会被误改
        assert!(urls
            .iter()
            .any(|url| url == "${MC_META}/v1/packages/x/1.json"));

        assert_eq!(resolve_urls(&mut version, Mirror::Bmclapi), 4);
        let urls = urls_of(&version);
        assert!(urls.contains(&"https://bmclapi2.bangbang93.com/maven/a.jar".to_string()));
        // 不可镜像的地址保持原样
        assert!(
            urls.contains(&"https://piston-data.mojang.com/v1/objects/x/client.jar".to_string())
        );
    }
}
