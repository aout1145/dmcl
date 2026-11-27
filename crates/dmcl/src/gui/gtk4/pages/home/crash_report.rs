//! 游戏异常退出的崩溃日志对话框与导出。
//!
//! 日志来源是启动器捕获的会话日志（见 `dmcl_launch::ProcessOptions::log_path`）。
//! 对话框读取完整日志；导出的 zip 以它为 `output.log`，并附带 Minecraft 自己的
//! `latest.log`、崩溃报告和启动器 / 系统信息。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::gtk::gio;
use relm4::gtk::glib;
use zip::write::SimpleFileOptions;

use dmcl_launch::PreparedLaunch;

/// 没有捕获到日志时显示在日志区里的文字。
const NO_LOG: &str = "没有捕获到日志。";
/// 最多附带几份 Minecraft 崩溃报告。
const MAX_CRASH_REPORTS: usize = 5;

/// 一次异常退出的现场。
#[derive(Debug, Clone)]
pub(crate) struct CrashReport {
    pub instance: String,
    /// 已经格式化好的退出状态。
    pub exit: String,
    pub launch: Arc<PreparedLaunch>,
    /// 会话日志文件；没有重定向或创建失败时为 `None`。
    pub log_path: Option<PathBuf>,
}

/// 把退出结果格式化成对话框和 `launch.txt` 里的一行文字。
pub(crate) fn format_exit_status(status: &Result<std::process::ExitStatus, String>) -> String {
    match status {
        Ok(status) => match status.code() {
            Some(code) => format!("退出码 {code}"),
            None => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(signal) = status.signal() {
                        return format!("被信号 {signal} 终止");
                    }
                }
                "异常终止".to_string()
            }
        },
        Err(err) => format!("无法获取退出状态：{err}"),
    }
}

/// 弹出崩溃日志对话框。
pub(crate) fn show(
    parent: &impl IsA<gtk::Widget>,
    report: &CrashReport,
    on_exported: impl Fn(std::result::Result<PathBuf, String>) + Clone + 'static,
) {
    let dialog = adw::Dialog::new();
    let title = format!("{} 异常退出", report.instance);
    dialog.set_title(&title);
    dialog.set_content_width(760);
    dialog.set_content_height(560);

    let log_view = gtk::TextView::new();
    log_view.set_editable(false);
    log_view.set_cursor_visible(false);
    log_view.set_monospace(true);
    log_view.set_wrap_mode(gtk::WrapMode::WordChar);
    log_view.set_left_margin(6);
    log_view.set_right_margin(6);
    log_view.buffer().set_text("正在读取日志…");

    relm4::view! {
        toolbar = adw::ToolbarView {
            add_top_bar = &adw::HeaderBar {},

            #[wrap(Some)]
            set_content = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,

                append: log_area = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_vexpand: true,
                    set_margin_top: 12,
                    set_margin_start: 18,
                    set_margin_end: 18,
                },

                append = &gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::End,
                    set_margin_top: 12,
                    set_margin_bottom: 12,
                    set_margin_start: 12,
                    set_margin_end: 12,

                    append: export_button = &gtk::Button {
                        set_label: "导出崩溃日志…",
                        set_tooltip_text: Some("把日志、崩溃报告和启动信息保存为 zip"),
                        add_css_class: "suggested-action",
                    },
                },
            },
        }
    }

    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .child(&log_view)
        .build();
    log_area.append(&scrolled);

    dialog.set_child(Some(&toolbar));

    // 日志可能很大：放到阻塞线程里整份读取，读完再回 GTK 主线程设置文本。
    let log_path = report.log_path.clone();
    let buffer = log_view.buffer();
    let scrolled_for_log = scrolled;
    glib::spawn_future_local(async move {
        let loaded = tokio::task::spawn_blocking(move || read_log(log_path.as_deref())).await;
        let text = loaded.ok().flatten();
        buffer.set_text(text.as_deref().unwrap_or(NO_LOG));
        if let Some(text) = &text {
            apply_log_highlight(&buffer, text);
        }
        // 等这一轮布局完成后再滚到最后，保证停在最新的一行。
        let scroll_buffer = buffer.clone();
        glib::idle_add_local_once(move || {
            let mut end = scroll_buffer.end_iter();
            log_view.scroll_to_iter(&mut end, 0.0, false, 0.0, 0.0);
            let adjustment = scrolled_for_log.vadjustment();
            adjustment.set_value(adjustment.upper());
        });
    });

    let report = report.clone();
    let dialog_for_export = dialog.clone();
    export_button.connect_clicked(move |_| {
        pick_export_path(&dialog_for_export, &report, on_exported.clone());
    });

    dialog.present(Some(parent));
}

