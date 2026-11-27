//! 游戏进程：配置、启动、等待退出 / 停止。

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use dmcl_task::{Task, TaskContext};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::arguments::{command_preview, PreparedLaunch};

/// 游戏进程的优先级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessPriority {
    /// 低于正常优先级。
    Low,
    /// 正常优先级。
    #[default]
    Normal,
    /// 高于正常优先级。
    High,
}

/// 游戏进程的配置。
#[derive(Debug, Clone)]
pub struct ProcessOptions {
    /// 游戏进程的工作目录（spawn 用）。
    pub working_dir: PathBuf,
    pub priority: ProcessPriority,
    /// 把 stdout / stderr 重定向到这个文件。
    ///
    /// `None` 表示继承调用方输出。位置由调用者决定，这里只负责创建父目录、
    /// 打开文件并接管道。
    pub log_path: Option<PathBuf>,
}

/// 被启动的游戏进程。
///
/// 这里持有子进程句柄，因此调用者可以等它退出，也可以主动把它停掉
/// （GUI 的"停止游戏"）。
pub struct GameMonitor {
    child: tokio::process::Child,
    /// 会话日志文件；没有重定向或创建失败时为 `None`。
    log_path: Option<PathBuf>,
    /// 把子进程 stdout / stderr 写进日志文件的后台任务。
    readers: Vec<tokio::task::JoinHandle<()>>,
}

impl fmt::Debug for GameMonitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameMonitor")
            .field("pid", &self.id())
            .field("log_path", &self.log_path)
            .finish()
    }
}

impl GameMonitor {
    /// 游戏进程的 PID（如果还知道）。
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// 会话日志文件路径；没有重定向或创建失败时为 `None`。
    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }

    /// 游戏是否已经退出（不阻塞）。
    pub fn try_status(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    /// 结束游戏进程。
    pub async fn kill(&mut self) -> Result<()> {
        self.child.kill().await?;
        Ok(())
    }

    /// 等到游戏退出。
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        let status = self.child.wait().await?;
        // 子进程退出后管道才到 EOF；等读取任务收尾，保证日志文件已经写完。
        for reader in self.readers.drain(..) {
            let _ = reader.await;
        }
        Ok(status)
    }
}

/// 在系统临时目录下生成一个会话日志路径：
/// `<temp>/dmcl/dmcl-<实例名>-<pid>-<时间戳>.log`。
///
/// 只生成路径，不创建文件或目录；[`spawn_game`] 负责创建父目录。
pub fn session_log_path(instance: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join("dmcl").join(format!(
        "dmcl-{}-{}-{nanos}.log",
        sanitize_file_name(instance),
        std::process::id()
    ))
}

/// 把文件名里不能用的字符换成 `_`；只做最小替换，不改写其它字符。
fn sanitize_file_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect()
}

/// 用给定的命令行参数与进程配置启动游戏子进程，返回**未 wait** 的 [`GameMonitor`]。
///
/// `args` 已经包含 java 可执行文件；工作目录与优先级来自 [`ProcessOptions`]。
/// 这里**不打印**任何东西：命令行由 [`LaunchTask`] 通过进度事件流展示。
pub(crate) fn spawn_game(args: &[String], process: &ProcessOptions) -> Result<GameMonitor> {
    if args.is_empty() {
        return Err(anyhow!("命令为空"));
    }

    let log = process
        .log_path
        .as_deref()
        .and_then(|path| open_log_file(path).map(|file| (path.to_path_buf(), file)));

    let mut command = tokio::process::Command::new(&args[0]);
    command.args(&args[1..]).current_dir(&process.working_dir);
    apply_priority(&mut command, process.priority);
    if log.is_some() {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = command.spawn()?;

    let (log_path, readers) = match log {
        Some((path, file)) => {
            let file = Arc::new(tokio::sync::Mutex::new(file));
            let mut readers = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                readers.push(spawn_output_reader(stdout, file.clone()));
            }
            if let Some(stderr) = child.stderr.take() {
                readers.push(spawn_output_reader(stderr, file));
            }
            (Some(path), readers)
        }
        None => (None, Vec::new()),
    };

    Ok(GameMonitor {
        child,
        log_path,
        readers,
    })
}

