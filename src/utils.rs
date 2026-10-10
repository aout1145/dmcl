//! Shared helpers (was `dmcl.utils`).
//!
//! The original D module hand-rolled a number of utilities only because Phobos
//! had no equivalent (no async io, no `zip`, no path normalisation, no sha1
//! hex formatting, no shell quoting, no tolerant version comparison). Those are
//! now delegated to purpose-built crates:
//!
//! | helper | crate |
//! | --- | --- |
//! | path normalisation (`get_path`) | [`path_clean`] |
//! | sha1 → hex (`sha1_hex`) | [`sha1`] + [`hex`] |
//! | shell quoting (`escape_shell_command`) | [`shell_words`] |
//! | zip reading (`extract_zip`, `zip_read_entry`) | [`zip`] |
//! | http (`get_text`, `get_download`, …) | [`reqwest`] |
//!
//! Version comparison is no longer here either: it belongs to `version-compare`
//! (see `GameLauncher::get_libraries`).
//!
//! What remains hand-written is Minecraft-specific logic (`check_rules` and
//! friends, which live in [`crate::mcjson`]) plus maven-coordinate handling,
//! for which no maintained crate reproduces the required shape.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use path_clean::PathClean;
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};

use crate::env::{LAUNCHER_NAME, LAUNCHER_VERSION};
use crate::mcjson::VersionJson;

/// Shared `reqwest` client (connection pooling, keep-alive).
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(format!("{LAUNCHER_NAME}/{LAUNCHER_VERSION}"))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// `getPath(...)` from the D code: make a path absolute (relative to the
/// current working directory) and normalise `.` / `..` / repeated separators.
///
/// The lexical part is [`path_clean`], which implements exactly the
/// `buildNormalizedPath` behaviour of Phobos.
pub fn get_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute.clean()
}

/// `getOSName()` — the names used inside Minecraft's rule objects
/// (`osx`, not `macos`: that is what the game's rules use).
pub fn get_os_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}

/// Path separator used inside a Java classpath (was `std.path.pathSeparator`).
/// This is a Java convention rather than a path separator, so there is no crate
/// for it.
pub fn classpath_separator() -> char {
    if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    }
}

/// Lower-case hex sha1 of a byte slice
/// (was `toHexString!(LetterCase.lower)(sha1Of(...))`).
pub fn sha1_hex(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// `extractZip` — unpack a jar/zip archive. The `zip` crate is synchronous, so
/// the actual work is offloaded to a blocking worker thread.
pub async fn extract_zip(
    filename: impl AsRef<Path> + Send + 'static,
    extract_to: impl AsRef<Path> + Send + 'static,
) -> Result<()> {
    let filename = filename.as_ref().to_path_buf();
    let extract_to = extract_to.as_ref().to_path_buf();
    tokio::task::spawn_blocking(move || extract_zip_blocking(&filename, &extract_to))
        .await
        .context("zip extraction task panicked")?
}

fn extract_zip_blocking(filename: &Path, extract_to: &Path) -> Result<()> {
    let file = std::fs::File::open(filename)
        .with_context(|| format!("failed to open {}", filename.display()))?;
    let mut archive = zip::ZipArchive::new(file)?;
    std::fs::create_dir_all(extract_to)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        if name.is_empty() {
            continue;
        }
        let out = extract_to.join(&name);
        if name.ends_with('/') {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            std::fs::write(&out, buf)?;
        }
    }
    Ok(())
}

/// Read one entry of an in-memory archive (was `zip.expand(zip.directory[name])`).
pub fn zip_read_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("zip entry not found: {name}"))?;
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Was `name in zip.directory`.
pub fn zip_has_entry<R: Read + Seek>(archive: &mut zip::ZipArchive<R>, name: &str) -> bool {
    archive.by_name(name).is_ok()
}

// ---------------------------------------------------------------------------
// maven coordinates
// ---------------------------------------------------------------------------
//
// Kept hand-written on purpose: crates.io has no maintained, widely used crate
// that parses `group:artifact:version[:classifier][@ext]`. See MIGRATION.md.

