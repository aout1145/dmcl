//! 安装 Mojang 的 Java 运行时（JRE）。
//!
//! 调用者决定装哪个组件、哪个平台、装到哪个目录，以及下载 `raw` 还是 `lzma`：
//!
//! ```no_run
//! use std::sync::Arc;
//! use dmcl_install::{install_jre_task, InstallOptions, JreFileFormat, JreInstallRequest};
//! use dmcl_mcmeta::os::{Arch, Os};
//! use dmcl_mcmeta::source::DownloadSource;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let options = Arc::new(InstallOptions::new("/tmp/.minecraft", DownloadSource::official()));
//! let request = JreInstallRequest::new(
//!     "java-runtime-gamma",
//!     (Os::current(), Arch::X86_64),
//!     "/tmp/.minecraft/runtime/gamma",
//! )
//! .with_format(JreFileFormat::Lzma);
//!
//! // 任务链最后返回安装好的 java 可执行文件路径。
//! let java = install_jre_task(options, request).run(()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! 目标目录按**增量缓存**处理：已经完整（sha1 / 大小一致）的文件会跳过，缺的、坏的重新
//! 下载；目录里其它文件不会被删除。安装失败时只删除入口（`bin/java` / `bin/javaw`，
//! Windows 带 `.exe`，macOS 在 `jre.bundle/Contents/Home/bin/` 下），避免半成品被误当成
//! 可用的 Java。
//!
//! 默认使用 lzma（没有 lzma 版的文件回退 `raw`）；调用者可以显式改成
//! [`JreFileFormat::Raw`]。

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use dmcl_download::{files_ok, Checksum, FileCheck};
use dmcl_mcmeta::os::{Arch, Os};
use dmcl_mcmeta::vanilla::{jre_platform_key, JreFileType, JreManifest, JreRuntimeManifest};
use dmcl_task::{task_fn, task_fn_io, TaskChain, TaskContext};

use crate::files::download_planned;
use crate::options::InstallOptions;
use crate::plan::{base_name, PlannedFile};

/// 运行时文件用 lzma 压缩版还是原始文件下载。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JreFileFormat {
    /// 优先下载 `downloads.lzma`；某个文件没有 lzma 版时回退到 `raw`。
    #[default]
    Lzma,
    /// 直接下载 `downloads.raw`。
    Raw,
}

/// 安装期找不到匹配 Java 时自动下载 JRE 的设置。
#[derive(Debug, Clone)]
pub struct JreAutoInstall {
    /// 官方启动器的 runtime 根目录。
    pub runtime_root: PathBuf,
    pub format: JreFileFormat,
}

impl JreAutoInstall {
    pub fn new(runtime_root: impl Into<PathBuf>) -> Self {
        Self {
            runtime_root: runtime_root.into(),
            format: JreFileFormat::default(),
        }
    }

    pub fn with_format(mut self, format: JreFileFormat) -> Self {
        self.format = format;
        self
    }
}

/// 一次 JRE 安装请求。
#[derive(Debug, Clone)]
pub struct JreInstallRequest {
    /// Mojang 的组件名，例如 `java-runtime-gamma`。
    pub component: String,
    /// 目标平台。
    pub platform: (Os, Arch),
    /// 安装目录，由调用者决定。
    pub target_dir: PathBuf,
    /// 下载格式。
    pub format: JreFileFormat,
}

impl JreInstallRequest {
    pub fn new(
        component: impl Into<String>,
        platform: (Os, Arch),
        target_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            component: component.into(),
            platform,
            target_dir: target_dir.into(),
            format: JreFileFormat::default(),
        }
    }

    pub fn with_format(mut self, format: JreFileFormat) -> Self {
        self.format = format;
        self
    }
}

/// 按 major 安装时的组件选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JreComponent {
    /// Mojang 的组件名，例如 `java-runtime-gamma`。
    pub name: String,
    /// 没有组件能匹配要求的大版本，退回了 `jre-legacy`。
    pub fallback: bool,
}

/// 按「组件缺失时按 major 挑」安装 JRE 的请求。
#[derive(Debug, Clone)]
pub struct JreForMajorRequest {
    /// 官方启动器的 runtime 根目录（调用者决定）。
    pub runtime_root: PathBuf,
    pub platform: (Os, Arch),
    /// 版本 JSON 的 `javaVersion.component`；没有就按 major 挑。
    pub preferred_component: Option<String>,
    pub required_major: u32,
    pub format: JreFileFormat,
}

impl JreForMajorRequest {
    pub fn new(
        runtime_root: impl Into<PathBuf>,
        platform: (Os, Arch),
        required_major: u32,
    ) -> Self {
        Self {
            runtime_root: runtime_root.into(),
            platform,
            preferred_component: None,
            required_major,
            format: JreFileFormat::default(),
        }
    }

