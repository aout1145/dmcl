//! Command line generation and game launching (was `dmcl.launch.launcher`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use version_compare::{compare_to, Cmp};

use crate::account::offline::sha1_uuid;
use crate::env::{LAUNCHER_NAME, LAUNCHER_VERSION};
use crate::mcjson::{rules_allow, Argument, VersionJson};
use crate::utils::{
    classpath_separator, drive_name, escape_shell_command, extract_zip, get_os_name,
    lib_name_get_name, lib_name_get_version, lib_name_to_path, read_version_json_safe, version_dir,
};

use super::java::{select_java, JavaOption};
use super::monitor::GameMonitor;
use super::option::LaunchOption;

/// `GameLauncher`.
pub struct GameLauncher {
    pub option: LaunchOption,
    /// `versions/<version_name>`.
    pub version_path: PathBuf,
    /// Scratch directory the native libraries are unpacked into.
    pub natives_path: PathBuf,
    pub java: Option<JavaOption>,
    pub json: VersionJson,
}

impl GameLauncher {
    /// `this(LaunchOption)`.
    pub async fn new(option: LaunchOption) -> Result<Self> {
        let version_path = version_dir(&option.root_path, &option.version_name);
        let natives_path =
            std::env::temp_dir().join(format!("dmcl-natives-{}", sha1_uuid(&option.version_name)));
        let json = read_version_json_safe(&option.root_path, &option.version_name).await?;
        Ok(Self {
            option,
            version_path,
            natives_path,
            java: None,
            json,
        })
    }

    /// `getLibraries` — build the classpath, extracting native libraries on the
    /// way. The result is computed once (the D version recomputed it for every
    /// single argument while replacing `${classpath}`).
    ///
    /// The classpath itself is a *command line value*, so it stays a `String`
    /// joined with the platform separator; only the individual entries are
    /// paths.
    pub async fn get_libraries(&self) -> Result<String> {
        let java = self
            .java
            .as_ref()
            .ok_or_else(|| anyhow!("java is not selected yet"))?;
        let libraries_dir = self.option.root_path.join("libraries");

        // name -> (version, absolute path)
        let mut libs: BTreeMap<String, (String, PathBuf)> = BTreeMap::new();

        for lib in &self.json.libraries {
            if !rules_allow(&lib.rules) {
                continue;
            }
            let coord = lib.name.as_str();
            let name = lib_name_get_name(coord);
            let ver = lib_name_get_version(coord);
            // `compareVersion(ver, libs[name].ver) > 0`, delegated to
            // `version-compare`; a version that cannot be parsed keeps the
            // entry that is already there.
            let better = match libs.get(&name) {
                None => true,
                Some((existing, _)) => compare_to(&ver, existing, Cmp::Gt).unwrap_or(false),
            };

            if let Some(downloads) = &lib.downloads {
                // vanilla libs
                if let Some(artifact) = &downloads.artifact {
                    if better {
                        libs.insert(
                            name,
                            (
                                ver,
                                libraries_dir.join(artifact.path.as_deref().unwrap_or("")),
                            ),
                        );
                    }
                }
            } else if better {
                // modloader libs
                libs.insert(name, (ver, libraries_dir.join(lib_name_to_path(coord))));
            }

            // native libs: extract
            let native_jar = lib
                .natives
                .as_ref()
                .and_then(|natives| natives.get(get_os_name()))
                .map(|native_name| native_name.replace("${arch}", &java.arch.to_string()))
                .and_then(|native_name| {
                    lib.downloads
                        .as_ref()
                        .and_then(|d| d.classifiers.as_ref())
                        .and_then(|classifiers| classifiers.get(&native_name))
                        .and_then(|artifact| artifact.path.clone())
                });
            if let Some(native_jar) = native_jar {
                extract_zip(libraries_dir.join(native_jar), self.natives_path.clone()).await?;
            }
        }

        let mut result = String::new();
        for (_, path) in libs.values() {
            result.push_str(&path.to_string_lossy());
            result.push_str("${classpath_separator}");
        }
        if let Some(oriver) = self.json.inherits_from.as_deref() {
            // if json based on another, add it to cp
            let parent_jar =
                version_dir(&self.option.root_path, oriver).join(format!("{oriver}.jar"));
            result.push_str(&parent_jar.to_string_lossy());
            result.push_str("${classpath_separator}");
        }
        let self_jar = self
            .version_path
            .join(format!("{}.jar", self.option.version_name));
        if tokio::fs::metadata(&self_jar).await.is_ok() {
            result.push_str(&self_jar.to_string_lossy());
            result.push_str("${classpath_separator}");
        }

        // `replaceLast("${classpath_separator}", "")`
        const SEP: &str = "${classpath_separator}";
        if let Some(idx) = result.rfind(SEP) {
            result.replace_range(idx..idx + SEP.len(), "");
        }
        Ok(result)
    }

