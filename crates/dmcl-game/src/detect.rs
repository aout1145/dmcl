//! 从版本 JSON 与主 jar 识别实例的 Minecraft 版本和已安装加载器。
//!
//! 优先相信 DMCL 写入的 [`VersionJson::dmcl_patches`]；没有 patch 历史时分析 maven 库坐标；
//! 仍然不知道 Minecraft 版本时才读主 jar。所有字段都允许为空，识别不出就不猜。

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use anyhow::Result;
use dmcl_mcmeta::vanilla::{Library, VersionJson};

use crate::class_parse::{class_strings, constant_pool_strings, Zip};
use crate::version::{main_jar_path, resolve_version, VersionRef};

const MINECRAFT_SERVER: &str = "net/minecraft/server/MinecraftServer.class";
const REALMS_CONSTANTS: &str = "net/minecraft/realms/RealmsSharedConstants.class";
const MINECRAFT_CLIENT: &str = "net/minecraft/client/Minecraft.class";
const CLASSIC_MINECRAFT: &str = "com/mojang/minecraft/Minecraft.class";

/// 一个实例的识别结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectedVersion {
    /// 显示用的 Minecraft 版本标签；旧版本可能是 `1.0.0`、`Alpha v1.2.6` 等。
    pub minecraft: Option<String>,
    /// 已识别出的加载器。
    pub loaders: Vec<DetectedLoader>,
}

/// 一个已安装的加载器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedLoader {
    pub kind: DetectedLoaderKind,
    /// 加载器版本；Forge / NeoForge 去掉 `{mc}-` 前缀，OptiFine 去掉 `{mc}_` 前缀。
    /// 来源没有版本时为 `None`。
    pub version: Option<String>,
}

/// 识别阶段支持的加载器种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DetectedLoaderKind {
    Fabric,
    Forge,
    NeoForge,
    OptiFine,
}

impl DetectedLoaderKind {
    /// 显示用的名字。
    pub fn display_name(self) -> &'static str {
        match self {
            DetectedLoaderKind::Fabric => "Fabric",
            DetectedLoaderKind::Forge => "Forge",
            DetectedLoaderKind::NeoForge => "NeoForge",
            DetectedLoaderKind::OptiFine => "OptiFine",
        }
    }
}

/// 只分析版本 JSON，不读 jar。
fn detect_json(json: &VersionJson) -> DetectedVersion {
    if json.dmcl_patches.is_empty() {
        detect_libraries(&json.libraries)
    } else {
        detect_patches(json)
    }
}

/// 从主 jar 提取适合显示的 Minecraft 版本标签；没有可靠的版本信息时返回 `None`。
async fn detect_jar_version(path: impl AsRef<Path>) -> Option<String> {
    let path = path.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || read_jar_version(&path))
        .await
        .ok()
        .flatten()
}

/// 解析已安装实例；版本 JSON 无法确定 Minecraft 版本时读主 jar。
pub async fn detect_version(reference: &VersionRef) -> Result<DetectedVersion> {
    let resolved = resolve_version(reference).await?;
    let mut detected = detect_json(&resolved.json);
    if detected.minecraft.is_none() {
        let jar = main_jar_path(&resolved.reference, &resolved.json);
        detected.minecraft = detect_jar_version(jar).await;
    }
    Ok(detected)
}

// ---------------------------------------------------------------------------
// patch 历史
// ---------------------------------------------------------------------------

fn detect_patches(json: &VersionJson) -> DetectedVersion {
    let minecraft = json.dmcl_patches[0]
        .get("id")
        .and_then(|value| value.as_str())
        .filter(|id| !id.is_empty())
        .map(str::to_owned);

    let mut loaders = LoaderSet::default();
    for patch in &json.dmcl_patches[1..] {
        let Some(name) = patch.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        let Some(kind) = loader_kind(name) else {
            continue;
        };
        let version = patch
            .get("version")
            .and_then(|value| value.as_str())
            .filter(|version| !version.is_empty())
            .map(|version| loader_version_label(kind, version));
        loaders.add(kind, version);
    }

    DetectedVersion {
        minecraft,
        loaders: loaders.finish(),
    }
}