/// 打开会话日志文件；目录或文件创建失败时返回 `None`，调用方退回继承输出。
fn open_log_file(path: &Path) -> Option<tokio::fs::File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).ok()?;
        }
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .ok()?;
    Some(tokio::fs::File::from_std(file))
}

/// 逐行读取子进程输出，统一换行后写进日志文件。
///
/// 读失败或写失败都只结束当前任务 / 丢弃这一行：必须继续消费管道，
/// 否则管道写满后游戏会卡住。
fn spawn_output_reader<R>(
    source: R,
    file: Arc<tokio::sync::Mutex<tokio::fs::File>>,
) -> tokio::task::JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut source = BufReader::new(source);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match source.read_until(b'\n', &mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if buffer.last() == Some(&b'\n') {
                buffer.pop();
                if buffer.last() == Some(&b'\r') {
                    buffer.pop();
                }
            }
            let line = String::from_utf8_lossy(&buffer);
            let mut file = file.lock().await;
            let _ = file.write_all(line.as_bytes()).await;
            let _ = file.write_all(b"\n").await;
        }
    })
}

/// 用 [`PreparedLaunch`] 里已经生成好的参数启动游戏，产出**未 wait** 的
/// [`GameMonitor`]（等待 / 结束由调用者决定）。
pub struct LaunchTask {
    pub process: ProcessOptions,
}

/// 一次已经启动的游戏会话：进程句柄与本次启动快照。
#[derive(Debug)]
pub struct GameSession {
    pub monitor: GameMonitor,
    pub launch: Arc<PreparedLaunch>,
}

#[async_trait::async_trait]
impl Task<Arc<PreparedLaunch>, GameSession> for LaunchTask {
    async fn run(&self, input: Arc<PreparedLaunch>, ctx: TaskContext) -> Result<GameSession> {
        // 命令行经由事件流显示（走进度打印机），避免直接 println 与进度行抢行
        ctx.log(format!(
            "命令：{}",
            command_preview(&input.game_dir, &input.args)
        ));
        let monitor = spawn_game(&input.args, &self.process)?;
        if self.process.log_path.is_some() && monitor.log_path().is_none() {
            ctx.log("无法创建游戏日志文件，游戏输出将直接继承终端".to_string());
        }
        Ok(GameSession {
            monitor,
            launch: input,
        })
    }
}

#[cfg(windows)]
fn apply_priority(command: &mut tokio::process::Command, priority: ProcessPriority) {
    // `CreateProcess` 的进程优先级类；正常优先级不需要额外标志。
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;

    let flags = match priority {
        ProcessPriority::Low => BELOW_NORMAL_PRIORITY_CLASS,
        ProcessPriority::Normal => 0,
        ProcessPriority::High => ABOVE_NORMAL_PRIORITY_CLASS,
    };
    if flags != 0 {
        command.creation_flags(flags);
    }
}

