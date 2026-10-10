//! 把任务事件流渲染成终端进度。

use std::io::Write;

use dialoguer::console::Term;
use dmcl_task::TaskEvent;

/// 子步骤内容相对外层步骤头的缩进（与 `[i/n] ` 等宽）。
const INDENT: &str = "      ";

/// 进度打印机。
///
/// 渲染成「步骤头 + 缩进内容」：
///
/// ```text
/// 安装 1.20.1（共 2 步）
/// [1/2] 获取原版元数据
///       查询版本列表并保存 1.20.1.json
///       完成
/// [2/2] 下载游戏文件
///       获取资源索引  100.0% (403.4 KiB / 403.4 KiB) 1/1 个文件
///       下载库文件  41.2% (34.9 MiB / 84.7 MiB) 35/120 个文件
///       完成
/// ```
///
/// 外层步骤头（`[i/n] 名字`）永远单独一行；转发来的子步骤内容缩进显示、只带
/// 子步骤名，不再重复外层步骤号。
///
/// 事件分两类：**进度**（`TaskEvent::Progress`）是"可原地刷新"的行——交互式终端上
/// 反复覆盖同一行，管道里跨 5% 刻度才打一行；**日志**（`TaskEvent::Log`）是"最终行"
/// ——逐条输出、不覆盖、不去重（命令行输出、缺件清单等）。
pub struct ProgressPrinter {
    term: Term,
    interactive: bool,
    /// 非交互模式：上一个进度百分比刻度。
    last_bucket: Option<u64>,
    /// 非交互模式：上一行文字（去重用）。
    last_line: String,
    /// 当前外层步骤名（用于区分"直接进度"与"子步骤进度"）。
    current_step_name: String,
    /// 交互模式：当前悬着那一行的前缀（判断要不要先换行）。
    live_subject: String,
    /// 交互模式：当前行是否"悬着"（还没换行，等着被覆盖 / 收尾）。
    mid_line: bool,
}

impl Default for ProgressPrinter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressPrinter {
    pub fn new() -> Self {
        let term = Term::stdout();
        let interactive = term.is_term();
        Self {
            term,
            interactive,
            last_bucket: None,
            last_line: String::new(),
            current_step_name: String::new(),
            live_subject: String::new(),
            mid_line: false,
        }
    }

    /// 直接交给 `TaskChain::run_with`。
    pub fn handle(&mut self, event: TaskEvent) {
        match event {
            TaskEvent::Started { name, steps } => {
                let text = if steps == 0 {
                    name.clone()
                } else {
                    format!("{name}（共 {steps} 步）")
                };
                self.print(&text, true, "");
            }
            TaskEvent::StepStarted { step, steps, name } => {
                self.current_step_name = name;
                self.last_bucket = None;
                self.print(&step_header(step, steps, &self.current_step_name), true, "");
            }
            TaskEvent::Log { name, message, .. } => {
                // 日志是一条“最终行”：逐条输出，不与进度行抢同一行、也不被覆盖。
                let subject = progress_subject(&name, &self.current_step_name);
                let text = format!("{subject}  {message}");
                self.print(&text, true, &subject);
            }
            TaskEvent::Warning { name, message, .. } => {
                // 警告也占一行最终行；是否继续由 CLI 的 on_event 询问后答复。
                let subject = progress_subject(&name, &self.current_step_name);
                let text = format!("{subject}  ⚠ 警告：{message}");
                self.print(&text, true, &subject);
            }
            TaskEvent::Progress { name, progress, .. } => {
                // 非交互模式的节流：字节进度只在跨 5% 刻度时打
                if !self.interactive {
                    let bucket = progress.percent().map(|percent| (percent / 5.0) as u64);
                    if progress.total.is_some() && bucket == self.last_bucket {
                        return;
                    }
                    self.last_bucket = bucket;
                }

                let subject = progress_subject(&name, &self.current_step_name);
                let text = format!("{subject}  {progress}");
                self.print(&text, false, &subject);
            }
            TaskEvent::StepFinished { .. } => {
                self.print(&format!("{INDENT}完成"), true, "");
            }
            TaskEvent::Failed { step, steps, name } => {
                // 只标记“哪一步失败了”；错误详情由调用方（main）打印一次
                let text = format!("{} 失败", step_header(step, steps, &name));
                self.print(&text, true, "");
            }
            TaskEvent::Finished { name } => {
                self.print(&format!("{name} 完成"), true, "");
            }
        }
    }

