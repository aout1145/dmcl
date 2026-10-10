//! 运行外部程序的任务。

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

use crate::context::{Cancelled, TaskContext};
use crate::task::{BoxFuture, Task};

/// 出错时最多保留多少行输出（防止把整份日志塞进错误信息）。
const TAIL_LINES: usize = 40;

/// 子进程的输出如何处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// 默认：接管 stdout / stderr，**每一行都作为一条进度消息**上报，
    /// 同时保留末尾若干行用于失败时构造错误信息。
    #[default]
    Capture,
    /// 让子进程直接继承本进程的终端（玩家能看到游戏自己打印的日志）。
    Inherit,
}

/// 运行一个外部程序的任务。
///
/// ```no_run
/// # use dmcl_task::CommandTask;
/// // java -version，并把输出逐行上报成进度
/// let task = CommandTask::new("java").arg("-version").named("检查 Java");
/// # let _ = task;
/// ```
pub struct CommandTask {
    name: String,
    program: OsString,
    args: Vec<OsString>,
    envs: Vec<(OsString, OsString)>,
    current_dir: Option<PathBuf>,
    mode: OutputMode,
}

impl CommandTask {
    /// 用可执行文件路径构造；默认任务名即该路径。
    pub fn new(program: impl Into<OsString>) -> Self {
        let program = program.into();
        let name = program.to_string_lossy().into_owned();
        Self {
            name,
            program,
            args: Vec::new(),
            envs: Vec::new(),
            current_dir: None,
            mode: OutputMode::default(),
        }
    }

    /// 指定任务名（进度里显示的名字）。
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    /// 让子进程继承当前终端（不逐行上报输出）。
    pub fn inherit_stdio(mut self) -> Self {
        self.mode = OutputMode::Inherit;
        self
    }

    /// 供日志/错误信息使用的命令行文本。
    pub fn command_line(&self) -> String {
        let mut out = self.program.to_string_lossy().into_owned();
        for arg in &self.args {
            out.push(' ');
            out.push_str(&arg.to_string_lossy());
        }
        out
    }

    fn build(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .envs(self.envs.iter().cloned())
            .stdin(Stdio::null())
            // future 被丢弃（例如取消）时顺手杀掉子进程
            .kill_on_drop(true);
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        match self.mode {
            OutputMode::Capture => {
                command.stdout(Stdio::piped()).stderr(Stdio::piped());
            }
            OutputMode::Inherit => {
                command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            }
        }
        command
    }
}

impl Task<(), ()> for CommandTask {
    fn name(&self) -> &str {
        &self.name
    }

    fn run<'a>(&'a self, _input: (), ctx: TaskContext) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            ctx.check_cancelled()?;

            let line = self.command_line();
            let mut child = self
                .build()
                .spawn()
                .with_context(|| format!("无法启动命令: {line}"))?;

            // 捕获模式下用来拼错误信息的输出尾巴；继承模式下不会写入。
            let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
            let cancel = ctx.cancel_handle();

            let status = match self.mode {
                OutputMode::Inherit => tokio::select! {
                    status = child.wait() => status.context("等待命令结束时出错")?,
                    _ = cancel.cancelled() => {
                        return Err(anyhow::Error::new(Cancelled));
                    }
                },
                OutputMode::Capture => {
                    let stdout = child.stdout.take();
                    let stderr = child.stderr.take();
                    let wait = async {
                        let (status, _, _) = tokio::join!(
                            child.wait(),
                            pump(stdout, ctx.clone(), tail.clone()),
                            pump(stderr, ctx.clone(), tail.clone())
                        );
                        status
                    };
                    tokio::pin!(wait);
                    tokio::select! {
                        status = &mut wait => status.context("等待命令结束时出错")?,
                        _ = cancel.cancelled() => {
                            // `wait` 在此被丢弃 → 子进程被 kill_on_drop 杀掉
                            return Err(anyhow::Error::new(Cancelled));
                        }
                    }
                }
            };

            if !status.success() {
                let mut message =
                    format!("命令失败（退出码 {}）: {line}", status.code().unwrap_or(-1));
                let tail = tail_text(&tail);
                if !tail.is_empty() {
                    message.push_str("\n--- 输出结尾 ---\n");
                    message.push_str(&tail);
                }
                anyhow::bail!("{message}");
            }
            Ok(())
        })
    }
}

/// 收集子进程输出的尾部若干行，并把每一行上报为一条进度。
async fn pump<R>(reader: Option<R>, ctx: TaskContext, tail: Arc<Mutex<VecDeque<String>>>)
where
    R: AsyncRead + Unpin,
{
    let Some(reader) = reader else {
        return;
    };
    let mut lines = BufReader::new(reader).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                ctx.log(&line);
                if let Ok(mut tail) = tail.lock() {
                    if tail.len() == TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
                if ctx.is_cancelled() {
                    return;
                }
            }
            Ok(None) => return,
            Err(_) => return,
        }
    }
}

/// 把已收集的尾部输出拼成错误信息里的最后一段。
fn tail_text(tail: &Mutex<VecDeque<String>>) -> String {
    match tail.lock() {
        Ok(lines) => lines.iter().cloned().collect::<Vec<_>>().join("\n"),
        Err(_) => String::new(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::chain::TaskChain;
    use std::time::Duration;

    #[tokio::test]
    async fn captured_output_becomes_logs() {
        let chain =
            TaskChain::new("run").then(CommandTask::new("sh").arg("-c").arg("echo hello"), "echo");
        let mut rx = chain.subscribe();
        let handle = tokio::spawn(async move { chain.run(()).await });

        let mut messages = Vec::new();
        while let Ok(event) = rx.recv().await {
            if let crate::event::TaskEvent::Log { message, .. } = event {
                messages.push(message);
            }
        }
        handle.await.unwrap().unwrap();
        assert!(messages.iter().any(|line| line == "hello"), "{messages:?}");
    }

    #[tokio::test]
    async fn a_non_zero_exit_is_an_error() {
        let chain = TaskChain::new("run").then(
            CommandTask::new("sh")
                .arg("-c")
                .arg("exit 3")
                .named("失败命令"),
            "失败命令",
        );
        let err = chain.run(()).await.unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("失败命令"), "{text}");
    }

    #[tokio::test]
    async fn a_missing_program_is_reported_with_its_command_line() {
        let chain = TaskChain::new("run").then(
            CommandTask::new("/definitely/not/here").arg("--x"),
            "坏命令",
        );
        let err = chain.run(()).await.unwrap_err();
        assert!(format!("{err:#}").contains("/definitely/not/here --x"));
    }

    #[tokio::test]
    async fn cancelling_kills_the_child() {
        let chain =
            TaskChain::new("run").then(CommandTask::new("sh").arg("-c").arg("sleep 30"), "长命令");
        let cancel = chain.cancel_handle();
        let handle = tokio::spawn(async move { chain.run(()).await });

        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();

        let err = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("取消应当立刻结束命令")
            .unwrap()
            .unwrap_err();
        assert!(err.downcast_ref::<Cancelled>().is_some());
    }

    #[tokio::test]
    async fn inherit_mode_just_waits() {
        let chain = TaskChain::new("run").then(
            CommandTask::new("sh").arg("-c").arg("true").inherit_stdio(),
            "true",
        );
        chain.run(()).await.unwrap();
    }
}
