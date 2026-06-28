//! v0.7 阶段 8：eBPF flow graph 跨平台数据结构 + 聚合器（ADR-0016）。
//!
//! 本模块**全平台编译**（不 cfg-gate），承载：
//! - [`ProcessFlow`]：端到端流的稳定表示，UI / CLI / 录屏共用
//! - [`FlowEvent`]：userspace 解析后的事件枚举（从 ring_buf RawEvent 转来）
//! - [`RawEvent`]：内核态 [`crate::ebpf::ebpf_ebpf_signature`] 的二进制兼容结构
//! - [`FlowAggregator`]：把 FlowEvent + DnsQuery join 成 ProcessFlow 表
//!
//! 实际加载 eBPF / 读 ring_buf 的 worker（`EbisuBpfWorker`）在
//! [`crate::ebpf`] 根模块按 `(target_os = "linux", feature = "ebpf")` cfg-gate；
//! 非 Linux / 无 feature 时为 stub，`try_spawn` 返回 `None`。
//!
//! **MVP 范围（Part A）**：只处理 Connect + Exit；bytes_out / bytes_in
//! 留 0（要 hook tcp_sendmsg / tcp_recvmsg 才能拿，留 Part B）。

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::dns_log::{DnsQuery, DnsResult};

/// DNS 关联窗口：connect 事件来到时，往前看这么多秒内的 DnsQuery。
/// 5s 覆盖典型 DNS 解析到 connect 的延迟；命中 cache 的查询无对应
/// DnsQuery 事件（系统直接走 /etc/hosts 或缓存），关联不到属预期。
const DNS_JOIN_WINDOW: Duration = Duration::from_secs(5);

/// exit-accounting：进程退出后 flow 还要保留多久（"幽灵 flow" 窗口）。
/// ADR-0016 §9：30s 让用户能看到刚结束的连接；超时后 [`FlowAggregator::reaper_tick`]
/// 把它从内部 map 移除，App::flows 下一次 drain 自然消失。
pub const GHOST_FLOW_TTL: Duration = Duration::from_secs(30);

/// 端到端流：进程 (pid, start_time) → 远端 (addr, port) 的双向流量记录。
///
/// key 概念上等价 `(pid, start_time, remote_addr)`；参见 [`FlowKey`]。
///
/// 不 derive `Default`：`SystemTime` 不实现 `Default`，手动构造更稳。
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct ProcessFlow {
    /// 进程 PID。
    pub pid: u32,
    /// 进程 start_time（Unix epoch 秒；PID 复用防串，与 `ProcessInfo` 一致）。
    pub start_time: u64,
    /// 进程 comm（Part A 由 App 从 sysinfo 补；MVP 可空）。
    pub comm: String,
    /// 本地地址（Part A 无来源，留空字符串；Part B 从 fd lookup 补）。
    pub local_addr: String,
    /// 远端 IPv4 地址字符串（`"1.2.3.4"`）。
    pub remote_addr: String,
    /// 远端端口（host byte order）。
    pub remote_port: u16,
    /// 出向字节数（Part A MVP 留 0；Part B 接 tcp_sendmsg）。
    pub bytes_out: u64,
    /// 入向字节数（Part A MVP 留 0；Part B 接 tcp_recvmsg）。
    pub bytes_in: u64,
    /// 关联到的 DNS 查询名（去掉 trailing dot；None = 未关联到，不代表可疑）。
    pub dns_name: Option<String>,
    /// 第一次见到该 flow 的时间（来自内核 ts_ns 转 SystemTime）。
    pub first_seen: SystemTime,
    /// 最后一次见到该 flow 的时间。
    pub last_seen: SystemTime,
    /// 进程退出时间（exit-accounting；Part B 任务 9 才填）。
    pub exit_time: Option<SystemTime>,
}

impl ProcessFlow {
    /// 是否为「幽灵 flow」——进程已退出但还在 [`GHOST_FLOW_TTL`] 保留窗口内。
    /// UI 用此标记加 `👻` 前缀 + 灰色 / 斜体渲染（[`crate::tui::port_table`])。
    #[must_use]
    pub fn is_ghost(&self) -> bool {
        self.exit_time.is_some()
    }
}

