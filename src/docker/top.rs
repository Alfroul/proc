//! E4 — 容器内进程列表（`docker top` 等价）。
//!
//! 调用 bollard `top_processes` 拿 `ContainerTopResponse { titles, processes }`,
//! 解析为 `ContainerTopProcess` 列表。`parse_top_output` 是纯文本解析函数，
//! 可独立测试（含 args 内空格、空表格、列序变化等用例）。

use crate::error::{ProcError, Result};

/// 容器内单个进程。字段映射 `ps -ef` 的标准列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerTopProcess {
    /// PID（字符串形式，跨平台兼容 Windows 容器的 LARGE_INTEGER）。
    pub pid: String,
    /// 启动用户（UID / 用户名）。
    pub user: String,
    /// 完整命令（含参数，保留空格）。
    pub command: String,
    /// CPU 累计时间（`TIME` 列，HH:MM:SS 或时间片段）。
    pub cpu_time: String,
    /// 启动时间（`STIME` 列，可能是 `Jan01` / `08:30:00` 等）。
    pub started: String,
}

impl std::fmt::Display for ContainerTopProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:<10} {:<10} {:<10} {}",
            self.user, self.pid, self.started, self.command
        )
    }
}

/// 把 `docker top` 文本表格（首行表头 + 数据行）解析为 `ContainerTopProcess` 列表。
///
/// 规则：
/// - 空输入 / 仅空白行 → 返回空 Vec（覆盖「空表格」测试用例）。
/// - 第一条非空行作为表头，按列名定位 `UID`/`PID`/`CMD`/`TIME`/`STIME` 的列索引。
/// - `CMD` 列视为「该位置之后整段」（容忍参数内空格），其它列按空白分隔取值。
/// - 表头缺少必需列时返回空 Vec（容错降级，不 panic）。
/// - Windows 容器走不同 ps 风格（`Name` / `PID` / ...），同样按列名查找；
///   找不到 `UID` 时 user 留空字符串，找不到 `STIME` / `TIME` 时同。
#[must_use]
pub fn parse_top_output(raw: &str) -> Vec<ContainerTopProcess> {
    let mut lines = raw.lines().filter(|l| !l.trim().is_empty());

    let header = match lines.next() {
        Some(h) => h,
        None => return Vec::new(),
    };

    let header_cols: Vec<&str> = header.split_whitespace().collect();
    let col_index = |names: &[&str]| -> Option<usize> {
        header_cols
            .iter()
            .position(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)))
    };

    let pid_idx = col_index(&["PID", "ProcessId"]);
    let cmd_idx = col_index(&["CMD", "COMMAND", "Command", "Image", "Name"]);
    // 找不到 PID 或 CMD → 无法解析（这俩是必需列），返回空 Vec。
    let (pid_idx, cmd_idx) = match (pid_idx, cmd_idx) {
        (Some(p), Some(c)) => (p, c),
        _ => return Vec::new(),
    };
    let user_idx = col_index(&["UID", "USER", "UserName", "User"]);
    let time_idx = col_index(&["TIME", "%CPU", "CPU"]);
    let stime_idx = col_index(&["STIME", "START", "Start", "StartTime"]);

    let mut result = Vec::new();
    for line in lines {
        // 关键：CMD 列起用「从 cmd_idx 开始取剩余整段」而非 split_whitespace，
        // 这样 `nginx: master process nginx -g daemon off;` 等带空格的命令完整保留。
        let leading: Vec<&str> = line.split_whitespace().take(cmd_idx).collect();
        if leading.len() < cmd_idx {
            continue; // 行内列数少于 CMD 应在位置 → 跳过（容错）。
        }

        let pid = leading.get(pid_idx).copied().unwrap_or("").to_string();
        let user = user_idx
            .and_then(|i| leading.get(i))
            .copied()
            .unwrap_or("")
            .to_string();
        let cpu_time = time_idx
            .and_then(|i| leading.get(i))
            .copied()
            .unwrap_or("")
            .to_string();
        let started = stime_idx
            .and_then(|i| leading.get(i))
            .copied()
            .unwrap_or("")
            .to_string();

        // CMD 列：从原行的 cmd_idx 个空白分隔 token 之后开始。
        // 实现：迭代原行 char_indices，跳过 cmd_idx 个空白组。
        let command = extract_command(line, cmd_idx);

        result.push(ContainerTopProcess {
            pid,
            user,
            command,
            cpu_time,
            started,
        });
    }

    result
}