    pub fn with_preferred_component(mut self, component: impl Into<String>) -> Self {
        self.preferred_component = Some(component.into());
        self
    }

    pub fn with_format(mut self, format: JreFileFormat) -> Self {
        self.format = format;
        self
    }
}

/// 安装完成的 JRE。
#[derive(Debug, Clone)]
pub struct JreInstalled {
    /// 实际安装的组件名。
    pub component: String,
    /// 安装好的 java 可执行文件。
    pub java: PathBuf,
    /// 组件是按 `jre-legacy` 兜底选的。
    pub fallback: bool,
}

/// 官方启动器的 runtime 目录布局：
/// `<runtime_root>/<component>/<Mojang 平台键>/<component>`。
pub fn runtime_dir(runtime_root: &Path, component: &str, platform: (Os, Arch)) -> Result<PathBuf> {
    let key = jre_platform_key(platform.0, platform.1)
        .ok_or_else(|| missing_runtime(component, platform))?;
    Ok(runtime_root.join(component).join(key).join(component))
}

/// 从已经取到的产品索引里选组件。
///
/// 顺序：`preferred`（版本 JSON 的 `component`）→ 条目版本号匹配 `required_major`
/// 且优先非 snapshot 的组件 → `jre-legacy` 兜底。
pub fn resolve_jre_component(
    index: &JreManifest,
    platform: (Os, Arch),
    preferred: Option<&str>,
    required_major: u32,
) -> Result<JreComponent> {
    let components = index
        .platforms
        .get(&platform)
        .ok_or_else(|| missing_runtime("Java", platform))?;

    if let Some(preferred) = preferred.filter(|name| !name.is_empty()) {
        if components
            .get(preferred)
            .is_some_and(|entries| !entries.is_empty())
        {
            return Ok(JreComponent {
                name: preferred.to_string(),
                fallback: false,
            });
        }
    }

    let mut snapshot = None;
    for (name, entries) in components {
        if name == "minecraft-java-exe" {
            continue;
        }
        let Some(major) = entries
            .first()
            .and_then(|entry| entry.version.as_ref())
            .and_then(|version| major_version_of(&version.name))
        else {
            continue;
        };
        if major != required_major {
            continue;
        }
        if name.ends_with("-snapshot") {
            snapshot.get_or_insert_with(|| name.clone());
        } else {
            return Ok(JreComponent {
                name: name.clone(),
                fallback: false,
            });
        }
    }

    if let Some(name) = snapshot {
        return Ok(JreComponent {
            name,
            fallback: false,
        });
    }

    Ok(JreComponent {
        name: "jre-legacy".to_string(),
        fallback: true,
    })
}

fn major_version_of(version: &str) -> Option<u32> {
    let mut parts = version.split('.');
    let first = leading_digits(parts.next()?)?;
    if first == 1 {
        leading_digits(parts.next()?)
    } else {
        Some(first)
    }
}

fn leading_digits(text: &str) -> Option<u32> {
    let digits: String = text.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn runtime_manifest_url<'a>(
    index: &'a JreManifest,
    component: &str,
    platform: (Os, Arch),
) -> Result<&'a str> {
    let entries = index
        .runtimes(component, platform)
        .ok_or_else(|| missing_runtime(component, platform))?;
    let entry = entries.first().ok_or_else(|| {
        anyhow!(
            "{component} 在 {} / {} 上没有可用的运行时条目",
            platform.0.mc_name(),
            platform.1.name()
        )
    })?;
    entry
        .manifest
        .as_ref()
        .map(|manifest| manifest.url.as_str())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| anyhow!("{component} 的运行时条目不包含清单地址"))
}

/// 按组件 / major 安装 JRE 到官方 runtime 布局，返回安装结果。
///
/// 和 [`install_jre`] 的区别：组件名可以不给，先按 `required_major` 从索引里挑；
/// 目标目录也按 `<runtime_root>/<component>/<平台键>/<component>` 算好。
pub async fn install_jre_for_major(
    options: &InstallOptions,
    request: &JreForMajorRequest,
    ctx: &TaskContext,
) -> Result<JreInstalled> {
    let index = options.source.fetch_jre_manifest().await?;
    let component = resolve_jre_component(
        &index,
        request.platform,
        request.preferred_component.as_deref(),
        request.required_major,
    )?;
    if component.fallback {
        ctx.log(format!(
            "警告：没有匹配 Java {} 的 Mojang 运行时，退回 {}",
            request.required_major, component.name
        ));
    }

    let url = runtime_manifest_url(&index, &component.name, request.platform)?;
    let manifest = options.source.fetch_jre_runtime_manifest(url).await?;
    let target_dir = runtime_dir(&request.runtime_root, &component.name, request.platform)?;
    let install = JreInstallRequest {
        component: component.name.clone(),
        platform: request.platform,
        target_dir,
        format: request.format,
    };
    let java = install_jre_manifest(options, &manifest, &install, ctx).await?;

    Ok(JreInstalled {
        component: component.name,
        java,
        fallback: component.fallback,
    })
}

