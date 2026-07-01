//! 阶段 8 D3：DNS 查询日志。
//!
//! 目标：让用户看到「哪个进程查询了哪个域名」。对标 Sysmon Event ID 22。
//!
//! # 架构
//!
//! - [`DnsLogCollector`] trait 抽象平台数据源（参考阶段 6/7 的 trait 模式）
//! - Windows 主路径 [`etw`]（v0.11 阶段 2 / ADR-0020）：手写 ETW 实时 session
//!   抓 `Microsoft-Windows-DNS-Client` event 3008/3010，< 50ms 延迟 + 100% 完整性
//! - Windows fallback [`windows_dns`]：PowerShell `Get-WinEvent` 子进程订阅
//!   `Microsoft-Windows-DNS-Client/Operational` channel（event 3010 含
//!   QueryName/QueryType/QueryStatus/QueryResults + ProcessId）。仅在 ETW 启动
//!   失败时启用（非管理员 / session 占用 / x86）
//! - Linux/macOS [`windows_dns`] 在非 Windows 平台 alias 到 [`unsupported`]，
//!   [`detect_collector`] 返回 `None`。Linux 走 DBus 路线在 stage-8.md 被列为
//!   推荐方案，但 systemd-resolved 的 DBus 接口不暴露 per-query 信号 —— 实际
//!   需要走 libpcap 抓 53 端口 + DNS 协议解析 + PID 关联，工程量超出单
//!   stage 范围，留作未来 feature。详见 `docs/adr/0006-dns-subprocess-not-etw-dbus.md`。
//! - [`worker::DnsLogWorker`] 复用 [`crate::worker::SnapshotWorker`]，
//!   `POLL_INTERVAL = 500ms`（DNS 查询高频，比阶段 7 NetFlow 的 1s 更短）
//!
//! # 隐私
//!
//! DNS 查询含敏感信息（用户访问的域名），**不持久化到磁盘**。
//! - [`DnsQuery`] 即便 derive 了 `Serialize`，也只用于内存 round-trip 测试；
//!   `record/` 录屏路径不序列化任何 DNS 数据（`App::dns_log_recent` 不在
//!   [`crate::collect::SystemSnapshot`] 之内）。
//! - 状态栏提示「DNS 日志记录中（仅内存）」让用户知道采集状态。
//!
//! # PID 复用
//!
//! `DnsQuery` 含 `start_time` 字段（阶段 11 P1-A5：之前没有，PID 复用场景下
//! UI Network Tab 会显示旧进程的 DNS 历史）。`PowershellDnsCollector::reader_loop`
//! 与 `EtwDnsCollector::drain` 都从 sysinfo 查 PID 的 `start_time` 填入；UI 用
//! `(pid, start_time)` 元组过滤避免误显示。`record/frame.rs` 不序列化 `DnsQuery`
//! （ADR-0006 隐私），新字段不影响录屏格式。

#[cfg(target_os = "windows")]
pub mod etw;
pub mod unsupported;
#[cfg(target_os = "windows")]
pub mod windows_dns;
#[cfg(not(target_os = "windows"))]
pub use unsupported as windows_dns;
pub mod worker;

use std::fmt;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

/// 单条 DNS 查询记录。`Serialize` 仅用于内存 round-trip 测试；不会落盘。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsQuery {
    /// OS 事件时间戳（事件本身的时间，不是采集时间）。
    pub timestamp: std::time::SystemTime,
    /// 发起查询的进程 PID（来自事件 header）。
    pub pid: u32,
    /// 进程 start_time（阶段 11 P1-A5：从 sysinfo 查，防 PID 复用串数据）。
    /// 0 表示 sysinfo 未刷新到（PID 已退出 / refresh 周期内未捕获）。
    pub start_time: u64,
    /// 进程名（collector 侧用 sysinfo 查；查不到为 `"?"`）。
    pub process_name: String,
    /// 查询的域名（如 `example.com.`，含 trailing dot 是 Windows DNS-Client 原样输出）。
    pub query_name: String,
    /// 查询类型字符串（`A` / `AAAA` / `MX` / `TXT` / `SRV` / `PTR` / `CNAME` / 等）。
    /// Windows DNS-Client event 3010 此字段是数字字符串（`1`/`28`/`15`），
    /// collector 用 [`parse_query_type`] 转成 RFC 1035 助记符。
    pub query_type: String,
    /// 解析结果。
    pub result: DnsResult,
}