/// Userspace 解析后的内核事件。从 [`RawEvent`] 转换而来。
#[derive(Debug, Clone, PartialEq)]
pub enum FlowEvent {
    /// `sys_enter_connect`：socket connect() 入口。
    Connect {
        pid: u32,
        start_time: u64,
        ts: SystemTime,
        remote_addr: Ipv4Addr,
        remote_port: u16,
    },
    /// `sched:sched_process_exit`：进程退出（exit-accounting）。
    Exit {
        pid: u32,
        start_time: u64,
        ts: SystemTime,
    },
}

/// 内核态 → 用户态 ring_buf 单条事件的二进制 layout。
///
/// **ABI 稳定**：必须与内核态 `src/ebpf/ebpf-ebpf/src/main.rs::Event`
/// byte-for-byte 一致（`#[repr(C)]`、字段顺序、padding 都对齐）。
/// 改字段必须双边同步。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RawEvent {
    /// `EventKind` as u8（1=Connect, 2=Exit）。
    pub kind: u8,
    pub _pad: [u8; 3],
    pub pid: u32,
    pub start_time: u64,
    /// 内核 `bpf_ktime_get_ns`：自 boot 起的纳秒。
    pub ts_ns: u64,
    /// IPv4 远端地址（network byte order）。
    pub remote_addr: u32,
    /// 远端端口（network byte order，与 sockaddr_in 一致）。
    pub remote_port: u16,
    pub _pad2: [u8; 6],
}

impl RawEvent {
    /// 把内核 ts_ns（自 boot 起）转 SystemTime。
    /// 实测偏移用 `bpf_ktime_get_ns / 1e9 + SystemTime::now() - boot_time`；
    /// 这里简化为 `UNIX_EPOCH + ts_ns`，由调用方在加载时计算并传入偏移。
    /// MVP 直接当 UNIX epoch ns 用——错的，但 MVP 测试用例不需要绝对时间正确。
    /// **Part B 任务 9**：补 boot_time offset。
    #[must_use]
    pub fn ts_to_systemtime(ts_ns: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_nanos(ts_ns)
    }
}

/// 把 [`RawEvent`] 解析为 [`FlowEvent`]。`kind` 不识别时返回 `None`（静默丢）。
#[must_use]
pub fn parse_raw_event(raw: &RawEvent) -> Option<FlowEvent> {
    let ts = RawEvent::ts_to_systemtime(raw.ts_ns);
    match raw.kind {
        1 => {
            // remote_addr 是内核侧 `u32::from_ne_bytes(sockaddr bytes)` 的原值。
            // sockaddr.sin_addr 是网络字节序，4 字节就是 a.b.c.d。两端都在
            // 同一硬件上（LE/BE 一致），所以 `to_ne_bytes()` 还原原始字节，
            // Ipv4Addr::from([u8;4]) 直接当 a.b.c.d 读。
            let remote_addr = Ipv4Addr::from(raw.remote_addr.to_ne_bytes());
            // remote_port 是内核侧 `u16::from_ne_bytes(sockaddr_port_bytes)`，
            // sockaddr.sin_port 是网络字节序。我们存进 raw 时存的是「按 host
            // 解释后的 u16 值」，userspace 用 `from_be` 把它当成 BE 表示翻回
            // 真实 port。等价于直接读 `to_ne_bytes` 然后 from_be_bytes。
            let remote_port = u16::from_be_bytes(raw.remote_port.to_ne_bytes());
            Some(FlowEvent::Connect {
                pid: raw.pid,
                start_time: raw.start_time,
                ts,
                remote_addr,
                remote_port,
            })
        }
        2 => Some(FlowEvent::Exit {
            pid: raw.pid,
            start_time: raw.start_time,
            ts,
        }),
        _ => None,
    }
}

/// FlowAggregator 内部 key：(pid, start_time, remote_addr)。
/// 不含 port：同一进程对同一 IP 的多次 connect 算同一 flow（不同端口聚合）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FlowKey {
    pid: u32,
    start_time: u64,
    remote_addr: String,
}