#[cfg(unix)]
fn apply_priority(command: &mut tokio::process::Command, priority: ProcessPriority) {
    let nice = match priority {
        ProcessPriority::Low => 10,
        ProcessPriority::Normal => 0,
        ProcessPriority::High => -5,
    };
    if nice == 0 {
        return;
    }
    // 负 nice 需要权限；没有权限时保持当前优先级启动，而不是让游戏起不来。
    unsafe {
        command.pre_exec(move || {
            if libc::setpriority(libc::PRIO_PROCESS, 0, nice) == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EPERM) | Some(libc::EACCES) => Ok(()),
                _ => Err(error),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个「睡很久」的命令 —— 两个平台都有。
    fn long_running() -> tokio::process::Command {
        #[cfg(windows)]
        {
            // Windows 没有 `sleep`，用 `ping` 当等待；`>NUL` 丢掉输出
            let mut command = tokio::process::Command::new("cmd");
            command.args(["/C", "ping -n 30 127.0.0.1 >NUL"]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = tokio::process::Command::new("sleep");
            command.arg("30");
            command
        }
    }

    fn echo_args(text: &str) -> Vec<String> {
        #[cfg(windows)]
        {
            vec!["cmd".to_string(), "/C".to_string(), format!("echo {text}")]
        }
        #[cfg(not(windows))]
        {
            vec!["echo".to_string(), text.to_string()]
        }
    }

    fn test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dmcl-{label}-{}", std::process::id()))
    }

    fn process(dir: &Path) -> ProcessOptions {
        ProcessOptions {
            working_dir: dir.to_path_buf(),
            priority: ProcessPriority::Normal,
            log_path: None,
        }
    }

    #[tokio::test]
    async fn a_monitor_reports_exit_and_can_kill() {
        let child = long_running().spawn().expect("spawn long-running command");
        let mut monitor = GameMonitor {
            child,
            log_path: None,
            readers: Vec::new(),
        };
        assert!(monitor.id().is_some());
        assert!(monitor.try_status().unwrap().is_none());
        monitor.kill().await.unwrap();
        // 被强杀：unix 死于信号（没有退出码），windows 是一个非 0 退出码
        assert!(!monitor.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn spawn_game_uses_the_working_directory() {
        let dir = test_dir("spawn-cwd");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        #[cfg(windows)]
        let args = vec![
            "cmd".to_string(),
            "/C".to_string(),
            "mkdir marker".to_string(),
        ];
        #[cfg(not(windows))]
        let args = vec!["mkdir".to_string(), "marker".to_string()];

        let mut monitor = spawn_game(&args, &process(&dir)).unwrap();
        assert!(monitor.wait().await.unwrap().success());
        assert!(dir.join("marker").is_dir());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn log_path_is_none_without_redirection() {
        let dir = test_dir("no-log");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let mut monitor = spawn_game(&echo_args("hello"), &process(&dir)).unwrap();
        assert!(monitor.log_path().is_none());
        assert!(monitor.wait().await.unwrap().success());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn captured_output_is_written_to_a_new_log_file() {
        let dir = test_dir("captured-output");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let log_path = dir.join("nested").join("session.log");

        let process = ProcessOptions {
            working_dir: dir.clone(),
            priority: ProcessPriority::Normal,
            log_path: Some(log_path.clone()),
        };
        let mut monitor = spawn_game(&echo_args("hello world"), &process).unwrap();
        assert_eq!(monitor.log_path(), Some(log_path.as_path()));
        assert!(monitor.wait().await.unwrap().success());
        let text = tokio::fs::read_to_string(&log_path).await.unwrap();
        assert_eq!(text, "hello world\n");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn an_unusable_log_location_falls_back_to_inherited_output() {
        let dir = test_dir("unusable-log");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let blocker = dir.join("not-a-directory");
        std::fs::write(&blocker, b"").unwrap();

        let process = ProcessOptions {
            working_dir: dir.clone(),
            priority: ProcessPriority::Normal,
            log_path: Some(blocker.join("session.log")),
        };
        let mut monitor = spawn_game(&echo_args("hello"), &process).unwrap();
        assert!(monitor.log_path().is_none());
        assert!(monitor.wait().await.unwrap().success());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn session_log_paths_are_unique_and_sanitized() {
        let first = session_log_path("1.20.1/forge");
        let second = session_log_path("1.20.1/forge");
        assert_ne!(first, second);
        assert_eq!(
            first.parent(),
            Some(std::env::temp_dir().join("dmcl").as_path())
        );
        assert_eq!(first.extension().and_then(|ext| ext.to_str()), Some("log"));
        let name = first.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("dmcl-1.20.1_forge-"), "{name}");
        assert!(!name.contains('/'), "{name}");

        let illegal = session_log_path("a:b?c");
        let illegal_name = illegal.file_name().unwrap().to_str().unwrap();
        assert!(illegal_name.starts_with("dmcl-a_b_c-"), "{illegal_name}");
    }
}