/// DNS 解析结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsResult {
    /// 成功，附带返回的 IP 列表（可能含多个 A/AAAA）。
    Success(Vec<IpAddr>),
    /// 域名不存在（NXDOMAIN）。
    NxDomain,
    /// 超时（query 发出但无响应）。
    Timeout,
    /// 其它错误，含 OS 错误码或人类可读描述。
    Error(String),
}

impl DnsResult {
    /// 把 Windows DNS-Client 的 `QueryStatus`（Win32 错误码或 DNS_STATUS）映射到
    /// [`DnsResult`]。`status == 0` → Success；`QueryResults` 字符串解析失败时
    /// 仍视为 Success 但 IP 列表为空（解析器是纯函数 [`parse_query_results`]，
    /// 测试覆盖典型格式）。
    #[must_use]
    pub fn from_windows_status(status: u32, results: &str) -> DnsResult {
        match status {
            0 => DnsResult::Success(parse_query_results(results)),
            // ERROR_DNS_NAME_NOT_FOUND (DNS_NAME_NOT_FOUND) = 14652
            // WSAHOST_NOT_FOUND = 11001
            14652 | 11001 => DnsResult::NxDomain,
            // ERROR_TIMEOUT / WSAETIMEDOUT = 10060
            10060 => DnsResult::Timeout,
            _ => DnsResult::Error(format!("win32:{status}")),
        }
    }

    /// UI 渲染 / 测试 anchor：单字符状态标记。
    #[must_use]
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Success(_) => "OK",
            Self::NxDomain => "NX",
            Self::Timeout => "TO",
            Self::Error(_) => "ERR",
        }
    }
}

impl fmt::Display for DnsResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success(ips) => {
                if ips.is_empty() {
                    write!(f, "OK")
                } else {
                    let joined = ips
                        .iter()
                        .map(std::net::IpAddr::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    write!(f, "OK:{joined}")
                }
            }
            Self::NxDomain => write!(f, "NXDOMAIN"),
            Self::Timeout => write!(f, "TIMEOUT"),
            Self::Error(s) => write!(f, "ERR:{s}"),
        }
    }
}

impl fmt::Display for DnsQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 单元测试 anchor；UI 渲染走 tui/port_panel.rs 自定义格式。
        write!(
            f,
            "[{}] pid={} {} {} {} -> {}",
            humantime_timestamp(self.timestamp),
            self.pid,
            self.process_name,
            self.query_type,
            self.query_name,
            self.result
        )
    }
}

/// DNS 查询日志采集器（参考 [`crate::net_flow::NetFlowCollector`] trait 模式）。
///
/// `drain` 是有状态的：每次调用返回「自上次调用以来 collector 内部缓冲的新事件」，
/// 内部缓冲被消费后清空。
pub trait DnsLogCollector: Send + Sync {
    /// 取出 collector 内部缓冲的所有新查询。空 Vec 表示 collector 不可用 / 暂无数据。
    fn drain(&mut self) -> Vec<DnsQuery>;

    /// 人类可读的 provider 名（用于日志 / 调试）。
    fn provider_name(&self) -> &'static str;
}

/// DNS collector 类型（v0.11 阶段 2 新增）。让 `proc diag` 输出当前实际使用
/// 的 collector，便于 bug 诊断（用户报「DNS 日志缺数据」时附上 collector 类型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DnsCollectorKind {
    /// Windows ETW `Microsoft-Windows-DNS-Client` real-time session（v0.11 阶段 2 主路径）。
    Etw,
    /// Windows PowerShell `Get-WinEvent` 子进程（fallback；ETW 失败时启用）。
    PowerShell,
    /// 平台不支持 / 所有 collector 启动失败。
    None,
}

impl DnsCollectorKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Etw => "etw",
            Self::PowerShell => "powershell",
            Self::None => "none",
        }
    }
}