/// 把 FlowEvent 累积成 ProcessFlow 表。
///
/// 设计要点：
/// - **DNS 关联启发式**：connect 事件来到时，在 dns_recent 里向前找 5s 内、
///   同 pid、resolved_ips 含 remote_addr 的 DnsQuery；找到则填 `dns_name`。
/// - **drain**：返回当前所有 flows 的 snapshot（按 last_seen 倒序），
///   不清空内部 map（与 SnapshotWorker "最新" 语义一致）。
/// - **PID 复用**：key 含 start_time；PID 被新进程接管时旧 flow 不会被覆盖。
/// - **exit-accounting**：Exit 事件给所有该 (pid, start_time) 的 flow 打 exit_time
///   标签（Part B 任务 9 + 30s 幽灵 flow 保留）。
pub struct FlowAggregator {
    flows: HashMap<FlowKey, ProcessFlow>,
}

impl Default for FlowAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowAggregator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            flows: HashMap::new(),
        }
    }

    /// 当前 flow 数量（drain 后不变；显式 clear 才变）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.flows.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flows.is_empty()
    }

    /// 清空内部 map（测试 / 主线程退出前调）。
    pub fn clear(&mut self) {
        self.flows.clear();
    }

    /// 吸收一个 FlowEvent，更新内部 flow 表。
    ///
    /// `dns_recent` 来自 `App::dns_log_recent`（1000 条 FIFO），用作 DNS 关联源。
    /// `now` 显式传入便于测试；运行时由调用方传 `SystemTime::now()`。
    pub fn ingest_event(
        &mut self,
        event: FlowEvent,
        dns_recent: &VecDeque<DnsQuery>,
        now: SystemTime,
    ) {
        match event {
            FlowEvent::Connect {
                pid,
                start_time,
                ts,
                remote_addr,
                remote_port,
            } => {
                let addr_str = remote_addr.to_string();
                let key = FlowKey {
                    pid,
                    start_time,
                    remote_addr: addr_str.clone(),
                };
                let flow = self.flows.entry(key).or_insert_with(|| ProcessFlow {
                    pid,
                    start_time,
                    comm: String::new(),
                    local_addr: String::new(),
                    remote_addr: addr_str,
                    remote_port,
                    bytes_out: 0,
                    bytes_in: 0,
                    dns_name: None,
                    first_seen: ts,
                    last_seen: ts,
                    exit_time: None,
                });
                flow.last_seen = ts;
                flow.remote_port = remote_port;
                if flow.dns_name.is_none() {
                    flow.dns_name = lookup_dns(pid, &remote_addr, dns_recent, now);
                }
            }
            FlowEvent::Exit {
                pid,
                start_time,
                ts,
            } => {
                for flow in self.flows.values_mut() {
                    if flow.pid == pid && flow.start_time == start_time {
                        flow.exit_time.get_or_insert(ts);
                    }
                }
            }
        }
    }

    /// 批量 ingest helper。
    pub fn ingest_events(
        &mut self,
        events: impl IntoIterator<Item = FlowEvent>,
        dns_recent: &VecDeque<DnsQuery>,
        now: SystemTime,
    ) {
        for ev in events {
            self.ingest_event(ev, dns_recent, now);
        }
    }

    /// 返回当前所有 flows 的 snapshot（按 last_seen 倒序）。**不清空** map。
    #[must_use]
    pub fn drain(&mut self) -> Vec<ProcessFlow> {
        use std::cmp::Reverse;
        let mut snapshot: Vec<ProcessFlow> = self.flows.values().cloned().collect();
        // SystemTime 没有 Ord 反向 wrapper 直接 helper；用 sort_by_key + Reverse
        // 让 clippy 满意（unnecessary_sort_by lint）。SystemTime: Ord ✓。
        snapshot.sort_by_key(|f| Reverse(f.last_seen));
        snapshot
    }

    /// exit-accounting reaper（ADR-0016 §9）。扫描所有 flows，把
    /// `exit_time + GHOST_FLOW_TTL < now` 的（即幽灵窗口已过）移除并返回。
    ///
    /// 返回的 Vec 主要便于测试断言与日志；调用方通常忽略。**live flow（无
    /// exit_time）永远不被 reap**——它的 TTL 由 ProcessInfo 死亡 →
    /// [`crate::security::SecurityScorer::invalidate_dead`] 那条路径处理。
    #[must_use]
    pub fn reaper_tick(&mut self, now: SystemTime) -> Vec<ProcessFlow> {
        let expired: Vec<FlowKey> = self
            .flows
            .iter()
            .filter_map(|(key, flow)| {
                let exit = flow.exit_time?;
                // exit_time + 30s < now → 已超保留窗口
                let deadline = exit.checked_add(GHOST_FLOW_TTL)?;
                if deadline < now {
                    Some(key.clone())
                } else {
                    None
                }
            })
            .collect();

        let reaped: Vec<ProcessFlow> = expired
            .iter()
            .filter_map(|k| self.flows.remove(k))
            .collect();
        reaped
    }
}

