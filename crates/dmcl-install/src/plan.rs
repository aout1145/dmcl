//! 规划：给定版本 JSON，算出"这个版本需要哪些文件"。
//!
//! 这一层是**纯函数**：不碰网络、不碰磁盘，所以原实现里那些绕来绕去的判断
//! （规则过滤、困难库、natives 分类器、虚拟资源目录、跳过音乐）都能被穷举单测覆盖。
//!
//! 产出的 [`PlannedFile::url`] 仍然是**模板**，真正下载时才由
//! [`InstallOptions::resolve`] 解析成具体镜像地址。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dmcl_download::DownloadItem;
use dmcl_mcmeta::{
    asset_object_url, assets_index_path, assets_objects_dir, assets_virtual_dir, get_os_name,
    lib_name_to_path, libraries_dir, maven_library_url, rules_allow, version_jar_path, Artifact,
    AssetIndexFile, Library, VersionJson,
};

use crate::options::InstallOptions;

/// 一个计划中的下载项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// URL 模板（`${MC_MAVEN}/…`），或不可镜像的普通 url。
    pub url: String,
    /// 目标路径。
    pub save_to: PathBuf,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    /// 进度里显示的名字。
    pub name: String,
}

impl PlannedFile {
    /// 变成真正的下载项：这里才把模板解析成具体地址。
    pub fn into_item(self, options: &InstallOptions) -> DownloadItem {
        DownloadItem::new(options.resolve(&self.url), &self.save_to)
            .with_sha1(self.sha1)
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
    options: &InstallOptions,
    json: &VersionJson,
    version_name: &str,
) -> Option<PlannedFile> {
    let client = json.downloads.as_ref()?.client.as_ref()?;
    let url = client.url.clone()?;
    Some(PlannedFile {
        url,
        save_to: version_jar_path(&options.root_path, version_name),
        sha1: client.sha1.clone(),
        size: client.size,
        name: format!("{version_name}.jar"),
    })
}

/// 资源索引本身（`assets/indexes/<id>.json`）。
pub fn plan_assets_index(options: &InstallOptions, json: &VersionJson) -> Option<PlannedFile> {
    let index = json.asset_index.as_ref()?;
    let id = json.asset_index_id();
    if id.is_empty() {
        return None;
    }
    let url = index.url.clone()?;
    Some(PlannedFile {
        url,
        save_to: assets_index_path(&options.root_path, id),
        sha1: index.sha1.clone(),
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

/// 库文件（含 natives 分类器 jar）。
pub fn plan_libraries(options: &InstallOptions, json: &VersionJson) -> Vec<PlannedFile> {
    let libraries = libraries_dir(&options.root_path);
    let mut planned = Vec::new();

    for lib in &json.libraries {
        if !rules_allow(&lib.rules) {
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
                        sha1: artifact.sha1.clone(),
                        size: artifact.size,
                        name: base_name(&relative),
                    });
                }
            }
            // 没有仓库地址的"困难库"：按 maven 坐标拼到官方库仓库
            None if lib.url.is_none() => planned.push(PlannedFile {
                url: maven_library_url(&lib_path),
                save_to: libraries.join(&lib_path),
                sha1: lib.sha1.clone(),
                size: None,
                name: base_name(&lib_path),
            }),
            // 自带仓库地址的"困难库"
            None => {
                if coord.starts_with("net.minecraftforge:forge:") {
                    // 这个文件就是下不下来（原实现的原话）
                    continue;
                }
                let base = lib.url.as_deref().unwrap_or("").trim_end_matches('/');
                planned.push(PlannedFile {
                    url: format!("{base}/{lib_path}"),
                    save_to: libraries.join(&lib_path),
                    sha1: lib.sha1.clone(),
                    size: None,
                    name: base_name(&lib_path),
                });
            }
        }

        // natives 分类器
        if let Some(artifact) = native_artifact(lib, options.natives_arch) {
            let (Some(url), Some(path)) = (artifact.url.clone(), artifact.path.clone()) else {
                // 文档里没给地址的 natives 直接跳过（原实现会拿空 url 去请求然后报错）
                continue;
            };
            planned.push(PlannedFile {
                url,
                save_to: libraries.join(&path),
                sha1: artifact.sha1.clone(),
                size: artifact.size,
                name: base_name(&path),
            });
        }
    }

    planned
}

/// 需要解压的 natives jar（路径 + 显示名）。
pub fn plan_natives(options: &InstallOptions, json: &VersionJson) -> Vec<(PathBuf, String)> {
    let libraries = libraries_dir(&options.root_path);
    json.libraries
        .iter()
        .filter(|lib| rules_allow(&lib.rules))
        .filter_map(|lib| native_artifact(lib, options.natives_arch))
        .filter_map(|artifact| artifact.path.clone())
        .map(|path| {
            let name = base_name(&path);
            (libraries.join(path), name)
        })
        .collect()
}

/// 资源对象。
pub fn plan_asset_objects(
    options: &InstallOptions,
    json: &VersionJson,
    index: &AssetIndexFile,
) -> Vec<PlannedFile> {
    let id = json.asset_index_id();
    let virtual_dir = assets_virtual_dir(&options.root_path, id);
    let objects_dir = assets_objects_dir(&options.root_path);
    let mut planned = Vec::new();

    for (key, asset) in &index.objects {
        if options.skip_music && is_music(key) {
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
            sha1: Some(asset.hash.clone()),
            size: Some(asset.size),
            name: base_name(key),
        });
    }

    planned
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
    use dmcl_mcmeta::{DownloadSource, Mirror};

    fn options() -> InstallOptions {
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
            client.into_item(&options).url,
            "https://piston-data.mojang.com/v1/objects/x/client.jar"
        );

        let index = plan_assets_index(&options, &json).unwrap();
        assert_eq!(index.url, "${MC_META}/v1/packages/beef/5.json");
        assert_eq!(index.save_to, PathBuf::from("/mc/assets/indexes/5.json"));
        assert_eq!(
            index.into_item(&options).url,
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
            vec!["lib-1.jar", "lib-2.jar", "lib-3.jar", "lib-4.jar"]
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
        // forge 那个下不下来的文件被跳过（即使它有 ${FORGE_MAVEN} 模板）
        assert!(!names.contains(&"forge-1.20.1-47.2.0.jar"));
        // 规则不通过的库不下
        assert!(!names.iter().any(|name| name.contains("lib-9")));
        // 只给了 url 没给 path 的 artifact：按 maven 坐标推导路径
        assert_eq!(planned[3].url, "https://example.com/x.jar");
        assert_eq!(
            planned[3].save_to,
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
        let options = options();
        let planned = PlannedFile {
            url: "${MC_MAVEN}/a/b.jar".to_string(),
            save_to: PathBuf::from("/mc/libraries/a/b.jar"),
            sha1: Some("abc".to_string()),
            size: Some(7),
            name: "b.jar".to_string(),
        };
        let item = planned.into_item(&options);
        assert_eq!(item.url, "https://bmclapi2.bangbang93.com/maven/a/b.jar");
        assert_eq!(item.sha1.as_deref(), Some("abc"));
        assert_eq!(item.size, Some(7));
        assert_eq!(item.name, "b.jar");
    }
}
