//! v0.6.0 阶段 3：worker 可观测性 + panic 崩溃报告。
//!
//! 两个职责：
//! - [`WorkerMetrics`]：atomic 计数器，worker 主循环每轮 record 一次。
//!   主线程通过 [`WorkerStats`] 快照查询；`proc diag` 序列化输出；`?`
//!   帮助页显示 `health_badge`。
//! - [`crash`] 模块：panic hook 写 `~/.config/proc/crashes/crash-{ts}.txt`，
//!   见 CONTEXT.md。

pub mod crash;

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Worker 自身的可观测计数器。无锁（atomic + Mutex 仅 last_error 一处）。
///
/// 字段定义见 CONTEXT.md 「WorkerMetrics」。每个 `SnapshotWorker` 持有一份
/// `Arc<WorkerMetrics>`，主线程通过 `metrics()` 拿另一个 `Arc` 副本查询。
#[derive(Default)]
pub struct WorkerMetrics {
    /// 总 poll 次数。
    poll_count: AtomicU64,
    /// 累计 poll 耗时（微秒）。avg = total / count。
    poll_total_us: AtomicU64,
    /// 单次 poll 最大耗时（微秒）。
    poll_max_us: AtomicU64,
    /// `try_send` 返回 `Full` 次数（主线程消费跟不上 → 丢帧）。
    channel_full_count: AtomicU64,
    /// 最近一次非致命错误（如 channel Disconnected）。
    last_error: Mutex<Option<(SystemTime, String)>>,
}

impl WorkerMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// worker 主循环每轮 record 一次 `elapsed`。
    pub fn record_poll(&self, elapsed: Duration) {
        let us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        self.poll_total_us.fetch_add(us, Ordering::Relaxed);
        // CAS 更新 max：load → compare_exchange_weak 循环到失败或更新成功。
        let mut current_max = self.poll_max_us.load(Ordering::Relaxed);
        while us > current_max {
            match self.poll_max_us.compare_exchange_weak(
                current_max,
                us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(now) => current_max = now,
            }
        }
    }

    /// `try_send` 返回 `Full` 时调一次。
    pub fn record_channel_full(&self) {
        self.channel_full_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 非致命错误（channel Disconnected / collector 临时失败）。
    pub fn record_error(&self, msg: impl Into<String>) {
        if let Ok(mut last) = self.last_error.lock() {
            *last = Some((SystemTime::now(), msg.into()));
        }
    }

    /// 取当前快照（所有 atomic 同时读，弱一致）。
    #[must_use]
    pub fn snapshot(&self) -> WorkerStats {
        let count = self.poll_count.load(Ordering::Relaxed);
        let total = self.poll_total_us.load(Ordering::Relaxed);
        let last_error = self
            .last_error
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|(t, m)| (*t, m.clone())));
        WorkerStats {
            poll_count: count,
            poll_total_us: total,
            avg_us: total.checked_div(count).unwrap_or(0),
            max_us: self.poll_max_us.load(Ordering::Relaxed),
            channel_full: self.channel_full_count.load(Ordering::Relaxed),
            last_error,
        }
    }
}

/// `WorkerMetrics::snapshot()` 的不可变快照。`proc diag --json` 直接序列化。
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkerStats {
    pub poll_count: u64,
    pub poll_total_us: u64,
    pub avg_us: u64,
    pub max_us: u64,
    pub channel_full: u64,
    pub last_error: Option<(SystemTime, String)>,
}

/// `proc diag` 的输出条目：worker 名 + stats。`#[serde(flatten)]` 让 JSON
/// 形如 `{"name": "port", "poll_count": 12, ...}`，扁平友好。
#[derive(Debug, Clone, serde::Serialize)]
pub struct NamedWorkerStats {
    pub name: &'static str,
    #[serde(flatten)]
    pub stats: WorkerStats,
}

impl WorkerStats {
    /// 健康徽章：`✓` 正常 / `⚠` 异常（丢帧多 / 单轮过慢 / 有错误）。
    #[must_use]
    pub fn health_badge(&self) -> &'static str {
        // 100ms 阈值：常规 worker 单轮应在 50ms 内；超过 100ms 一定是 syscall 阻塞。
        const SLOW_US: u64 = 100_000;
        if self.channel_full > 10 {
            return "⚠";
        }
        if self.max_us > SLOW_US {
            return "⚠";
        }
        if self.last_error.is_some() {
            return "⚠";
        }
        "✓"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn record_poll_updates_count_total_max() {
        let m = WorkerMetrics::new();
        m.record_poll(Duration::from_micros(500));
        m.record_poll(Duration::from_micros(1_500));
        m.record_poll(Duration::from_micros(1_000));
        let s = m.snapshot();
        assert_eq!(s.poll_count, 3);
        assert_eq!(s.poll_total_us, 3_000);
        assert_eq!(s.avg_us, 1_000);
        assert_eq!(s.max_us, 1_500);
        assert_eq!(s.channel_full, 0);
        assert!(s.last_error.is_none());
        assert_eq!(s.health_badge(), "✓");
    }

    #[test]
    fn record_channel_full_increments() {
        let m = WorkerMetrics::new();
        for _ in 0..15 {
            m.record_channel_full();
        }
        let s = m.snapshot();
        assert_eq!(s.channel_full, 15);
        assert_eq!(s.health_badge(), "⚠");
    }

    #[test]
    fn record_error_stores_latest() {
        let m = WorkerMetrics::new();
        m.record_error("first");
        m.record_error("second");
        let s = m.snapshot();
        assert_eq!(s.last_error.as_ref().unwrap().1, "second");
        assert_eq!(s.health_badge(), "⚠");
    }

    #[test]
    fn slow_max_poll_marks_unhealthy() {
        let m = WorkerMetrics::new();
        m.record_poll(Duration::from_micros(150_000)); // > 100ms
        let s = m.snapshot();
        assert_eq!(s.max_us, 150_000);
        assert_eq!(s.health_badge(), "⚠");
    }

    #[test]
    fn concurrent_record_poll_is_safe() {
        // 10 线程并发 record 1000 次 — CAS 路径无死锁。
        let m = Arc::new(WorkerMetrics::new());
        let mut handles = Vec::new();
        for _ in 0..10 {
            let m = Arc::clone(&m);
            handles.push(thread::spawn(move || {
                for i in 0..1000 {
                    m.record_poll(Duration::from_micros(i));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let s = m.snapshot();
        assert_eq!(s.poll_count, 10_000);
    }
}
