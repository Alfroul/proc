//! DNS 查询日志后台 worker：复用 [`crate::worker::SnapshotWorker`]，500ms poll。
//!
//! Worker body 持有 `Box<dyn DnsLogCollector>`，每 ~500ms 调一次 [`DnsLogCollector::drain`]，
//! 把新查询通过 `sync_channel(1)` 推给主线程；主线程 `try_recv_latest` drain，
//! 追加到 `App::dns_log_recent: VecDeque<DnsQuery>`（cap=1000 FIFO）。
//!
//! 平台不支持 / PowerShell 不可用 / 主线程慢消费：[`crate::dns_log::detect_collector`]
//! 返回 None，worker 不 spawn；主线程 `dns_log_worker: Option<DnsLogWorker>`，
//! `try_recv_latest` 返回 None → VecDeque 保持空。

use std::time::Duration;

use crate::dns_log::{DnsLogCollector, DnsQuery};
use crate::worker::{SnapshotWorker, run_poll_loop};

/// 500ms poll：DNS 查询高频，比阶段 7 NetFlow 的 1s 更短。再短会让 PowerShell
/// 子进程的 Get-WinEvent 节奏跟不上（脚本内部 Start-Sleep 400ms）。
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Default)]
pub struct DnsLogSnapshot {
    pub queries: Vec<DnsQuery>,
}

pub type DnsLogWorker = SnapshotWorker<DnsLogSnapshot>;

/// 启动 DNS 日志 worker。`collector` 由调用方通过
/// [`crate::dns_log::detect_collector`] 选出。
#[must_use]
pub fn spawn(mut collector: Box<dyn DnsLogCollector>) -> DnsLogWorker {
    SnapshotWorker::spawn("dns-log-worker", move |snap_tx, shutdown_rx| {
        run_poll_loop(&snap_tx, &shutdown_rx, POLL_INTERVAL, || {
            let queries = collector.drain();
            if queries.is_empty() {
                None
            } else {
                Some(DnsLogSnapshot { queries })
            }
        });
    })
}