fn loader_kind(name: &str) -> Option<DetectedLoaderKind> {
    match name.to_ascii_lowercase().as_str() {
        "fabric" => Some(DetectedLoaderKind::Fabric),
        "forge" => Some(DetectedLoaderKind::Forge),
        "neoforge" => Some(DetectedLoaderKind::NeoForge),
        "optifine" => Some(DetectedLoaderKind::OptiFine),
        _ => None,
    }
}

/// Forge / NeoForge 的版本号常带 `{mc}-` 前缀，OptiFine 用 `{mc}_`；显示时去掉。
fn loader_version_label(kind: DetectedLoaderKind, version: &str) -> String {
    let separator = match kind {
        DetectedLoaderKind::Forge | DetectedLoaderKind::NeoForge => '-',
        DetectedLoaderKind::OptiFine => '_',
        DetectedLoaderKind::Fabric => return version.to_string(),
    };
    let Some((prefix, rest)) = version.split_once(separator) else {
        return version.to_string();
    };
    if !is_release_version(prefix) {
        return version.to_string();
    }
    let suffix = format!("{separator}{prefix}");
    rest.strip_suffix(&suffix).unwrap_or(rest).to_string()
}

/// 按首次出现顺序收集加载器；同一加载器出现多个不同版本时版本字段留空。
#[derive(Default)]
struct LoaderSet {
    entries: Vec<(DetectedLoaderKind, BTreeSet<String>)>,
}

impl LoaderSet {
    fn add(&mut self, kind: DetectedLoaderKind, version: Option<String>) {
        let index = match self.entries.iter().position(|(seen, _)| *seen == kind) {
            Some(index) => index,
            None => {
                self.entries.push((kind, BTreeSet::new()));
                self.entries.len() - 1
            }
        };
        if let Some(version) = version {
            self.entries[index].1.insert(version);
        }
    }