/// `libNameToPath` — maven coordinate to its relative repository path.
pub fn lib_name_to_path(libname: &str) -> String {
    let libname = if !libname.contains('@') {
        format!("{libname}@jar")
    } else {
        libname.to_string()
    };
    let mut rawstr = libname.splitn(2, '@');
    let coord = rawstr.next().unwrap_or("");
    let ext = rawstr.next().unwrap_or("jar");
    let rawname: Vec<&str> = coord.split(':').collect();
    let pkg = rawname.first().copied().unwrap_or("");
    let name = rawname.get(1).copied().unwrap_or("");
    let mut path = format!(
        "{}/{}/{}/{}",
        pkg.replace('.', "/"),
        name,
        rawname.get(2).copied().unwrap_or(""),
        name
    );
    for part in rawname.iter().skip(2) {
        path.push('-');
        path.push_str(part);
    }
    path.push('.');
    path.push_str(ext);
    path
}

/// `libNameGetName` — `group:artifact[:classifier...]` (version stripped).
pub fn lib_name_get_name(libname: &str) -> String {
    let rawname: Vec<&str> = libname.split(':').collect();
    let mut name = format!(
        "{}:{}",
        rawname.first().copied().unwrap_or(""),
        rawname.get(1).copied().unwrap_or("")
    );
    if rawname.len() > 3 {
        for part in rawname.iter().skip(3) {
            name.push(':');
            name.push_str(part);
        }
    }
    name
}

