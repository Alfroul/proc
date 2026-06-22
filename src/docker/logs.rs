//! E1 — 容器日志流（`docker logs` 等价）。
//!
//! 调用 bollard `logs`（`LogsOptions { follow, stdout, stderr, timestamps, tail }`）
//! 拿日志流。`parse_log_timestamp` 是纯函数：剥离 Docker 加的 RFC3339 时间戳前缀，
//! 保留正文（含 ANSI 颜色码）。
//!
//! 解析时间戳的目的：用户开了 `timestamps: true` 让 docker 给每行加 RFC3339 前缀，
//! 但 UI 渲染时只展示正文 + 我们自己用 `chrono`-less 简单分桶，不需要 docker 的时间戳。

use crate::error::{ProcError, Result};

/// 一条解析后的日志条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// 原始时间戳字符串（如 `2026-06-20T08:30:45.123456789Z`），无时间戳时 None。
    pub timestamp: Option<String>,
    /// 正文（含 ANSI 颜色码），已去除尾随 `\n` / `\r`。
    pub message: String,
    /// 输出流：true = stderr，false = stdout。
    pub is_stderr: bool,
}

/// 把一行带时间戳前缀的原始日志拆成 `(timestamp, message)`。
///
/// Docker timestamps 格式：`2026-06-20T08:30:45.123456789Z ` 后跟正文。
/// 时间戳特征：以 `T` 连接日期/时间、以 `Z` 或 `+08:00` 收尾。
///
/// 输入不带时间戳（`timestamps: false`）→ `(None, 原文)`。
/// 输入是 docker 流首字符（`StdErr { ... }` / `StdOut { ... }`）已经被 bollard
/// 切干净，调用方传入的是 `Display` 后的字符串。
#[must_use]
pub fn parse_log_timestamp(line: &str) -> (Option<String>, String) {
    let trimmed_end = line.trim_end_matches(['\n', '\r']);
    // RFC3339 时间戳至少 20 字符：`YYYY-MM-DDTHH:MM:SSZ`
    if trimmed_end.len() < 20 {
        return (None, trimmed_end.to_string());
    }
    // 验证前 20 字符长得像 RFC3339。
    let bytes = trimmed_end.as_bytes();
    let looks_like_rfc3339 = bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':';
    if !looks_like_rfc3339 {
        return (None, trimmed_end.to_string());
    }

    // 找时间戳尾（Z 或 +HH:MM / -HH:MM）后的第一个空格。
    let tail_idx = find_timestamp_tail(trimmed_end);
    let Some(end) = tail_idx else {
        return (None, trimmed_end.to_string());
    };

    let ts = trimmed_end[..end].to_string();
    // 跳过时间戳后的 1 个空格。
    let body_start = trimmed_end[end..]
        .char_indices()
        .skip_while(|(_, c)| c.is_whitespace())
        .map(|(i, _)| end + i)
        .next()
        .unwrap_or(trimmed_end.len());
    let message = trimmed_end[body_start..].to_string();
    (Some(ts), message)
}

/// 找 RFC3339 时间戳的末尾字节索引（指向 Z 或时区偏移的最后一个字符）。
fn find_timestamp_tail(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    // 从 T 之后开始扫描，找 Z 或 ±HH:MM 结尾。
    // 简单实现：在 20-32 字符窗口内找 Z。
    let upper = bytes.len().min(40);
    for i in 19..upper {
        if bytes[i] == b'Z' {
            return Some(i + 1);
        }
        if bytes[i] == b'+' || bytes[i] == b'-' {
            // 期望后续是 HH:MM（5 字符），共 6 字符到尾。
            if i + 6 <= bytes.len() && bytes[i + 3] == b':' {
                return Some(i + 6);
            }
        }
    }
    None
}

/// 调用 bollard `logs` 拉一次（`follow=false`，整段返回）。
///
/// 主用于 CLI 模式 `--logs`（输出后退出），TUI 跟随模式用 [`stream_container_logs_follow`]。
pub fn collect_container_logs(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
    name: &str,
    tail: Option<&str>,
) -> Result<Vec<LogLine>> {
    use bollard::container::LogsOptions;
    use futures_util::stream::TryStreamExt;

    let options = LogsOptions::<String> {
        follow: false,
        stdout: true,
        stderr: true,
        timestamps: true,
        tail: tail.unwrap_or("all").to_string(),
        ..Default::default()
    };

    let mut stream = docker.logs(name, Some(options));
    let mut out = Vec::new();
    runtime.block_on(async {
        while let Some(log) = stream
            .try_next()
            .await
            .map_err(|e| ProcError::docker_with(format!("获取容器 {name} 日志失败"), e))?
        {
            push_log_line(&log, &mut out);
        }
        Ok::<(), ProcError>(())
    })?;
    Ok(out)
}

