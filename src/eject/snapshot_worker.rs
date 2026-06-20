//! USB 设备快照后台 worker。
//!
//! 把 `scan_all_devices`(Win32 盘符枚举 + `GetVolumeInformationW` 等)
//! 挪出 TUI 主线程。Worker 每 [`POLL_INTERVAL`] 扫一次,通过
//! `sync_channel(1)` 推最新 `Vec<RemovableDevice>`;主线程 tick 只
//! `try_recv`,拿到后再做 `is_occupied` 合并。
//!
//! 设备锁查询(`scan_device_locks_with_processes`)仍按需同步触发 —
//! 用户按 `r` 或 `Enter` 选中设备时才执行,可接受短暂等待。

use std::time::Duration;

use crate::worker::{SnapshotWorker, run_poll_loop};

use super::RemovableDevice;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

pub struct UsbSnapshot {
    pub devices: Vec<RemovableDevice>,
}

pub type UsbSnapshotWorker = SnapshotWorker<UsbSnapshot>;

#[must_use]
pub fn spawn() -> UsbSnapshotWorker {
    SnapshotWorker::spawn("usb-snapshot-worker", |snap_tx, shutdown_rx| {
        run_poll_loop(&snap_tx, &shutdown_rx, POLL_INTERVAL, || {
            // scan_all_devices 失败(Linux/macOS 或 Win32 异常)时返回 None,
            // 不推送 — 主线程保留上一份 devices，与原 refresh_device_list
            // 失败行为一致。
            super::scan_all_devices()
                .ok()
                .map(|devices| UsbSnapshot { devices })
        });
    })
}