impl fmt::Display for DnsCollectorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 按平台 + 权限返回合适的 collector。Windows 上先尝试 ETW（v0.11 阶段 2 主路径），
/// 失败 fallback PowerShell；其它平台返回 `(None, None)`。
///
/// 返回 tuple `(collector, kind)`：调用方需把 kind 存到 App 字段供 `proc diag`
/// 输出（worker spawn 时 collector move 进 worker body，kind 需独立保留）。
#[must_use]
pub fn detect_collector() -> (Option<Box<dyn DnsLogCollector>>, DnsCollectorKind) {
    #[cfg(target_os = "windows")]
    {
        // 主路径：ETW（v0.11 阶段 2 / ADR-0020）
        match self::etw::EtwDnsCollector::new() {
            Ok(c) => {
                tracing::info!("DNS collector: windows-etw (主路径)");
                return (Some(Box::new(c)), DnsCollectorKind::Etw);
            }
            Err(e) => {
                tracing::warn!("DNS ETW collector 启动失败，尝试 PowerShell fallback: {e}");
            }
        }
        // Fallback：PowerShell（v0.5.0 阶段 8 路径，ADR-0006）
        match self::windows_dns::PowershellDnsCollector::new() {
            Ok(c) => {
                tracing::info!("DNS collector: windows-powershell (fallback)");
                return (Some(Box::new(c)), DnsCollectorKind::PowerShell);
            }
            Err(e) => {
                tracing::warn!("Windows PowerShell DNS collector 也失败，DNS 日志为空: {e}");
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Linux/macOS：DBus 路线不暴露 per-query 信号，pcap 路线工程量大，
        // 当前 stage 不落地。详见 ADR-0006。
        tracing::debug!("DNS log collector: 此平台暂不支持，UI 将显示空列表");
    }

    (None, DnsCollectorKind::None)
}

// ------------ 纯函数解析器（单元测试 anchor） ------------

/// 把 Windows DNS-Client event 3010 的 `QueryType` 数字字符串转成 RFC 1035
/// 助记符。未知类型保留原数字字符串。
#[must_use]
pub fn parse_query_type(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('\0');
    match trimmed {
        "1" => "A".into(),
        "2" => "NS".into(),
        "5" => "CNAME".into(),
        "6" => "SOA".into(),
        "12" => "PTR".into(),
        "15" => "MX".into(),
        "16" => "TXT".into(),
        "28" => "AAAA".into(),
        "33" => "SRV".into(),
        "35" => "NAPTR".into(),
        "43" => "DS".into(),
        "46" => "RRSIG".into(),
        "48" => "DNSKEY".into(),
        "65" => "HTTPS".into(),
        // 已是助记符 / 未知类型 → 原样返回
        other => other.to_string(),
    }
}

/// 解析 Windows DNS-Client `QueryResults` 字段为 `Vec<IpAddr>`。
///
/// 格式样例（Win10/11 DNS-Client event 3010 `QueryResults`）：
/// - `1.2.3.4;5.6.7.8;;` → IPv4 列表（`;` 分隔，trailing `;;` 表示 TTL/type 后缀省略）
/// - `fe80::1;2606:4700::1;;`
/// - 空 → 空 Vec
///
/// 任何无法解析的分片忽略；不抛错（DNS 查询高频，不应因解析失败阻塞采集）。
#[must_use]
pub fn parse_query_results(raw: &str) -> Vec<IpAddr> {
    let trimmed = raw.trim().trim_end_matches('\0');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split(';')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            s.parse::<IpAddr>().ok()
        })
        .collect()
}