/// 取产品索引里该组件 / 平台的运行时文件清单。
pub async fn fetch_jre_runtime_manifest(
    options: &InstallOptions,
    component: &str,
    platform: (Os, Arch),
) -> Result<JreRuntimeManifest> {
    let index = options.source.fetch_jre_manifest().await?;
    let url = runtime_manifest_url(&index, component, platform)?;
    options.source.fetch_jre_runtime_manifest(url).await
}

fn missing_runtime(component: &str, platform: (Os, Arch)) -> anyhow::Error {
    anyhow!(
        "Mojang 没有提供 {component} 在 {} / {} 上的 Java 运行时",
        platform.0.mc_name(),
        platform.1.name()
    )
}

/// 安装一个 JRE 到请求里的目标目录，返回 java 可执行文件路径。
///
/// 这是底层入口；需要进度事件时用 [`install_jre_task`]。
pub async fn install_jre(
    options: &InstallOptions,
    request: &JreInstallRequest,
    ctx: &TaskContext,
) -> Result<PathBuf> {
    let manifest =
        fetch_jre_runtime_manifest(options, &request.component, request.platform).await?;
    install_jre_manifest(options, &manifest, request, ctx).await
}

/// JRE 安装任务链：取运行时清单 → 下载并展开。
pub fn install_jre_task(
    options: Arc<InstallOptions>,
    request: JreInstallRequest,
) -> TaskChain<(), PathBuf> {
    let fetch_options = options.clone();
    let fetch_component = request.component.clone();
    let fetch_platform = request.platform;

    TaskChain::new(format!("安装 Java 运行时：{}", request.component))
        .then(
            task_fn(move |_ctx| {
                let options = fetch_options.clone();
                let component = fetch_component.clone();
                async move { fetch_jre_runtime_manifest(&options, &component, fetch_platform).await }
            }),
            "获取运行时清单",
        )
        .then(
            task_fn_io(move |manifest, ctx| {
                let options = options.clone();
                let request = request.clone();
                async move { install_jre_manifest(&options, &manifest, &request, &ctx).await }
            }),
            "下载并展开运行时",
        )
}

/// 下载 + 展开一份已经取到的文件清单。
async fn install_jre_manifest(
    options: &InstallOptions,
    manifest: &JreRuntimeManifest,
    request: &JreInstallRequest,
    ctx: &TaskContext,
) -> Result<PathBuf> {
    // 规划阶段只看文档、不写盘；这里失败时目标目录还是原样，不需要清理入口。
    let plan = plan_jre_files(manifest, &request.target_dir, request.format)?;
    match install_jre_plan(options, &plan, request, ctx).await {
        Ok(java) => Ok(java),
        Err(err) => {
            // 半成品不能看起来像一个可用的 Java。
            remove_java_entry_points(&request.target_dir);
            Err(err)
        }
    }
}

async fn install_jre_plan(
    options: &InstallOptions,
    plan: &JrePlan,
    request: &JreInstallRequest,
    ctx: &TaskContext,
) -> Result<PathBuf> {
    let target = &request.target_dir;

    for dir in &plan.dirs {
        tokio::fs::create_dir_all(dir)
            .await
            .with_context(|| format!("无法创建 {}", dir.display()))?;
    }

    // lzma 文件的下载项只校验压缩包；最终文件是否已经完整要单独判断。
    let checks: Vec<FileCheck> = plan
        .files
        .iter()
        .filter(|file| file.compressed)
        .map(|file| FileCheck {
            path: file.final_path.clone(),
            checksum: file.final_checksum.clone(),
            size: file.final_size,
        })
        .collect();
    let checked = files_ok(checks).await?;
    let mut done = vec![false; plan.files.len()];
    let mut index = 0;
    for (position, file) in plan.files.iter().enumerate() {
        if !file.compressed {
            continue;
        }
        done[position] = checked[index];
        index += 1;
    }

    let mut planned = Vec::new();
    for (position, file) in plan.files.iter().enumerate() {
        if done[position] {
            continue;
        }
        if let Some(download) = &file.download {
            planned.push(download.clone());
        }
    }
    download_planned(options, "Java 运行时", planned, ctx).await?;

    for (position, file) in plan.files.iter().enumerate() {
        if !file.compressed || done[position] {
            continue;
        }
        let download = file
            .download
            .as_ref()
            .expect("压缩文件的下载项在规划阶段已经填好");
        ctx.log(format!("解压 {}", file.relative.display()));
        decompress_lzma(download.save_to.clone(), file.final_path.clone()).await?;
    }

    for file in &plan.files {
        if file.executable {
            set_executable(&file.final_path).await?;
        }
    }

    for (path, link_target) in &plan.links {
        ctx.log(format!("创建链接 {}", path.display()));
        replace_link(path, link_target).await?;
    }

    let java = java_executable(target);
    if !java.is_file() {
        return Err(anyhow!("运行时安装完成后找不到 {}", java.display()));
    }

    // 全部成功后清掉压缩包的临时目录；失败时保留，下次重跑可以复用。
    let temp = target.join(TEMP_DIR);
    match tokio::fs::remove_dir_all(&temp).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => ctx.log(format!("警告：无法清理 {}：{err}", temp.display())),
    }

    Ok(java)
}

