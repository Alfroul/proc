//! v0.7 阶段 8：eBPF flow graph worker（ADR-0016）。
//!
//! 跨平台入口。本模块只做 cfg-gate 分发：
//! - `(target_os = "linux", feature = "ebpf")` → [`worker::EbisuBpfWorker`] 实装
//! - 其它平台 / 无 feature → [`stub::EbisuBpfWorker`] 占位（`try_spawn` 返 None）
//!
//! 跨平台数据结构（[`flow`]）总是编译；UI / CLI / 录屏路径无需 cfg-gate 即可引用
//! `ProcessFlow` / `FlowEvent`。
//!
//! 设计参考 v0.7 阶段 7 disk_io_etw：跨平台 mod.rs + cfg-gated 子模块 +
//! `try_spawn() -> Option<Worker>` 接口；非 Linux / 无权限 / attach 失败时
//! `try_spawn` 返回 `None`，UI 走 fallback。

pub mod flow;

#[cfg(all(target_os = "linux", feature = "ebpf"))]
mod worker;

#[cfg(all(target_os = "linux", feature = "ebpf"))]
mod elf_loader;

#[cfg(not(all(target_os = "linux", feature = "ebpf")))]
mod stub;

#[cfg(all(target_os = "linux", feature = "ebpf"))]
pub use worker::EbisuBpfWorker;

#[cfg(not(all(target_os = "linux", feature = "ebpf")))]
pub use stub::EbisuBpfWorker;

use std::sync::mpsc::Sender;

use crate::ebpf::flow::FlowEvent;
use crate::metrics::crash::WorkerCrash;

/// 当前构建是否启用了真实 eBPF 加载（Linux + feature `ebpf`）。
pub const EBPF_ENABLED: bool = cfg!(all(target_os = "linux", feature = "ebpf"));

/// 启动 eBPF worker。失败时（非 Linux / 无 feature / 无权限 / attach 失败）
/// 返回 `None`，调用方应保持 `App::flows = Vec::new()` 走 fallback。
///
/// **Part A 状态**：跨平台 stub 在非 Linux / 无 feature 时直接返回 `None`；
/// Linux + feature 的真实实装 [`worker::EbisuBpfWorker::try_spawn`] 待 Linux 会话验证
/// （tracepoint attach 需 root / CAP_BPF，Windows 上无法 smoke-test）。
#[must_use]
pub fn try_spawn(_crash_tx: Option<Sender<WorkerCrash>>) -> Option<EbisuBpfWorker> {
    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    {
        worker::try_spawn_impl(_crash_tx)
    }
    #[cfg(not(all(target_os = "linux", feature = "ebpf")))]
    {
        // 跨平台 stub：无可启动 worker。
        None
    }
}

/// 一次拉取所有已 buffer 的 FlowEvent（主线程 1s tick 调）。
/// stub 平台返回空 Vec。Linux + feature 调内部 worker 的 try_recv。
#[must_use]
pub fn drain_events(worker: Option<&EbisuBpfWorker>) -> Vec<FlowEvent> {
    #[cfg(all(target_os = "linux", feature = "ebpf"))]
    {
        worker.map(|w| w.try_recv_events()).unwrap_or_default()
    }
    #[cfg(not(all(target_os = "linux", feature = "ebpf")))]
    {
        // stub：worker 类型无字段；显式忽略参数保持 API 一致。
        let _ = worker;
        Vec::new()
    }
}