/// 读取完整日志；文件不存在、不可读或为空时返回 `None`。
fn read_log(path: Option<&Path>) -> Option<String> {
    let bytes = std::fs::read(path?).ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// 日志行的语义级别；只用于显示着色，默认级别不建标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogLevel {
    Error,
    Warning,
    Muted,
}

/// 判断日志行属于哪种级别。
///
/// 兼容 Minecraft 常见的 `[main/ERROR]`、`[Render thread/WARN]`、`[ERROR]` 等格式，
/// 以及 Java 异常堆栈的开头行。
fn classify_log_line(line: &str) -> Option<LogLevel> {
    let line = line.trim_start();
    if line.starts_with("WARNING:")
        || line.starts_with("WARN:")
        || line.starts_with("ERROR:")
        || line.starts_with("FATAL:")
    {
        return Some(
            if line.starts_with("WARNING:") || line.starts_with("WARN:") {
                LogLevel::Warning
            } else {
                LogLevel::Error
            },
        );
    }
    for (token, level) in [
        ("/FATAL]", LogLevel::Error),
        ("[FATAL]", LogLevel::Error),
        ("/ERROR]", LogLevel::Error),
        ("[ERROR]", LogLevel::Error),
        ("/WARN]", LogLevel::Warning),
        ("[WARN]", LogLevel::Warning),
        ("/DEBUG]", LogLevel::Muted),
        ("[DEBUG]", LogLevel::Muted),
        ("/TRACE]", LogLevel::Muted),
        ("[TRACE]", LogLevel::Muted),
    ] {
        if line.contains(token) {
            return Some(level);
        }
    }
    if line.contains("Minecraft Crash Report")
        || line.contains("Exception")
        || line.contains("Error:")
        || line.starts_with("Caused by:")
        || line.starts_with("Suppressed:")
        || line.starts_with("Exception in thread")
        || line.starts_with("at ")
        || line.starts_with("... ")
        || line.starts_with("java.")
    {
        return Some(LogLevel::Error);
    }
    None
}

/// 给完整日志着色：错误红色加粗，警告黄色，调试 / 跟踪灰色，其余用默认前景色。
fn apply_log_highlight(buffer: &gtk::TextBuffer, text: &str) {
    let dark = adw::StyleManager::default().is_dark();
    let table = buffer.tag_table();
    let error = log_tag(
        &table,
        "dmcl-log-error",
        if dark { "#ff7b72" } else { "#c01c28" },
        Some(700),
    );
    let warning = log_tag(
        &table,
        "dmcl-log-warning",
        if dark { "#f5c211" } else { "#9a6700" },
        None,
    );
    let muted = log_tag(
        &table,
        "dmcl-log-muted",
        if dark { "#9a9996" } else { "#77767b" },
        None,
    );

    let mut start = buffer.start_iter();
    for line in text.split_inclusive('\n') {
        let mut end = start;
        end.forward_chars(line.chars().count() as i32);
        match classify_log_line(line) {
            Some(LogLevel::Error) => buffer.apply_tag(&error, &start, &end),
            Some(LogLevel::Warning) => buffer.apply_tag(&warning, &start, &end),
            Some(LogLevel::Muted) => buffer.apply_tag(&muted, &start, &end),
            None => {}
        }
        start = end;
    }
}