    /// `getArguments`.
    pub fn get_arguments(&self, key: &str) -> Vec<String> {
        let mut args = Vec::new();
        let Some(json_args) = self.json.arguments_for(key) else {
            // 1.12.2 or above: no `arguments` object at all
            return args;
        };
        for arg in json_args {
            match arg {
                Argument::Plain(value) => args.push(value.clone()),
                Argument::Ruled(ruled) => {
                    // `else if ("rules" in arg && checkRules(arg["rules"]))`
                    if ruled.rules.is_some() && rules_allow(&ruled.rules) {
                        if let Some(value) = &ruled.value {
                            value.push_into(&mut args);
                        }
                    }
                }
                Argument::Other(_) => {}
            }
        }
        args
    }

    /// `genArgs`.
    pub async fn gen_args(&mut self) -> Result<Vec<String>> {
        let mut args: Vec<String> = Vec::new();

        // java path
        let java = select_java(self.json.required_java_major()).await?;
        args.push(java.path.to_string_lossy().into_owned());
        self.java = Some(java);

        // jvm args
        if self.json.uses_arguments() {
            // 1.13 or later
            args.extend(self.get_arguments("jvm"));
        } else {
            // 1.12.2 or above
            args.push("-Djava.library.path=${natives_directory}".to_string());
            args.push("-cp".to_string());
            args.push("${classpath}".to_string());
        }

        // additional jvm args (the D `split(" ")`, minus the empty tokens a
        // run of spaces used to produce)
        args.extend(
            self.option
                .additional_jvm
                .split_whitespace()
                .map(str::to_string),
        );
        // memory limits
        args.push(format!("-Xmn{}m", self.option.min_memory));
        args.push(format!("-Xmx{}m", self.option.max_memory));
        // main class
        args.push(self.json.main_class.clone().unwrap_or_default());

        // game args
        if self.json.uses_arguments() {
            // 1.13 or later
            args.extend(self.get_arguments("game"));
        } else {
            // 1.12.2 or above
            args.extend(
                self.json
                    .minecraft_arguments
                    .as_deref()
                    .unwrap_or("")
                    .split_whitespace()
                    .map(str::to_string),
            );
        }

        // width and height
        args.push("--width".to_string());
        args.push(self.option.window_width.to_string());
        args.push("--height".to_string());
        args.push(self.option.window_height.to_string());
        // additional game args
        args.extend(
            self.option
                .additional_game
                .split_whitespace()
                .map(str::to_string),
        );

        // templates: these are all substituted *into* command line arguments,
        // so they have to be strings
        let classpath = self.get_libraries().await?;
        let natives_directory = self.natives_path.to_string_lossy().into_owned();
        let library_directory = self
            .option
            .root_path
            .join("libraries")
            .to_string_lossy()
            .into_owned();
        let assets_root = self
            .option
            .root_path
            .join("assets")
            .to_string_lossy()
            .into_owned();
        let asset_index_id = self.json.asset_index_id().to_string();
        let game_assets = if asset_index_id == "legacy" {
            self.option
                .root_path
                .join("assets")
                .join("virtual")
                .join("legacy")
        } else {
            self.option.root_path.join("assets")
        }
        .to_string_lossy()
        .into_owned();
        let version_id = self.json.id.clone().unwrap_or_default();
        let version_path = self.version_path.to_string_lossy().into_owned();
        let account = self.option.account.clone();
        let custom_info = self.option.custom_info.clone();
        let cp_sep = classpath_separator().to_string();

        for arg in args.iter_mut() {
            *arg = arg
                .replace("${natives_directory}", &natives_directory)
                .replace("${launcher_name}", LAUNCHER_NAME)
                .replace("${launcher_version}", LAUNCHER_VERSION)
                .replace("${classpath}", &classpath)
                .replace("${library_directory}", &library_directory)
                .replace("${auth_player_name}", account.get_name())
                .replace("${auth_uuid}", &account.get_uuid())
                .replace("${uuid}", &account.get_uuid())
                .replace("${version_name}", &version_id)
                .replace("${game_directory}", &version_path)
                .replace("${assets_root}", &assets_root)
                .replace("${assets_index_name}", &asset_index_id)
                .replace("${auth_access_token}", &account.get_access_token())
                .replace("${auth_session}", &account.get_access_token())
                .replace("${user_type}", account.get_type())
                .replace("${version_type}", &custom_info)
                .replace("${user_properties}", "{}")
                .replace("${game_assets}", &game_assets)
                .replace("${classpath_separator}", &cp_sep);
        }
        Ok(args)
    }

