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

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// "最新快照" 语义：容量 1 即可，worker 端 `try_send` 满=丢弃旧，主线程端
/// `try_recv` drain 到最新一份。
const SNAPSHOT_CHANNEL_CAPACITY: usize = 1;

/// 周期性把 `T` 推给主线程的后台 worker。
///
/// 调用方负责定义 worker body —— 接收一个 `SyncSender<T>` 用于推快照、一个
/// `Receiver<()>` 用于 shutdown 信号。body 内部应：
/// 1. 采集 → `try_send`（满即忽略，符合"最新"语义）
/// 2. `shutdown_rx.recv_timeout(poll_interval)` 决定 continue / break
///
/// Drop 时 worker 自动停：`shutdown_tx` 走 `Option::take` + drop，body 内的
/// `recv_timeout` 必然返回 `Disconnected`，循环跳出。
pub struct SnapshotWorker<T> {
    shutdown_tx: Option<Sender<()>>,
    snapshot_rx: Receiver<T>,
    thread: Option<JoinHandle<()>>,
}

impl<T: Send + 'static> SnapshotWorker<T> {
    /// 启动 worker。`poll_interval` 仅用于文档/调用方自查；实际节奏由 body
    /// 内的 `recv_timeout` 控制。
    ///
    /// `body` 收到 `(sender, shutdown_rx)`，循环跑采集 + 推送 +
    /// `recv_timeout` 即可。body 返回时线程结束。
    #[must_use]
    pub fn spawn<F>(thread_name: &str, body: F) -> Self
    where
        F: FnOnce(SyncSender<T>, Receiver<()>) + Send + 'static,
    {
        let (snap_tx, snap_rx) = mpsc::sync_channel(SNAPSHOT_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        let handle = thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || body(snap_tx, shutdown_rx))
            .unwrap_or_else(|e| panic!("spawn {thread_name}: {e:?}"));

        Self {
            shutdown_tx: Some(shutdown_tx),
            snapshot_rx: snap_rx,
            thread: Some(handle),
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
/// 适合「无外部状态、每轮独立采集」的简单 worker；持状态（如
/// `LightWorker` 的 disks/components 字段）的 worker 自己写循环更直观。
pub fn run_poll_loop<T>(
    snap_tx: &SyncSender<T>,
    shutdown_rx: &Receiver<()>,
    poll_interval: Duration,
    mut collect: impl FnMut() -> Option<T>,
) {
    loop {
        if let Some(snap) = collect() {
            match snap_tx.try_send(snap) {
                Ok(()) => {}
                // Channel 满 = 主线程还没消费上一份，直接丢新的（"最新" 语义正确）。
                Err(mpsc::TrySendError::Full(_)) => {}
                Err(mpsc::TrySendError::Disconnected(_)) => break,
            }
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