/// lzma 下载先落到目标目录里的这个隐藏目录，成功后再删掉。
const TEMP_DIR: &str = ".dmcl-tmp";

/// 一个运行时文件条目。
struct PlannedJreFile {
    /// 清单里的相对路径，只用于日志。
    relative: PathBuf,
    /// 最终落盘位置。
    final_path: PathBuf,
    /// 是否需要可执行位。
    executable: bool,
    /// 最终内容（未压缩）的校验值。
    final_checksum: Option<Checksum>,
    final_size: Option<u64>,
    /// 需要下载时用哪份数据。
    download: Option<PlannedFile>,
    /// 下载项是不是 lzma 压缩包。
    compressed: bool,
}

struct JrePlan {
    dirs: Vec<PathBuf>,
    files: Vec<PlannedJreFile>,
    links: Vec<(PathBuf, String)>,
}

/// 纯规划：把文件清单变成目录 / 链接 / 下载项。不做网络和磁盘操作。
fn plan_jre_files(
    manifest: &JreRuntimeManifest,
    target: &Path,
    format: JreFileFormat,
) -> Result<JrePlan> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut links = Vec::new();

    for (relative, entry) in &manifest.files {
        let relative_path = safe_relative_path(relative)?;
        let path = target.join(&relative_path);

        match entry.kind.as_ref() {
            Some(JreFileType::Directory) => dirs.push(path),
            Some(JreFileType::Link) => {
                let link_target = entry
                    .target
                    .as_deref()
                    .filter(|target| !target.is_empty())
                    .ok_or_else(|| anyhow!("链接条目缺少 target：{relative}"))?;
                validate_link_target(&relative_path, link_target)?;
                links.push((path, link_target.to_string()));
            }
            Some(JreFileType::File) => files.push(plan_jre_file(
                relative,
                relative_path,
                path,
                entry,
                target,
                format,
            )?),
            Some(JreFileType::Other(kind)) => {
                return Err(anyhow!("不认识的运行时文件类型：{kind}（{relative}）"))
            }
            None => return Err(anyhow!("运行时清单条目缺少 type：{relative}")),
        }
    }

    Ok(JrePlan { dirs, files, links })
}

fn plan_jre_file(
    relative: &str,
    relative_path: PathBuf,
    final_path: PathBuf,
    entry: &dmcl_mcmeta::vanilla::JreRuntimeFile,
    target: &Path,
    format: JreFileFormat,
) -> Result<PlannedJreFile> {
    let downloads = entry
        .downloads
        .as_ref()
        .ok_or_else(|| anyhow!("运行时文件缺少 downloads：{relative}"))?;
    let raw = downloads
        .raw
        .as_ref()
        .filter(|artifact| artifact.url.as_deref().is_some_and(|url| !url.is_empty()))
        .ok_or_else(|| anyhow!("运行时文件缺少 raw 下载信息：{relative}"))?;

    let (artifact, compressed) = match format {
        JreFileFormat::Raw => (raw, false),
        JreFileFormat::Lzma => match downloads
            .lzma
            .as_ref()
            .filter(|artifact| artifact.url.as_deref().is_some_and(|url| !url.is_empty()))
        {
            Some(lzma) => (lzma, true),
            None => (raw, false),
        },
    };

    let save_to = if compressed {
        lzma_temp_path(target, &relative_path)
    } else {
        final_path.clone()
    };

    Ok(PlannedJreFile {
        relative: relative_path.clone(),
        final_path,
        executable: entry.executable.unwrap_or(false),
        final_checksum: raw.sha1.clone().map(Checksum::sha1),
        final_size: raw.size,
        download: Some(PlannedFile {
            url: artifact.url.clone().unwrap_or_default(),
            save_to,
            checksum: artifact.sha1.clone().map(Checksum::sha1),
            size: artifact.size,
            name: base_name(&relative_path),
        }),
        compressed,
    })
}