/// 启动 follow 流（spawn 到独立线程，调用方负责消费 [`LogChunk`]）。
///
/// 用法：日志 worker 内部调用，把每条 LogOutput 转 LogLine，按 chunk 累积推送。
/// 直接调用方通常不需要这个 — 走 [`super::logs_worker::spawn`]。
#[must_use]
pub fn make_follow_options(tail: Option<&str>) -> bollard::container::LogsOptions<String> {
    bollard::container::LogsOptions::<String> {
        follow: true,
        stdout: true,
        stderr: true,
        timestamps: true,
        tail: tail.unwrap_or("all").to_string(),
        ..Default::default()
    }
}

fn push_log_line(log: &bollard::container::LogOutput, out: &mut Vec<LogLine>) {
    let raw = String::from_utf8_lossy(log.as_ref()).into_owned();
    let is_stderr = matches!(log, bollard::container::LogOutput::StdErr { .. });
    // 一行 docker LogOutput 可能含多行 `\n`（多行 stacktrace），按 `\n` 拆开各自加时间戳。
    for piece in raw.split_inclusive('\n') {
        let (timestamp, message) = parse_log_timestamp(piece);
        if message.is_empty() && timestamp.is_none() {
            continue;
        }
        out.push(LogLine {
            timestamp,
            message,
            is_stderr,
        });
    }
}

impl std::fmt::Display for LogLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_stderr {
            write!(f, "[stderr] {}", self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rfc3339_with_z() {
        let line = "2026-06-20T08:30:45.123456789Z hello world\n";
        let (ts, msg) = parse_log_timestamp(line);
        assert_eq!(ts.as_deref(), Some("2026-06-20T08:30:45.123456789Z"));
        assert_eq!(msg, "hello world");
    }

    #[test]
    fn parses_rfc3339_with_timezone_offset() {
        let line = "2026-06-20T08:30:45+08:00 hi there\n";
        let (ts, msg) = parse_log_timestamp(line);
        assert_eq!(ts.as_deref(), Some("2026-06-20T08:30:45+08:00"));
        assert_eq!(msg, "hi there");
    }

    #[test]
    fn no_timestamp_keeps_full_message() {
        let line = "just a log line\n";
        let (ts, msg) = parse_log_timestamp(line);
        assert!(ts.is_none());
        assert_eq!(msg, "just a log line");
    }

    #[test]
    fn preserves_ansi_color_codes() {
        let line = "2026-06-20T08:30:45Z \x1b[32mINFO\x1b[0m boot ok\n";
        let (ts, msg) = parse_log_timestamp(line);
        assert_eq!(ts.as_deref(), Some("2026-06-20T08:30:45Z"));
        assert_eq!(msg, "\x1b[32mINFO\x1b[0m boot ok");
    }

    #[test]
    fn handles_carriage_returns() {
        let line = "2026-06-20T08:30:45Z line\r\n";
        let (_, msg) = parse_log_timestamp(line);
        assert_eq!(msg, "line");
    }

    #[test]
    fn too_short_returns_none() {
        let line = "short";
        let (ts, msg) = parse_log_timestamp(line);
        assert!(ts.is_none());
        assert_eq!(msg, "short");
    }

    #[test]
    fn empty_message_after_timestamp_still_returns_timestamp() {
        let line = "2026-06-20T08:30:45Z \n";
        let (ts, msg) = parse_log_timestamp(line);
        assert_eq!(ts.as_deref(), Some("2026-06-20T08:30:45Z"));
        assert!(msg.is_empty());
    }

    #[test]
    fn log_line_display_stderr() {
        let l = LogLine {
            timestamp: None,
            message: "boom".to_string(),
            is_stderr: true,
        };
        assert_eq!(format!("{l}"), "[stderr] boom");
    }
}
