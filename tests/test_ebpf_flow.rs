//! v0.7 阶段 8 Part A：eBPF flow graph 单元测试（ADR-0016）。
//!
//! 这些测试**跨平台**：覆盖 [`proc::ebpf::flow`] 的纯逻辑（FlowAggregator /
//! RawEvent 解析 / DNS 关联），不依赖实际 eBPF 加载。Linux + ebpf feature
//! 的真实 tracepoint attach 测试需要 root + bpf-linker，留 Linux 会话手动验证。

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, SystemTime};

use proc::dns_log::{DnsQuery, DnsResult};
use proc::ebpf::flow::{FlowAggregator, FlowEvent, ProcessFlow, RawEvent, parse_raw_event};

fn dns_query(ts: SystemTime, pid: u32, name: &str, ips: &[&str]) -> DnsQuery {
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

/// Connect 事件应在 flow 表里创建一条新条目；基本字段（pid/remote/port/...）
/// 应原样保留；bytes_out / bytes_in MVP 应为 0；dns_name 在无 DNS 关联源时
/// 应为 None。
#[test]
fn connect_event_creates_flow() {
    let mut agg = FlowAggregator::new();
    let dns: VecDeque<DnsQuery> = VecDeque::new();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);

    agg.ingest_event(mk_connect(1234, [1, 2, 3, 4], 443, now), &dns, now);

    assert_eq!(agg.len(), 1);
    let flows = agg.drain();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].pid, 1234);
    assert_eq!(flows[0].remote_addr, "1.2.3.4");
    assert_eq!(flows[0].remote_port, 443);
    assert_eq!(flows[0].bytes_out, 0, "MVP bytes_out 应为 0");
    assert_eq!(flows[0].bytes_in, 0, "MVP bytes_in 应为 0");
    assert!(flows[0].dns_name.is_none());
    assert!(flows[0].exit_time.is_none());
    assert_eq!(flows[0].first_seen, now);
    assert_eq!(flows[0].last_seen, now);
}

/// 同 (pid, start_time, remote_addr) 多次 connect 应聚合为一条 flow；
/// last_seen 更新为最新 ts。
#[test]
fn repeated_connect_updates_last_seen() {
    let mut agg = FlowAggregator::new();
    let dns: VecDeque<DnsQuery> = VecDeque::new();
    let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let t1 = t0 + Duration::from_secs(3);

    agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, t0), &dns, t0);
    agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, t1), &dns, t1);

    assert_eq!(agg.len(), 1);
    let flows = agg.drain();
    assert_eq!(flows[0].first_seen, t0);
    assert_eq!(flows[0].last_seen, t1);
}

/// DNS 关联命中：dns_recent 里 5s 内、同 pid、resolved_ips 含 remote → 应填
/// dns_name（去掉 trailing dot）。
#[test]
fn dns_join_within_window_fills_name() {
    let mut agg = FlowAggregator::new();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let mut dns: VecDeque<DnsQuery> = VecDeque::new();
    dns.push_back(dns_query(
        now - Duration::from_secs(2),
        1234,
        "example.com.",
        &["1.2.3.4"],
    ));

    agg.ingest_event(mk_connect(1234, [1, 2, 3, 4], 443, now), &dns, now);

    let flows = agg.drain();
    assert_eq!(flows[0].dns_name.as_deref(), Some("example.com"));
}

/// DNS 关联窗口外的查询不应被关联。
#[test]
fn dns_join_outside_window_skipped() {
    let mut agg = FlowAggregator::new();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let mut dns: VecDeque<DnsQuery> = VecDeque::new();
    // 6 秒前：超出 5s 窗口
    dns.push_back(dns_query(
        now - Duration::from_secs(6),
        1234,
        "example.com.",
        &["1.2.3.4"],
    ));

    agg.ingest_event(mk_connect(1234, [1, 2, 3, 4], 443, now), &dns, now);

    let flows = agg.drain();
    assert!(flows[0].dns_name.is_none());
}