/// 格式化 SystemTime 为稳定的可读字符串（不依赖 chrono，避免新依赖）。
/// 格式：`HH:MM:SS`（本地时区，由 `local_offset_hours()` 提供）。
fn humantime_timestamp(t: std::time::SystemTime) -> String {
    let dur = match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return "?".into(),
    };
    let secs = dur.as_secs();
    let offset = crate::local_offset_hours() * 3600;
    let local = (secs as i64 + offset).max(0) as u64;
    let h = ((local % 86_400) / 3600) as u32;
    let m = ((local % 3600) / 60) as u32;
    let s = (local % 60) as u32;
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn dns_query_display_basic() {
        let q = DnsQuery {
            timestamp: SystemTime::UNIX_EPOCH,
            pid: 1234,
            start_time: 0,
            process_name: "chrome.exe".into(),
            query_name: "example.com.".into(),
            query_type: "A".into(),
            result: DnsResult::Success(vec!["1.2.3.4".parse().unwrap()]),
        };
        let s = q.to_string();
        assert!(s.contains("pid=1234"), "s = {s}");
        assert!(s.contains("chrome.exe"), "s = {s}");
        assert!(s.contains("example.com."), "s = {s}");
        assert!(s.contains("OK:1.2.3.4"), "s = {s}");
    }

    #[test]
    fn dns_query_serde_round_trip() {
        let q = DnsQuery {
            timestamp: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_781_308_800),
            pid: 42,
            start_time: 0,
            process_name: "curl".into(),
            query_name: "rust-lang.org.".into(),
            query_type: "AAAA".into(),
            result: DnsResult::NxDomain,
        };
        let json = serde_json::to_string(&q).expect("serialize");
        let back: DnsQuery = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(q, back);
    }

    #[test]
    fn dns_result_from_windows_status_success_empty_results() {
        let r = DnsResult::from_windows_status(0, "");
        assert!(matches!(r, DnsResult::Success(ref v) if v.is_empty()));
    }

    #[test]
    fn dns_result_from_windows_status_success_with_ips() {
        let r = DnsResult::from_windows_status(0, "1.2.3.4;5.6.7.8;;");
        match r {
            DnsResult::Success(ips) => assert_eq!(ips.len(), 2),
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[test]
    fn dns_result_from_windows_status_nxdomain() {
        assert!(matches!(
            DnsResult::from_windows_status(14652, ""),
            DnsResult::NxDomain
        ));
        assert!(matches!(
            DnsResult::from_windows_status(11001, ""),
            DnsResult::NxDomain
        ));
    }

    #[test]
    fn dns_result_from_windows_status_timeout() {
        assert!(matches!(
            DnsResult::from_windows_status(10060, ""),
            DnsResult::Timeout
        ));
    }

    #[test]
    fn dns_result_from_windows_status_error() {
        match DnsResult::from_windows_status(12345, "") {
            DnsResult::Error(s) => assert_eq!(s, "win32:12345"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn dns_result_badge_stable() {
        assert_eq!(DnsResult::Success(vec![]).badge(), "OK");
        assert_eq!(DnsResult::NxDomain.badge(), "NX");
        assert_eq!(DnsResult::Timeout.badge(), "TO");
        assert_eq!(DnsResult::Error("x".into()).badge(), "ERR");
    }

    #[test]
    fn parse_query_type_known_and_unknown() {
        assert_eq!(parse_query_type("1"), "A");
        assert_eq!(parse_query_type("28"), "AAAA");
        assert_eq!(parse_query_type("15"), "MX");
        assert_eq!(parse_query_type("65"), "HTTPS");
        // 已是助记符 / 未知 → 原样返回
        assert_eq!(parse_query_type("A"), "A");
        assert_eq!(parse_query_type("999"), "999");
        assert_eq!(parse_query_type("1\0"), "A", "trailing NUL 容忍");
    }

    #[test]
    fn parse_query_results_formats() {
        assert!(parse_query_results("").is_empty());
        assert!(parse_query_results("   ").is_empty());
        let r1 = parse_query_results("1.2.3.4;;");
        assert_eq!(r1.len(), 1);
        let r2 = parse_query_results("1.2.3.4;5.6.7.8;;");
        assert_eq!(r2.len(), 2);
        let r3 = parse_query_results("fe80::1;2606:4700::1;;");
        assert_eq!(r3.len(), 2);
        // 非法分片忽略，不抛错
        let r4 = parse_query_results("garbage;1.2.3.4");
        assert_eq!(r4.len(), 1);
    }

    #[test]
    fn dns_query_clone_eq() {
        let q1 = DnsQuery {
            timestamp: SystemTime::UNIX_EPOCH,
            pid: 1,
            start_time: 0,
            process_name: "p".into(),
            query_name: "n".into(),
            query_type: "A".into(),
            result: DnsResult::Timeout,
        };
        let q2 = q1.clone();
        assert_eq!(q1, q2);
    }
}