    /// `generateCommand` — the escaped command line, for a front-end that wants
    /// to show or hand off the launch command without spawning it. The CLI does
    /// not call this (it goes straight through [`Self::launch_game`]), it exists
    /// for the future GUI.
    pub async fn generate_command(&mut self) -> Result<String> {
        Ok(escape_shell_command(&self.gen_args().await?))
    }

    /// `launchGame` — start the game process. Unlike the D version, stdio is
    /// inherited so the player sees the game console, and the returned
    /// [`GameMonitor`] can be awaited.
    pub async fn launch_game(&mut self) -> Result<GameMonitor> {
        let args = self.gen_args().await?;
        if args.is_empty() {
            return Err(anyhow!("empty command"));
        }
        println!(
            "command: {} && {} && {}",
            escape_shell_command(&["cd".to_string(), drive_name(&self.version_path)]),
            escape_shell_command(&[
                "cd".to_string(),
                self.version_path.to_string_lossy().into_owned()
            ]),
            escape_shell_command(&args)
        );

        let mut command = tokio::process::Command::new(&args[0]);
        command.args(&args[1..]).current_dir(&self.version_path);
        let child = command.spawn()?;
        Ok(GameMonitor::new(child))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "keep the newest duplicate library" step now relies on
    /// `version-compare`. These are the version shapes actually found in
    /// Minecraft version jsons, pinning the semantics we depend on.
    #[test]
    fn library_versions_are_ordered_by_version_compare() {
        assert!(compare_to("1.20.1", "1.20.10", Cmp::Lt).unwrap());
        assert!(compare_to("1.20.10", "1.20.1", Cmp::Gt).unwrap());
        assert!(compare_to("3.3.2", "3.3.1", Cmp::Gt).unwrap());
        assert!(compare_to("1.20.1", "1.20.1", Cmp::Eq).unwrap());
        // forge-style coordinates with a build number
        assert!(compare_to("1.20.1-47.3.0", "1.20.1-47.2.0", Cmp::Gt).unwrap());
        // trailing zeros do not make a version bigger ("1.0" == "1.0.0"),
        // which is also what the old D implementation concluded
        assert!(compare_to("1.0", "1.0.0", Cmp::Eq).unwrap());
        // ... but a non-zero extra part does
        assert!(compare_to("1.0", "1.0.1", Cmp::Lt).unwrap());
    }

    /// `split_whitespace` replaces the hand-written `split_space`: the empty
    /// input case (the only one that actually matters, since
    /// `launch.additional_game` defaults to `""`) behaves like the D `split`,
    /// while runs of spaces no longer produce empty arguments.
    #[test]
    fn argument_strings_split_like_d_split() {
        let split = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert!(split("").is_empty());
        assert_eq!(split("a b"), vec!["a", "b"]);
        assert_eq!(split("  a   b  "), vec!["a", "b"]);
        // the D `split(" ")` produced `["a", "", "b"]` here
        assert_eq!(split("a  b"), vec!["a", "b"]);
    }
}
