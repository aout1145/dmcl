//! 规划：给定版本 JSON，算出"这个版本需要哪些文件"。
//!
//! 这一层是**纯函数**：不碰网络、不碰磁盘，所以规则过滤、困难库、natives 分类器、
//! 虚拟资源目录、跳过音乐这些判断都能被穷举单测覆盖。
//!
//! 产出的 [`PlannedFile::url`] 仍然是**模板**，真正下载时才由
//! [`dmcl_mcmeta::source::DownloadSource::resolve`] 解析成具体镜像地址。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dmcl_download::{Checksum, DownloadItem};
use dmcl_mcmeta::layout::{
    assets_index_path, assets_objects_dir, assets_virtual_dir, libraries_dir, version_jar_path,
};
use dmcl_mcmeta::maven::lib_name_to_path;
use dmcl_mcmeta::os::get_os_name;
use dmcl_mcmeta::vanilla::{
    asset_object_url, maven_library_url, rules_allow, Artifact, AssetIndexFile, Library,
    RuleContext, VersionJson,
};

use crate::options::InstallOptions;

/// 规划一个版本需要哪些文件时，只依赖这几项配置。
///
/// 单独拆出来是为了让**别的 crate 也能复用规划逻辑**（例如 `dmcl-launch` 的完整性
/// 检查），而不必携带下载源、java 提供者这些安装才用得上的东西。
#[derive(Debug, Clone)]
pub struct PlanContext {
    /// `.minecraft` 根目录。
    pub root_path: PathBuf,
    /// 是否跳过 `*.ogg` 资源。
    pub skip_music: bool,
    /// natives 分类器里 `${arch}` 的取值。
    pub natives_arch: u32,
}

impl PlanContext {
    pub fn new(root_path: impl Into<PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
            skip_music: false,
            natives_arch: crate::options::default_natives_arch(),
        }
    }

    pub fn with_skip_music(mut self, skip_music: bool) -> Self {
        self.skip_music = skip_music;
        self
    }

    pub fn with_natives_arch(mut self, natives_arch: u32) -> Self {
        self.natives_arch = natives_arch;
        self
    }
}

/// 一个计划中的下载项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// URL 模板（`${MC_MAVEN}/…`），或不可镜像的普通 url。
    pub url: String,
    /// 目标路径。
    pub save_to: PathBuf,
    /// 期望的校验值（由 SHA-1 摘要构造，Minecraft 文档里给的就是 sha1）。
    pub checksum: Option<Checksum>,
    pub size: Option<u64>,
    /// 进度里显示的名字。
    pub name: String,
}

impl PlannedFile {
    /// 变成真正的下载项：这里才把模板解析成具体地址。
    pub fn into_item(self, options: &InstallOptions) -> DownloadItem {
        DownloadItem::new(options.source.resolve(&self.url), &self.save_to)
            .with_checksum(self.checksum)
            .with_size(self.size)
            .named(self.name)
    }
}

