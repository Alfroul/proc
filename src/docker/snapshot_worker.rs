//! Docker 容器列表快照后台 worker。
//!
//! 把 `DockerMonitor::list_containers`(主线程同步 `block_on`,
//! daemon 响应慢时 UI 完全冻结)挪到独立线程。Worker 每
//! [`POLL_INTERVAL`] 拉一次容器列表,通过 `sync_channel(1)` 推最新
//! `Result<Vec<ContainerInfo>, ProcError>`;主线程 `try_recv`,失败时
//! 只更新 status_message,保留上一份容器列表。
//!
//! `DockerMonitor` 持有 `tokio::Runtime` + `bollard::Docker`,通过
//! `Arc<Mutex<DockerMonitor>>` 在 worker 与 panel 之间共享。锁竞争
//! 很轻(worker 5s 一次、用户操作偶发);`list_containers` 同步耗时
//! 期间用户 restart/stop 操作排队等待是 acceptable 的。

use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::error::ProcError;
use crate::worker::SnapshotWorker;

use super::{ContainerInfo, DockerMonitor};

/// 5s 间隔:容器变化没那么快;`list_containers` 本身几百毫秒,频率
/// 太高主线程受益递减且 daemon 压力上升。
const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub struct DockerSnapshot {
    pub result: std::result::Result<Vec<ContainerInfo>, ProcError>,
}

pub type DockerSnapshotWorker = SnapshotWorker<DockerSnapshot>;

/// 启动 worker。`monitor` 由调用方在首次 `DockerMonitor::connect()` 成功
/// 后通过 `Arc::clone` 传入;worker 与 panel 共享同一个 Arc。
#[must_use]
pub fn spawn(monitor: Arc<Mutex<DockerMonitor>>) -> DockerSnapshotWorker {
    SnapshotWorker::spawn("docker-snapshot-worker", move |snap_tx, shutdown_rx| {
        worker_loop(monitor, snap_tx, shutdown_rx);
    })
}

fn worker_loop(
    monitor: Arc<Mutex<DockerMonitor>>,
    snap_tx: SyncSender<DockerSnapshot>,
    shutdown_rx: Receiver<()>,
) {
    use std::sync::mpsc::{RecvTimeoutError, TrySendError};

    loop {
        let result = match monitor.lock() {
            Ok(monitor) => monitor.list_containers(true),
            // poisoned: 前一次 panic，把最终错误推一份给主线程后立即退出，
            // 避免每 5s 重复推同一错误造成 toast 风暴 + 线程泄漏。
            Err(e) => {
                tracing::warn!("DockerMonitor mutex poisoned: {:?}", e);
                let _ = snap_tx.try_send(DockerSnapshot {
                    result: Err(ProcError::docker("DockerMonitor mutex poisoned")),
                });
                break;
            }
        };
        match snap_tx.try_send(DockerSnapshot { result }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => break,
        }

        match shutdown_rx.recv_timeout(POLL_INTERVAL) {
            Ok(_) => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}
