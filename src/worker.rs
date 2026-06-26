//! 通用「后台周期快照 worker」模板。
//!
//! `port_worker` / `eject::snapshot_worker` / `docker::snapshot_worker` 三个
//! 文件原本是同一份模板的拷贝：`Option<Sender<()>> + Receiver<T> +
//! Option<JoinHandle<()>>` + 几乎相同的 `Drop` 和 `try_recv_latest`。本模块
//! 把模板抽出来，调用点只需写 worker body。
//!
//! 模式：
//! - `mpsc::sync_channel(1)` + worker 端 `try_send`（满即丢，"最新" 语义）
//! - 主线程端 `try_recv` drain 到最新一份
//! - `Drop` 时先 drop `shutdown_tx` 触发 worker `recv_timeout` 立即返回
//!   `Disconnected`，再 `join()` 等线程干净退出
//!
//! v0.6.0 阶段 3 新增：
//! - 每个 worker 持 `Arc<WorkerMetrics>`，主循环每轮 record（耗时 / 丢帧）。
//! - spawn 时接 `crash_tx`；body 用 `catch_unwind` 包裹，panic 时把
//!   `WorkerCrash` 推给主线程显示 banner，避免线程静默死亡。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::metrics::WorkerMetrics;
use crate::metrics::crash::WorkerCrash;

/// "最新快照" 语义：容量 1 即可，worker 端 `try_send` 满=丢弃旧，主线程端
/// `try_recv` drain 到最新一份。
const SNAPSHOT_CHANNEL_CAPACITY: usize = 1;

/// 周期性把 `T` 推给主线程的后台 worker。
///
/// 调用方负责定义 worker body —— 接收一个 `SyncSender<T>` 用于推快照、一个
/// `Receiver<()>` 用于 shutdown 信号、一份 `Arc<WorkerMetrics>` 用于 record。
/// body 内部应：
/// 1. 采集 → `try_send`（满即 record + 忽略，符合"最新"语义）
/// 2. `shutdown_rx.recv_timeout(poll_interval)` 决定 continue / break
/// 3. panic 时 spawn 外层的 `catch_unwind` 捕获，通过 `crash_tx` 通知主线程
///
/// Drop 时 worker 自动停：`shutdown_tx` 走 `Option::take` + drop，body 内的
/// `recv_timeout` 必然返回 `Disconnected`，循环跳出。
pub struct SnapshotWorker<T> {
    shutdown_tx: Option<Sender<()>>,
    snapshot_rx: Receiver<T>,
    thread: Option<JoinHandle<()>>,
    /// v0.6.0 阶段 3：worker 自身指标。主线程读 `snapshot()` 聚合显示。
    pub metrics: Arc<WorkerMetrics>,
}

impl<T: Send + 'static> SnapshotWorker<T> {
    /// 启动 worker。`poll_interval` 仅用于文档/调用方自查；实际节奏由 body
    /// 内的 `recv_timeout` 控制。
    ///
    /// `body` 收到 `(sender, shutdown_rx, metrics)`，循环跑采集 + 推送 +
    /// `recv_timeout` 即可。body 返回时线程结束。
    ///
    /// v0.6.0 阶段 3：body 外包 `catch_unwind`，panic 时通过 `crash_tx`
    /// （如有）把 `WorkerCrash` 推给主线程，避免线程静默死亡。`crash_tx`
    /// 为 `None` 时 panic 仍被吞掉（防整进程崩），但不通知 UI。
    #[must_use]
    pub fn spawn<F>(
        thread_name: &'static str,
        crash_tx: Option<Sender<WorkerCrash>>,
        body: F,
    ) -> Self
    where
        F: FnOnce(SyncSender<T>, Receiver<()>, Arc<WorkerMetrics>) + Send + 'static,
    {
        let (snap_tx, snap_rx) = mpsc::sync_channel(SNAPSHOT_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
        let metrics = Arc::new(WorkerMetrics::new());
        let metrics_for_body = Arc::clone(&metrics);

        let handle = thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    body(snap_tx, shutdown_rx, metrics_for_body);
                }));
                if let Err(payload) = result {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "<non-string panic>".to_string());
                    let backtrace = std::backtrace::Backtrace::force_capture();
                    tracing::error!(worker = thread_name, panic = %msg, "worker panicked");
                    // v0.6.0 阶段 8（REVIEW-7.md P1-7）：catch_unwind 截获 panic 后
                    // 不会触发全局 panic hook（标准库语义），导致 worker 崩溃无磁盘
                    // crash report。这里显式写一份到 crashes/ 目录，bug 报告时可附上。
                    // 文件名带 worker 名字以与主线程 panic 区分。
                    let bt_str = backtrace.to_string();
                    if let Err(e) =
                        crate::metrics::crash::write_worker_crash_report(thread_name, &msg, &bt_str)
                    {
                        tracing::warn!(
                            worker = thread_name,
                            error = %e,
                            "无法写 worker crash report 到磁盘",
                        );
                    }
                    if let Some(tx) = crash_tx.as_ref() {
                        let _ = tx.send(WorkerCrash {
                            worker: thread_name,
                            message: msg,
                            backtrace: bt_str,
                            timestamp: std::time::SystemTime::now(),
                        });
                    }
                }
            })
            .unwrap_or_else(|e| panic!("spawn {thread_name}: {e:?}"));

        Self {
            shutdown_tx: Some(shutdown_tx),
            snapshot_rx: snap_rx,
            thread: Some(handle),
            metrics,
        }
    }

    /// Drain 到最新一份；无新快照时返回 `None`。
    #[must_use]
    pub fn try_recv_latest(&self) -> Option<T> {
        let mut latest = None;
        while let Ok(snap) = self.snapshot_rx.try_recv() {
            latest = Some(snap);
        }
        latest
    }
}