/// PID 复用：同 pid 但 start_time 不同应建独立 flow（防数据串）。
#[test]
fn pid_reuse_separates_flows_by_start_time() {
    let mut agg = FlowAggregator::new();
    let dns: VecDeque<DnsQuery> = VecDeque::new();
    let now = SystemTime::UNIX_EPOCH;

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
            remote_addr: Ipv4Addr::new(1, 1, 1, 1),
            remote_port: 80,
        },
        &dns,
        now,
    );

    assert_eq!(agg.len(), 2);
}

/// drain 返回 snapshot 但**不清空** map（与 SnapshotWorker "最新" 语义一致）。
#[test]
fn drain_returns_snapshot_without_clearing() {
    let mut agg = FlowAggregator::new();
    let dns: VecDeque<DnsQuery> = VecDeque::new();
    let now = SystemTime::UNIX_EPOCH;

    agg.ingest_event(mk_connect(1, [1, 1, 1, 1], 80, now), &dns, now);
    let _ = agg.drain();
    assert_eq!(agg.len(), 1, "drain 不应清空");
    agg.clear();
    assert!(agg.is_empty());
}

/// drain 返回按 last_seen 倒序的列表。
#[test]
fn drain_sorts_by_last_seen_desc() {
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
    assert_eq!(flows[0].last_seen, t2);
    assert_eq!(flows[1].last_seen, t1);
    assert_eq!(flows[2].last_seen, t0);
}

