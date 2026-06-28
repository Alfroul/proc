//! Concurrency tests for `BackgroundScorer`.
//!
//! These tests exercise the channel-based handoff between the main thread
//! (`request` / `poll_results`) and the background scoring thread. The scorer
//! is intentionally minimal in its API surface, so the tests lean on timing
//! (`std::thread::sleep`) — but each test asserts a specific contract that
//! would catch regressions even on a slow CI machine.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use proc::collect::ProcessInfo;
use proc::ebpf::flow::ProcessFlow;
use proc::port_map::PortEntry;
use proc::security::BackgroundScorer;

type ScorerRequest = (
    Arc<Vec<ProcessInfo>>,
    Arc<Vec<PortEntry>>,
    Arc<Vec<ProcessFlow>>,
);

fn empty_request() -> ScorerRequest {
    (
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
    )
}

/// When the worker is still busy with a previous request, the second request
/// must be silently dropped (sync_channel(1) full → try_send fails). The
/// contract: `request` never blocks the caller.
#[test]
fn test_scorer_request_drops_when_busy() {
    let scorer = BackgroundScorer::new();

    // Prime the channel with one request so it's full.
    let (procs, ports, flows) = big_request();
    scorer.request(procs.clone(), ports.clone(), flows.clone());
    // Second request immediately — channel is full, must drop without blocking.
    let start = std::time::Instant::now();
    scorer.request(procs, ports, flows);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(50),
        "second request should not block, took {:?}",
        elapsed
    );
}

/// When no scoring has been requested, poll_results must return None
/// immediately (no blocking).
#[test]
fn test_scorer_poll_results_non_blocking() {
    let scorer = BackgroundScorer::new();
    let start = std::time::Instant::now();
    let result = scorer.poll_results();
    let elapsed = start.elapsed();
    assert!(result.is_none(), "expected None when no scoring happened");
    assert!(
        elapsed < Duration::from_millis(10),
        "poll_results must not block, took {:?}",
        elapsed
    );
}

/// End-to-end round trip: request → wait → poll_results yields Some(scores).
#[test]
fn test_scorer_round_trip() {
    let scorer = BackgroundScorer::new();
    let procs = Arc::new(vec![ProcessInfo {
        pid: 1234,
        name: std::sync::Arc::from("test.exe"),
        cpu_usage: 1.0,
        memory: 1024,
        virtual_memory: 0,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: proc::collect::ProcessStatus::Run,
        exe: None,
        cmd: std::sync::Arc::from(Vec::<String>::new()),
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
        name_lower: std::sync::Arc::from("test.exe"),
        throttled: proc::throttle::EcoQoSState::default(),
    }]);
    let ports = Arc::new(Vec::<PortEntry>::new());
    let flows = Arc::new(Vec::<ProcessFlow>::new());

    scorer.request(procs, ports, flows);

    // Poll for up to 2 seconds — the scorer thread should produce a result.
    let mut scores = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if let Some(s) = scorer.poll_results() {
            scores = Some(s);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let scores = scores.expect("no scoring result within 2 seconds");
    assert_eq!(scores.len(), 1, "expected one score for one process");
    assert!(scores.contains_key(&1234), "score should be keyed by pid");
}

/// Many threads can submit requests concurrently without deadlocking.
/// The contract: `request` is safe to call from multiple producers.
#[test]
fn test_scorer_concurrent_requests() {
    // BackgroundScorer wraps a Receiver (which is !Sync), so we wrap the whole
    // scorer in a Mutex to share across producer threads. request() is &self,
    // so the lock is held only briefly per call.
    let scorer = Arc::new(Mutex::new(BackgroundScorer::new()));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let scorer = scorer.clone();
            std::thread::spawn(move || {
                for _ in 0..10 {
                    let (procs, ports, flows) = empty_request();
                    let s = scorer.lock().expect("scorer mutex poisoned");
                    s.request(procs, ports, flows);
                }
            })
        })
        .collect();
    // Join every producer — if any deadlocks, this test hangs.
    for handle in handles {
        handle.join().expect("producer thread panicked");
    }
}

/// Dropping the scorer must not panic and must signal the worker thread.
/// Valgrind / thread-leak detectors are the real assertion here, but we at
/// least verify that the second scorer created in this test process can spawn
/// and drop cleanly.
#[test]
fn test_scorer_shutdown() {
    for _ in 0..3 {
        let scorer = BackgroundScorer::new();
        let (procs, ports, flows) = empty_request();
        scorer.request(procs, ports, flows);
        // Drain once to make sure the worker is responsive before drop.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if scorer.poll_results().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(scorer);
    }
}

// Helper: a "big" request that keeps the worker busy long enough for the
// second request to be dropped in `test_scorer_request_drops_when_busy`.
fn big_request() -> ScorerRequest {
    let procs: Vec<ProcessInfo> = (0..200)
        .map(|i| {
            let name = format!("proc_{i}.exe");
            ProcessInfo {
                pid: i,
                name: std::sync::Arc::from(name.as_str()),
                cpu_usage: 0.0,
                memory: 0,
                virtual_memory: 0,
                disk_usage: (0, 0),
                disk_read_speed: 0,
                disk_write_speed: 0,
                net_sent_rate: 0,
                net_recv_rate: 0,
                status: proc::collect::ProcessStatus::Run,
                exe: Some(std::sync::Arc::from(
                    format!("C:\\fake\\proc_{i}.exe").as_str(),
                )),
                cmd: std::sync::Arc::from(Vec::<String>::new()),
                cwd: None,
                parent_pid: None,
                session_id: None,
                user_id: None,
                start_time: 0,
                run_time: 0,
                name_lower: std::sync::Arc::from(name.to_lowercase().as_str()),
                throttled: proc::throttle::EcoQoSState::default(),
            }
        })
        .collect();
    (Arc::new(procs), Arc::new(Vec::new()), Arc::new(Vec::new()))
}