/// 路径的最后一段（用于进度显示）。
pub fn base_name(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// 客户端 jar（`downloads.client`）。
pub fn plan_client_jar(
    plan: &PlanContext,
    json: &VersionJson,
    version_name: &str,
) -> Option<PlannedFile> {
    let client = json.downloads.as_ref()?.client.as_ref()?;
    let url = client.url.clone()?;
    Some(PlannedFile {
        url,
        save_to: version_jar_path(&plan.root_path, version_name),
        checksum: client.sha1.clone().map(Checksum::sha1),
        size: client.size,
        name: format!("{version_name}.jar"),
    })
}

/// 资源索引本身（`assets/indexes/<id>.json`）。
pub fn plan_assets_index(plan: &PlanContext, json: &VersionJson) -> Option<PlannedFile> {
    let index = json.asset_index.as_ref()?;
    let id = json.asset_index_id();
    if id.is_empty() {
        return None;
    }
    let url = index.url.clone()?;
    Some(PlannedFile {
        url,
        save_to: assets_index_path(&plan.root_path, id),
        checksum: index.sha1.clone().map(Checksum::sha1),
        size: index.size,
        name: format!("{id}.json"),
    })
}

/// natives 里该平台需要的分类器名（`${arch}` 会被替换）。
pub fn native_classifier(natives: &BTreeMap<String, String>, arch: u32) -> Option<String> {
    let raw = natives.get(get_os_name())?;
    Some(raw.replace("${arch}", &arch.to_string()))
}

/// 库条目里该平台 natives jar 对应的 artifact。
///
/// 先按替换过 `${arch}` 的名字找，找不到再退回原名（有些版本的 `natives`
/// 与 `classifiers` 用的名字并不完全一致）。
pub fn native_artifact(lib: &Library, arch: u32) -> Option<&Artifact> {
    let raw = lib.natives.as_ref()?.get(get_os_name())?;
    let resolved = raw.replace("${arch}", &arch.to_string());
    let classifiers = lib.downloads.as_ref()?.classifiers.as_ref()?;
    classifiers
        .get(&resolved)
        .or_else(|| classifiers.get(raw.as_str()))
}

/// 文档给出的库级校验值里，取**最强**的一个（sha512 > sha256 > sha1 > md5）。
///
/// Fabric 之类的加载器会把校验值直接写在库条目上；Minecraft 自己的 `downloads`
/// 只给 sha1。没有校验值就返回 `None`（交给大小 / 存在性判断）。
fn lib_checksum(lib: &Library) -> Option<Checksum> {
    if let Some(hex) = &lib.sha512 {
        return Some(Checksum::sha512(hex.clone()));
    }
    if let Some(hex) = &lib.sha256 {
        return Some(Checksum::sha256(hex.clone()));
    }
    if let Some(hex) = &lib.sha1 {
        return Some(Checksum::sha1(hex.clone()));
    }
    lib.md5.clone().map(Checksum::md5)
}

/// 库文件（含 natives 分类器 jar）。
pub fn plan_libraries(plan: &PlanContext, json: &VersionJson) -> Vec<PlannedFile> {
    let libraries = libraries_dir(&plan.root_path);
    let rules = RuleContext::current();
    let mut planned = Vec::new();

    for lib in &json.libraries {
        if !rules_allow(&lib.rules, &rules) {
            continue;
        }
        let coord = lib.name.as_str();
        let lib_path = lib_name_to_path(coord);

        match lib.downloads.as_ref() {
            // 有 downloads 的"普通库"：直接用文档里给的地址
            Some(downloads) => {
                if let Some(artifact) = downloads.artifact.as_ref().filter(|a| a.url.is_some()) {
                    // `path` 是仓库里的相对路径；极少数文档不带它，这时按 maven 坐标推导
                    let relative = artifact
                        .path
                        .clone()
                        .unwrap_or_else(|| lib_name_to_path(coord));
                    planned.push(PlannedFile {
                        url: artifact.url.clone().unwrap_or_default(),
                        save_to: libraries.join(&relative),
                        checksum: artifact.sha1.clone().map(Checksum::sha1),
                        size: artifact.size,
                        name: base_name(&relative),
                    });
                }
            }
            // 没有仓库地址的"困难库"：按 maven 坐标拼到官方库仓库
            None if lib.url.is_none() => planned.push(PlannedFile {
                url: maven_library_url(&lib_path),
                save_to: libraries.join(&lib_path),
                checksum: lib_checksum(lib),
                size: None,
                name: base_name(&lib_path),
            }),
            // 自带仓库地址的"困难库"
            None => {
                let base = lib.url.as_deref().unwrap_or("").trim_end_matches('/');
                planned.push(PlannedFile {
                    url: format!("{base}/{lib_path}"),
                    save_to: libraries.join(&lib_path),
                    checksum: lib_checksum(lib),
                    size: None,
                    name: base_name(&lib_path),
                });
            }
        }

        // natives 分类器
        if let Some(artifact) = native_artifact(lib, plan.natives_arch) {
            let (Some(url), Some(path)) = (artifact.url.clone(), artifact.path.clone()) else {
                // 文档里没给地址的 natives 直接跳过
                continue;
            };
            planned.push(PlannedFile {
                url,
                save_to: libraries.join(&path),
                checksum: artifact.sha1.clone().map(Checksum::sha1),
                size: artifact.size,
                name: base_name(&path),
            });
        }
    }

    planned
}

/// 需要解压的 natives jar（路径 + 显示名）。
pub fn plan_natives(plan: &PlanContext, json: &VersionJson) -> Vec<(PathBuf, String)> {
    let libraries = libraries_dir(&plan.root_path);
    let rules = RuleContext::current();
    json.libraries
        .iter()
        .filter(|lib| rules_allow(&lib.rules, &rules))
        .filter_map(|lib| native_artifact(lib, plan.natives_arch))
        .filter_map(|artifact| artifact.path.clone())
        .map(|path| {
            let name = base_name(&path);
            (libraries.join(path), name)
        })
        .collect()
}

/// 资源对象。
pub fn plan_asset_objects(
    plan: &PlanContext,
    json: &VersionJson,
    index: &AssetIndexFile,
) -> Vec<PlannedFile> {
    let id = json.asset_index_id();
    let virtual_dir = assets_virtual_dir(&plan.root_path, id);
    let objects_dir = assets_objects_dir(&plan.root_path);
    let mut planned = Vec::new();

    for (key, asset) in &index.objects {
        if plan.skip_music && is_music(key) {
            continue;
        }
        // hash 太短（脏数据）时跳过
        let Some(relative) = asset.path() else {
            continue;
        };
        // 1.6 及更早用虚拟目录，其余用 objects 目录
        let save_to = if index.is_virtual() {
            virtual_dir.join(key)
        } else {
            objects_dir.join(&relative)
        };
        planned.push(PlannedFile {
            url: asset_object_url(&relative),
            save_to,
            checksum: Some(Checksum::sha1(asset.hash.clone())),
            size: Some(asset.size),
            name: base_name(key),
        });
    }

    planned
}

/// 这个版本需要下载的全部文件：客户端 jar + 库 + 资源索引文件（+ 给了 `index` 时的资源对象）。
pub fn plan_game_files(
    plan: &PlanContext,
    json: &VersionJson,
    version_name: &str,
    index: Option<&AssetIndexFile>,
) -> Vec<PlannedFile> {
    let mut files = Vec::new();
    if let Some(client) = plan_client_jar(plan, json, version_name) {
        files.push(client);
    }
    files.extend(plan_libraries(plan, json));
    if let Some(index_file) = plan_assets_index(plan, json) {
        files.push(index_file);
    }
    if let Some(index) = index {
        files.extend(plan_asset_objects(plan, json, index));
    }
    files
}

/// `*.ogg` 就是音乐 / 音效。
fn is_music(key: &str) -> bool {
    Path::new(key)
        .extension()
        .map(|extension| extension == "ogg")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_mcmeta::source::{DownloadSource, Mirror};

    /// 规划只依赖 [`PlanContext`]（launch 做完整性检查时用的也是它）。
    fn options() -> PlanContext {
        PlanContext::new("/mc").with_natives_arch(64)
    }

    /// 把规划结果变成下载项时需要完整的安装配置（下载源）。
    fn install_options() -> InstallOptions {
        InstallOptions::new("/mc", DownloadSource::new(Mirror::Bmclapi)).with_natives_arch(64)
    }

    fn version(json: &str) -> VersionJson {
        let mut json: VersionJson = serde_json::from_str(json).unwrap();
        json.templatize_urls();
        json
    }

    #[test]
    fn client_jar_and_asset_index_are_planned_with_templates() {
        let options = options();
        let install = install_options();
        let json = version(
            r#"{
                "downloads": {"client": {"url": "https://piston-data.mojang.com/v1/objects/x/client.jar",
                                          "sha1": "cafe", "size": 100}},
                "assetIndex": {"id": "5", "sha1": "beef", "size": 20,
                                "url": "https://piston-meta.mojang.com/v1/packages/beef/5.json"}
            }"#,
        );

        let client = plan_client_jar(&options, &json, "1.20.1").unwrap();
        assert_eq!(client.url, "${MC_DATA}/v1/objects/x/client.jar");
        assert_eq!(
            client.save_to,
            PathBuf::from("/mc/versions/1.20.1/1.20.1.jar")
        );
        assert_eq!(client.size, Some(100));
        // 不可镜像的地址解析后仍是官方地址
        assert_eq!(
            client.into_item(&install).url,
            "https://piston-data.mojang.com/v1/objects/x/client.jar"
        );

        let index = plan_assets_index(&options, &json).unwrap();
        assert_eq!(index.url, "${MC_META}/v1/packages/beef/5.json");
        assert_eq!(index.save_to, PathBuf::from("/mc/assets/indexes/5.json"));
        assert_eq!(
            index.into_item(&install).url,
            "https://bmclapi2.bangbang93.com/v1/packages/beef/5.json"
        );
    }

    #[test]
    fn libraries_cover_normal_difficult_and_forge_cases() {
        let options = options();
        let json = version(
            r#"{
                "libraries": [
                    {"name": "normal:lib:1", "downloads": {"artifact": {
                        "url": "https://libraries.minecraft.net/normal/lib/1/lib-1.jar",
                        "path": "normal/lib/1/lib-1.jar", "sha1": "a", "size": 5}}},
                    {"name": "difficult:lib:2"},
                    {"name": "reposed:lib:3", "url": "https://maven.example.com/repo/"},
                    {"name": "net.minecraftforge:forge:1.20.1-47.2.0", "url": "https://maven.minecraftforge.net/"},
                    {"name": "skipped:lib:9", "rules": [{"action": "allow", "os": {"name": "plan9"}}]},
                    {"name": "old:lib:4", "downloads": {"artifact": {"url": "https://example.com/x.jar"}}}
                ]
            }"#,
        );

        let planned = plan_libraries(&options, &json);
        let names: Vec<&str> = planned.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "lib-1.jar",
                "lib-2.jar",
                "lib-3.jar",
                "forge-1.20.1-47.2.0.jar",
                "lib-4.jar"
            ]
        );

        // 普通库：用文档里的（已模板化的）地址
        assert_eq!(planned[0].url, "${MC_MAVEN}/normal/lib/1/lib-1.jar");
        // 没有仓库地址的困难库：走官方库仓库
        assert_eq!(planned[1].url, "${MC_MAVEN}/difficult/lib/2/lib-2.jar");
        assert_eq!(
            planned[1].save_to,
            PathBuf::from("/mc/libraries/difficult/lib/2/lib-2.jar")
        );
        // 自带仓库地址的困难库：仓库地址不认得，原样拼上去
        assert_eq!(
            planned[2].url,
            "https://maven.example.com/repo/reposed/lib/3/lib-3.jar"
        );
        // forge 的困难库同样按 {url}/{lib_path} 下载（不再跳过）
        assert_eq!(
            planned[3].url,
            "${FORGE_MAVEN}/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0.jar"
        );
        // 规则不通过的库不下
        assert!(!names.iter().any(|name| name.contains("lib-9")));
        // 只给了 url 没给 path 的 artifact：按 maven 坐标推导路径
        assert_eq!(planned[4].url, "https://example.com/x.jar");
        assert_eq!(
            planned[4].save_to,
            PathBuf::from("/mc/libraries/old/lib/4/lib-4.jar")
        );
    }

    #[test]
    fn natives_are_planned_and_collected_for_extraction() {
        let options = options();
        let json = version(
            r#"{
                "libraries": [
                    {
                        "name": "org.lwjgl:lwjgl:3.3.1",
                        "natives": { "linux": "natives-linux" },
                        "downloads": {
                            "artifact": {
                                "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar",
                                "path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar"
                            },
                            "classifiers": {
                                "natives-linux": {
                                    "url": "https://libraries.minecraft.net/org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar",
                                    "path": "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar"
                                }
                            }
                        }
                    },
                    {
                        "name": "arch:lib:1",
                        "natives": { "linux": "natives-linux-${arch}" },
                        "downloads": {
                            "classifiers": {
                                "natives-linux-64": {
                                    "url": "https://libraries.minecraft.net/arch/lib/1/arch-1-natives-linux-64.jar",
                                    "path": "arch/lib/1/arch-1-natives-linux-64.jar"
                                }
                            }
                        }
                    }
                ]
            }"#,
        );

        let planned = plan_libraries(&options, &json);
        let names: Vec<&str> = planned.iter().map(|file| file.name.as_str()).collect();
        // 平台不匹配的 natives 不下载；`${arch}` 会被替换成 64
        #[cfg(target_os = "linux")]
        {
            assert!(names.contains(&"lwjgl-3.3.1.jar"));
            assert!(names.contains(&"lwjgl-3.3.1-natives-linux.jar"));
            assert!(names.contains(&"arch-1-natives-linux-64.jar"));
            let natives = plan_natives(&options, &json);
            assert_eq!(natives.len(), 2);
            assert_eq!(
                natives[0].0,
                PathBuf::from("/mc/libraries/org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar")
            );
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(!names.iter().any(|name| name.contains("natives")));
            assert!(plan_natives(&options, &json).is_empty());
        }

        // 32 位时 `${arch}` 取 32，找不到对应分类器就不下载
        let mut options32 = options.clone();
        options32.natives_arch = 32;
        let planned = plan_libraries(&options32, &json);
        assert!(!planned.iter().any(|file| file.name.contains("arch-1")));
    }

    #[test]
    fn asset_objects_respect_virtual_indexes_and_music_skipping() {
        let options = options().with_skip_music(true);
        let json = version(r#"{"assetIndex": {"id": "legacy"}}"#);

        let index: AssetIndexFile = serde_json::from_str(
            r#"{"virtual": true, "objects": {
                "music/a.ogg": {"hash": "aa11", "size": 1},
                "icons/icon_16x16.png": {"hash": "bb22", "size": 2},
                "bad": {"hash": "c", "size": 3}
            }}"#,
        )
        .unwrap();

        let planned = plan_asset_objects(&options, &json, &index);
        let names: Vec<&str> = planned.iter().map(|file| file.name.as_str()).collect();
        // 虚拟索引：音乐被跳过，hash 太短的被跳过
        assert_eq!(names, vec!["icon_16x16.png"]);
        assert_eq!(
            planned[0].save_to,
            PathBuf::from("/mc/assets/virtual/legacy/icons/icon_16x16.png")
        );
        assert_eq!(planned[0].url, "${MC_ASSETS}/bb/bb22");

        // 不跳过音乐 + 非虚拟索引 → 走 objects 目录
        let options = options.with_skip_music(false);
        let index: AssetIndexFile =
            serde_json::from_str(r#"{"objects": {"music/a.ogg": {"hash": "aa11", "size": 1}}}"#)
                .unwrap();
        let planned = plan_asset_objects(&options, &json, &index);
        assert_eq!(planned.len(), 1);
        assert_eq!(
            planned[0].save_to,
            PathBuf::from("/mc/assets/objects/aa/aa11")
        );
    }

    #[test]
    fn planned_files_resolve_against_the_configured_mirror() {
        let options = install_options();
        let planned = PlannedFile {
            url: "${MC_MAVEN}/a/b.jar".to_string(),
            save_to: PathBuf::from("/mc/libraries/a/b.jar"),
            checksum: Some(Checksum::sha1("abc")),
            size: Some(7),
            name: "b.jar".to_string(),
        };
        let item = planned.into_item(&options);
        assert_eq!(item.url, "https://bmclapi2.bangbang93.com/maven/a/b.jar");
        assert_eq!(item.checksum.as_ref().map(|c| c.hex()), Some("abc"));
        assert_eq!(item.size, Some(7));
        assert_eq!(item.name, "b.jar");
    }
}
