//! 从游戏日志判断异常退出的可能原因。
//!
//! 按日志里的特征串识别加载器、运行时与环境错误，给出结论和建议。规则参考 HMCL 的
//! `CrashReportAnalyzer` 与 PCL2 的 `ModCrash`，按「先精确、后模糊」分三层：加载器问题
//! 命中后不再往下猜，运行时问题命中后不再使用泛化结论。
//!
//! 这里只做纯文本分析：不读文件、不依赖 GTK，调用方负责拼接日志（会话日志加上日志末尾
//! 指出的崩溃报告）。

mod exit;
mod rules;

pub use exit::Exit;

/// 日志里标记崩溃报告位置的串。
const CRASH_REPORT_MARKER: &str = "#@!@# Game crashed! Crash report saved to: #@!@#";

/// 一条可能原因及其建议。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub cause: String,
    pub advice: String,
}

impl Finding {
    pub(crate) fn new(cause: impl Into<String>, advice: impl Into<String>) -> Self {
        Self {
            cause: cause.into(),
            advice: advice.into(),
        }
    }
}

/// 相同结论只保留第一次出现的那条。
pub(crate) fn push(findings: &mut Vec<Finding>, finding: Finding) {
    if !findings
        .iter()
        .any(|existing| existing.cause == finding.cause)
    {
        findings.push(finding);
    }
}

/// 分析日志和退出方式；结论按优先级排列，没有命中时返回空列表。
pub fn analyze(text: &str, exit: &Exit) -> Vec<Finding> {
    let [loader, mut runtime, generic] = rules::analyze_tiers(text);
    if !loader.is_empty() {
        return loader;
    }

    if let Some(finding) = exit.finding() {
        push(&mut runtime, finding);
    }
    if !runtime.is_empty() {
        return runtime;
    }

    generic
}

/// 从日志里找出崩溃报告路径（`#@!@# Game crashed! Crash report saved to: #@!@#`）。
pub fn crash_report_path(log: &str) -> Option<&str> {
    let start = log.rfind(CRASH_REPORT_MARKER)? + CRASH_REPORT_MARKER.len();
    let path = log[start..].lines().next()?.trim().trim_matches('"');
    (!path.is_empty()).then_some(path)
}

#[cfg(test)]
pub(crate) fn causes(text: &str) -> Vec<String> {
    analyze(text, &Exit::Code(1))
        .into_iter()
        .map(|finding| finding.cause)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_tiers_hide_lower_ones() {
        let loader_and_runtime = "Missing or unsupported mandatory dependencies:\n\
                                  \tMod ID: 'ferritecore', Requested by: 'modernfix'\n\
                                  java.lang.NoClassDefFoundError: foo/Bar\n\
                                  java.lang.OutOfMemoryError: Java heap space\n";
        assert_eq!(causes(loader_and_runtime), ["缺少前置 mod：ferritecore"]);

        let runtime_and_generic = "java.lang.OutOfMemoryError: Java heap space\n\
                                   java.lang.NoClassDefFoundError: foo/Bar\n";
        assert_eq!(causes(runtime_and_generic), ["游戏内存不足"]);
    }

    #[test]
    fn no_match_returns_nothing() {
        assert!(causes("[12:00:00] [main/INFO]: started").is_empty());
    }

    #[test]
    fn crash_report_path_is_read_from_the_log_tail() {
        assert_eq!(
            crash_report_path(
                "some output\n#@!@# Game crashed! Crash report saved to: #@!@# /tmp/crash.txt\n"
            ),
            Some("/tmp/crash.txt")
        );
        assert_eq!(crash_report_path("no marker here"), None);
    }
}