impl<T> Drop for SnapshotWorker<T> {
    fn drop(&mut self) {
        // Drop shutdown sender BEFORE join so worker's `recv_timeout` returns
        // `Disconnected` immediately on its next iteration. Sending first
        // is best-effort: it triggers the worker's `Ok(_)` arm to break
        // even mid-period (no need to wait for POLL_INTERVAL to elapse).
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            // tx drop 在此 — worker 下一轮 `recv_timeout` 必然 Disconnected。
            drop(tx);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// Worker body 的循环骨架：跑一次 `collect`、推送、按 `poll_interval` 等待
/// shutdown。返回 `true` 表示 shutdown 已触发（调用方应 `break`）。
///
/// v0.6.0 阶段 3：每轮 record `elapsed` 到 `metrics`；`try_send` 返回 `Full`
/// 时 `record_channel_full`，主线程消费跟不上 → 用户能在 `proc diag` 看到。
///
/// 适合「无外部状态、每轮独立采集」的简单 worker；持状态（如
/// `LightWorker` 的 disks/components 字段）的 worker 自己写循环更直观。
pub fn run_poll_loop<T>(
    snap_tx: &SyncSender<T>,
    shutdown_rx: &Receiver<()>,
    metrics: &WorkerMetrics,
    poll_interval: Duration,
    mut collect: impl FnMut() -> Option<T>,
) {
    loop {
        let t0 = std::time::Instant::now();
        if let Some(snap) = collect() {
            metrics.record_poll(t0.elapsed());
            match snap_tx.try_send(snap) {
                Ok(()) => {}
                // Channel 满 = 主线程还没消费上一份，直接丢新的（"最新" 语义正确）。
                Err(mpsc::TrySendError::Full(_)) => metrics.record_channel_full(),
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
        } else {
            // collector 返回 None（如 USB 扫描失败）—— 仍 record 一次耗时，
            // 但不送 frame。
            metrics.record_poll(t0.elapsed());
        }

        match shutdown_rx.recv_timeout(poll_interval) {
            // 主线程 drop worker 时显式 send(()) → 立即退出。
            Ok(_) => break,
            // 计时到点：进入下一轮采集。
            Err(RecvTimeoutError::Timeout) => continue,
            // shutdown_tx 已 drop → 主线程在清理，退出。
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    /// v0.6.0 阶段 3：worker body panic 时，spawn 外层 catch_unwind 应该
    /// 通过 `crash_tx` 发送 `WorkerCrash`，而不是让线程静默死亡或整进程崩。
    #[test]
    fn catch_unwind_sends_worker_crash_on_panic() {
        let (tx, rx) = mpsc::channel::<WorkerCrash>();
        let _worker: SnapshotWorker<()> = SnapshotWorker::spawn(
            "test-panic",
            Some(tx),
            |_snap_tx, _shutdown_rx, _metrics| {
                panic!("boom");
            },
        );
        // 等 worker 线程 panic + catch_unwind + send
        let crash = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("crash_tx should receive WorkerCrash");
        assert_eq!(crash.worker, "test-panic");
        assert!(crash.message.contains("boom"));
        assert!(!crash.backtrace.is_empty());
    }

    /// 验证 `metrics` 字段可从 spawn 返回值读出，主线程无需 join。
    #[test]
    fn metrics_field_accessible_after_spawn() {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&collected);
        let worker: SnapshotWorker<u64> = SnapshotWorker::spawn(
            "test-metrics",
            None,
            move |snap_tx, shutdown_rx, _metrics| {
                let mut i = 0u64;
                loop {
                    let _ = snap_tx.try_send(i);
                    i += 1;
                    if i >= 3 {
                        break;
                    }
                    if shutdown_rx.recv_timeout(Duration::from_millis(10)).is_ok() {
                        break;
                    }
                }
                captured.lock().unwrap().push(i);
            },
        );
        // 主线程立即读 metrics — spawn 后任何时刻都应有数据（除非 race）。
        let start = Instant::now();
        while worker.metrics.snapshot().poll_count == 0 && start.elapsed() < Duration::from_secs(1)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        // drop 触发 shutdown + join
        drop(worker);
        let captured = collected.lock().unwrap();
        assert!(!captured.is_empty(), "body should have run");
    }
}