fn log_tag(
    table: &gtk::TextTagTable,
    name: &str,
    color: &str,
    weight: Option<i32>,
) -> gtk::TextTag {
    let tag = gtk::TextTag::new(Some(name));
    tag.set_foreground(Some(color));
    if let Some(weight) = weight {
        tag.set_weight(weight);
    }
    table.add(&tag);
    tag
}

/// 让用户选保存位置，然后在后台生成 zip。
fn pick_export_path(
    dialog: &adw::Dialog,
    report: &CrashReport,
    on_exported: impl Fn(std::result::Result<PathBuf, String>) + Clone + 'static,
) {
    let file_dialog = gtk::FileDialog::builder()
        .title("导出崩溃日志")
        .accept_label("保存")
        .initial_name(default_file_name(&report.instance))
        .build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Zip 压缩包"));
    filter.add_pattern("*.zip");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    file_dialog.set_filters(Some(&filters));
    file_dialog.set_default_filter(Some(&filter));

    let parent = dialog.root().and_downcast::<gtk::Window>();
    let report = report.clone();
    file_dialog.save(parent.as_ref(), None::<&gio::Cancellable>, move |result| {
        let Some(mut path) = result.ok().and_then(|file| file.path()) else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("zip");
        }
        let report = report.clone();
        glib::spawn_future_local(async move {
            let exported = tokio::task::spawn_blocking(move || {
                write_zip(&report, &path)
                    .map(|()| path)
                    .map_err(|err| format!("{err:#}"))
            })
            .await;
            on_exported(exported.unwrap_or_else(|err| Err(format!("导出任务失败：{err}"))));
        });
    });
}

/// 默认文件名：`dmcl-crash-<实例名>-<yyyyMMdd-HHmmss>.zip`。
fn default_file_name(instance: &str) -> String {
    let stamp = glib::DateTime::now_local()
        .and_then(|date| date.format("%Y%m%d-%H%M%S"))
        .map(|stamp| stamp.to_string())
        .unwrap_or_else(|_| {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or_default();
            seconds.to_string()
        });
    let instance: String = instance
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    format!("dmcl-crash-{instance}-{stamp}.zip")
}

/// 生成崩溃日志 zip；失败时删掉未完成的文件。
fn write_zip(report: &CrashReport, destination: &Path) -> Result<()> {
    write_zip_files(report, destination).inspect_err(|_| {
        let _ = std::fs::remove_file(destination);
    })
}