    /// 收尾：交互式下若还有一行悬着，补一个换行，别让后续输出接在进度后面。
    pub fn finish(&mut self) {
        if self.interactive && self.mid_line {
            let _ = writeln!(self.term);
            let _ = self.term.flush();
            self.mid_line = false;
        }
    }

    /// 是否是交互式终端（决定要不要弹确认）。
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// `end_line=true` 是最终行（两模式都换行）；`false` 是进度行（交互式原地刷新）。
    ///
    /// `subject` 是交互式下判断"换不换行"的前缀：同一步骤 / 同一子步骤的进度
    /// 前缀不变就覆盖，换了才先换行。日志、步骤头等都走 `end_line=true`。
    fn print(&mut self, text: &str, end_line: bool, subject: &str) {
        if self.interactive {
            if self.mid_line && (end_line || subject != self.live_subject) {
                let _ = writeln!(self.term);
            }
            let _ = self.term.clear_line();
            if end_line {
                let _ = writeln!(self.term, "{text}");
                self.mid_line = false;
            } else {
                let _ = write!(self.term, "\r{text}");
                self.mid_line = true;
                self.live_subject = subject.to_string();
            }
            let _ = self.term.flush();
        } else {
            // 非交互：只有"可原地刷新"的进度行去重；日志 / 结构行逐条输出
            if !end_line && text == self.last_line {
                return;
            }
            println!("{text}");
            self.last_line = text.to_string();
        }
    }
}

/// 外层步骤头：`[i/n] 名字`。
fn step_header(step: usize, steps: usize, name: &str) -> String {
    format!("[{}/{}] {name}", step + 1, steps)
}

/// 一条进度行的前缀：子步骤名与外层步骤名相同时（直接进度）只缩进；
/// 否则缩进 + 子步骤名（转发来的子步骤进度）。
fn progress_subject(name: &str, current_step_name: &str) -> String {
    if name == current_step_name {
        INDENT.to_string()
    } else {
        format!("{INDENT}{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmcl_task::Progress;

    #[test]
    fn headers_are_one_indexed() {
        assert_eq!(step_header(0, 3, "下载"), "[1/3] 下载");
        assert_eq!(step_header(2, 3, "下载"), "[3/3] 下载");
    }

    /// 子步骤进度：缩进 + 子步骤名，不再带外层 `[i/n]`。
    #[test]
    fn nested_progress_is_indented_and_does_not_repeat_the_step_number() {
        let subject = progress_subject("下载库文件", "下载游戏文件");
        let line = format!(
            "{subject}  {}",
            Progress::new(10, Some(20), "35/120 个文件")
        );
        assert_eq!(line, "      下载库文件  50.0% (10 B / 20 B) 35/120 个文件");
    }

    /// 直接进度（子步骤名 == 外层步骤名）只缩进，不重复步骤名。
    #[test]
    fn direct_progress_is_indented_without_repeating_the_name() {
        assert_eq!(progress_subject("下载", "下载"), "      ");
    }

    /// 纯文字进度（current=0、无总量）不带没意义的 `(0 B)`。
    #[test]
    fn text_only_progress_has_no_bogus_zero_bytes() {
        let subject = progress_subject("解压 natives", "下载游戏文件");
        let line = format!("{subject}  {}", Progress::new(0, None, "解压 liblwjgl.so"));
        assert_eq!(line, "      解压 natives  解压 liblwjgl.so");
    }

    #[test]
    fn failures_keep_the_step_header() {
        let failed = format!("{} 失败：404", step_header(1, 2, "下载"));
        assert_eq!(failed, "[2/2] 下载 失败：404");
    }
}
