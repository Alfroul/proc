//! v0.10 阶段 2：Windows Schannel ETW SNI worker（Windows）/ stub（其它平台）。
//!
//! 对应 ADR-0018。设计要点：
//! - **Windows 实装（阶段 2 落地）**：手写 windows-rs ETW
//!   （`Win32_System_Diagnostics_Etw` + `Win32_System_Diagnostics_Tdh`）开
//!   `Microsoft-Windows-Schannel-Events` session（GUID
//!   `{91CC1150-71AA-47E2-AE18-C96E61736B6F}` —— **阶段 2 实测修订**，原
//!   `{37D2C3CD-...}` 实测对 TLS handshake 不 fire event），`EnableTraceEx2`
//!   启用 provider，OpenTraceW 注册 callback。callback fast-filter event 1793
//!   → 用 TDH（`TdhGetEventInformation` + `TdhGetPropertySize`）按 property name
//!   找 `TargetName` 字段（UTF-16 LE）→ push `SniRecord` 到共享 `Vec`。
//! - **跨平台**：Linux/macOS 编译 stub，`try_spawn` 返回 `None`。
//! - **降级**：Windows 非管理员 / `StartTraceW` 失败 / `EnableTraceEx2` 失败 /
//!   x86 进程 → 返回 `None`。worker 启停失败不影响其它路径。
//!
//! **与 disk_io_etw（ADR-0015）的关键差异**：
//! - disk_io_etw 走 NT Kernel Logger（固定 logger name + EnableFlags + thread→pid map）
//! - schannel_etw 走普通 session + `EnableTraceEx2` 启用 manifest-based provider
//! - disk_io_etw 用硬编码偏移（schema 稳定 + 公开 MOF 文档）；schannel_etw
//!   走 TDH 动态 schema（**阶段 2 实测**：SNI event ID = 1793，字段名 = `TargetName`，
//!   但解析时仍按 property name 找，不硬编码偏移，跨 Win10/Win11 版本兼容）
//! - Schannel event 自带 `EVENT_HEADER.ProcessId`，**不复用** disk_io_etw 的
//!   thread→pid map（后者是 NT Kernel Logger 才需要）

#[cfg(target_os = "windows")]
mod provider;

mod parser;

pub use parser::{SniRecord, read_utf16_le_until_null};

use std::sync::mpsc::Sender;

use crate::metrics::crash::WorkerCrash;
use crate::worker::SnapshotWorker;

/// Worker 句柄类型。`Option<SchannelEtwWorker>`：Windows 管理员 + 启动成功时为 `Some`。
///
/// snapshot 类型 `Vec<SniRecord>`：自上次 1s flush 以来 callback 解出的 SNI 列表；
/// 主线程 drain 后由阶段 3 接 `App::overlay_flow_sni_schannel` merge 到 ProcessFlow.sni。
pub type SchannelEtwWorker = SnapshotWorker<Vec<SniRecord>>;

/// 尝试启动 Schannel ETW SNI worker。失败时（非 Windows / 非管理员 /
/// session 启动失败 / x86）返回 `None`，调用方走降级（sni 永远 None）。
///
/// `crash_tx` 与其它 SnapshotWorker 一致：worker panic 时通知主线程显示 banner。
#[must_use]
pub fn try_spawn(crash_tx: Option<Sender<WorkerCrash>>) -> Option<SchannelEtwWorker> {
    #[cfg(target_os = "windows")]
    {
        provider::try_spawn_windows(crash_tx)
    }
}