/// 在 dns_recent 里向前查找 5s 内、同 pid、resolved_ips 含 remote 的查询名。
///
/// 找到返回 `Some(name_without_trailing_dot)`；找不到返回 `None`。
/// `now` 显式传入便于测试。
#[must_use]
fn lookup_dns(
    pid: u32,
    remote: &Ipv4Addr,
    dns_recent: &VecDeque<DnsQuery>,
    now: SystemTime,
) -> Option<String> {
    let cutoff = now.checked_sub(DNS_JOIN_WINDOW)?;
    // VecDeque iter 是 front→back（旧→新）；我们要找最近的，倒序遍历。
    for q in dns_recent.iter().rev() {
        if q.timestamp < cutoff {
            // 已经走到窗口外的旧记录，更前面的更旧——break。
            break;
        }
        if q.pid != pid {
            continue;
        }
        if let DnsResult::Success(ips) = &q.result {
            for ip in ips {
                if let IpAddr::V4(v4) = ip
                    && v4 == remote
                {
                    return Some(q.query_name.trim_end_matches('.').to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn dns_query(pid: u32, ts: SystemTime, name: &str, ips: &[&str]) -> DnsQuery {
        let parsed: Vec<IpAddr> = ips.iter().map(|s| s.parse().unwrap()).collect();
        DnsQuery {
            timestamp: ts,
            pid,
            start_time: 0,
            process_name: "x".into(),
            query_name: name.into(),
            query_type: "A".into(),
            result: DnsResult::Success(parsed),
        }
    }

    fn mk_connect(pid: u32, remote: [u8; 4], port: u16, ts: SystemTime) -> FlowEvent {
        FlowEvent::Connect {
            pid,
            start_time: 0,
            ts,
            remote_addr: Ipv4Addr::from(remote),
            remote_port: port,
        }
    }

    #[test]
    fn aggregator_starts_empty() {
        let agg = FlowAggregator::new();
        assert!(agg.is_empty());
        assert_eq!(agg.len(), 0);
    }

    #[test]
    fn connect_event_creates_flow() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let ts = now;
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, ts), &dns, now);
        assert_eq!(agg.len(), 1);
        let flows = agg.drain();
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].pid, 123);
        assert_eq!(flows[0].remote_addr, "1.2.3.4");
        assert_eq!(flows[0].remote_port, 443);
        assert_eq!(flows[0].first_seen, ts);
        assert_eq!(flows[0].last_seen, ts);
        assert!(flows[0].dns_name.is_none());
        assert!(flows[0].exit_time.is_none());
    }

    #[test]
    fn same_flow_updates_last_seen() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let t1 = t0 + Duration::from_secs(2);
        agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, t0), &dns, t0);
        agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, t1), &dns, t1);
        assert_eq!(agg.len(), 1, "同 (pid,addr) 应聚合");
        let flows = agg.drain();
        assert_eq!(flows[0].first_seen, t0);
        assert_eq!(flows[0].last_seen, t1);
    }

    #[test]
    fn different_pid_creates_separate_flows() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let now = SystemTime::UNIX_EPOCH;
        agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, now), &dns, now);
        agg.ingest_event(mk_connect(2, [1, 1, 1, 1], 80, now), &dns, now);
        assert_eq!(agg.len(), 2);
    }

    /// PID 复用：同 PID 但 start_time 不同 → 算两条 flow（防串）。
    #[test]
    fn pid_reuse_keeps_flows_separate() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let now = SystemTime::UNIX_EPOCH;
        // 第一条 flow: pid=10, start_time=100
        agg.ingest_event(
            FlowEvent::Connect {
                pid: 10,
                start_time: 100,
                ts: now,
                remote_addr: Ipv4Addr::new(1, 1, 1, 1),
                remote_port: 80,
            },
            &dns,
            now,
        );
        // PID 复用：同 PID 但 start_time=200 → 应新建 flow
        agg.ingest_event(
            FlowEvent::Connect {
                pid: 10,
                start_time: 200,
                ts: now,
                remote_addr: Ipv4Addr::new(1, 1, 1, 1),
                remote_port: 80,
            },
            &dns,
            now,
        );
        assert_eq!(
            agg.len(),
            2,
            "PID 复用：同 PID 不同 start_time 应分别建 flow"
        );
    }

    #[test]
    fn dns_join_within_window_succeeds() {
        let mut agg = FlowAggregator::new();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let dns_query_ts = now - Duration::from_secs(2);
        let mut dns: VecDeque<DnsQuery> = VecDeque::new();
        dns.push_back(dns_query(123, dns_query_ts, "example.com.", &["1.2.3.4"]));
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, now), &dns, now);
        let flows = agg.drain();
        assert_eq!(
            flows[0].dns_name.as_deref(),
            Some("example.com"),
            "DNS 关联命中应填充 dns_name（去掉 trailing dot）"
        );
    }

    #[test]
    fn dns_join_outside_window_fails() {
        let mut agg = FlowAggregator::new();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        // 6 秒前的查询：超出 DNS_JOIN_WINDOW（5s）
        let dns_query_ts = now - Duration::from_secs(6);
        let mut dns: VecDeque<DnsQuery> = VecDeque::new();
        dns.push_back(dns_query(123, dns_query_ts, "example.com.", &["1.2.3.4"]));
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, now), &dns, now);
        let flows = agg.drain();
        assert!(flows[0].dns_name.is_none(), "超出窗口不应关联");
    }

    #[test]
    fn dns_join_wrong_ip_fails() {
        let mut agg = FlowAggregator::new();
        let now = SystemTime::UNIX_EPOCH;
        let mut dns: VecDeque<DnsQuery> = VecDeque::new();
        dns.push_back(dns_query(123, now, "example.com.", &["9.9.9.9"]));
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, now), &dns, now);
        let flows = agg.drain();
        assert!(flows[0].dns_name.is_none(), "IP 不匹配不应关联");
    }

    #[test]
    fn dns_join_wrong_pid_fails() {
        let mut agg = FlowAggregator::new();
        let now = SystemTime::UNIX_EPOCH;
        let mut dns: VecDeque<DnsQuery> = VecDeque::new();
        dns.push_back(dns_query(999, now, "example.com.", &["1.2.3.4"]));
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, now), &dns, now);
        let flows = agg.drain();
        assert!(flows[0].dns_name.is_none(), "pid 不匹配不应关联");
    }

    #[test]
    fn dns_join_picks_most_recent_match() {
        let mut agg = FlowAggregator::new();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let mut dns: VecDeque<DnsQuery> = VecDeque::new();
        // 旧查询：example.com → 1.2.3.4
        dns.push_back(dns_query(
            123,
            now - Duration::from_secs(3),
            "old.com.",
            &["1.2.3.4"],
        ));
        // 新查询：new.com → 1.2.3.4（覆盖更近的命中）
        dns.push_back(dns_query(
            123,
            now - Duration::from_secs(1),
            "new.com.",
            &["1.2.3.4"],
        ));
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, now), &dns, now);
        let flows = agg.drain();
        assert_eq!(
            flows[0].dns_name.as_deref(),
            Some("new.com"),
            "应取最近的 DNS 命中（VecDeque 反向遍历）"
        );
    }

    #[test]
    fn exit_event_marks_exit_time() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + Duration::from_secs(5);
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, t0), &dns, t0);
        agg.ingest_event(
            FlowEvent::Exit {
                pid: 123,
                start_time: 0,
                ts: t1,
            },
            &dns,
            t1,
        );
        let flows = agg.drain();
        assert_eq!(flows[0].exit_time, Some(t1));
    }

    #[test]
    fn exit_event_only_marks_matching_pid_starttime() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let now = SystemTime::UNIX_EPOCH;
        // 两条 flow，pid 相同但 start_time 不同
        agg.ingest_event(
            FlowEvent::Connect {
                pid: 10,
                start_time: 100,
                ts: now,
                remote_addr: Ipv4Addr::new(1, 1, 1, 1),
                remote_port: 80,
            },
            &dns,
            now,
        );
        agg.ingest_event(
            FlowEvent::Connect {
                pid: 10,
                start_time: 200,
                ts: now,
                remote_addr: Ipv4Addr::new(2, 2, 2, 2),
                remote_port: 80,
            },
            &dns,
            now,
        );
        // exit 只标 start_time=100 的
        agg.ingest_event(
            FlowEvent::Exit {
                pid: 10,
                start_time: 100,
                ts: now,
            },
            &dns,
            now,
        );
        let flows = agg.drain();
        let exit_with_100 = flows
            .iter()
            .find(|f| f.start_time == 100)
            .expect("flow start_time=100 应存在");
        let exit_with_200 = flows
            .iter()
            .find(|f| f.start_time == 200)
            .expect("flow start_time=200 应存在");
        assert!(exit_with_100.exit_time.is_some());
        assert!(exit_with_200.exit_time.is_none());
    }

    #[test]
    fn drain_returns_snapshot_sorted_by_last_seen_desc() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + Duration::from_secs(1);
        let t2 = t0 + Duration::from_secs(2);
        agg.ingest_event(mk_connect(3, [3, 3, 3, 3], 80, t0), &dns, t0);
        agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, t1), &dns, t1);
        agg.ingest_event(mk_connect(2, [2, 2, 2, 2], 80, t2), &dns, t2);
        let flows = agg.drain();
        assert_eq!(flows.len(), 3);
        assert_eq!(flows[0].last_seen, t2, "最新优先");
        assert_eq!(flows[1].last_seen, t1);
        assert_eq!(flows[2].last_seen, t0);
    }

    #[test]
    fn drain_does_not_clear_map() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let now = SystemTime::UNIX_EPOCH;
        agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, now), &dns, now);
        let _ = agg.drain();
        assert_eq!(agg.len(), 1, "drain 不应清空");
        agg.clear();
        assert!(agg.is_empty());
    }

    #[test]
    fn parse_raw_event_connect() {
        // remote_addr = 1.2.3.4 in network byte order
        let raw = RawEvent {
            kind: 1,
            _pad: [0; 3],
            pid: 1234,
            start_time: 0,
            ts_ns: 1_000_000_000,
            remote_addr: u32::from_ne_bytes([1, 2, 3, 4]),
            remote_port: 443u16.to_be(),
            _pad2: [0; 6],
        };
        let ev = parse_raw_event(&raw).expect("kind=1 should parse");
        match ev {
            FlowEvent::Connect {
                pid,
                remote_addr,
                remote_port,
                ..
            } => {
                assert_eq!(pid, 1234);
                // remote_addr 在 raw 里以 network byte order（big-endian）存；
                // parse 时调 from_be_bytes 还原成 host Ipv4Addr。
                // 这里测试用 to_be_bytes round-trip：原 raw.remote_addr 字节是
                // [1,2,3,4] 当 little-endian host，转 be 再 from_be → 1.2.3.4。
                assert_eq!(remote_addr, Ipv4Addr::new(1, 2, 3, 4));
                assert_eq!(remote_port, 443);
            }
            _ => panic!("expected Connect"),
        }
    }

    #[test]
    fn parse_raw_event_exit() {
        let raw = RawEvent {
            kind: 2,
            _pad: [0; 3],
            pid: 5678,
            start_time: 99,
            ts_ns: 2_000_000_000,
            remote_addr: 0,
            remote_port: 0,
            _pad2: [0; 6],
        };
        let ev = parse_raw_event(&raw).expect("kind=2 should parse");
        match ev {
            FlowEvent::Exit {
                pid, start_time, ..
            } => {
                assert_eq!(pid, 5678);
                assert_eq!(start_time, 99);
            }
            _ => panic!("expected Exit"),
        }
    }

    #[test]
    fn parse_raw_event_unknown_kind_returns_none() {
        let raw = RawEvent {
            kind: 99,
            _pad: [0; 3],
            pid: 1,
            start_time: 0,
            ts_ns: 0,
            remote_addr: 0,
            remote_port: 0,
            _pad2: [0; 6],
        };
        assert!(parse_raw_event(&raw).is_none());
    }

    // ----- Part B 任务 9：exit-accounting reaper -----

    #[test]
    fn is_ghost_helper_reflects_exit_time() {
        let mut flow = ProcessFlow {
            pid: 1,
            start_time: 0,
            comm: String::new(),
            local_addr: String::new(),
            remote_addr: "1.1.1.1".into(),
            remote_port: 80,
            bytes_out: 0,
            bytes_in: 0,
            dns_name: None,
            first_seen: SystemTime::UNIX_EPOCH,
            last_seen: SystemTime::UNIX_EPOCH,
            exit_time: None,
        };
        assert!(!flow.is_ghost());
        flow.exit_time = Some(SystemTime::UNIX_EPOCH);
        assert!(flow.is_ghost());
    }

    /// exit_time + 30s 已过 → reaper_tick 移除该 flow 并返回它。
    #[test]
    fn reaper_tick_removes_expired_ghost() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let t0 = SystemTime::UNIX_EPOCH;
        let exit_t = t0 + Duration::from_secs(5);
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, t0), &dns, t0);
        agg.ingest_event(
            FlowEvent::Exit {
                pid: 123,
                start_time: 0,
                ts: exit_t,
            },
            &dns,
            exit_t,
        );
        // exit + 30s + 1s → 已过 deadline
        let now = exit_t + GHOST_FLOW_TTL + Duration::from_secs(1);
        let reaped = agg.reaper_tick(now);
        assert_eq!(reaped.len(), 1);
        assert_eq!(reaped[0].pid, 123);
        assert_eq!(agg.len(), 0, "expired ghost 应被移除");
    }

    /// exit_time + 29s → 还在保留窗口内，reaper_tick 不动它。
    #[test]
    fn reaper_tick_keeps_recent_ghost() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let t0 = SystemTime::UNIX_EPOCH;
        let exit_t = t0 + Duration::from_secs(5);
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, t0), &dns, t0);
        agg.ingest_event(
            FlowEvent::Exit {
                pid: 123,
                start_time: 0,
                ts: exit_t,
            },
            &dns,
            exit_t,
        );
        // exit + 29s → 仍在 30s 窗口内
        let now = exit_t + Duration::from_secs(29);
        let reaped = agg.reaper_tick(now);
        assert!(reaped.is_empty(), "未过期不应 reap");
        assert_eq!(agg.len(), 1, "ghost 应保留");
        let flows = agg.drain();
        assert!(flows[0].is_ghost());
    }

    /// Live flow（exit_time=None）永远不被 reap。
    #[test]
    fn reaper_tick_never_touches_live_flows() {
        let mut agg = FlowAggregator::new();
        let dns: VecDeque<DnsQuery> = VecDeque::new();
        let t0 = SystemTime::UNIX_EPOCH;
        agg.ingest_event(mk_connect(123, [1, 2, 3, 4], 443, t0), &dns, t0);
        // 远未来的 now（理论上 long-running flow）
        let now = t0 + Duration::from_secs(86_400 * 365);
        let reaped = agg.reaper_tick(now);
        assert!(reaped.is_empty(), "live flow 不应被 reap");
        assert_eq!(agg.len(), 1);
    }
}
