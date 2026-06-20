//! Port snapshot background worker.
//!
//! Decouples `netstat2::get_sockets_info` (几十~几百毫秒的同步 syscall)
//! from the TUI main thread. Worker 每隔 [`POLL_INTERVAL`] 跑一次完整
//! sockets 采集,通过 `sync_channel(1)` 推最新结果;主线程 tick 只
//! `try_recv`,拿到新快照后才做 `scan_ports_with_names` + diff/group/anomaly
//! 等几毫秒级的纯内存计算。

use std::time::Duration;

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
#[must_use]
pub fn spawn() -> PortSnapshotWorker {
    SnapshotWorker::spawn("port-snapshot-worker", |snap_tx, shutdown_rx| {
        run_poll_loop(&snap_tx, &shutdown_rx, POLL_INTERVAL, || {
            Some(PortSnapshot {
                sockets: collect_sockets(),
            })
        });
    })
}

fn collect_sockets() -> Vec<netstat2::SocketInfo> {
    let af_flags = netstat2::AddressFamilyFlags::IPV4 | netstat2::AddressFamilyFlags::IPV6;
    let proto_flags = netstat2::ProtocolFlags::TCP | netstat2::ProtocolFlags::UDP;
    netstat2::get_sockets_info(af_flags, proto_flags).unwrap_or_default()
}