/// `libNameGetVersion` — the version part of a maven coordinate.
pub fn lib_name_get_version(libname: &str) -> String {
    libname
        .split('@')
        .next()
        .unwrap_or("")
        .split(':')
        .nth(2)
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// version json
// ---------------------------------------------------------------------------

/// `versions/<version_name>` — the directory holding one version's files.
pub fn version_dir(root_path: &Path, version_name: &str) -> PathBuf {
    root_path.join("versions").join(version_name)
}

/// `versions/<version_name>/<version_name>.json`.
pub fn version_json_path(root_path: &Path, version_name: &str) -> PathBuf {
    version_dir(root_path, version_name).join(format!("{version_name}.json"))
}

/// `versions/<version_name>/<version_name>.jar`.
pub fn version_jar_path(root_path: &Path, version_name: &str) -> PathBuf {
    version_dir(root_path, version_name).join(format!("{version_name}.jar"))
}

/// `readVersionJSONSafe` — read a version json, resolving `inheritsFrom`.
pub async fn read_version_json_safe(root_path: &Path, version_name: &str) -> Result<VersionJson> {
    let path = version_json_path(root_path, version_name);
    let mut json = read_version_json(&path).await?;

    if let Some(orig) = json.inherits_from.clone() {
        // if json based on another, merge them
        let mut orijson = read_version_json(&version_json_path(root_path, &orig)).await?;
        orijson.merge(&json)?;
        json = orijson;
    }
    Ok(json)
}

async fn read_version_json(path: &Path) -> Result<VersionJson> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// `writeSafe` — create parent directories then write the file.
pub async fn write_safe(path: impl AsRef<Path>, context: &[u8]) -> Result<()> {
    let path = get_path(path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    tokio::fs::write(&path, context)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Serialise + write, matching the original's compact `json.toString()`.
pub async fn write_json_safe<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    let text = serde_json::to_string(value)?;
    write_safe(path, text.as_bytes()).await
}

// ---------------------------------------------------------------------------
// http
// ---------------------------------------------------------------------------

/// GET a url as text, with a descriptive error (this replaces the throwing
/// `std.net.curl.get` of the original).
pub async fn get_text(url: &str) -> Result<String> {
    let resp = http_client()
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to GET {url}"))?;
    let resp = resp
        .error_for_status()
        .with_context(|| format!("failed to GET {url}"))?;
    resp.text()
        .await
        .with_context(|| format!("failed to read body of {url}"))
}

/// GET + deserialise into a concrete type (was `parseJSON(get(url))`, now typed).
pub async fn get_json_as<T: DeserializeOwned>(url: &str) -> Result<T> {
    let text = get_text(url).await?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse json from {url}"))
}

/// `getDownload` — GET a url as bytes, verifying the sha1 and retrying up to 6
/// times in total (identical retry accounting to the original).
pub async fn get_download(url: &str, sha1: Option<&str>) -> Result<Vec<u8>> {
    let mut retrycnt = 0u32;
    loop {
        let err: String = match http_client().get(url).send().await {
            Ok(resp) => match resp.error_for_status() {
                Ok(resp) => match resp.bytes().await {
                    Ok(bytes) => {
                        let bytes = bytes.to_vec();
                        match sha1 {
                            Some(expected) if sha1_hex(&bytes) != expected => {
                                "Wrong sha1sum".to_string()
                            }
                            _ => return Ok(bytes),
                        }
                    }
                    Err(e) => e.to_string(),
                },
                Err(e) => e.to_string(),
            },
            Err(e) => e.to_string(),
        };
        if retrycnt == 5 {
            eprintln!("Failed to GET {url}: Retried too many times");
            return Err(anyhow!("Failed to GET {url}: {err}"));
        }
        retrycnt += 1;
        eprintln!("Failed to GET {url}: {err}. Retrying...");
    }
}

/// `downloadSafe` — download a url into a file.
pub async fn download_safe(url: &str, save_to: impl AsRef<Path>, sha1: Option<&str>) -> Result<()> {
    let res = get_download(url, sha1).await?;
    write_safe(save_to, &res).await
}

/// `checkFile` — does the file exist and match the expected sha1?
pub async fn check_file(path: impl AsRef<Path>, sha1: Option<&str>) -> bool {
    let path = get_path(path);
    match tokio::fs::metadata(&path).await {
        Ok(_) => {}
        Err(_) => return false,
    }
    match sha1 {
        None => true,
        Some(expected) => match tokio::fs::read(&path).await {
            Ok(data) => sha1_hex(&data) == expected,
            Err(_) => false,
        },
    }
}

// ---------------------------------------------------------------------------
// misc
// ---------------------------------------------------------------------------

/// `escapeShellCommand` — render an argv for human display.
///
/// Delegates to `shell_words::join`, which produces the same POSIX
/// single-quoting the original emitted by hand.
pub fn escape_shell_command(args: &[String]) -> String {
    shell_words::join(args)
}

/// `driveName` — the Windows drive prefix of `path` (`"C:"`), or `""` when
/// there is none. Only used to echo the `cd <drive>` part of the launch
/// command; matching on `Component::Prefix` makes the platform check
/// unnecessary.
pub fn drive_name(path: &Path) -> String {
    match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => {
            prefix.as_os_str().to_string_lossy().into_owned()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maven_coordinates_map_to_repository_paths() {
        assert_eq!(
            lib_name_to_path("com.example:demo:1.0"),
            "com/example/demo/1.0/demo-1.0.jar"
        );
        assert_eq!(
            lib_name_to_path("net.minecraftforge:forge:1.20.1-47.2.0"),
            "net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0.jar"
        );
        assert_eq!(
            lib_name_to_path("org.lwjgl:lwjgl:3.3.1:natives-linux@zip"),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.zip"
        );
    }

    #[test]
    fn maven_name_and_version_keep_classifiers() {
        assert_eq!(
            lib_name_get_name("org.lwjgl:lwjgl:3.3.1:natives-linux"),
            "org.lwjgl:lwjgl:natives-linux"
        );
        assert_eq!(
            lib_name_get_name("com.example:demo:1.0"),
            "com.example:demo"
        );
        assert_eq!(lib_name_get_version("com.example:demo:1.0@jar"), "1.0");
    }

    #[test]
    fn paths_are_absolute_and_normalised() {
        let path = get_path("a/./b/../c");
        assert!(path.is_absolute());
        assert!(path.ends_with("a/c"), "{path:?}");
    }

    #[test]
    fn sha1_hex_is_the_known_digest_of_empty_input() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn shell_escaping_quotes_only_when_needed() {
        assert_eq!(
            escape_shell_command(&["java".to_string(), "-Xmx2048m".to_string()]),
            "java -Xmx2048m"
        );
        assert_eq!(
            escape_shell_command(&["-Dfoo=bar baz".to_string()]),
            "'-Dfoo=bar baz'"
        );
    }
}
