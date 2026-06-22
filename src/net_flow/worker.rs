//! NetFlow 后台 worker：复用 [`crate::worker::SnapshotWorker`]，1s poll。
//!
//! Worker body 持有 `Box<dyn NetFlowCollector>`，每秒调一次 `per_process_rates()`，
//! 把结果通过 `sync_channel(1)` 推给主线程；主线程 `try_recv_latest` drain 到
//! 最新一份。
//!
//! 平台不支持 / 二进制缺失时，[`detect_collector`] 返回 None，worker 不 spawn；
//! 主线程 `net_flow_worker: Option<NetFlowWorker>`，`try_recv_latest` 返回 None
//! → `ProcessInfo.net_*_rate` 保持默认 0。

use std::time::Duration;

use crate::net_flow::{NetFlowCollector, ProcessNetRate};
use crate::worker::{SnapshotWorker, run_poll_loop};

/// 1 秒采集间隔：网络流量在秒级尺度上才有意义；再短 ETW / nethogs 的统计窗口
/// 太窄，数字波动大。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Default)]
pub struct NetFlowSnapshot {
    pub rates: Vec<ProcessNetRate>,
}

pub type NetFlowWorker = SnapshotWorker<NetFlowSnapshot>;

/// 启动 net_flow worker。`collector` 由调用方通过 [`crate::net_flow::detect_collector`]
/// 选出；None 时返回 None（worker 不启动，主线程 net 列保持 0）。
#[must_use]
pub fn spawn(mut collector: Box<dyn NetFlowCollector>) -> NetFlowWorker {
    SnapshotWorker::spawn("net-flow-worker", move |snap_tx, shutdown_rx| {
        run_poll_loop(&snap_tx, &shutdown_rx, POLL_INTERVAL, || {
            let rates = collector.per_process_rates();
            Some(NetFlowSnapshot { rates })
        });
    })
}
