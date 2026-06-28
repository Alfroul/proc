//! v0.6.0 阶段 5：所有后台 worker 句柄的统一持有者 + metrics 聚合。
//!
//! 设计：
//! - `App` 持 `workers: WorkerManager`，所有 `*_worker` 字段集中于此。
//! - [`WorkerManager::new`] 接收 crash channel 的 sender（`App::new` 创建后传入），
//!   内部 clone 给每个 worker，对外保持单参数构造。
//! - [`WorkerManager::metrics_snapshot`] 聚合直管 4 个 worker 的 stats，供
//!   `App::worker_metrics` / `proc diag` 消费。Docker worker 仍由 `DockerPanel`
//!   自管（其生命周期与 panel 绑定），由调用方追加。
//!
//! `restart(name)` 故障恢复方法尚未落地 —— 当前无调用方，按 surgical 原则
//! 不预实现；阶段 5 后续会话或新阶段真正需要时再加。

use std::sync::mpsc::Sender;

use crate::metrics::NamedWorkerStats;
use crate::metrics::crash::WorkerCrash;

/// 所有后台 worker 句柄的统一持有者。字段名与原 `App::*_worker` 一致，
/// 避免 call site 大规模改名（surgical 原则）。
///
/// 字段语义见 `App` 旧文档与 CONTEXT.md「后台 worker」段：
/// - `port_worker` / `usb_worker`：平台无关，恒为 `Some`。
/// - `net_flow_worker` / `dns_log_worker`：当前平台不支持时为 `None`。
pub struct WorkerManager {
    pub port_worker: crate::port_worker::PortSnapshotWorker,
    pub usb_worker: crate::eject::snapshot_worker::UsbSnapshotWorker,
    pub net_flow_worker: Option<crate::net_flow::worker::NetFlowWorker>,
    pub dns_log_worker: Option<crate::dns_log::worker::DnsLogWorker>,
    /// v0.7 阶段 7：ETW per-process disk IO（Windows 管理员 / x64 only）。
    /// 其它场景为 None，主线程走 sysinfo delta fallback（v0.6 行为）。
    pub disk_io_etw_worker: Option<crate::disk_io_etw::DiskIoEtwWorker>,
    /// v0.7 阶段 8：eBPF flow graph（Linux + feature `ebpf` only）。
    /// 非 Linux / 无 feature / attach 失败时为 None，主线程走 fallback
    /// （`App::flows` 保持空，UI 显示「需要 Linux + ebpf feature」提示）。
    /// 详见 ADR-0016 + `src/ebpf/mod.rs`。
    pub ebpf_worker: Option<crate::ebpf::EbisuBpfWorker>,
    /// v0.10 阶段 2：Schannel ETW SNI worker（Windows 管理员 + x64 only）。
    /// 其它场景为 None。worker drain 出 `Vec<SniRecord>`，阶段 3 接
    /// `App::overlay_flow_sni_schannel` merge 到 `ProcessFlow.sni`；阶段 2
    /// 只接 worker + diag 行，UI 不动。详见 ADR-0018。
    pub schannel_etw_worker: Option<crate::schannel_etw::SchannelEtwWorker>,
}

impl WorkerManager {
    /// 启动全部 worker。`crash_tx` 传入时，clone 给每个 worker；
    /// 传 `None` 表示 CLI 模式（如 `proc diag`）不消费 crash。
    #[must_use]
    pub fn new(crash_tx: Option<&Sender<WorkerCrash>>) -> Self {
        let port_worker = crate::port_worker::spawn(crash_tx.cloned());
        let usb_worker = crate::eject::snapshot_worker::spawn(crash_tx.cloned());
        let net_flow_worker = crate::net_flow::detect_collector()
            .map(|c| crate::net_flow::worker::spawn(c, crash_tx.cloned()));
        let dns_log_worker = crate::dns_log::detect_collector()
            .map(|c| crate::dns_log::worker::spawn(c, crash_tx.cloned()));
        let disk_io_etw_worker = crate::disk_io_etw::try_spawn(crash_tx.cloned());
        let ebpf_worker = crate::ebpf::try_spawn(crash_tx.cloned());
        let schannel_etw_worker = crate::schannel_etw::try_spawn(crash_tx.cloned());
        Self {
            port_worker,
            usb_worker,
            net_flow_worker,
            dns_log_worker,
            disk_io_etw_worker,
            ebpf_worker,
            schannel_etw_worker,
        }
    }

    /// 聚合直管 4 个 worker 的 metrics 快照。Docker worker 由 `DockerPanel`
    /// 自管，调用方按需追加。
    #[must_use]
    pub fn metrics_snapshot(&self) -> Vec<NamedWorkerStats> {
        let mut out: Vec<NamedWorkerStats> = Vec::new();
        out.push(NamedWorkerStats {
            name: "port",
            stats: self.port_worker.metrics.snapshot(),
        });
        out.push(NamedWorkerStats {
            name: "usb",
            stats: self.usb_worker.metrics.snapshot(),
        });
        if let Some(w) = &self.net_flow_worker {
            out.push(NamedWorkerStats {
                name: "net_flow",
                stats: w.metrics.snapshot(),
            });
        }
        if let Some(w) = &self.dns_log_worker {
            out.push(NamedWorkerStats {
                name: "dns_log",
                stats: w.metrics.snapshot(),
            });
        }
        if let Some(w) = &self.disk_io_etw_worker {
            out.push(NamedWorkerStats {
                name: "disk_io_etw",
                stats: w.metrics.snapshot(),
            });
        }
        if let Some(w) = &self.schannel_etw_worker {
            out.push(NamedWorkerStats {
                name: "schannel_etw",
                stats: w.metrics.snapshot(),
            });
        }
        out
    }
}