/// 拒绝绝对路径、`..` 和盘符；只接受归档内的普通相对路径。
fn safe_relative_path(relative: &str) -> Result<PathBuf> {
    if relative.is_empty() {
        return Err(anyhow!("运行时清单里有空路径"));
    }
    let mut path = PathBuf::new();
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            _ => return Err(anyhow!("运行时清单里的路径不安全：{relative}")),
        }
    }
    if path.as_os_str().is_empty() {
        return Err(anyhow!("运行时清单里有空路径"));
    }
    Ok(path)
}

/// 链接目标可以带 `..`（运行时里很多链接指向兄弟目录），但不能是绝对路径。
fn validate_link_target(relative: &Path, target: &str) -> Result<()> {
    let path = Path::new(target);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(anyhow!(
            "运行时清单里的链接目标不安全：{} -> {target}",
            relative.display()
        ));
    }
    Ok(())
}

fn lzma_temp_path(target: &Path, relative: &Path) -> PathBuf {
    let mut path = target.join(TEMP_DIR).join(relative).into_os_string();
    path.push(".lzma");
    PathBuf::from(path)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut text = path.as_os_str().to_os_string();
    text.push(suffix);
    PathBuf::from(text)
}

fn java_executable(target: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        // macOS 的运行时是一个 .app bundle。
        target.join("jre.bundle/Contents/Home/bin/java")
    } else if cfg!(windows) {
        target.join("bin/java.exe")
    } else {
        target.join("bin/java")
    }
}

/// 安装失败时只删掉入口，避免半成品被当成可用的 Java。
fn remove_java_entry_points(target: &Path) {
    let bin = target.join("bin");
    for name in ["java", "javaw", "java.exe", "javaw.exe"] {
        let _ = std::fs::remove_file(bin.join(name));
    }
    let mac_bin = target.join("jre.bundle/Contents/Home/bin");
    for name in ["java", "javaw"] {
        let _ = std::fs::remove_file(mac_bin.join(name));
    }
}

async fn decompress_lzma(source: PathBuf, dest: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        use std::io::BufReader;
        use std::io::BufWriter;
        use std::io::Write;

        let input = std::fs::File::open(&source)
            .with_context(|| format!("无法打开 {}", source.display()))?;
        let mut input = BufReader::new(input);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建 {}", parent.display()))?;
        }
        let part = with_suffix(&dest, ".part");
        let output =
            std::fs::File::create(&part).with_context(|| format!("无法写入 {}", part.display()))?;
        let mut output = BufWriter::new(output);
        if let Err(err) = lzma_rs::lzma_decompress(&mut input, &mut output) {
            let _ = std::fs::remove_file(&part);
            return Err(err).with_context(|| format!("无法解压 {}", source.display()));
        }
        output.flush()?;
        drop(output);
        // Windows 的 rename 不会覆盖已有文件。
        let _ = std::fs::remove_file(&dest);
        std::fs::rename(&part, &dest).with_context(|| format!("无法写入 {}", dest.display()))?;
        Ok(())
    })
    .await
    .context("解压 lzma 的任务异常终止")?
}

#[cfg(unix)]
async fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("无法读取 {}", path.display()))?;
    let mode = metadata.permissions().mode();
    if mode & 0o111 == 0o111 {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(mode | 0o111);
    tokio::fs::set_permissions(path, permissions)
        .await
        .with_context(|| format!("无法设置可执行位：{}", path.display()))
}

#[cfg(not(unix))]
async fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

async fn replace_link(path: &Path, target: &str) -> Result<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                if tokio::fs::read_link(path).await.ok().as_deref() == Some(Path::new(target)) {
                    return Ok(());
                }
                tokio::fs::remove_file(path)
                    .await
                    .with_context(|| format!("无法替换 {}", path.display()))?;
            } else if metadata.is_dir() {
                tokio::fs::remove_dir_all(path)
                    .await
                    .with_context(|| format!("无法替换 {}", path.display()))?;
            } else {
                tokio::fs::remove_file(path)
                    .await
                    .with_context(|| format!("无法替换 {}", path.display()))?;
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("无法读取 {}", path.display())),
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    create_symlink(path, target).with_context(|| format!("无法创建链接 {}", path.display()))
}

