//! v0.6.0 阶段 3：worker metrics + catch_unwind 集成测试。
//!
//! 覆盖：
//! - `SnapshotWorker` body panic 时通过 `crash_tx` 发送 `WorkerCrash`
//! - `metrics` 字段在 worker 运行时被 record
//! - `WorkerStats::health_badge` 在不同状态下正确切换

use proc::metrics::crash::WorkerCrash;
use proc::metrics::{NamedWorkerStats, WorkerMetrics, WorkerStats};
use proc::worker::SnapshotWorker;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[test]
fn snapshot_worker_panic_sends_worker_crash() {
    let (tx, rx) = mpsc::channel::<WorkerCrash>();
    let _worker: SnapshotWorker<()> = SnapshotWorker::spawn("panic-test", Some(tx), |_, _, _| {
        panic!("integration test panic");
    });
    let crash = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("crash_tx should receive WorkerCrash");
    assert_eq!(crash.worker, "panic-test");
    assert!(crash.message.contains("integration test panic"));
    // backtrace 在 release + 没有 RUST_BACKTRACE 时可能为空，但 force_capture
    // 通常会拿到至少栈顶几帧。
    let _ = crash.backtrace; // 不强校验
}

#[test]
fn snapshot_worker_metrics_records_after_polls() {
    // 启一个每 5ms 推一次的 worker，主线程读 metrics 应见 poll_count > 0。
    let worker: SnapshotWorker<u64> =
        SnapshotWorker::spawn("metrics-test", None, |snap_tx, shutdown_rx, metrics| {
            // 注意：spawn 闭包内手动 record — 调用 run_poll_loop 才会自动 record。
            let mut counter = 0u64;
            loop {
                let t0 = Instant::now();
                let _ = snap_tx.try_send(counter);
                counter = counter.wrapping_add(1);
                metrics.record_poll(t0.elapsed());
                use std::sync::mpsc::RecvTimeoutError;
                match shutdown_rx.recv_timeout(Duration::from_millis(5)) {
                    Ok(_) => break,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    let start = Instant::now();
    while worker.metrics.snapshot().poll_count < 5 && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let stats = worker.metrics.snapshot();
    assert!(
        stats.poll_count >= 5,
        "expected poll_count >= 5, got {}",
        stats.poll_count
    );
}

#[test]
fn named_worker_stats_serialize_to_json() {
    // proc diag --json 输出 — 验证 serde 序列化能跑通。
    let entry = NamedWorkerStats {
        name: "port",
        stats: WorkerStats {
            poll_count: 100,
            poll_total_us: 20_000,
            avg_us: 200,
            max_us: 500,
            channel_full: 0,
            last_error: None,
        },
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert!(json.contains("\"name\":\"port\""));
    assert!(json.contains("\"poll_count\":100"));
    assert!(json.contains("\"avg_us\":200"));
    // flatten 后不应有嵌套 "stats": {...}
    assert!(!json.contains("\"stats\""));
}

#[test]
fn health_badge_transitions_on_state_change() {
    let m = Arc::new(WorkerMetrics::new());
    // 初始：✓
    assert_eq!(m.snapshot().health_badge(), "✓");
    // record 一次慢 poll：⚠
    m.record_poll(Duration::from_micros(150_000));
    assert_eq!(m.snapshot().health_badge(), "⚠");
    // 新建一个 metrics，触发 channel_full > 10
    let m2 = WorkerMetrics::new();
    for _ in 0..15 {
        m2.record_channel_full();
    }
    assert_eq!(m2.snapshot().health_badge(), "⚠");
    // 第三个：record_error
    let m3 = WorkerMetrics::new();
    m3.record_error("channel disconnected");
    assert_eq!(m3.snapshot().health_badge(), "⚠");
}

#[test]
fn worker_crash_with_no_crash_tx_does_not_panic() {
    // crash_tx=None 时 panic 仍被 catch_unwind 吞掉，不让整进程崩。
    let _worker: SnapshotWorker<()> = SnapshotWorker::spawn("silent-panic", None, |_, _, _| {
        panic!("this panic should be silently caught");
    });
    // 给 worker 一点时间 panic
    std::thread::sleep(Duration::from_millis(100));
    // 测试通过 = 进程没崩。
}
