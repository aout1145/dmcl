//! Forge/NeoForge installer "processor" execution.
//!
//! Both `installForge` and `installNeoForge` contain a byte-for-byte identical
//! block that downloads the installer libraries and then runs the JVM
//! processors described in `install_profile.json`. It is factored out here so
//! the algorithm exists once; the callers keep the exact original ordering
//! (including the Forge-only `data/client.lzma` guard).

use std::io::Cursor;
use std::path::Path;

use anyhow::{anyhow, Result};
use tokio::process::Command;

use super::game::download_libraries;
use super::mirrors::{mirror_url, Mirror};
use super::multitasks::{download_file, wait_downloads, DownloadFileMeta};
use crate::launch::select_java;
use crate::mcjson::{InstallProfile, VersionJson};
use crate::utils::{
    check_file, classpath_separator, lib_name_to_path, version_jar_path, write_safe, zip_read_entry,
};

/// Download the installer libraries, extract `client.lzma` and execute every
/// client-side processor of `install_profile.json`.
pub(crate) async fn run_installer_processors(
    mirror: Mirror,
    root_path: &Path,
    vername: &str,
    forgever: &str,
    profile: &InstallProfile,
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    mc_json: &VersionJson,
) -> Result<()> {
    let libraries_dir = root_path.join("libraries");

    // download libraries declared by the installer profile
    for lib in &profile.libraries {
        let Some(artifact) = lib.downloads.as_ref().and_then(|d| d.artifact.as_ref()) else {
            continue;
        };
        download_file(DownloadFileMeta::new(
            mirror_url(mirror, None, artifact.url.as_deref().unwrap_or("")),
            libraries_dir.join(artifact.path.as_deref().unwrap_or("")),
            artifact.sha1.clone(),
        ));
    }
    download_libraries(mirror, root_path, vername).await?;
    wait_downloads().await;

    // extract client.lzma into a scratch directory
    let tmp_path = std::env::temp_dir().join(format!("dmcl-forge-{forgever}-autoinstall"));
    let lzma_path = tmp_path.join("client.lzma");
    write_safe(&lzma_path, &zip_read_entry(zip, "data/client.lzma")?).await?;

    let java = select_java(mc_json.required_java_major()).await?;

    // `getArg` from the original: resolve `{...}` placeholders, the lzma path,
    // `[maven:coord]` classpath entries and surrounding single quotes.
    let get_arg = |arg: &str| -> String {
        let mut arg = arg.to_string();
        if arg.len() >= 2 && arg.starts_with('{') && arg.ends_with('}') {
            let inner = &arg[1..arg.len() - 1];
            arg = match inner {
                "MINECRAFT_JAR" => version_jar_path(root_path, vername)
                    .to_string_lossy()
                    .into_owned(),
                "SIDE" => "client".to_string(),
                key => profile
                    .data
                    .get(key)
                    .and_then(|entry| entry.client())
                    .unwrap_or("")
                    .to_string(),
            };
        }
        if arg == "/data/client.lzma" {
            arg = lzma_path.to_string_lossy().into_owned();
        }
        if arg.len() >= 2 && arg.starts_with('[') && arg.ends_with(']') {
            let inner = &arg[1..arg.len() - 1];
            arg = libraries_dir
                .join(lib_name_to_path(inner))
                .to_string_lossy()
                .into_owned();
        }
        if arg.len() >= 2 && arg.starts_with('\'') && arg.ends_with('\'') {
            arg = arg[1..arg.len() - 1].to_string();
        }
        arg
    };

    for step in &profile.processors {
        if !step.is_client_side() {
            continue;
        }
        let jar_coord = step.jar.as_str();
        let jar_path = libraries_dir.join(lib_name_to_path(jar_coord));

        // classpath: the processor jar first, then its classpath entries
        let mut cp_str = jar_path.to_string_lossy().into_owned();
        for cp in &step.classpath {
            cp_str.push(classpath_separator());
            cp_str.push_str(&libraries_dir.join(lib_name_to_path(cp)).to_string_lossy());
        }

        // arguments
        let args: Vec<String> = step.args.iter().map(|arg| get_arg(arg)).collect();

        // main class, read from the processor jar manifest
        let jar_bytes = tokio::fs::read(&jar_path).await?;
        let mut jar = zip::ZipArchive::new(Cursor::new(jar_bytes))?;
        let manifest = {
            let bytes = zip_read_entry(&mut jar, "META-INF/MANIFEST.MF")?;
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let mainclass = manifest
            .split("Main-Class: ")
            .nth(1)
            .unwrap_or("")
            .split('\n')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();

        // execute
        let output = Command::new(&java.path)
            .arg("-cp")
            .arg(&cp_str)
            .arg(&mainclass)
            .args(&args)
            .current_dir(&tmp_path)
            .output()
            .await?;
        if !output.status.success() {
            println!("{}", String::from_utf8_lossy(&output.stdout));
            println!("{}", String::from_utf8_lossy(&output.stderr));
            return Err(anyhow!(
                "failed to run {jar_coord}. command: {} -cp {} {} {}",
                java.path.display(),
                cp_str,
                mainclass,
                args.join(" ")
            ));
        }

        // checksums
        if let Some(outputs) = &step.outputs {
            for (key, value) in outputs {
                let file = get_arg(key);
                let expected = get_arg(value);
                if !check_file(&file, Some(&expected)).await {
                    return Err(anyhow!("{file}: wrong sha1sum"));
                }
            }
        }

        println!("finished {jar_coord}");
    }
    Ok(())
}
