//! 阶段 8 D3 DNS 查询日志 — 集成测试。
//!
//! 单元测试覆盖在 `src/dns_log/mod.rs`（trait + 类型）和
//! `src/dns_log/windows_dns.rs`（PowerShell 事件解析）。本文件聚焦：
//! - VecDeque cap=1000 FIFO 行为（模拟 [`App::tick_dns_log`]）
//! - worker + collector 集成（mock collector → SnapshotWorker → 主线程）
//! - detect_collector 跨平台不 panic

use std::collections::VecDeque;
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use proc::dns_log::worker::DnsLogSnapshot;
use proc::dns_log::{
    DnsLogCollector, DnsQuery, DnsResult, detect_collector, parse_query_results, parse_query_type,
};

/// 模拟 [`proc::app::App`] 持有的 cap=1000 FIFO 缓冲。
/// 主线程 drain worker snapshot → push_back → 超限 pop_front。
const DNS_LOG_BUFFER_CAP: usize = 1000;

fn append_with_cap(buf: &mut VecDeque<DnsQuery>, queries: Vec<DnsQuery>) {
    for q in queries {
        buf.push_back(q);
        if buf.len() > DNS_LOG_BUFFER_CAP {
            buf.pop_front();
        }
    }
}

fn mk_query(pid: u32, name: &str) -> DnsQuery {
    DnsQuery {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(pid as u64),
        pid,
        process_name: name.into(),
        query_name: format!("{pid}.example.com"),
        query_type: "A".into(),
        result: DnsResult::Success(vec!["1.2.3.4".parse::<IpAddr>().unwrap()]),
    }
}

#[test]
fn dns_buffer_cap_1000_fifo() {
    let mut buf: VecDeque<DnsQuery> = VecDeque::new();
    // 推入 1500 条 → 只保留最后 1000 条
    for i in 0..1500 {
        append_with_cap(&mut buf, vec![mk_query(i, &format!("p{i}"))]);
    }
    assert_eq!(buf.len(), DNS_LOG_BUFFER_CAP);
    // FIFO 丢弃最旧：剩下的 PID 范围是 500..=1499
    assert_eq!(buf.front().unwrap().pid, 500);
    assert_eq!(buf.back().unwrap().pid, 1499);
}

#[test]
fn dns_buffer_batch_push_keeps_fifo_order() {
    let mut buf: VecDeque<DnsQuery> = VecDeque::new();
    append_with_cap(
        &mut buf,
        vec![mk_query(1, "a"), mk_query(2, "b"), mk_query(3, "c")],
    );
    assert_eq!(buf.len(), 3);
    let pids: Vec<u32> = buf.iter().map(|q| q.pid).collect();
    assert_eq!(pids, vec![1, 2, 3]);
}

#[test]
fn dns_buffer_clear_empties() {
    let mut buf: VecDeque<DnsQuery> = VecDeque::new();
    append_with_cap(&mut buf, vec![mk_query(1, "a"), mk_query(2, "b")]);
    assert_eq!(buf.len(), 2);
    buf.clear();
    assert!(buf.is_empty());
}

/// mock collector：spawn 前预填一批 DnsQuery，drain 时一次性返回全部。
struct MockCollector {
    pending: std::sync::Mutex<Vec<DnsQuery>>,
}

impl DnsLogCollector for MockCollector {
    fn drain(&mut self) -> Vec<DnsQuery> {
        let mut guard = self.pending.lock().expect("mock mutex");
        std::mem::take(&mut *guard)
    }
    fn provider_name(&self) -> &'static str {
        "mock"
    }
}

#[test]
fn dns_worker_round_trip_through_snapshot_channel() {
    use proc::dns_log::worker::spawn as spawn_dns_worker;

    let queries = vec![
        mk_query(100, "chrome.exe"),
        mk_query(200, "curl.exe"),
        mk_query(300, "ping.exe"),
    ];
    let collector: Box<dyn DnsLogCollector> = Box::new(MockCollector {
        pending: std::sync::Mutex::new(queries.clone()),
    });
    let worker = spawn_dns_worker(collector);

    // worker 内部 500ms poll；给 1.5s 充分时间 drain 一份 snapshot。
    let mut got: Vec<DnsQuery> = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        if let Some(snap) = worker.try_recv_latest() {
            got.extend(snap.queries);
            if got.len() >= 3 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(got.len(), 3, "应收到 mock collector 预填的全部 3 条");
    let got_pids: Vec<u32> = got.iter().map(|q| q.pid).collect();
    assert!(got_pids.contains(&100));
    assert!(got_pids.contains(&200));
    assert!(got_pids.contains(&300));
    // worker drop 时干净退出（不卡死）—— 抵达此行即隐式验证。
    drop(worker);
}

use std::time::Instant;

#[test]
fn dns_worker_dropped_cleanly_when_no_data() {
    use proc::dns_log::worker::spawn as spawn_dns_worker;

    // 空 collector + 立即 drop —— 验证 worker 不卡死（recv_timeout 路径）。
    let collector: Box<dyn DnsLogCollector> = Box::new(MockCollector {
        pending: std::sync::Mutex::new(Vec::new()),
    });
    let worker = spawn_dns_worker(collector);
    let _ = worker.try_recv_latest();
    drop(worker);
    // 抵达此行 = worker 干净退出
}

#[test]
fn dns_detect_collector_does_not_panic() {
    // 平台无关：detect_collector 必须可调用并返回 Option，不允许 panic。
    // Windows 上通常返回 Some；其它平台返回 None。
    let _ = detect_collector();
}

#[test]
fn dns_parse_query_type_known_types() {
    // 集成层 anchor：覆盖解析器对一组常见类型的映射。
    assert_eq!(parse_query_type("1"), "A");
    assert_eq!(parse_query_type("28"), "AAAA");
    assert_eq!(parse_query_type("15"), "MX");
    assert_eq!(parse_query_type("16"), "TXT");
    assert_eq!(parse_query_type("33"), "SRV");
    assert_eq!(parse_query_type("65"), "HTTPS");
}

#[test]
fn dns_parse_query_results_multiple_ipv4() {
    let ips = parse_query_results("10.0.0.1;10.0.0.2;10.0.0.3;;");
    assert_eq!(ips.len(), 3);
}

#[test]
fn dns_parse_query_results_mixed_v4_v6() {
    let ips = parse_query_results("192.168.1.1;fe80::1;2606:4700::1");
    assert_eq!(ips.len(), 3);
}

#[test]
fn dns_result_badge_round_trip() {
    // badge 是 UI 稳定 anchor —— 验证 4 个分支都返回固定字面量
    assert_eq!(DnsResult::Success(vec![]).badge(), "OK");
    assert_eq!(DnsResult::NxDomain.badge(), "NX");
    assert_eq!(DnsResult::Timeout.badge(), "TO");
    assert_eq!(DnsResult::Error("e".into()).badge(), "ERR");
}

#[test]
fn dns_snapshot_default_is_empty() {
    let s = DnsLogSnapshot::default();
    assert!(s.queries.is_empty());
}
