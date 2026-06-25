//! Port snapshot background worker.
//!
//! Decouples `netstat2::get_sockets_info` (几十~几百毫秒的同步 syscall)
//! from the TUI main thread. Worker 每隔 [`POLL_INTERVAL`] 跑一次完整
//! sockets 采集,通过 `sync_channel(1)` 推最新结果;主线程 tick 只
//! `try_recv`,拿到新快照后才做 `scan_ports_with_names` + diff/group/anomaly
//! 等几毫秒级的纯内存计算。

use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::metrics::crash::WorkerCrash;
use crate::worker::{SnapshotWorker, run_poll_loop};

/// 3 秒采集间隔:连接变化没那么快,netstat2 本身开销大,频率太高收益递减。
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// 后台采集到的原始 socket 列表。主线程拿到后用当前 `SystemSnapshot` 的
/// `process_name_map()` 做 name 解析,因此 worker 无需访问 sysinfo。
pub struct PortSnapshot {
    pub sockets: Vec<netstat2::SocketInfo>,
}

pub type PortSnapshotWorker = SnapshotWorker<PortSnapshot>;

/// 启动 port snapshot worker。
///
/// v0.6.0 阶段 3：`crash_tx` 由 `App::new` 传入，worker panic 时把
/// `WorkerCrash` 推给主线程显示 banner。`None` 时 panic 静默吞掉（不推荐）。
#[must_use]
pub fn spawn(crash_tx: Option<Sender<WorkerCrash>>) -> PortSnapshotWorker {
    SnapshotWorker::spawn(
        "port-snapshot-worker",
        crash_tx,
        |snap_tx, shutdown_rx, metrics| {
            run_poll_loop(&snap_tx, &shutdown_rx, &metrics, POLL_INTERVAL, || {
                Some(PortSnapshot {
                    sockets: collect_sockets(),
                })
            });
        },
    )
}

fn collect_sockets() -> Vec<netstat2::SocketInfo> {
    let af_flags = netstat2::AddressFamilyFlags::IPV4 | netstat2::AddressFamilyFlags::IPV6;
    let proto_flags = netstat2::ProtocolFlags::TCP | netstat2::ProtocolFlags::UDP;
    netstat2::get_sockets_info(af_flags, proto_flags).unwrap_or_default()
}