/// Exit 事件应给所有匹配 (pid, start_time) 的 flow 打 exit_time 标签。
#[test]
fn exit_event_marks_flows() {
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

/// ProcessFlow 序列化为 JSON 应包含所有字段（用于诊断 / 未来 MCP 导出）。
#[test]
fn process_flow_serde_round_trip() {
    let flow = ProcessFlow {
        pid: 1234,
        start_time: 9_999,
        comm: "curl".into(),
        local_addr: "192.168.1.1:50000".into(),
        remote_addr: "1.2.3.4".into(),
        remote_port: 443,
        bytes_out: 0,
        bytes_in: 0,
        dns_name: Some("example.com".into()),
        sni: Some("example.com".into()),
        source: proc::ebpf::flow::FlowSource::Ebpf,
        first_seen: SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
        last_seen: SystemTime::UNIX_EPOCH + Duration::from_secs(1005),
        exit_time: None,
    };
    let json = serde_json::to_string(&flow).expect("serialize");
    let back: ProcessFlow = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(flow, back);
}

/// RawEvent → FlowEvent 解析：kind=1 应得 Connect；kind=2 应得 Exit；
/// 未知 kind 应返回 None（静默丢）。
#[test]
fn parse_raw_event_kind_dispatch() {
    let connect_raw = RawEvent {
        kind: 1,
        _pad: [0; 3],
        pid: 100,
        start_time: 999,
        ts_ns: 1_000_000_000,
        remote_addr: u32::from_ne_bytes([1, 2, 3, 4]),
        remote_port: 443u16.to_be(),
        _pad2: [0; 6],
    };
    let exit_raw = RawEvent {
        kind: 2,
        _pad: [0; 3],
        pid: 200,
        start_time: 888,
        ts_ns: 2_000_000_000,
        remote_addr: 0,
        remote_port: 0,
        _pad2: [0; 6],
    };
    let unknown_raw = RawEvent {
        kind: 99,
        _pad: [0; 3],
        pid: 1,
        start_time: 0,
        ts_ns: 0,
        remote_addr: 0,
        remote_port: 0,
        _pad2: [0; 6],
    };

    assert!(matches!(
        parse_raw_event(&connect_raw),
        Some(FlowEvent::Connect { pid: 100, .. })
    ));
    assert!(matches!(
        parse_raw_event(&exit_raw),
        Some(FlowEvent::Exit { pid: 200, .. })
    ));
    assert!(parse_raw_event(&unknown_raw).is_none());
}

/// RawEvent 内存 layout 应满足 ABI 稳定性要求（不随编译器版本变化）。
/// 字段顺序、padding 与内核态 `Event`（src/ebpf/ebpf-ebpf/src/main.rs）对齐。
#[test]
fn raw_event_abi_layout() {
    use std::mem::{align_of, size_of};
    // 期望：1 + 3 (pad) + 4 (pid) + 8 (start_time) + 8 (ts_ns) + 4 (addr) + 2 (port) + 6 (pad2) = 36
    // 但 align(8) → round up to 40
    assert_eq!(
        size_of::<RawEvent>(),
        40,
        "RawEvent ABI 大小应符合内核态约定"
    );
    assert_eq!(align_of::<RawEvent>(), 8);
    // offset 检查：kind @ 0, pid @ 4, start_time @ 8, ts_ns @ 16, remote_addr @ 24, remote_port @ 28
    let raw = RawEvent {
        kind: 1,
        _pad: [0; 3],
        pid: 0xDEAD_BEEF,
        start_time: 0,
        ts_ns: 0,
        remote_addr: 0,
        remote_port: 0,
        _pad2: [0; 6],
    };
    let ptr = &raw as *const _ as *const u8;
    unsafe {
        let pid_byte = *ptr.add(4);
        assert_eq!(pid_byte, 0xEF, "pid offset=4 (little-endian low byte)");
    }
}

/// DNS 关联：多条查询命中时取最近的（VecDeque 反向遍历）。
#[test]
fn dns_join_picks_most_recent_match() {
    let mut agg = FlowAggregator::new();
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
    let mut dns: VecDeque<DnsQuery> = VecDeque::new();
    // 旧命中
    dns.push_back(dns_query(
        now - Duration::from_secs(3),
        1234,
        "old.com.",
        &["1.2.3.4"],
    ));
    // 新命中（覆盖）
    dns.push_back(dns_query(
        now - Duration::from_secs(1),
        1234,
        "new.com.",
        &["1.2.3.4"],
    ));

    agg.ingest_event(mk_connect(1234, [1, 2, 3, 4], 443, now), &dns, now);

    let flows = agg.drain();
    assert_eq!(flows[0].dns_name.as_deref(), Some("new.com"));
}

/// `EbisuBpfWorker` / `try_spawn` 在非 Linux / 无 feature 平台应返回 None。
/// Linux + ebpf feature 平台若 attach 失败（无 root / 内核太老）也应返回 None
/// 而非 panic —— 此测试不验证 root 路径（需要 root 才有意义），只验证 API
/// 不 panic 即可。
#[test]
fn try_spawn_does_not_panic_when_unavailable() {
    let _worker = proc::ebpf::try_spawn(None);
    // 任何结果（Some / None）都可接受，关键是 try_spawn 不 panic。
}

/// `drain_events` stub 路径：worker=None 时应返回空 Vec（非 Linux 平台 /
/// 无 feature 走此路径）。
#[test]
fn drain_events_with_no_worker_returns_empty() {
    let events = proc::ebpf::drain_events(None);
    assert!(events.is_empty());
}

/// `EBPF_ENABLED` 常量应反映当前编译配置：
/// - Linux + feature `ebpf` → true
/// - 其它 → false
///
/// 这条测试 anchor 给上层 UI 一个稳定契约。
#[test]
fn ebpf_enabled_constant_matches_cfg() {
    let expected = cfg!(all(target_os = "linux", feature = "ebpf"));
    assert_eq!(proc::ebpf::EBPF_ENABLED, expected);
}

// ---------------------------------------------------------------------------
// Linux + ebpf feature only：真实加载 eBPF 的集成测试。
// 手动跑（CI 无 root）：sudo cargo test --release --features ebpf --test test_ebpf_flow -- --ignored
// ---------------------------------------------------------------------------

#[cfg(all(target_os = "linux", feature = "ebpf"))]
#[test]
#[ignore = "需要 root + Linux 5.10+ + bpf-linker；CI 不跑"]
fn ebpf_attaches_under_root() {
    // spawn worker；attach 应成功（root + 5.10+ 内核）。
    // 测试本身不强依赖外部流量；只要 try_spawn 返回 Some 即通过。
    let worker = proc::ebpf::try_spawn(None);
    assert!(
        worker.is_some(),
        "Linux + ebpf feature + root 下 try_spawn 应返回 Some；\
         None 表示 attach 失败（检查内核版本 / CAP_BPF）"
    );
}
