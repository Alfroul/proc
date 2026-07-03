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
//! v0.11.0 阶段 1：[`WorkerManager::restart`] / [`WorkerManager::restart_tick`] /
//! [`WorkerManager::restart_status`] 实现 worker panic 后的指数退避热恢复
//! （ADR-0019 / TD-4 真正实装）。

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::SystemTime;

use crate::metrics::NamedWorkerStats;
use crate::metrics::crash::WorkerCrash;

use super::restart::{RestartState, RestartStatus};

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
    /// v0.10 阶段 2：Schannel ETW SNI worker（Windows 管理员 + x64 only）。
    /// 其它场景为 None。worker drain 出 `Vec<SniRecord>`，阶段 3 接
    /// `App::overlay_flow_sni_schannel` merge 到 `ProcessFlow.sni`；阶段 2
    /// 只接 worker + diag 行，UI 不动。详见 ADR-0018。
    pub schannel_etw_worker: Option<crate::schannel_etw::SchannelEtwWorker>,
    /// v0.11.0 阶段 1：每个 worker 的 restart 状态机（ADR-0019）。
    /// key = thread_name（与 `WorkerCrash.worker` 同源），例如
    /// `"port-snapshot-worker"` / `"dns-log-worker"`。
    pub restart_history: HashMap<&'static str, RestartState>,
    /// v0.11.0 阶段 2：DNS collector 实际选用的类型（ADR-0020）。`detect_collector`
    /// 返回 `(collector, kind)` tuple，collector move 进 worker body 后此字段
    /// 独立保留供 `proc diag` 输出。worker restart 路径不更新此字段（重启
    /// 瞬间的 kind 差异不影响功能；用户报告问题时主要关心启动时的 collector）。
    pub dns_collector_kind: crate::dns_log::DnsCollectorKind,
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
        let (dns_collector, dns_collector_kind) = crate::dns_log::detect_collector();
        let dns_log_worker =
            dns_collector.map(|c| crate::dns_log::worker::spawn(c, crash_tx.cloned()));
        let disk_io_etw_worker = crate::disk_io_etw::try_spawn(crash_tx.cloned());
        let schannel_etw_worker = crate::schannel_etw::try_spawn(crash_tx.cloned());
        Self {
            port_worker,
            usb_worker,
            net_flow_worker,
            dns_log_worker,
            disk_io_etw_worker,
            schannel_etw_worker,
            restart_history: HashMap::new(),
            dns_collector_kind,
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

    /// v0.11.0 阶段 1（ADR-0019）：worker panic 时调用。记录 `last_crash`，
    /// 并尝试在 backoff 窗口到期时 respawn。返回 true 表示此次调用触发了
    /// respawn（首次调用因 backoff 未到通常返回 false）；后续 [`Self::restart_tick`]
    /// 在窗口到期时会触发实际 respawn。
    ///
    /// `name` 是 `WorkerCrash.worker` 字段值（thread_name，如
    /// `"port-snapshot-worker"`）；不在已知 worker 列表中（如测试用 mock
    /// thread_name）时直接返回 false。
    pub fn restart(
        &mut self,
        name: &str,
        now: SystemTime,
        crash_tx: Option<&Sender<WorkerCrash>>,
    ) -> bool {
        let Some(canonical) = canonical_worker_thread_name(name) else {
            return false;
        };
        // note_crash: 写入 restart_history + reset 检查 + 永久失败检查。
        let accepted = self
            .restart_history
            .entry(canonical)
            .or_insert_with(|| RestartState::new(now))
            .record_crash(now);
        if !accepted {
            return false;
        }
        self.try_respawn(canonical, now, crash_tx)
    }

    /// v0.11.0 阶段 1（ADR-0019）：主线程 tick 每 1s 调一次。遍历
    /// `restart_history` 中 pending crash 的 worker，backoff 到期就 respawn。
    /// 返回本次 tick 触发 respawn 的 worker 列表（thread_name），调用方可用于
    /// 推 status_message。
    pub fn restart_tick(
        &mut self,
        now: SystemTime,
        crash_tx: Option<&Sender<WorkerCrash>>,
    ) -> Vec<&'static str> {
        let pending: Vec<&'static str> = self
            .restart_history
            .iter()
            .filter_map(|(name, state)| {
                if state.last_crash.is_some() && !state.is_permanent_failure() {
                    Some(*name)
                } else {
                    None
                }
            })
            .collect();

        let mut restarted = Vec::new();
        for name in pending {
            if self.try_respawn(name, now, crash_tx) {
                restarted.push(name);
            }
        }
        restarted
    }

    /// v0.11.0 阶段 1（ADR-0019）：banner 渲染查此方法得到当前 worker 状态。
    /// `name` 与 [`Self::restart`] 同款（thread_name）。无 history 时返回 Healthy。
    #[must_use]
    pub fn restart_status(&self, name: &str, now: SystemTime) -> RestartStatus {
        let Some(canonical) = canonical_worker_thread_name(name) else {
            return RestartStatus::Healthy;
        };
        let Some(state) = self.restart_history.get(canonical) else {
            return RestartStatus::Healthy;
        };
        RestartStatus::from_state(state, now)
    }

    /// 决策 + spawn。返回 true 表示已 respawn（包含 state 更新）。
    fn try_respawn(
        &mut self,
        name: &'static str,
        now: SystemTime,
        crash_tx: Option<&Sender<WorkerCrash>>,
    ) -> bool {
        // 先借 immutable 读 state 判 backoff，避免与 spawn_one 的 mutable 借用冲突。
        let should_spawn = self
            .restart_history
            .get(name)
            .is_some_and(|s| s.decide_restart(now).is_some());
        if !should_spawn {
            return false;
        }
        let spawned = self.spawn_one(name, crash_tx);
        if spawned {
            if let Some(state) = self.restart_history.get_mut(name) {
                state.on_respawned(now);
            }
        }
        spawned
    }

    /// 真实 spawn 一个新 worker 替换旧的（旧的 drop 时 shutdown + join 旧线程）。
    /// net_flow / dns_log / disk_io_etw / schannel_etw 是 Option 字段：detect/try_spawn
    /// 失败时返回 false（不更新 state）。
    fn spawn_one(&mut self, name: &'static str, crash_tx: Option<&Sender<WorkerCrash>>) -> bool {
        match name {
            "port-snapshot-worker" => {
                let new = crate::port_worker::spawn(crash_tx.cloned());
                let _old = std::mem::replace(&mut self.port_worker, new);
                true
            }
            "usb-snapshot-worker" => {
                let new = crate::eject::snapshot_worker::spawn(crash_tx.cloned());
                let _old = std::mem::replace(&mut self.usb_worker, new);
                true
            }
            "net-flow-worker" => {
                let Some(collector) = crate::net_flow::detect_collector() else {
                    return false;
                };
                let new = crate::net_flow::worker::spawn(collector, crash_tx.cloned());
                self.net_flow_worker = Some(new);
                true
            }
            "dns-log-worker" => {
                let (collector, _new_kind) = crate::dns_log::detect_collector();
                let Some(collector) = collector else {
                    return false;
                };
                let new = crate::dns_log::worker::spawn(collector, crash_tx.cloned());
                self.dns_log_worker = Some(new);
                true
            }
            "disk-io-etw-worker" => {
                let Some(new) = crate::disk_io_etw::try_spawn(crash_tx.cloned()) else {
                    return false;
                };
                self.disk_io_etw_worker = Some(new);
                true
            }
            "schannel-etw-worker" => {
                let Some(new) = crate::schannel_etw::try_spawn(crash_tx.cloned()) else {
                    return false;
                };
                self.schannel_etw_worker = Some(new);
                true
            }
            _ => false,
        }
    }
}

