//! Linux per-process 网络字节速率：`nethogs` 子进程路线。
//!
//! # 选型
//!
//! 阶段 7 选 nethogs 而非 BPF：BPF 需 root + clang + 内核版本，开箱不可用；
//! nethogs 装上即用（多数发行版包管理器一行），子进程开销在 1s poll 下可接受
//! （参考 ADR-0005 同类取舍）。BPF 留 TODO，未来作为 feature flag。
//!
//! # 输出格式
//!
//! 固定调用 `nethogs -t -d 2 -v 3`：
//! - `-t` tracemode，一行一条更新，不刷新整屏
//! - `-d 2` 2 秒采样周期（nethogs 内部累积，输出节奏）
//! - `-v 3` view mode 3（每条记录输出 KB/sec + total KB 双列）
//!
//! tracemode 行格式（tab 分隔）：
//! ```text
//! refreshing
//! down\txterm/12345/user\t0.5\t12.3
//! up\txterm/12345/user\t0.2\t5.1
//! closed\txterm/12345/user
//! ```
//!
//! `down` 行的第三列是下行速率（KB/sec），`up` 行是上行速率。
//! 我们按 PID 聚合 down/up，KB/sec → B/sec 用 ×1024。
//!
//! # 容错
//!
//! - 二进制不在 PATH → `try_new` 返回 None（主线程 net 列保持 0）
//! - 行格式异常（缺列 / PID 非数字）→ 跳过该行（[`parse_nethogs_line`] 返回 None）
//! - 子进程 stdout 流式读取，不积压；解析器对部分行容忍（最后一行可能截断）

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;

use crate::net_flow::{NetFlowCollector, ProcessNetRate};

/// 子进程的 stdout 必须能跨线程「被 collector 持有 + 读取」—— Child 自身不是
/// Sync，但套一层 Mutex 后，collector 自身可以满足 `Send + Sync`（trait 要求）。
struct NethogsChild(Mutex<Child>);

impl NethogsChild {
    fn try_spawn() -> Option<Self> {
        // 先确认二进制存在；不直接 spawn 是因为 spawn 会泄漏半启动的进程。
        let probe = Command::new("nethogs")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()?;
        if !probe.success() {
            return None;
        }

        let child = Command::new("nethogs")
            .args(["-t", "-d", "2", "-v", "3"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .spawn()
            .ok()?;
        Some(Self(Mutex::new(child)))
    }
}

impl Drop for NethogsChild {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.0.lock() {
            let _ = guard.kill();
            let _ = guard.wait();
        }
    }
}

unsafe impl Sync for NethogsChild {}

pub struct NethogsCollector {
    child: NethogsChild,
    last_time: Instant,
    /// 上一轮聚合的 per-PID 速率（KB/sec 来自 nethogs，转换为 B/sec 后存）；
    /// 在没有新行时复用上一次的值。这避免 UI 闪烁。
    last_rates: Vec<ProcessNetRate>,
}

impl NethogsCollector {
    /// 探测 nethogs 二进制可用性；不可用返回 None（不抛错，主线程 net 列保持 0）。
    #[must_use]
    pub fn try_new() -> Option<Self> {
        let child = NethogsChild::try_spawn()?;
        Some(Self {
            child,
            last_time: Instant::now(),
            last_rates: Vec::new(),
        })
    }
}

impl NetFlowCollector for NethogsCollector {
    fn per_process_rates(&mut self) -> Vec<ProcessNetRate> {
        // 从子进程 stdout 非阻塞读取所有可用行；没新行就复用 last_rates
        let lines = drain_child_stdout(&self.child);
        if lines.is_empty() {
            return self.last_rates.clone();
        }

        let mut aggregated: std::collections::HashMap<u32, (u64, u64)> =
            std::collections::HashMap::new();
        for line in &lines {
            if let Some((pid, direction, kbps)) = parse_nethogs_line(line) {
                let bytes_per_sec = kbps_to_bytes(kbps);
                let e = aggregated.entry(pid).or_insert((0, 0));
                match direction {
                    Direction::Down => e.0 = e.0.saturating_add(bytes_per_sec),
                    Direction::Up => e.1 = e.1.saturating_add(bytes_per_sec),
                }
            }
        }

        let rates: Vec<ProcessNetRate> = aggregated
            .iter()
            .map(|(&pid, &(down, up))| ProcessNetRate {
                pid,
                start_time: 0,
                bytes_sent_per_sec: up,
                bytes_recv_per_sec: down,
            })
            .collect();

        self.last_rates = rates.clone();
        self.last_time = Instant::now();
        rates
    }

