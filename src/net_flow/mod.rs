//! 阶段 7 D1：per-process 网络流量观测。
//!
//! 目标：让用户知道「哪个进程在占带宽」。Mission Center 1.0 专门补的缺口。
//!
//! # 架构
//!
//! - [`NetFlowCollector`] trait 抽象数据源（参考阶段 6 [`crate::gpu::GpuProvider`] 模式）
//! - Windows 实现 [`windows`] 走 IP Helper API（`GetExtendedTcpTable` +
//!   `GetPerTcpConnectionEStats`，复用 [`crate::estats`] 的同类调用，按 PID 聚合）
//! - [`worker::NetFlowWorker`] 复用 [`crate::worker::SnapshotWorker`]，1s poll
//!
//! # PID 复用
//!
//! `ProcessNetRate` 带 `start_time` 字段（参考 ADR-0003 缓存键策略）。Windows
//! IP Helper 不暴露 start_time，collector 本身填 0；主线程在把速率贴回
//! `ProcessInfo` 时按 PID 查 `cached_processes` 拿真实 start_time。PID 在 1s
//! poll 间隔内被复用是极小概率事件；collector 内部用「计数回退」检测 PID
//! 复用（当前累计 < 上次累计 → 视为新进程，速率按 0 计）。
//!
//! # 选型说明
//!
//! 阶段 7 原计划 Windows 走 ETW Kernel-Network provider。实际落地走 IP Helper
//! 备选路线，理由：ETW 实时 session 需要单独消费者线程 + `ProcessTrace` 阻塞
//! 调用 + 大量 native FFI，工作量与 ROI 不匹配；IP Helper 路径复用已有
//! `estats.rs` 的成熟 Win32 调用，1s 周期下 CPU 开销 < 1%。详见
//! `docs/adr/0005-netflow-windows-iphelper-not-etw.md`。

pub mod windows;
pub mod worker;

use std::fmt;

/// 单进程的字节速率快照（自上次 collector refresh 以来的差值，已归一化到 bytes/sec）。
///
/// `start_time` 用于防 PID 复用（ADR-0003）。collector 本身不总能拿到
/// start_time（如 Windows IP Helper 不暴露），此时填 0；消费者按 PID 查
/// `cached_processes` 拿真实 start_time 后再做 (pid, start_time) tuple 缓存。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessNetRate {
    pub pid: u32,
    pub start_time: u64,
    pub bytes_sent_per_sec: u64,
    pub bytes_recv_per_sec: u64,
}

/// per-process 网络字节速率采集器（参考 [`crate::gpu::GpuProvider`] trait 模式）。
///
/// `per_process_rates` 是有状态的：每次调用返回「自上次调用以来的差值，已
/// 归一化到 bytes/sec」。实现内部维护累计计数器 + 上次时间戳。
pub trait NetFlowCollector: Send + Sync {
    /// 返回每进程的字节速率。空 Vec 表示 collector 不可用 / 暂无数据。
    fn per_process_rates(&mut self) -> Vec<ProcessNetRate>;

    /// 人类可读的 provider 名（用于日志 / 调试）。
    fn provider_name(&self) -> &'static str;
}

/// 返回合适的 collector。无可用 collector 时返回 None
/// （主线程 net 列保持 0，不阻塞其它功能）。
#[must_use]
pub fn detect_collector() -> Option<Box<dyn NetFlowCollector>> {
    match self::windows::IphelperCollector::new() {
        Ok(c) => return Some(Box::new(c)),
        Err(e) => {
            tracing::warn!("Windows IP Helper collector 初始化失败，per-process 网络列保持 0: {e}")
        }
    }

    None
}

impl fmt::Display for ProcessNetRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 单元测试 anchor；非关键 UI 路径（速率格式化由 format::format_speed 负责）。
        write!(
            f,
            "pid={} ↑{}B/s ↓{}B/s",
            self.pid, self.bytes_sent_per_sec, self.bytes_recv_per_sec
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_net_rate_display_basic() {
        let r = ProcessNetRate {
            pid: 1234,
            start_time: 0,
            bytes_sent_per_sec: 100,
            bytes_recv_per_sec: 200,
        };
        assert_eq!(r.to_string(), "pid=1234 ↑100B/s ↓200B/s");
    }

    #[test]
    fn process_net_rate_display_zero() {
        let r = ProcessNetRate {
            pid: 0,
            start_time: 0,
            bytes_sent_per_sec: 0,
            bytes_recv_per_sec: 0,
        };
        assert_eq!(r.to_string(), "pid=0 ↑0B/s ↓0B/s");
    }

    #[test]
    fn detect_collector_does_not_panic() {
        // 平台无关：detect_collector 必须能调用并返回 Option，不允许 panic。
        let _ = detect_collector();
    }
}