/// 从原行中提取第 `cmd_idx` 个 token 之后的所有字符（保留 CMD 内的空格）。
fn extract_command(line: &str, cmd_idx: usize) -> String {
    let mut tokens = 0usize;
    let mut chars = line.char_indices().peekable();
    let mut last_token_start = None;

    // 先跳过 (cmd_idx) 个 token，记下第 (cmd_idx+1) 个 token 的起始字节偏移。
    while let Some(&(i, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else {
            tokens += 1;
            last_token_start = Some(i);
            // 跳过整个 token。
            while let Some(&(_, c)) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                chars.next();
            }
            if tokens > cmd_idx {
                break;
            }
        }
    }

    let Some(start) = last_token_start else {
        return String::new();
    };
    if tokens <= cmd_idx {
        return String::new();
    }
    line[start..].trim().to_string()
}

/// 调用 bollard `top_processes` 拉容器内进程。
///
/// `ps_options` 固定 `"-ef"`（Linux 容器标准）。Windows 容器 Docker daemon
/// 内部会改走 `tasklist` 等价命令，返回的 titles 仍是表头字符串。
pub fn get_container_top(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
    name: &str,
) -> Result<Vec<ContainerTopProcess>> {
    use bollard::container::TopOptions;

    let resp = runtime
        .block_on(async {
            docker
                .top_processes(
                    name,
                    Some(TopOptions {
                        ps_args: "-ef".to_string(),
                    }),
                )
                .await
        })
        .map_err(|e| ProcError::docker_with(format!("获取容器 {name} 进程列表失败"), e))?;

    Ok(parse_top_response(&resp))
}

