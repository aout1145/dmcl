//! Compile-time constants (was `dmcl.env`).

/// Name reported to Minecraft (`--versionType`, launcher name placeholder).
pub const LAUNCHER_NAME: &str = "dmcl";
/// Launcher version reported to Minecraft.
pub const LAUNCHER_VERSION: &str = "0.0.1";
/// Default `launcher_profiles.json` written into a fresh `.minecraft` folder.
pub const LAUNCHER_PROFILES_JSON: &str = r#"{"selectedProfile": "(Default)","profiles": {"(Default)": {"name": "(Default)"}},"clientToken": "88888888-8888-8888-8888-888888888888"}"#;
