//! 实例图标的来源、内置素材与自动选择规则。
//!
//! 安装页和主页共用同一套图标：主页优先读取 `versions/<name>/logo.png`，没有自定义图标
//! 时按版本类型与加载器自动选择；安装页只做预览，自动图标不落盘，用户选的自定义图片在
//! 安装成功后才写入 `logo.png`。

use std::path::{Path, PathBuf};

use relm4::gtk::gdk::prelude::*;
use relm4::gtk::{gdk, glib};

use dmcl_game::{DetectedLoaderKind, DetectedVersion, DetectedVersionKind};
use dmcl_install::LoaderKind;

const VANILLA_RELEASE: &[u8] = include_bytes!("../assets/icons/vanilla_release.png");
const VANILLA_SNAPSHOT: &[u8] = include_bytes!("../assets/icons/vanilla_snapshot.png");
const VANILLA_OLD: &[u8] = include_bytes!("../assets/icons/vanilla_old.png");
const FORGE: &[u8] = include_bytes!("../assets/icons/forge.png");
const NEOFORGE: &[u8] = include_bytes!("../assets/icons/neoforge.png");
const FABRIC: &[u8] = include_bytes!("../assets/icons/fabric.png");
const OPTIFINE: &[u8] = include_bytes!("../assets/icons/optifine.png");
const BROKEN: &[u8] = include_bytes!("../assets/icons/broken.png");

/// 一个内置的实例图标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconKind {
    Release,
    Snapshot,
    Old,
    Forge,
    NeoForge,
    Fabric,
    OptiFine,
    /// 版本 JSON 读不出来，或者识别不出版本类型。
    Broken,
}

impl IconKind {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Release => VANILLA_RELEASE,
            Self::Snapshot => VANILLA_SNAPSHOT,
            Self::Old => VANILLA_OLD,
            Self::Forge => FORGE,
            Self::NeoForge => NEOFORGE,
            Self::Fabric => FABRIC,
            Self::OptiFine => OPTIFINE,
            Self::Broken => BROKEN,
        }
    }

    pub(crate) fn texture(self) -> Option<gdk::Texture> {
        gdk::Texture::from_bytes(&glib::Bytes::from_static(self.bytes())).ok()
    }

    /// 安装页的自动图标：加载器优先于原版类型，Forge 又优先于其它加载器。
    pub(crate) fn auto(mc_kind: Option<&str>, loaders: &[LoaderKind]) -> Self {
        for kind in [
            LoaderKind::Forge,
            LoaderKind::NeoForge,
            LoaderKind::Fabric,
            LoaderKind::OptiFine,
        ] {
            if loaders.contains(&kind) {
                return match kind {
                    LoaderKind::Forge => Self::Forge,
                    LoaderKind::NeoForge => Self::NeoForge,
                    LoaderKind::Fabric => Self::Fabric,
                    LoaderKind::OptiFine => Self::OptiFine,
                };
            }
        }

        mc_kind
            .and_then(Self::from_version_kind)
            .unwrap_or(Self::Broken)
    }

    /// 已安装实例的自动图标：同样由加载器压过原版类型。
    pub(crate) fn from_detected(detected: &DetectedVersion) -> Self {
        for kind in [
            DetectedLoaderKind::Forge,
            DetectedLoaderKind::NeoForge,
            DetectedLoaderKind::Fabric,
            DetectedLoaderKind::OptiFine,
        ] {
            if detected.loaders.iter().any(|loader| loader.kind == kind) {
                return match kind {
                    DetectedLoaderKind::Forge => Self::Forge,
                    DetectedLoaderKind::NeoForge => Self::NeoForge,
                    DetectedLoaderKind::Fabric => Self::Fabric,
                    DetectedLoaderKind::OptiFine => Self::OptiFine,
                };
            }
        }

        match detected.kind {
            Some(DetectedVersionKind::Release) => Self::Release,
            Some(DetectedVersionKind::Snapshot) => Self::Snapshot,
            Some(DetectedVersionKind::Old) => Self::Old,
            None => Self::Broken,
        }
    }

    fn from_version_kind(kind: &str) -> Option<Self> {
        match kind {
            "release" => Some(Self::Release),
            "snapshot" => Some(Self::Snapshot),
            "old" | "old_alpha" | "old_beta" | "alpha" | "beta" => Some(Self::Old),
            _ => None,
        }
    }
}

/// 磁贴 / 预览里显示的图标来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstanceIcon {
    /// 内置素材。
    Asset(IconKind),
    /// 用户选择的本地图片（安装成功后转成 `logo.png`）。
    File(PathBuf),
}

impl InstanceIcon {
    pub(crate) fn texture(&self) -> Option<gdk::Texture> {
        match self {
            Self::Asset(kind) => kind.texture(),
            Self::File(path) => gdk::Texture::from_filename(path).ok(),
        }
    }
}

/// 把用户选择的图片转成 PNG 写入目标路径。
pub(crate) fn save_png(source: &Path, destination: &Path) -> Result<(), String> {
    let texture =
        gdk::Texture::from_filename(source).map_err(|err| format!("无法读取图标图片：{err}"))?;
    texture
        .save_to_png(destination)
        .map_err(|err| format!("无法写入图标 PNG：{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_icon_prefers_loaders_and_forge_wins_over_optifine() {
        let loaders = [LoaderKind::OptiFine, LoaderKind::Forge];
        assert_eq!(IconKind::auto(Some("release"), &loaders), IconKind::Forge);

        let loaders = [LoaderKind::OptiFine, LoaderKind::Fabric];
        assert_eq!(IconKind::auto(Some("snapshot"), &loaders), IconKind::Fabric);
    }

    #[test]
    fn auto_icon_falls_back_to_the_minecraft_kind() {
        assert_eq!(IconKind::auto(Some("release"), &[]), IconKind::Release);
        assert_eq!(IconKind::auto(Some("snapshot"), &[]), IconKind::Snapshot);
        assert_eq!(IconKind::auto(Some("old"), &[]), IconKind::Old);
        assert_eq!(IconKind::auto(None, &[]), IconKind::Broken);
        assert_eq!(IconKind::auto(Some("unobfuscated"), &[]), IconKind::Broken);
    }

    #[test]
    fn detected_loaders_override_the_version_kind() {
        let mut detected = DetectedVersion {
            kind: Some(DetectedVersionKind::Release),
            ..Default::default()
        };
        assert_eq!(IconKind::from_detected(&detected), IconKind::Release);

        detected.loaders = vec![dmcl_game::DetectedLoader {
            kind: DetectedLoaderKind::OptiFine,
            version: None,
        }];
        assert_eq!(IconKind::from_detected(&detected), IconKind::OptiFine);

        detected.loaders.push(dmcl_game::DetectedLoader {
            kind: DetectedLoaderKind::Forge,
            version: None,
        });
        assert_eq!(IconKind::from_detected(&detected), IconKind::Forge);
    }

    #[test]
    fn detected_without_kind_or_loader_is_broken() {
        assert_eq!(
            IconKind::from_detected(&DetectedVersion::default()),
            IconKind::Broken
        );
    }

    #[test]
    fn every_builtin_icon_is_a_png() {
        for kind in [
            IconKind::Release,
            IconKind::Snapshot,
            IconKind::Old,
            IconKind::Forge,
            IconKind::NeoForge,
            IconKind::Fabric,
            IconKind::OptiFine,
            IconKind::Broken,
        ] {
            assert!(kind.bytes().starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }
}