    fn finish(self) -> Vec<DetectedLoader> {
        self.entries
            .into_iter()
            .map(|(kind, versions)| DetectedLoader {
                kind,
                version: single_version(versions),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// libraries
// ---------------------------------------------------------------------------

#[derive(Default)]
struct LoaderVersions {
    present: bool,
    versions: BTreeSet<String>,
}

impl LoaderVersions {
    fn seen(&mut self, version: &str) {
        self.present = true;
        self.versions.insert(version.to_string());
    }

    fn present(&mut self) {
        self.present = true;
    }

    fn finish(self, kind: DetectedLoaderKind) -> Option<DetectedLoader> {
        self.present.then(|| DetectedLoader {
            kind,
            version: single_version(self.versions),
        })
    }
}

fn detect_libraries(libraries: &[Library]) -> DetectedVersion {
    let mut minecraft = BTreeSet::new();
    let mut fabric = LoaderVersions::default();
    let mut forge = LoaderVersions::default();
    let mut neoforge = LoaderVersions::default();
    let mut optifine = LoaderVersions::default();

    for library in libraries {
        let Some((group, artifact, version)) = coordinate(&library.name) else {
            continue;
        };
        match (group, artifact) {
            ("net.fabricmc", "fabric-loader") => fabric.seen(version),
            ("net.fabricmc", "intermediary") => insert_version(&mut minecraft, version),
            ("net.fabricmc", "yarn") => {
                if let Some(mc) = version.split('+').next() {
                    insert_version(&mut minecraft, mc);
                }
            }
            ("net.minecraftforge", "forge" | "fmlloader" | "fmlearlydisplay") => {
                forge.seen(&loader_version_label(DetectedLoaderKind::Forge, version));
                if let Some(mc) = version.split('-').next() {
                    insert_version(&mut minecraft, mc);
                }
            }
            ("net.neoforged", "neoforge") => {
                neoforge.seen(&loader_version_label(DetectedLoaderKind::NeoForge, version));
            }
            ("net.neoforged", "forge") => {
                neoforge.seen(&loader_version_label(DetectedLoaderKind::NeoForge, version));
                if version.contains('-') {
                    if let Some(mc) = version.split('-').next() {
                        insert_version(&mut minecraft, mc);
                    }
                }
            }
            ("net.neoforged.fancymodloader", "loader" | "earlydisplay") => neoforge.present(),
            ("optifine", artifact) if artifact.eq_ignore_ascii_case("OptiFine") => {
                optifine.seen(&loader_version_label(DetectedLoaderKind::OptiFine, version));
                if let Some(mc) = version.split('_').next() {
                    insert_version(&mut minecraft, mc);
                }
            }
            _ => {}
        }
    }

    DetectedVersion {
        minecraft: single_version(minecraft),
        loaders: [
            (DetectedLoaderKind::Fabric, fabric),
            (DetectedLoaderKind::Forge, forge),
            (DetectedLoaderKind::NeoForge, neoforge),
            (DetectedLoaderKind::OptiFine, optifine),
        ]
        .into_iter()
        .filter_map(|(kind, loader)| loader.finish(kind))
        .collect(),
    }
}

/// `group:artifact:version[:classifier][@ext]` 的前三段。
fn coordinate(name: &str) -> Option<(&str, &str, &str)> {
    let coordinate = name.split('@').next()?;
    let mut parts = coordinate.split(':');
    let group = parts.next()?;
    let artifact = parts.next()?;
    let version = parts.next()?;
    if group.is_empty() || artifact.is_empty() || version.is_empty() {
        return None;
    }
    Some((group, artifact, version))
}

fn insert_version(versions: &mut BTreeSet<String>, version: &str) {
    if !version.is_empty() {
        versions.insert(version.to_string());
    }
}

fn single_version(versions: BTreeSet<String>) -> Option<String> {
    let mut versions = versions.into_iter();
    match (versions.next(), versions.next()) {
        (Some(version), None) => Some(version),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 主 jar
// ---------------------------------------------------------------------------

fn read_jar_version(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    if let Some(version) = read_version_json(&mut archive) {
        return Some(version);
    }

    for entry in [MINECRAFT_SERVER, REALMS_CONSTANTS] {
        let versions = class_versions(&mut archive, entry);
        if let Some(version) = single_version(versions) {
            return Some(version);
        }
    }

    if let Some(version) = read_minecraft_client(&mut archive) {
        return Some(version);
    }

    let titles = class_titles(&mut archive, CLASSIC_MINECRAFT);
    if let Some(version) = single_version(titles) {
        return Some(version);
    }

    scan_class_titles(&mut archive)
}

fn read_version_json(archive: &mut Zip) -> Option<String> {
    let mut entry = archive.by_name("version.json").ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    ["id", "name", "release_target"].iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn read_minecraft_client(archive: &mut Zip) -> Option<String> {
    let strings = class_strings(archive, MINECRAFT_CLIENT)?;

    let versions = strings
        .iter()
        .filter(|value| is_release_version(value) || is_snapshot_version(value))
        .cloned()
        .collect();
    if let Some(version) = single_version(versions) {
        return Some(version);
    }

    let titles = strings
        .iter()
        .filter_map(|value| title_label(value))
        .collect();
    single_version(titles)
}

fn class_versions(archive: &mut Zip, entry: &str) -> BTreeSet<String> {
    class_strings(archive, entry)
        .unwrap_or_default()
        .into_iter()
        .filter(|value| is_release_version(value) || is_snapshot_version(value))
        .collect()
}

fn class_titles(archive: &mut Zip, entry: &str) -> BTreeSet<String> {
    class_strings(archive, entry)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| title_label(&value))
        .collect()
}

fn scan_class_titles(archive: &mut Zip) -> Option<String> {
    let mut titles = BTreeSet::new();
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        let Ok(name) = entry.name() else {
            continue;
        };
        let name = name.to_string();
        if !name.ends_with(".class") || name.starts_with("META-INF/") {
            continue;
        }
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_err() {
            continue;
        }
        let Some(strings) = constant_pool_strings(&bytes) else {
            continue;
        };
        titles.extend(strings.iter().filter_map(|value| title_label(value)));
    }
    single_version(titles)
}

/// 正式版 `1.x` / `1.x.y`。
fn is_release_version(value: &str) -> bool {
    let mut parts = value.split('.');
    if parts.next() != Some("1") {
        return false;
    }
    let mut count = 0;
    for part in parts {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        count += 1;
    }
    matches!(count, 1 | 2)
}

/// Mojang 快照 `NNwNNa`。
fn is_snapshot_version(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 6
        && bytes[..2].iter().all(|byte| byte.is_ascii_digit())
        && bytes[2] == b'w'
        && bytes[3..5].iter().all(|byte| byte.is_ascii_digit())
        && bytes[5].is_ascii_lowercase()
}

/// 把 `Minecraft …` 形式的标题转成显示标签；不映射成官方清单 id。
fn title_label(value: &str) -> Option<String> {
    let mut rest = value;
    while let Some(stripped) = rest.strip_prefix("Minecraft ") {
        rest = stripped;
    }
    if rest == value {
        return None;
    }

    if rest == "Infdev" {
        return Some("Infdev".to_string());
    }
    if let Some(alpha) = rest.strip_prefix("Alpha v") {
        return Some(format!("Alpha v{alpha}"));
    }
    if let Some(beta) = rest.strip_prefix("Beta ") {
        return Some(format!("Beta {beta}"));
    }
    if rest.starts_with("0.") {
        return Some(format!("Classic {rest}"));
    }
    if is_release_version(rest) || is_snapshot_version(rest) {
        return Some(rest.to_string());
    }
    None
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::path::PathBuf;

    fn loader(kind: DetectedLoaderKind, version: &str) -> DetectedLoader {
        DetectedLoader {
            kind,
            version: Some(version.to_string()),
        }
    }

    fn version(value: serde_json::Value) -> VersionJson {
        serde_json::from_value(value).unwrap()
    }

    fn class_with(strings: &[&str]) -> Vec<u8> {
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 52];
        bytes.extend_from_slice(&(strings.len() as u16 + 1).to_be_bytes());
        for string in strings {
            bytes.push(1);
            bytes.extend_from_slice(&(string.len() as u16).to_be_bytes());
            bytes.extend_from_slice(string.as_bytes());
        }
        bytes
    }

    fn jar(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default();
            for (name, bytes) in entries {
                writer.start_file(*name, options).unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer
    }

    fn work_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dmcl-game-detect-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    async fn detect_bytes(tag: &str, bytes: &[u8]) -> Option<String> {
        let dir = work_dir(tag);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("client.jar");
        std::fs::write(&path, bytes).unwrap();
        let version = detect_jar_version(&path).await;
        let _ = std::fs::remove_dir_all(&dir);
        version
    }

    #[test]
    fn patches_take_precedence_over_libraries() {
        let json = version(serde_json::json!({
            "dmcl-patches": [
                {"id": "1.20.1"},
                {"name": "forge", "version": "1.20.1-47.2.0"},
                {"name": "optifine", "version": "1.20.1_HD_U_I6"}
            ],
            "libraries": [{"name": "net.fabricmc:fabric-loader:0.15.0"}]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(
            detected.loaders,
            [
                loader(DetectedLoaderKind::Forge, "47.2.0"),
                loader(DetectedLoaderKind::OptiFine, "HD_U_I6"),
            ]
        );
    }

    #[test]
    fn patches_without_an_id_do_not_fall_back_to_libraries_for_loaders() {
        let json = version(serde_json::json!({
            "dmcl-patches": [{"jar": "vanilla"}],
            "libraries": [{"name": "net.fabricmc:fabric-loader:0.15.0"}]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft, None);
        assert!(detected.loaders.is_empty());
    }

    #[test]
    fn conflicting_patch_loader_versions_leave_the_version_empty() {
        let json = version(serde_json::json!({
            "dmcl-patches": [
                {"id": "1.20.1"},
                {"name": "forge", "version": "1.20.1-47.2.0"},
                {"name": "Forge", "version": "1.20.1-47.3.0"}
            ]
        }));

        let detected = detect_json(&json);
        assert_eq!(
            detected.loaders,
            [DetectedLoader {
                kind: DetectedLoaderKind::Forge,
                version: None,
            }]
        );
    }

    #[test]
    fn fabric_libraries_give_the_loader_version_and_intermediary_minecraft() {
        let json = version(serde_json::json!({
            "libraries": [
                {"name": "net.fabricmc:fabric-loader:0.15.0"},
                {"name": "net.fabricmc:intermediary:1.20.1"},
                {"name": "net.fabricmc:yarn:1.20.1+build.1"}
            ]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(
            detected.loaders,
            [loader(DetectedLoaderKind::Fabric, "0.15.0")]
        );
    }

    #[test]
    fn fabric_without_intermediary_uses_yarn() {
        let json = version(serde_json::json!({
            "libraries": [
                {"name": "net.fabricmc:fabric-loader:0.15.0"},
                {"name": "net.fabricmc:yarn:1.19.4+build.1"}
            ]
        }));

        assert_eq!(detect_json(&json).minecraft.as_deref(), Some("1.19.4"));
    }

    #[test]
    fn forge_libraries_give_the_minecraft_prefix() {
        let json = version(serde_json::json!({
            "libraries": [
                {"name": "net.minecraftforge:forge:1.20.1-47.2.0"},
                {"name": "net.minecraftforge:fmlloader:1.20.1-47.2.0"}
            ]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(
            detected.loaders,
            [loader(DetectedLoaderKind::Forge, "47.2.0")]
        );
    }

    #[test]
    fn neoforge_libraries_are_recognized_without_guessing_from_the_loader_version() {
        let json = version(serde_json::json!({
            "libraries": [
                {"name": "net.neoforged:neoforge:21.1.265"},
                {"name": "net.neoforged.fancymodloader:loader:4.0.7"}
            ]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft, None);
        assert_eq!(
            detected.loaders,
            [loader(DetectedLoaderKind::NeoForge, "21.1.265")]
        );
    }

    #[test]
    fn old_neoforge_forge_coordinates_give_the_minecraft_prefix() {
        let json = version(serde_json::json!({
            "libraries": [{"name": "net.neoforged:forge:1.20.1-47.1.0"}]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(
            detected.loaders,
            [loader(DetectedLoaderKind::NeoForge, "47.1.0")]
        );
    }

    #[test]
    fn fancymodloader_alone_only_identifies_the_loader() {
        let json = version(serde_json::json!({
            "libraries": [{"name": "net.neoforged.fancymodloader:earlydisplay:4.0.7"}]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft, None);
        assert_eq!(
            detected.loaders,
            [DetectedLoader {
                kind: DetectedLoaderKind::NeoForge,
                version: None,
            }]
        );
    }

    #[test]
    fn optifine_libraries_give_the_minecraft_prefix() {
        let json = version(serde_json::json!({
            "libraries": [{"name": "optifine:OptiFine:1.12.2_HD_U_G5"}]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft.as_deref(), Some("1.12.2"));
        assert_eq!(
            detected.loaders,
            [loader(DetectedLoaderKind::OptiFine, "HD_U_G5")]
        );
    }

    #[test]
    fn multiple_different_minecraft_candidates_are_not_guessed() {
        let json = version(serde_json::json!({
            "libraries": [
                {"name": "net.fabricmc:intermediary:1.20.1"},
                {"name": "optifine:OptiFine:1.19.4_HD_U_I6"}
            ]
        }));

        assert_eq!(detect_json(&json).minecraft, None);
    }

    #[test]
    fn an_empty_document_has_no_identifiable_version() {
        let detected = detect_json(&VersionJson::default());
        assert_eq!(detected, DetectedVersion::default());
    }

    #[test]
    fn a_foreign_patches_field_is_ignored() {
        let json = version(serde_json::json!({
            "patches": [{"id": "resolved.1.12.2-forge"}],
            "libraries": [{"name": "net.minecraftforge:forge:1.12.2-14.23.5.2864"}]
        }));

        let detected = detect_json(&json);
        assert_eq!(detected.minecraft.as_deref(), Some("1.12.2"));
        assert_eq!(
            detected.loaders,
            [loader(DetectedLoaderKind::Forge, "14.23.5.2864")]
        );
    }

    #[test]
    fn loader_versions_drop_the_minecraft_prefix() {
        for (kind, version, expected) in [
            (DetectedLoaderKind::Forge, "1.20.1-47.4.28", "47.4.28"),
            (
                DetectedLoaderKind::Forge,
                "1.7.10-10.13.4.1614-1.7.10",
                "10.13.4.1614",
            ),
            (DetectedLoaderKind::NeoForge, "1.20.1-47.1.106", "47.1.106"),
            (DetectedLoaderKind::NeoForge, "21.1.265", "21.1.265"),
            (DetectedLoaderKind::OptiFine, "1.12.2_HD_U_G5", "HD_U_G5"),
            (DetectedLoaderKind::OptiFine, "HD_U_G5", "HD_U_G5"),
            (DetectedLoaderKind::Fabric, "0.15.0", "0.15.0"),
        ] {
            assert_eq!(loader_version_label(kind, version), expected, "{version}");
        }
    }

    #[tokio::test]
    async fn the_jar_version_json_is_preferred() {
        let bytes = jar(&[
            ("version.json", br#"{"id":"1.20.1"}"#.to_vec()),
            (MINECRAFT_SERVER, class_with(&["1.19.4"])),
        ]);

        assert_eq!(
            detect_bytes("version-json", &bytes).await.as_deref(),
            Some("1.20.1")
        );
    }

    #[tokio::test]
    async fn minecraft_server_constants_give_the_release_version() {
        let bytes = jar(&[(MINECRAFT_SERVER, class_with(&["1.12.2"]))]);
        assert_eq!(
            detect_bytes("server", &bytes).await.as_deref(),
            Some("1.12.2")
        );
    }

    #[tokio::test]
    async fn realms_constants_are_a_fallback() {
        let bytes = jar(&[(REALMS_CONSTANTS, class_with(&["1.8.9"]))]);
        assert_eq!(
            detect_bytes("realms", &bytes).await.as_deref(),
            Some("1.8.9")
        );
    }

    #[tokio::test]
    async fn the_client_title_gives_the_release_version() {
        let bytes = jar(&[(MINECRAFT_CLIENT, class_with(&["Minecraft Minecraft 1.2.5"]))]);
        assert_eq!(
            detect_bytes("client", &bytes).await.as_deref(),
            Some("1.2.5")
        );
    }

    #[tokio::test]
    async fn old_titles_are_kept_as_display_labels() {
        for (tag, title, expected) in [
            ("title-1.0", "Minecraft Minecraft 1.0.0", "1.0.0"),
            (
                "title-alpha",
                "Minecraft Minecraft Alpha v1.2.6",
                "Alpha v1.2.6",
            ),
            ("title-beta", "Minecraft Minecraft Beta 1.7.3", "Beta 1.7.3"),
            ("title-classic", "Minecraft 0.0.13a", "Classic 0.0.13a"),
            ("title-classic-030", "Minecraft 0.30", "Classic 0.30"),
            ("title-infdev", "Minecraft Infdev", "Infdev"),
            ("title-infdev-long", "Minecraft Minecraft Infdev", "Infdev"),
        ] {
            let bytes = jar(&[(CLASSIC_MINECRAFT, class_with(&[title]))]);
            assert_eq!(
                detect_bytes(tag, &bytes).await.as_deref(),
                Some(expected),
                "标题「{title}」"
            );
        }
    }

    #[tokio::test]
    async fn conflicting_versions_in_one_class_are_not_guessed() {
        let bytes = jar(&[(MINECRAFT_SERVER, class_with(&["1.12.2", "1.13.2"]))]);
        assert_eq!(detect_bytes("conflict", &bytes).await, None);
    }

    #[tokio::test]
    async fn a_jar_without_version_information_returns_none() {
        let bytes = jar(&[(
            "com/mojang/rubydung/Level.class",
            class_with(&["com/mojang/rubydung/Level"]),
        )]);
        assert_eq!(detect_bytes("empty", &bytes).await, None);
    }

    #[tokio::test]
    async fn a_missing_or_corrupt_jar_returns_none() {
        assert_eq!(
            detect_jar_version(work_dir("missing").join("absent.jar")).await,
            None
        );
        assert_eq!(detect_bytes("corrupt", b"not a zip").await, None);
    }

    #[tokio::test]
    async fn detect_version_falls_back_to_the_inherited_jar() {
        let root = work_dir("resolve");
        let base = root.join("versions").join("base");
        let child = root.join("versions").join("custom");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&child).unwrap();

        std::fs::write(
            base.join("base.json"),
            serde_json::json!({"id": "base"}).to_string(),
        )
        .unwrap();
        std::fs::write(
            base.join("base.jar"),
            jar(&[("version.json", br#"{"id":"1.20.1"}"#.to_vec())]),
        )
        .unwrap();
        std::fs::write(
            child.join("custom.json"),
            serde_json::json!({"id": "custom", "inheritsFrom": "base"}).to_string(),
        )
        .unwrap();

        let detected = detect_version(&VersionRef::new(&root, "custom"))
            .await
            .unwrap();
        assert_eq!(detected.minecraft.as_deref(), Some("1.20.1"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn detect_version_reads_loaders_from_the_inherited_libraries() {
        let root = work_dir("resolve-libraries");
        let base = root.join("versions").join("base");
        let child = root.join("versions").join("custom");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&child).unwrap();

        std::fs::write(
            base.join("base.json"),
            serde_json::json!({
                "id": "base",
                "libraries": [{"name": "net.fabricmc:fabric-loader:0.15.0"}]
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            child.join("custom.json"),
            serde_json::json!({"id": "custom", "inheritsFrom": "base"}).to_string(),
        )
        .unwrap();

        let detected = detect_version(&VersionRef::new(&root, "custom"))
            .await
            .unwrap();
        assert_eq!(
            detected.loaders,
            [loader(DetectedLoaderKind::Fabric, "0.15.0")]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    #[ignore = "需要 DMCL_GAME_JAR_DIR 指向本地官方 jar 目录"]
    async fn real_official_jars_are_recognized() {
        let Ok(dir) = std::env::var("DMCL_GAME_JAR_DIR") else {
            return;
        };
        let cases = [
            ("1.14.4", Some("1.14.4")),
            ("1.13.2", Some("1.13.2")),
            ("1.12.2", Some("1.12.2")),
            ("1.8.9", Some("1.8.9")),
            ("1.6.4", Some("1.6.4")),
            ("1.3.2", Some("1.3.2")),
            ("1.2.5", Some("1.2.5")),
            ("1.1", Some("1.1")),
            ("1.0", Some("1.0.0")),
            ("a1.2.6", Some("Alpha v1.2.6")),
            ("b1.7.3", Some("Beta 1.7.3")),
            ("c0.0.13a", Some("Classic 0.0.13a")),
            ("c0.30_01c", Some("Classic 0.30")),
            ("inf-20100618", Some("Infdev")),
            ("rd-132211", None),
        ];

        for (name, expected) in cases {
            let path = Path::new(&dir).join(format!("{name}.jar"));
            assert_eq!(
                detect_jar_version(&path).await.as_deref(),
                expected,
                "{name}"
            );
        }
    }
}