#[cfg(unix)]
fn create_symlink(path: &Path, target: &str) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(windows)]
fn create_symlink(_path: &Path, _target: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Windows 的 Java 运行时清单包含意料之外的符号链接",
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use dmcl_download::{hash_bytes, HashAlgorithm};
    use dmcl_mcmeta::source::DownloadSource;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dmcl-install-jre-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn manifest(json: &str) -> JreRuntimeManifest {
        serde_json::from_str(json).unwrap()
    }

    /// 一个只会回固定响应的极简 HTTP 服务器；路径不在表里就返回 404。
    async fn serve(listener: TcpListener, responses: Arc<HashMap<String, Vec<u8>>>) {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let responses = responses.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 2048];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let body = responses.get(path);
                let (status, body): (&str, &[u8]) = match body {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", b""),
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body).await;
            });
        }
    }

    fn java_name() -> &'static str {
        if cfg!(target_os = "macos") {
            "jre.bundle/Contents/Home/bin/java"
        } else if cfg!(windows) {
            "bin/java.exe"
        } else {
            "bin/java"
        }
    }

    #[test]
    fn lzma_is_used_when_available_and_raw_is_the_fallback() {
        let manifest = manifest(
            r#"{
                "files": {
                    "bin": {"type": "directory"},
                    "bin/java": {
                        "type": "file",
                        "executable": true,
                        "downloads": {
                            "raw": {"sha1": "raw-java", "size": 10, "url": "https://example.com/raw/java"},
                            "lzma": {"sha1": "lzma-java", "size": 5, "url": "https://example.com/lzma/java"}
                        }
                    },
                    "lib/a.jar": {
                        "type": "file",
                        "downloads": {
                            "raw": {"sha1": "raw-a", "size": 20, "url": "https://example.com/raw/a"}
                        }
                    },
                    "legal/LICENSE": {"type": "link", "target": "../java.base/LICENSE"}
                }
            }"#,
        );
        let target = Path::new("/jre");

        let plan = plan_jre_files(&manifest, target, JreFileFormat::Lzma).unwrap();
        assert_eq!(plan.dirs, vec![PathBuf::from("/jre/bin")]);
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.links.len(), 1);

        let java = plan
            .files
            .iter()
            .find(|file| file.relative == Path::new("bin/java"))
            .unwrap();
        assert!(java.compressed);
        assert!(java.executable);
        let download = java.download.as_ref().unwrap();
        assert_eq!(download.url, "https://example.com/lzma/java");
        assert_eq!(
            download.save_to,
            PathBuf::from("/jre/.dmcl-tmp/bin/java.lzma")
        );
        // 最终文件按 raw 的校验值判断是否已经完整
        assert_eq!(java.final_checksum.as_ref().unwrap().hex(), "raw-java");
        assert_eq!(java.final_size, Some(10));

        // 没有 lzma 的文件回退到 raw
        let library = plan
            .files
            .iter()
            .find(|file| file.relative == Path::new("lib/a.jar"))
            .unwrap();
        assert!(!library.compressed);
        assert_eq!(
            library.download.as_ref().unwrap().url,
            "https://example.com/raw/a"
        );
        assert_eq!(
            library.download.as_ref().unwrap().save_to,
            PathBuf::from("/jre/lib/a.jar")
        );

        // Raw 格式一律用 raw
        let plan = plan_jre_files(&manifest, target, JreFileFormat::Raw).unwrap();
        let java = plan
            .files
            .iter()
            .find(|file| file.relative == Path::new("bin/java"))
            .unwrap();
        assert!(!java.compressed);
        assert_eq!(
            java.download.as_ref().unwrap().url,
            "https://example.com/raw/java"
        );
        assert_eq!(
            java.download.as_ref().unwrap().save_to,
            PathBuf::from("/jre/bin/java")
        );
    }

    #[test]
    fn unsafe_paths_and_link_targets_are_rejected() {
        for json in [
            r#"{"files": {"../evil": {"type": "directory"}}}"#,
            r#"{"files": {"/evil": {"type": "directory"}}}"#,
            r#"{"files": {"a": {"type": "link", "target": "/etc/passwd"}}}"#,
        ] {
            let err = plan_jre_files(&manifest(json), Path::new("/jre"), JreFileFormat::Raw)
                .err()
                .expect("应当拒绝不安全路径");
            assert!(format!("{err:#}").contains("不安全"), "{err:#}");
        }
    }

    #[test]
    fn components_are_chosen_by_preferred_then_major_then_fallback() {
        let index: JreManifest = serde_json::from_str(
            r#"{
                "linux": {
                    "java-runtime-delta": [{"version": {"name": "21.0.1"}}],
                    "java-runtime-gamma": [{"version": {"name": "17.0.9"}}],
                    "java-runtime-gamma-snapshot": [{"version": {"name": "17.0.9"}}],
                    "jre-legacy": [{"version": {"name": "1.8.0_411"}}],
                    "minecraft-java-exe": [{"version": {"name": "1.0"}}]
                }
            }"#,
        )
        .unwrap();
        let platform = (Os::Linux, Arch::X86_64);

        let delta = resolve_jre_component(&index, platform, None, 21).unwrap();
        assert_eq!(delta.name, "java-runtime-delta");
        assert!(!delta.fallback);

        // snapshot 不是稳定组件的对手
        let gamma = resolve_jre_component(&index, platform, None, 17).unwrap();
        assert_eq!(gamma.name, "java-runtime-gamma");

        let legacy = resolve_jre_component(&index, platform, None, 8).unwrap();
        assert_eq!(legacy.name, "jre-legacy");
        assert!(!legacy.fallback, "按版本号匹配到 jre-legacy 不算兜底");

        // 显式 preferred 优先，即使 major 对不上
        let preferred =
            resolve_jre_component(&index, platform, Some("java-runtime-delta"), 8).unwrap();
        assert_eq!(preferred.name, "java-runtime-delta");

        // 空 preferred 当作没给
        let empty = resolve_jre_component(&index, platform, Some(""), 17).unwrap();
        assert_eq!(empty.name, "java-runtime-gamma");

        // 没有任何组件匹配才退回 jre-legacy
        let fallback = resolve_jre_component(&index, platform, None, 30).unwrap();
        assert_eq!(fallback.name, "jre-legacy");
        assert!(fallback.fallback);
    }

    #[test]
    fn runtime_dirs_follow_the_official_launcher_layout() {
        let root = Path::new("runtime");
        assert_eq!(
            runtime_dir(root, "java-runtime-gamma", (Os::Linux, Arch::X86_64)).unwrap(),
            PathBuf::from("runtime/java-runtime-gamma/linux/java-runtime-gamma")
        );
        assert_eq!(
            runtime_dir(root, "java-runtime-gamma", (Os::Windows, Arch::X86_64)).unwrap(),
            PathBuf::from("runtime/java-runtime-gamma/windows-x64/java-runtime-gamma")
        );
        assert_eq!(
            runtime_dir(root, "java-runtime-gamma", (Os::MacOs, Arch::Aarch64)).unwrap(),
            PathBuf::from("runtime/java-runtime-gamma/mac-os-arm64/java-runtime-gamma")
        );
        // Mojang 没有 Linux arm64 运行时
        assert!(runtime_dir(root, "java-runtime-gamma", (Os::Linux, Arch::Aarch64)).is_err());
    }

    #[test]
    fn failed_installs_remove_only_the_java_entry_points() {
        let dir = temp_dir("cleanup");
        let bin = dir.join("bin");
        let mac_bin = dir.join("jre.bundle/Contents/Home/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&mac_bin).unwrap();
        for name in ["java", "javaw", "java.exe", "javaw.exe", "other"] {
            std::fs::write(bin.join(name), b"x").unwrap();
        }
        for name in ["java", "javaw", "other"] {
            std::fs::write(mac_bin.join(name), b"x").unwrap();
        }

        remove_java_entry_points(&dir);

        for name in ["java", "javaw", "java.exe", "javaw.exe"] {
            assert!(!bin.join(name).exists(), "{name} 应当被删掉");
        }
        for name in ["java", "javaw"] {
            assert!(!mac_bin.join(name).exists(), "macOS 的 {name} 应当被删掉");
        }
        assert!(bin.join("other").exists(), "不能动其它文件");
        assert!(mac_bin.join("other").exists(), "不能动其它文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn lzma_files_are_decompressed_into_place() {
        // `printf 'hello java runtime\n' | xz --format=lzma -c`
        const COMPRESSED: &[u8] = &[
            0x5d, 0x00, 0x00, 0x80, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00,
            0x34, 0x19, 0x49, 0xee, 0x8d, 0xe9, 0x13, 0xfa, 0xb6, 0x7e, 0x50, 0x55, 0x7f, 0x39,
            0x32, 0x18, 0xce, 0x57, 0x16, 0xb8, 0x3c, 0xee, 0xee, 0xff, 0xff, 0x6f, 0x7c, 0x00,
            0x00,
        ];
        let dir = temp_dir("lzma");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("java.lzma");
        tokio::fs::write(&source, COMPRESSED).await.unwrap();

        let dest = dir.join("bin/java");
        decompress_lzma(source, dest.clone()).await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&dest).await.unwrap(),
            "hello java runtime\n"
        );
        assert!(
            !with_suffix(&dest, ".part").exists(),
            "解压完成后不应留下 .part"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn executable_files_get_the_execute_bits() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("exec");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("java");
        tokio::fs::write(&path, b"x").await.unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        set_executable(&path).await.unwrap();

        let mode = tokio::fs::metadata(&path)
            .await
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
        // 原有的读写位保留
        assert_eq!(mode & 0o600, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn existing_links_are_replaced_when_the_target_changes() {
        let dir = temp_dir("link");
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("b")).unwrap();
        let path = dir.join("current");
        std::fs::write(dir.join("a/target"), b"a").unwrap();
        std::fs::write(dir.join("b/target"), b"b").unwrap();

        replace_link(&path, "a/target").await.unwrap();
        assert_eq!(
            std::fs::read_link(&path).unwrap(),
            PathBuf::from("a/target")
        );

        // 目标没变时不重建
        replace_link(&path, "a/target").await.unwrap();
        assert_eq!(
            std::fs::read_link(&path).unwrap(),
            PathBuf::from("a/target")
        );

        replace_link(&path, "b/target").await.unwrap();
        assert_eq!(
            std::fs::read_link(&path).unwrap(),
            PathBuf::from("b/target")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_jre_is_downloaded_decompressed_and_repaired_incrementally() {
        const RAW: &[u8] = b"hello java runtime\n";
        // `printf 'hello java runtime\n' | xz --format=lzma -c`
        const COMPRESSED: &[u8] = &[
            0x5d, 0x00, 0x00, 0x80, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00,
            0x34, 0x19, 0x49, 0xee, 0x8d, 0xe9, 0x13, 0xfa, 0xb6, 0x7e, 0x50, 0x55, 0x7f, 0x39,
            0x32, 0x18, 0xce, 0x57, 0x16, 0xb8, 0x3c, 0xee, 0xee, 0xff, 0xff, 0x6f, 0x7c, 0x00,
            0x00,
        ];
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut responses = HashMap::new();
        responses.insert("/java.raw".to_string(), RAW.to_vec());
        responses.insert("/java.lzma".to_string(), COMPRESSED.to_vec());
        let server = tokio::spawn(serve(listener, Arc::new(responses)));

        let json = format!(
            r#"{{
                "files": {{
                    "bin": {{"type": "directory"}},
                    "{}": {{
                        "type": "file",
                        "executable": true,
                        "downloads": {{
                            "raw": {{
                                "sha1": "{}", "size": {},
                                "url": "http://{addr}/java.raw"
                            }},
                            "lzma": {{
                                "sha1": "{}", "size": {},
                                "url": "http://{addr}/java.lzma"
                            }}
                        }}
                    }}
                }}
            }}"#,
            java_name(),
            hash_bytes(HashAlgorithm::Sha1, RAW),
            RAW.len(),
            hash_bytes(HashAlgorithm::Sha1, COMPRESSED),
            COMPRESSED.len(),
        );
        let manifest = manifest(&json);
        let dir = temp_dir("install");
        let options = InstallOptions::new(&dir, DownloadSource::official());
        let request = JreInstallRequest::new("test", (Os::Linux, Arch::X86_64), &dir);
        let ctx = TaskContext::detached("测试");

        let java = install_jre_manifest(&options, &manifest, &request, &ctx)
            .await
            .unwrap();
        assert_eq!(java, java_executable(&dir));
        assert_eq!(
            tokio::fs::read_to_string(&java).await.unwrap(),
            "hello java runtime\n"
        );
        assert!(!dir.join(TEMP_DIR).exists(), "临时目录应当被清理");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&java)
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }

        // 停掉服务器后重跑：最终文件已经完整，不应再访问网络。
        server.abort();
        install_jre_manifest(&options, &manifest, &request, &ctx)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&java).await.unwrap(),
            "hello java runtime\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_failed_download_removes_the_java_entry_points() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // 所有请求都 404
        let server = tokio::spawn(serve(listener, Arc::new(HashMap::new())));

        let json = format!(
            r#"{{
                "files": {{
                    "{}": {{
                        "type": "file",
                        "downloads": {{
                            "raw": {{
                                "sha1": "{}", "size": 1,
                                "url": "http://{addr}/missing"
                            }}
                        }}
                    }}
                }}
            }}"#,
            java_name(),
            hash_bytes(HashAlgorithm::Sha1, b"x"),
        );
        let manifest = manifest(&json);
        let dir = temp_dir("fail");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/java"), b"old").unwrap();

        let options = InstallOptions::new(&dir, DownloadSource::official());
        let request = JreInstallRequest::new("test", (Os::Linux, Arch::X86_64), &dir);
        let err = install_jre_manifest(
            &options,
            &manifest,
            &request,
            &TaskContext::detached("测试"),
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("下载失败"), "{err:#}");
        assert!(!dir.join("bin/java").exists(), "半成品入口应当被删掉");

        server.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