fn write_zip_files(report: &CrashReport, destination: &Path) -> Result<()> {
    let file = std::fs::File::create(destination)
        .with_context(|| format!("无法创建 {}", destination.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("launch.txt", options)?;
    zip.write_all(launch_text(report).as_bytes())?;

    zip.start_file("system.txt", options)?;
    zip.write_all(system_text(report).as_bytes())?;

    if let Some(path) = &report.log_path {
        add_file(&mut zip, "output.log", path, options)?;
    }

    let launch = &report.launch;
    let latest = launch.game_dir.join("logs").join("latest.log");
    add_file(&mut zip, "latest.log", &latest, options)?;

    let version_json = dmcl_mcmeta::layout::version_json_path(
        &launch.resolved.reference.root,
        &launch.resolved.reference.name,
    );
    add_file(&mut zip, "version.json", &version_json, options)?;

    for (name, path) in recent_crash_reports(&launch.game_dir.join("crash-reports")) {
        add_file(&mut zip, &format!("crash-reports/{name}"), &path, options)?;
    }

    zip.finish()?;
    Ok(())
}

/// 把文件原样加进 zip；文件不存在或不可读时跳过，不让导出失败。
fn add_file(
    zip: &mut zip::ZipWriter<std::fs::File>,
    name: &str,
    path: &Path,
    options: SimpleFileOptions,
) -> Result<()> {
    let Ok(bytes) = std::fs::read(path) else {
        return Ok(());
    };
    zip.start_file(name, options)?;
    zip.write_all(&bytes)?;
    Ok(())
}

/// `crash-reports/` 下最新的几份 `crash-*.txt`。
fn recent_crash_reports(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut reports: Vec<(SystemTime, String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();
            if !(name.starts_with("crash-") && name.ends_with(".txt") && path.is_file()) {
                return None;
            }
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            Some((modified, name, path))
        })
        .collect();
    reports.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    reports
        .into_iter()
        .take(MAX_CRASH_REPORTS)
        .map(|(_, name, path)| (name, path))
        .collect()
}

fn launch_text(report: &CrashReport) -> String {
    let launch = &report.launch;
    let mut text = format!(
        "实例: {}\n退出状态: {}\n版本目录: {}\n游戏目录: {}\nJava: {}（{}，{}）\nDMCL: {}\n系统: {} / {}\n\n启动参数:\n",
        report.instance,
        report.exit,
        launch.resolved.reference.dir().display(),
        launch.game_dir.display(),
        launch.java.path.display(),
        launch.java.major,
        launch.java.arch.name(),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    for arg in &launch.args {
        text.push_str(arg);
        text.push('\n');
    }
    text
}

fn system_text(report: &CrashReport) -> String {
    let java = &report.launch.java;
    format!(
        "DMCL: {}\nOS: {}\nArch: {}\nJava: {}\nJava 版本: {}\nJava 架构: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        java.path.display(),
        java.major,
        java.arch.name(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_game::{ResolvedVersion, VersionRef};
    use dmcl_java::JavaOption;
    use dmcl_mcmeta::os::Arch;
    use dmcl_mcmeta::vanilla::VersionJson;
    use std::io::Read;

    #[cfg(unix)]
    fn exit_code(code: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit_code(code: i32) -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }

    #[cfg(unix)]
    fn exit_signal(signal: i32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(signal)
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dmcl-crash-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_report(root: &Path) -> CrashReport {
        let reference = VersionRef::new(root, "1.0");
        let launch = PreparedLaunch {
            resolved: ResolvedVersion {
                reference: reference.clone(),
                json: VersionJson::default(),
            },
            game_dir: reference.dir(),
            args: vec![
                "--username".to_string(),
                "player".to_string(),
                "--accessToken".to_string(),
                "secret-token".to_string(),
            ],
            java: JavaOption {
                path: PathBuf::from("/usr/bin/java"),
                major: 17,
                arch: Arch::X86_64,
            },
        };
        CrashReport {
            instance: "1.0".to_string(),
            exit: "退出码 1".to_string(),
            launch: Arc::new(launch),
            log_path: None,
        }
    }

    #[test]
    fn log_levels_are_classified_from_common_line_formats() {
        assert_eq!(
            classify_log_line("[12:00:00] [main/ERROR]: boom"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            classify_log_line("[12:00:00] [Render thread/WARN]: slow"),
            Some(LogLevel::Warning)
        );
        assert_eq!(
            classify_log_line("[12:00:00] [main/DEBUG]: detail"),
            Some(LogLevel::Muted)
        );
        assert_eq!(classify_log_line("[12:00:00] [main/INFO]: started"), None);
        assert_eq!(
            classify_log_line("WARNING: coremods are present:"),
            Some(LogLevel::Warning)
        );
        assert_eq!(
            classify_log_line("net.minecraftforge.fml.common.LoaderException: boom"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            classify_log_line("[12:00:00] [main/INFO]: ---- Minecraft Crash Report ----"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            classify_log_line("\tat com.example.Main.main(Main.java:1)"),
            Some(LogLevel::Error)
        );
        assert_eq!(
            classify_log_line("Caused by: java.lang.RuntimeException"),
            Some(LogLevel::Error)
        );
    }

    #[test]
    fn exit_status_is_formatted_for_the_dialog() {
        assert_eq!(format_exit_status(&Ok(exit_code(0))), "退出码 0");
        assert_eq!(format_exit_status(&Ok(exit_code(1))), "退出码 1");
        assert!(format_exit_status(&Err("等待失败".to_string())).contains("等待失败"));

        #[cfg(unix)]
        assert_eq!(format_exit_status(&Ok(exit_signal(9))), "被信号 9 终止");
    }

    #[test]
    fn read_log_returns_the_whole_file_and_handles_missing_or_empty_files() {
        let dir = temp_dir("read-log");

        assert_eq!(read_log(None), None);
        assert_eq!(read_log(Some(&dir.join("missing.log"))), None);

        let empty = dir.join("empty.log");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(read_log(Some(&empty)), None);

        let full = dir.join("full.log");
        std::fs::write(&full, b"first\n\xff\nlast\n").unwrap();
        let text = read_log(Some(&full)).unwrap();
        assert!(text.starts_with("first\n"));
        assert!(text.contains('\u{fffd}'));
        assert!(text.ends_with("last\n"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zip_contains_the_logs_and_launcher_information() {
        let dir = temp_dir("zip");
        let root = dir.join("mc");
        let mut report = sample_report(&root);

        let game_dir = report.launch.game_dir.clone();
        std::fs::create_dir_all(game_dir.join("logs")).unwrap();
        std::fs::create_dir_all(game_dir.join("crash-reports")).unwrap();
        std::fs::write(game_dir.join("logs/latest.log"), "[latest]").unwrap();
        std::fs::write(
            game_dir.join("crash-reports/crash-2026-01-01_00.00.00-client.txt"),
            "boom",
        )
        .unwrap();
        std::fs::write(
            dmcl_mcmeta::layout::version_json_path(&root, "1.0"),
            r#"{"id":"1.0"}"#,
        )
        .unwrap();

        let log = dir.join("output.log");
        std::fs::write(&log, "session output\n").unwrap();
        report.log_path = Some(log);

        let destination = dir.join("export.zip");
        write_zip(&report, &destination).unwrap();

        let file = std::fs::File::open(&destination).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().unwrap().to_string())
            .collect();
        for expected in [
            "launch.txt",
            "system.txt",
            "output.log",
            "latest.log",
            "version.json",
            "crash-reports/crash-2026-01-01_00.00.00-client.txt",
        ] {
            assert!(names.iter().any(|name| name == expected), "{expected}");
        }
        assert!(!names.iter().any(|name| name == "instance/dmcl.json"));

        let mut output = String::new();
        archive
            .by_name("output.log")
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert_eq!(output, "session output\n");

        let mut launch = String::new();
        archive
            .by_name("launch.txt")
            .unwrap()
            .read_to_string(&mut launch)
            .unwrap();
        assert!(launch.contains("secret-token"), "{launch}");
        assert!(launch.contains("player"), "{launch}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_optional_files_do_not_fail_the_export() {
        let dir = temp_dir("zip-missing");
        let root = dir.join("mc");
        let report = sample_report(&root);
        let destination = dir.join("export.zip");

        write_zip(&report, &destination).unwrap();

        let file = std::fs::File::open(&destination).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().unwrap().to_string())
            .collect();
        assert!(names.iter().any(|name| name == "launch.txt"));
        assert!(names.iter().any(|name| name == "system.txt"));
        assert!(!names.iter().any(|name| name == "output.log"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_the_newest_five_crash_reports_are_kept() {
        let dir = temp_dir("crash-reports");
        for index in 0..7 {
            std::fs::write(
                dir.join(format!("crash-{index:02}-client.txt")),
                format!("report {index}"),
            )
            .unwrap();
        }
        std::fs::write(dir.join("not-a-crash.txt"), "ignore").unwrap();

        let reports = recent_crash_reports(&dir);
        assert_eq!(reports.len(), MAX_CRASH_REPORTS);
        assert!(reports.iter().all(|(name, _)| name.starts_with("crash-")));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