/// 把任意 thread_name 字符串规范化为已知的 worker thread_name 字面量。
/// 与 `WorkerCrash.worker` / `SnapshotWorker::spawn(thread_name, ...)` 同源。
/// 未知 thread_name（如测试 mock）返回 None。
fn canonical_worker_thread_name(name: &str) -> Option<&'static str> {
    match name {
        "port-snapshot-worker" => Some("port-snapshot-worker"),
        "usb-snapshot-worker" => Some("usb-snapshot-worker"),
        "net-flow-worker" => Some("net-flow-worker"),
        "dns-log-worker" => Some("dns-log-worker"),
        "disk-io-etw-worker" => Some("disk-io-etw-worker"),
        "schannel-etw-worker" => Some("schannel-etw-worker"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_returns_known_thread_names() {
        assert_eq!(
            canonical_worker_thread_name("port-snapshot-worker"),
            Some("port-snapshot-worker")
        );
        assert_eq!(
            canonical_worker_thread_name("dns-log-worker"),
            Some("dns-log-worker")
        );
        assert_eq!(canonical_worker_thread_name("totally-fake"), None);
        assert_eq!(canonical_worker_thread_name(""), None);
    }

    #[test]
    fn restart_returns_false_for_unknown_worker() {
        let mut mgr = WorkerManager::new(None);
        let now = SystemTime::now();
        assert!(!mgr.restart("totally-fake-worker", now, None));
        assert!(mgr.restart_history.is_empty());
    }

    #[test]
    fn restart_records_state_for_known_worker() {
        // 第一次 restart() 调用时只记录 last_crash，backoff_for(0)=5s 未到
        // 所以 try_respawn 返回 false、retry_count 保持 0。验证「last_crash
        // 已记」+「backoff 内 retry_count 不变」两件事。
        let mut mgr = WorkerManager::new(None);
        let now = SystemTime::now();
        let _ = mgr.restart("dns-log-worker", now, None);
        let state = mgr
            .restart_history
            .get("dns-log-worker")
            .expect("state recorded");
        assert_eq!(state.last_crash, Some(now));
        assert_eq!(state.retry_count, 0, "backoff 内 retry_count 应保持 0");
    }

    #[test]
    fn restart_status_healthy_for_unknown_worker() {
        let mgr = WorkerManager::new(None);
        let now = SystemTime::now();
        assert_eq!(
            mgr.restart_status("totally-fake-worker", now),
            RestartStatus::Healthy
        );
    }

    #[test]
    fn backoff_for_module_public() {
        // sanity check：restart.rs 的纯函数通过 manager.rs 可访问。
        assert_eq!(
            super::super::restart::backoff_for(0),
            std::time::Duration::from_secs(5)
        );
    }
}
