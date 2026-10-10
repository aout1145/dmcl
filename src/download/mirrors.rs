//! Download mirrors (was `dmcl.download.mirrors`).

use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

use clap::ValueEnum;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Which download source to use.
///
/// The D version kept this as a plain `string` and matched on it in several
/// places, silently falling back to `official` for anything unknown. As an enum
/// the comparisons disappear and `--mirror` is validated by `clap`.
///
/// Parsing stays **lossy on purpose**: an unrecognised value (a typo in
/// `dmcl.json`) becomes [`Mirror::Official`] instead of failing the whole
/// config load, which is exactly what the original did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum Mirror {
    #[default]
    Official,
    Bmclapi,
}

impl Mirror {
    /// The name used in the config file and on the command line.
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
    type Err = Infallible;

    /// Lossy: anything unrecognised is `official`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "bmclapi" => Mirror::Bmclapi,
            _ => Mirror::Official,
        })
    }
}

impl Serialize for Mirror {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Mirror {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(String::deserialize(deserializer)?
            .parse()
            .unwrap_or(Mirror::Official))
    }
}

/// A repository base url per mirror.
pub type MirrorTable = &'static [(Mirror, &'static str)];

pub const MC_META: MirrorTable = &[
    (Mirror::Official, "https://piston-meta.mojang.com"),
    (Mirror::Bmclapi, "https://bmclapi2.bangbang93.com"),
];

pub const MC_MAVEN: MirrorTable = &[
    (Mirror::Official, "https://libraries.minecraft.net"),
    (Mirror::Bmclapi, "https://bmclapi2.bangbang93.com/maven"),
];

pub const MC_ASSETS: MirrorTable = &[
    (Mirror::Official, "https://resources.download.minecraft.net"),
    (Mirror::Bmclapi, "https://bmclapi2.bangbang93.com/assets"),
];

pub const FORGE_MAVEN: MirrorTable = &[
    (Mirror::Official, "https://files.minecraftforge.net/maven"),
    (Mirror::Bmclapi, "https://bmclapi2.bangbang93.com/maven"),
];

pub const NEOFORGE_MAVEN: MirrorTable = &[
    (Mirror::Official, "https://maven.neoforged.net/releases"),
    (Mirror::Bmclapi, "https://bmclapi2.bangbang93.com/maven"),
];

pub const FABRIC_META: MirrorTable = &[
    (Mirror::Official, "https://meta.fabricmc.net"),
    (
        Mirror::Bmclapi,
        "https://bmclapi2.bangbang93.com/fabric-meta",
    ),
];

pub const FABRIC_MAVEN: MirrorTable = &[
    (Mirror::Official, "https://maven.fabricmc.net"),
    (Mirror::Bmclapi, "https://bmclapi2.bangbang93.com/maven"),
];

/// Base url of `mirror` inside a mirror table (falls back to `official`).
pub fn base<'a>(table: &'a [(Mirror, &'a str)], mirror: Mirror) -> &'a str {
    table
        .iter()
        .find(|(m, _)| *m == mirror)
        .or_else(|| table.iter().find(|(m, _)| *m == Mirror::Official))
        .or_else(|| table.first())
        .map(|(_, url)| *url)
        .unwrap_or_default()
}

// Base url of one table for the selected mirror. The D code indexed the mirror
// tables directly (`mc_meta[mirror]`); these are the equivalent accessors, one
// per table the launcher actually uses.
pub fn mc_meta(mirror: Mirror) -> &'static str {
    base(MC_META, mirror)
}
pub fn mc_maven(mirror: Mirror) -> &'static str {
    base(MC_MAVEN, mirror)
}
pub fn forge_maven(mirror: Mirror) -> &'static str {
    base(FORGE_MAVEN, mirror)
}
pub fn neoforge_maven(mirror: Mirror) -> &'static str {
    base(NEOFORGE_MAVEN, mirror)
}
pub fn fabric_meta(mirror: Mirror) -> &'static str {
    base(FABRIC_META, mirror)
}

/// `mirrorUrl(mirror, table, url)`.
///
/// * `Some(table)` — rewrite the `official` prefix of `url` with the mirror one.
/// * `None` — detect the repository from the url and rewrite it accordingly
///   (identical fall-through order to the original).
pub fn mirror_url(mirror: Mirror, table: Option<MirrorTable>, url: &str) -> String {
    match table {
        Some(table) => url.replace(base(table, Mirror::Official), base(table, mirror)),
        None => {
            for rules in [
                MC_MAVEN,
                MC_ASSETS,
                FORGE_MAVEN,
                NEOFORGE_MAVEN,
                FABRIC_MAVEN,
            ] {
                let mirror_base = base(rules, mirror);
                if !mirror_base.is_empty() && url.contains(mirror_base) {
                    return url.replace(base(rules, Mirror::Official), mirror_base);
                }
            }
            url.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_names_round_trip_and_typos_are_tolerated() {
        assert_eq!(Mirror::Official.as_str(), "official");
        assert_eq!(Mirror::Bmclapi.to_string(), "bmclapi");
        assert_eq!("bmclapi".parse::<Mirror>().unwrap(), Mirror::Bmclapi);
        // an unknown value behaves like `official`, like the D implementation
        assert_eq!("typo".parse::<Mirror>().unwrap(), Mirror::Official);
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
    fn explicit_tables_rewrite_the_official_base() {
        assert_eq!(
            mirror_url(
                Mirror::Bmclapi,
                Some(MC_MAVEN),
                "https://libraries.minecraft.net/a/b.jar"
            ),
            "https://bmclapi2.bangbang93.com/maven/a/b.jar"
        );
        assert_eq!(
            mirror_url(
                Mirror::Bmclapi,
                Some(MC_META),
                "https://piston-meta.mojang.com/x.json"
            ),
            "https://bmclapi2.bangbang93.com/x.json"
        );
        assert_eq!(
            mirror_url(
                Mirror::Official,
                Some(MC_MAVEN),
                "https://libraries.minecraft.net/a/b.jar"
            ),
            "https://libraries.minecraft.net/a/b.jar"
        );
    }

    #[test]
    fn table_less_mode_reproduces_the_original_no_op() {
        // The original `mirrorUrl(mirror, null, url)` only rewrites a url that
        // *already contains* the target mirror's base, so an official url is
        // returned untouched. Kept byte-for-byte compatible on purpose.
        assert_eq!(
            mirror_url(
                Mirror::Bmclapi,
                None,
                "https://libraries.minecraft.net/a/b.jar"
            ),
            "https://libraries.minecraft.net/a/b.jar"
        );
        assert_eq!(
            mirror_url(Mirror::Bmclapi, None, "https://example.com/a.jar"),
            "https://example.com/a.jar"
        );
        // already-mirrored urls are normalised back to the requested mirror
        assert_eq!(
            mirror_url(
                Mirror::Bmclapi,
                None,
                "https://bmclapi2.bangbang93.com/maven/a/b.jar"
            ),
            "https://bmclapi2.bangbang93.com/maven/a/b.jar"
        );
    }
}