/// 把 bollard 结构化响应转成 `ContainerTopProcess` 列表。
///
/// 与 [`parse_top_output`] 行为一致，但输入是 bollard 已经按列切好的
/// `Vec<Vec<String>>`，无需文本 re-parse。CMD 列直接取最后一列（保留空格）。
#[must_use]
pub fn parse_top_response(
    resp: &bollard::models::ContainerTopResponse,
) -> Vec<ContainerTopProcess> {
    let titles: &[String] = match resp.titles.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return Vec::new(),
    };
    let rows: &[Vec<String>] = match resp.processes.as_deref() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let col_index = |names: &[&str]| -> Option<usize> {
        titles
            .iter()
            .position(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)))
    };

    let pid_idx = col_index(&["PID", "ProcessId"]);
    let cmd_idx = col_index(&["CMD", "COMMAND", "Command", "Image", "Name"]);
    let (pid_idx, cmd_idx) = match (pid_idx, cmd_idx) {
        (Some(p), Some(c)) => (p, c),
        _ => return Vec::new(),
    };
    let user_idx = col_index(&["UID", "USER", "UserName", "User"]);
    let time_idx = col_index(&["TIME", "%CPU", "CPU"]);
    let stime_idx = col_index(&["STIME", "START", "Start", "StartTime"]);

    rows.iter()
        .map(|row| {
            // CMD 列：直接取 cmd_idx 处的值（bollard 已经把 CMD 内部参数 join 好了）。
            ContainerTopProcess {
                pid: row.get(pid_idx).cloned().unwrap_or_default(),
                user: user_idx
                    .and_then(|i| row.get(i))
                    .cloned()
                    .unwrap_or_default(),
                command: row.get(cmd_idx).cloned().unwrap_or_default(),
                cpu_time: time_idx
                    .and_then(|i| row.get(i))
                    .cloned()
                    .unwrap_or_default(),
                started: stime_idx
                    .and_then(|i| row.get(i))
                    .cloned()
                    .unwrap_or_default(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LINUX: &str = "\
UID        PID  PPID  C STIME TTY          TIME CMD
root         1     0  0 Jan01 ?        00:00:01 /sbin/init
root        42     1  0 Jan01 ?        00:00:00 nginx: master process nginx -g daemon off;
nobody     100    42  0 10:30 ?        00:00:05 /usr/bin/python3 app.py --port 8080 --debug
";

    #[test]
    fn parses_typical_linux_ps_ef() {
        let out = parse_top_output(SAMPLE_LINUX);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].pid, "1");
        assert_eq!(out[0].user, "root");
        assert_eq!(out[0].command, "/sbin/init");
        assert_eq!(out[0].cpu_time, "00:00:01");
        assert_eq!(out[0].started, "Jan01");
    }

    #[test]
    fn preserves_spaces_in_command_args() {
        let out = parse_top_output(SAMPLE_LINUX);
        // nginx 行带空格的 CMD 必须完整保留
        assert_eq!(out[1].command, "nginx: master process nginx -g daemon off;");
        // python 行多参数
        assert_eq!(
            out[2].command,
            "/usr/bin/python3 app.py --port 8080 --debug"
        );
        assert_eq!(out[2].pid, "100");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(parse_top_output("").is_empty());
        assert!(parse_top_output("   \n  \n").is_empty());
    }

    #[test]
    fn header_only_returns_empty() {
        let header_only = "UID PID PPID C STIME TTY TIME CMD\n";
        assert!(parse_top_output(header_only).is_empty());
    }

    #[test]
    fn missing_required_columns_returns_empty() {
        // 缺 PID 列：返回空 Vec（不能解析）。
        let bad = "USER NAME\nroot alice\n";
        assert!(parse_top_output(bad).is_empty());
        // 缺 CMD 列：返回空。
        let bad2 = "UID PID\nroot 1\n";
        assert!(parse_top_output(bad2).is_empty());
    }

    #[test]
    fn alternate_column_order() {
        let alt = "USER  PID  START  TIME  COMMAND\nroot  42  08:30  00:01:23  sleep 60\n";
        let out = parse_top_output(alt);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pid, "42");
        assert_eq!(out[0].user, "root");
        assert_eq!(out[0].started, "08:30");
        assert_eq!(out[0].cpu_time, "00:01:23");
        assert_eq!(out[0].command, "sleep 60");
    }

    #[test]
    fn windows_style_columns_via_structured_response() {
        // 文本解析器是 Linux-focused；Windows 容器走 bollard 结构化路径，
        // cmd_idx 处的值已经包含完整内容。
        let resp = bollard::models::ContainerTopResponse {
            titles: Some(vec![
                "Name".to_string(),
                "PID".to_string(),
                "UserName".to_string(),
                "CPU".to_string(),
                "StartTime".to_string(),
            ]),
            processes: Some(vec![vec![
                "node.exe".to_string(),
                "4".to_string(),
                "Administrator".to_string(),
                "0.5".to_string(),
                "1/1/2026".to_string(),
            ]]),
        };
        let out = parse_top_response(&resp);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pid, "4");
        assert_eq!(out[0].user, "Administrator");
        // cmd_idx 找到 "Name"（位置 0），command = row[0] = "node.exe"。
        assert_eq!(out[0].command, "node.exe");
    }

    #[test]
    fn display_formats_columns() {
        let p = ContainerTopProcess {
            pid: "42".to_string(),
            user: "root".to_string(),
            command: "nginx".to_string(),
            cpu_time: "00:00:01".to_string(),
            started: "Jan01".to_string(),
        };
        let s = format!("{p}");
        assert!(s.contains("root"));
        assert!(s.contains("42"));
        assert!(s.contains("nginx"));
    }
}
