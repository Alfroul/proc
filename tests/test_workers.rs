//! 后台 worker smoke 测试。
//!
//! 覆盖 P2-14 抽出的 `SnapshotWorker<T>` 通用模板：
//! - spawn → 立刻 drop：线程在合理时间内 join（不漏 join、不死锁）
//! - spawn → try_recv_latest：能拿到至少一份推送
//! - shutdown 信号：drop 后 worker 不会无限循环（通过 spawn 一个会 sleep
//!   长时间的 body 验证 drop 不阻塞超过合理时长）
//!
//! PortSnapshotWorker / UsbSnapshotWorker 是 `SnapshotWorker<T>` 的具现，
//! 共享 Drop / try_recv_latest 实现，这里只测模板本身；不依赖外部系统
//! 状态（端口扫描 / USB 枚举），保证 CI Linux/macOS 都能跑。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proc::worker::{SnapshotWorker, run_poll_loop};

#[test]
fn snapshot_worker_drops_without_deadlock() {
    // worker body 每 10ms 推一个数字；Drop 必须能在 1s 内返回。
    // 没有显式 shutdown 信号时也要干净退出（drop tx 触发 recv_timeout
    // Disconnected）。
    let worker: SnapshotWorker<u32> = SnapshotWorker::spawn("test-fast", |tx, rx| {
        run_poll_loop(&tx, &rx, Duration::from_millis(10), || Some(1));
    });
    let start = Instant::now();
    drop(worker);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "Drop took {:?}, expected < 1s",
        elapsed
    );
}

#[test]
fn snapshot_worker_try_recv_latest_drains_to_newest() {
    // poll 1ms 推递增数字，主线程 sleep 100ms 后 drain；应拿到最大值。
    // 验证"满即丢、drain 到最新"语义。
    let counter = Arc::new(Mutex::new(0u32));
    let counter_for_thread = Arc::clone(&counter);

    let worker: SnapshotWorker<u32> = SnapshotWorker::spawn("test-drain", move |tx, rx| {
        run_poll_loop(&tx, &rx, Duration::from_millis(1), move || {
            let mut c = counter_for_thread.lock().unwrap();
            *c += 1;
            Some(*c)
        });
    });

    std::thread::sleep(Duration::from_millis(100));
    let latest = worker.try_recv_latest();
    drop(worker);

    let latest = latest.expect("should have received at least one snapshot");
    let counter_val = *counter.lock().unwrap();
    assert!(
        latest <= counter_val,
        "latest ({latest}) should be ≤ total produced ({counter_val})"
    );
    assert!(latest >= 1, "latest ({latest}) should be ≥ 1");
}

#[test]
fn snapshot_worker_shutdown_breaks_long_running_body() {
    // poll_interval 远大于测试时长；shutdown 信号必须打断 recv_timeout，
    // 而不是等满 poll_interval。
    let worker: SnapshotWorker<()> = SnapshotWorker::spawn("test-shutdown", |tx, rx| {
        // poll 1 hour — 测试通过 Drop 提前打断。
        run_poll_loop(&tx, &rx, Duration::from_secs(3600), || Some(()));
    });
    let start = Instant::now();
    drop(worker);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "Drop took {:?}, expected < 2s even with 1h poll interval",
        elapsed
    );
}

#[test]
fn snapshot_worker_handles_collect_returning_none() {
    // collect 永远返回 None → 不推送，但循环正常运转；Drop 仍能干净退出。
    let worker: SnapshotWorker<u32> = SnapshotWorker::spawn("test-none", |tx, rx| {
        run_poll_loop(&tx, &rx, Duration::from_millis(5), || None);
    });
    std::thread::sleep(Duration::from_millis(50));
    // 没有数据，但 try_recv_latest 不得 panic。
    assert!(worker.try_recv_latest().is_none());
    drop(worker);
}
