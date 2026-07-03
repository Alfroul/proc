//! v0.7 阶段 7：per-process 磁盘 IO BPS via ETW（Windows）/ stub（其它平台）。
//!
//! 对应 ADR-0015。设计要点：
//! - **Windows 实装**：手写 windows-rs ETW（`Win32_System_Diagnostics_Etw`），
//!   NT Kernel Logger session + `EVENT_TRACE_FLAG_DISK_IO` + DiskIo_TypeGroup1。
//!   不引 ferrisetw（用户偏好「更可控」，~200-300 行 windows-rs 直调）。
//! - **跨平台**：Linux/macOS 编译 stub，`try_spawn` 返回 `None`，UI 沿用
//!   sysinfo 的 `disk_usage` delta（v0.6 行为）。
//! - **降级**：Windows 非管理员 / NT Kernel Logger 已被占用（如资源监视器开着）/
//!   x86 进程 → `try_spawn` 返回 `None`，主线程走 sysinfo fallback。
//!
//! Worker 复用 [`crate::worker::SnapshotWorker`] 模板：
//! - ETW ProcessTrace 在子线程阻塞（ferrisetw 同款），callback 写共享 accum map
//! - worker body 跑 [`crate::worker::run_poll_loop`]，每 1s drain accum → push channel
//! - drop worker → shutdown 信号 → run_poll_loop 退出 → body 内 ControlTraceW(STOP)
//!   → ProcessTrace 线程返回 → join 干净退出
//!
//! 详见 `provider::spawn_disk_io_trace`（Windows）/ stub（其它）。

#[cfg(target_os = "windows")]
mod provider;

#[cfg(target_os = "windows")]
mod thread_map;

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::metrics::crash::WorkerCrash;
use crate::worker::SnapshotWorker;

/// 单个进程的磁盘 IO 速率（bytes/sec）。
///
/// `read_bps` / `write_bps` 是过去 1s 内的累计字节数（不是瞬时速率）。
/// ETW worker 每 1s flush 一次，主线程 drain 后直接贴到 `ProcessInfo::disk_read_speed`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct DiskIoStats {
    pub read_bps: u64,
    pub write_bps: u64,
}

/// PID → 磁盘 IO 速率快照。1s 一份，主线程按 PID merge 到 `ProcessInfo`。
pub type DiskIoMap = HashMap<u32, DiskIoStats>;

/// Worker 句柄类型。`Option<DiskIoEtwWorker>`：Windows 管理员 + 启动成功时为 `Some`。
pub type DiskIoEtwWorker = SnapshotWorker<DiskIoMap>;

/// 尝试启动 ETW disk IO worker。失败时（非 Windows / 非管理员 / session 占用）
/// 返回 `None`，调用方应保持 sysinfo fallback。
///
/// `crash_tx` 与其它 SnapshotWorker 一致：worker panic 时通知主线程显示 banner。
#[must_use]
pub fn try_spawn(crash_tx: Option<Sender<WorkerCrash>>) -> Option<DiskIoEtwWorker> {
    #[cfg(target_os = "windows")]
    {
        provider::try_spawn_windows(crash_tx)
    }
}