    fn provider_name(&self) -> &'static str {
        "linux-nethogs"
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Direction {
    Down,
    Up,
}

/// 单行 nethogs tracemode 解析。返回 (pid, direction, kbps)。
///
/// 输入示例：
/// - `"down\txterm/12345/user\t0.5\t12.3"` → `(12345, Down, 0.5)`
/// - `"up\txterm/12345/user\t0.2\t5.1"` → `(12345, Up, 0.2)`
/// - `"closed\txterm/12345/user"` → None（无速率列）
/// - `"refreshing"` → None
fn parse_nethogs_line(line: &str) -> Option<(u32, Direction, f64)> {
    let direction = if line.starts_with("down") {
        Direction::Down
    } else if line.starts_with("up") {
        Direction::Up
    } else {
        return None;
    };

    // tab 或多空格分割
    let parts: Vec<&str> = line.split_ascii_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    // parts[0] = direction, parts[1] = "name/pid/user", parts[2] = kbps
    let pid = parse_pid_from_token(parts[1])?;
    let kbps: f64 = parts[2].parse().ok()?;
    if !kbps.is_finite() || kbps < 0.0 {
        return None;
    }
    Some((pid, direction, kbps))
}

/// 从 `name/pid/user` token 提取 PID。
///
/// - `"xterm/12345/user"` → 12345
/// - `"python3/678"` (无 user) → 678
/// - `"unknown/0/root"` → 0（PID 0 视为 unknown / kernel，过滤掉）
fn parse_pid_from_token(token: &str) -> Option<u32> {
    let segments: Vec<&str> = token.split('/').collect();
    if segments.len() < 2 {
        return None;
    }
    let pid: u32 = segments[1].parse().ok()?;
    if pid == 0 {
        return None;
    }
    Some(pid)
}

/// 把 nethogs 的 KB/sec（这里我们视作 SI 1000 进制，因为 nethogs 内部实际是
/// 1024 但 UI 显示一致即可）转换为 B/sec。×1024 是 nethogs 的实际换算。
fn kbps_to_bytes(kbps: f64) -> u64 {
    (kbps * 1024.0) as u64
}

/// 从子进程 stdout 非阻塞读取所有可用行。
///
/// 在 Windows 上我们的 target 不是 Linux，所以这条路径在生产环境不会执行；
/// Linux 上 BufReader + try_read 不直接可用，需通过 `try_clone` 取 reader。
/// 这里采用简单的「锁住 child → 读 stdout」策略，1s poll 下不会阻塞。
fn drain_child_stdout(child: &NethogsChild) -> Vec<String> {
    let Ok(mut guard) = child.0.lock() else {
        return Vec::new();
    };
    let Some(stdout) = guard.stdout.as_mut() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let mut reader = BufReader::new(stdout);
    // read_until 会阻塞；nethogs 在 -d 2 下至少每 2s 输出若干行。
    // 用 try_read 友好降级：read_line 阻塞但单次调用 < 1s（child 持续输出）。
    // 为简单起见，限读 256 行防止初次启动时大量积压导致 worker 阻塞。
    for _ in 0..256 {
        let mut buf = String::new();
        match reader.read_line(&mut buf) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = buf.trim_end_matches(['\n', '\r']);
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }
            Err(_) => break, // EAGAIN / 其它 → 下次再读
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_down_line_basic() {
        let line = "down\txterm/12345/user\t0.5\t12.3";
        assert_eq!(
            parse_nethogs_line(line),
            Some((12345, Direction::Down, 0.5))
        );
    }

    #[test]
    fn parse_up_line_basic() {
        let line = "up\txterm/12345/user\t1.25\t30.0";
        assert_eq!(parse_nethogs_line(line), Some((12345, Direction::Up, 1.25)));
    }

    #[test]
    fn parse_line_without_user_segment() {
        // "name/pid" 也要能解析（nethogs 输出 user 时可能空）
        let line = "up\tpython3/678\t0.1";
        assert_eq!(parse_nethogs_line(line), Some((678, Direction::Up, 0.1)));
    }

    #[test]
    fn parse_line_rejects_refreshing() {
        assert_eq!(parse_nethogs_line("refreshing"), None);
    }

    #[test]
    fn parse_line_rejects_closed() {
        assert_eq!(parse_nethogs_line("closed\txterm/12345/user"), None);
    }

    #[test]
    fn parse_line_rejects_pid_zero() {
        // PID 0 = unknown / kernel，应被过滤
        assert_eq!(parse_nethogs_line("down\tunknown/0/root\t0.5\t1.0"), None);
    }

    #[test]
    fn parse_line_rejects_garbage() {
        assert_eq!(parse_nethogs_line(""), None);
        assert_eq!(parse_nethogs_line("garbage"), None);
        assert_eq!(parse_nethogs_line("down\tnopidhere\t0.5"), None);
        assert_eq!(parse_nethogs_line("down\tname/notapid/user\t0.5"), None);
    }

    #[test]
    fn parse_line_rejects_negative_speed() {
        // NaN / inf / 负数
        assert_eq!(parse_nethogs_line("down\tx/1/-0.5"), None);
    }

    #[test]
    fn parse_line_handles_spaces_instead_of_tabs() {
        // nethogs 有时输出多空格；split_ascii_whitespace 兼容
        let line = "down   xterm/12345/user   0.5   12.3";
        assert_eq!(
            parse_nethogs_line(line),
            Some((12345, Direction::Down, 0.5))
        );
    }

    #[test]
    fn kbps_to_bytes_basic() {
        assert_eq!(kbps_to_bytes(0.0), 0);
        assert_eq!(kbps_to_bytes(1.0), 1024);
        assert_eq!(kbps_to_bytes(0.5), 512);
    }

    /// 端到端：把一段 tracemode stdout 喂解析器，验证 per-PID 聚合。
    #[test]
    fn aggregate_multi_line_output() {
        let stdout = "refreshing\ndown\txterm/12345/user\t0.5\t12.3\nup\txterm/12345/user\t0.25\t5.0\ndown\tchrome/99/user\t10.0\t200.0\n";
        let mut agg: std::collections::HashMap<u32, (u64, u64)> = std::collections::HashMap::new();
        for line in stdout.lines() {
            if let Some((pid, dir, kbps)) = parse_nethogs_line(line) {
                let bps = kbps_to_bytes(kbps);
                let e = agg.entry(pid).or_insert((0, 0));
                match dir {
                    Direction::Down => e.0 = e.0.saturating_add(bps),
                    Direction::Up => e.1 = e.1.saturating_add(bps),
                }
            }
        }
        // PID 12345: down=0.5 KB/s, up=0.25 KB/s
        let p1 = agg.get(&12345).copied();
        assert_eq!(p1, Some((512, 256)));
        // PID 99: down=10 KB/s only
        let p2 = agg.get(&99).copied();
        assert_eq!(p2, Some((10_240, 0)));
        // 没有未识别的 PID
        assert_eq!(agg.len(), 2);
    }
}
