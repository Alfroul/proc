//! v0.11.0 阶段 1：Worker Restart Policy — 指数退避 + 最大重试 + reset 计数。
//!
//! 详见 ADR-0019。本模块只放纯数据结构 + 纯函数决策（无 IO，便于单元测试），
//! spawn_one / restart_tick 涉及真实 worker spawn 的部分在 `manager.rs`。
//!
//! 设计要点：
//! - `RestartState` 记录每个 worker 的 `retry_count` / `last_crash` /
//!   `last_restart` / `last_reset`
//! - `backoff_for(retry_count)`：5s / 30s / 5min（达到 MAX_RETRIES 时返回 0）
//! - `RESET_WINDOW = 1h`：上次成功 spawn 距今 ≥ 1h 且 retry_count > 0 时归零
//! - `MAX_RETRIES = 3`：3 次 respawn 后仍 panic 视为「永久失败」，不再 spawn
//!
//! 状态机调用顺序：
//! 1. crash 发生时 App 调 `WorkerManager::restart(name, now, crash_tx)`：
//!    - `note_crash` 记录 `last_crash = now`
//!    - 同 tick 调 `try_respawn`（backoff 未到返回 false，到期则 spawn + retry_count+=1）
//! 2. `App::tick` 每 1s 调 `WorkerManager::restart_tick(now, crash_tx)`：
//!    - 遍历 `restart_history` 中 `last_crash.is_some()` 的 worker
//!    - 时间到 backoff 就 spawn + retry_count+=1
//! 3. `WorkerManager::restart_status(name, now)` 返回 banner 渲染用的状态

use std::time::{Duration, SystemTime};

/// 最大重试次数。3 次 respawn 后仍 panic 视为永久失败。
pub const MAX_RETRIES: u32 = 3;

/// reset 窗口：上次成功 spawn 距今 ≥ 此窗口且 retry_count > 0 时归零。
pub const RESET_WINDOW: Duration = Duration::from_secs(3600);

/// 指数退避时长。`retry_count == 0` → 5s（第一次 panic 后等 5s 再 respawn），
/// `1` → 30s，`2` → 5min。`>= MAX_RETRIES` 永久失败，返回 0（决策函数会拒绝）。
#[must_use]
pub fn backoff_for(retry_count: u32) -> Duration {
    match retry_count {
        0 => Duration::from_secs(5),
        1 => Duration::from_secs(30),
        2 => Duration::from_secs(300),
        _ => Duration::ZERO,
    }
}

/// 单个 worker 的 restart 状态机。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartState {
    /// 已成功 respawn 的次数。达到 [`MAX_RETRIES`] 时进入「永久失败」。
    pub retry_count: u32,
    /// 最近一次 panic 时刻（backoff 起算点）。spawn 成功后清空。
    pub last_crash: Option<SystemTime>,
    /// 最近一次 respawn 时刻（reset 起算点）。
    pub last_restart: Option<SystemTime>,
    /// 最近一次 retry_count 归零时刻。
    pub last_reset: SystemTime,
}

impl RestartState {
    /// 用 now 初始化（last_reset = now，retry_count = 0）。
    #[must_use]
    pub fn new(now: SystemTime) -> Self {
        Self {
            retry_count: 0,
            last_crash: None,
            last_restart: None,
            last_reset: now,
        }
    }

    /// 是否处于「永久失败」状态（retry_count ≥ MAX_RETRIES）。
    #[must_use]
    pub fn is_permanent_failure(&self) -> bool {
        self.retry_count >= MAX_RETRIES
    }

    /// 应用一次 panic 事件。reset 检查（1h 无 panic 后归零）+ 永久失败检查 +
    /// 记录 `last_crash = now`。返回 true 表示该 panic 被纳入重启队列，返回
    /// false 表示已被永久失败拦截。
    pub fn record_crash(&mut self, now: SystemTime) -> bool {
        // reset：上次成功 spawn 距今 ≥ RESET_WINDOW 且 retry_count > 0 → 归零。
        if self.retry_count > 0
            && let Some(last) = self.last_restart
            && now.duration_since(last).unwrap_or_default() >= RESET_WINDOW
        {
            self.retry_count = 0;
            self.last_reset = now;
        }

        // 永久失败：不再记录。
        if self.retry_count >= MAX_RETRIES {
            return false;
        }

        self.last_crash = Some(now);
        true
    }

    /// 决策：当前时刻 `now` 是否应触发 respawn。返回 `Some(())` 表示应 spawn +
    /// 调用方需在 spawn 成功后调 [`Self::on_respawned`]；返回 `None` 表示还在
    /// 退避窗口 / 无 pending crash / 永久失败。
    #[must_use]
    pub fn decide_restart(&self, now: SystemTime) -> Option<()> {
        if self.retry_count >= MAX_RETRIES {
            return None;
        }
        let crash = self.last_crash?;
        let backoff = backoff_for(self.retry_count);
        if now.duration_since(crash).unwrap_or_default() >= backoff {
            Some(())
        } else {
            None
        }
    }

    /// spawn 成功后调用：retry_count+=1，更新 last_restart，清空 last_crash。
    pub fn on_respawned(&mut self, now: SystemTime) {
        self.retry_count = self.retry_count.saturating_add(1);
        self.last_restart = Some(now);
        self.last_crash = None;
    }
}

/// banner 渲染用的 worker 当前状态。由 `WorkerManager::restart_status` 返回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartStatus {
    /// 该 worker 没有 restart 历史（没崩过）。
    Healthy,
    /// 在退避窗口内等待 respawn。
    Restarting {
        retry_count: u32,
        /// 距离 respawn 的剩余秒数（向上取整）。
        remaining_secs: u64,
    },
    /// 最近成功 respawn 过（用户可见「已重启」反馈）。
    Restarted {
        retry_count: u32,
        /// 距离 respawn 已过的秒数。
        elapsed_secs: u64,
    },
    /// 永久失败（retry_count ≥ MAX_RETRIES）。
    PermanentFailure { retry_count: u32 },
}

impl RestartStatus {
    /// 从 RestartState 推导当前 banner 状态。`now` 是当前时刻。
    #[must_use]
    pub fn from_state(state: &RestartState, now: SystemTime) -> Self {
        if state.is_permanent_failure() {
            return Self::PermanentFailure {
                retry_count: state.retry_count,
            };
        }
        if let Some(crash) = state.last_crash {
            // pending crash：检查 backoff 是否到期。
            let backoff = backoff_for(state.retry_count);
            let elapsed = now.duration_since(crash).unwrap_or_default();
            if elapsed >= backoff {
                // 即将 spawn（restart_tick 会触发），暂归到 Restarting 显示。
                Self::Restarting {
                    retry_count: state.retry_count,
                    remaining_secs: 0,
                }
            } else {
                let remaining = backoff.saturating_sub(elapsed);
                Self::Restarting {
                    retry_count: state.retry_count,
                    remaining_secs: remaining.as_secs(),
                }
            }
        } else if let Some(last_restart) = state.last_restart {
            // 没 pending crash，但有过 restart：显示已重启反馈。
            let elapsed = now.duration_since(last_restart).unwrap_or_default();
            Self::Restarted {
                retry_count: state.retry_count,
                elapsed_secs: elapsed.as_secs(),
            }
        } else {
            Self::Healthy
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    #[test]
    fn backoff_for_matches_adr_0019_table() {
        assert_eq!(backoff_for(0), Duration::from_secs(5));
        assert_eq!(backoff_for(1), Duration::from_secs(30));
        assert_eq!(backoff_for(2), Duration::from_secs(300));
        // retry_count >= MAX_RETRIES 时不再 spawn，返回 0（决策函数会拒绝）。
        assert_eq!(backoff_for(3), Duration::ZERO);
        assert_eq!(backoff_for(u32::MAX), Duration::ZERO);
    }

    #[test]
    fn record_crash_sets_last_crash_for_first_panic() {
        let mut s = RestartState::new(t0());
        assert!(s.record_crash(t0()));
        assert_eq!(s.last_crash, Some(t0()));
        assert_eq!(s.retry_count, 0);
    }

    #[test]
    fn record_crash_refused_after_permanent_failure() {
        let mut s = RestartState::new(t0());
        // 模拟 3 次 spawn → retry_count = 3
        for _ in 0..MAX_RETRIES {
            s.record_crash(t0());
            s.on_respawned(t0());
        }
        assert_eq!(s.retry_count, MAX_RETRIES);
        assert!(s.is_permanent_failure());

        // 第 4 次 panic 不再记录。
        let before = s.clone();
        assert!(!s.record_crash(t0() + Duration::from_secs(60)));
        assert_eq!(s, before);
    }

    #[test]
    fn reset_window_zeroes_retry_count_after_1h() {
        let mut s = RestartState::new(t0());
        s.retry_count = 2;
        s.last_restart = Some(t0());
        // 1h 后下一次 panic 应触发 reset。
        let later = t0() + RESET_WINDOW + Duration::from_secs(1);
        assert!(s.record_crash(later));
        assert_eq!(s.retry_count, 0, "retry_count 应被 reset 为 0");
        assert_eq!(s.last_reset, later);
        assert_eq!(s.last_crash, Some(later));
    }

    #[test]
    fn reset_window_skipped_within_1h() {
        let mut s = RestartState::new(t0());
        s.retry_count = 2;
        s.last_restart = Some(t0());
        // 1h - 1s 时下一次 panic 不应触发 reset。
        let within = t0() + RESET_WINDOW - Duration::from_secs(1);
        assert!(s.record_crash(within));
        assert_eq!(s.retry_count, 2, "retry_count 不应被 reset");
    }

    #[test]
    fn decide_restart_returns_none_when_no_crash() {
        let s = RestartState::new(t0());
        assert!(s.decide_restart(t0()).is_none());
    }

    #[test]
    fn decide_restart_returns_none_within_backoff() {
        let mut s = RestartState::new(t0());
        s.record_crash(t0());
        // backoff(0) = 5s；4s 时还未到期。
        assert!(s.decide_restart(t0() + Duration::from_secs(4)).is_none());
    }

    #[test]
    fn decide_restart_returns_some_when_backoff_elapsed() {
        let mut s = RestartState::new(t0());
        s.record_crash(t0());
        // 5s 时到期。
        assert!(s.decide_restart(t0() + Duration::from_secs(5)).is_some());
    }

    #[test]
    fn decide_restart_uses_indexed_backoff_for_higher_retry_counts() {
        let mut s = RestartState::new(t0());
        s.retry_count = 1;
        s.record_crash(t0());
        // backoff(1) = 30s；29s 时未到期，30s 时到期。
        assert!(s.decide_restart(t0() + Duration::from_secs(29)).is_none());
        assert!(s.decide_restart(t0() + Duration::from_secs(30)).is_some());
    }

    #[test]
    fn on_respawned_increments_and_clears_last_crash() {
        let mut s = RestartState::new(t0());
        s.record_crash(t0());
        let now = t0() + Duration::from_secs(5);
        s.on_respawned(now);
        assert_eq!(s.retry_count, 1);
        assert_eq!(s.last_restart, Some(now));
        assert!(s.last_crash.is_none());
    }

    #[test]
    fn four_panics_drive_to_permanent_failure() {
        // stage-1.md 任务 6：4 次 panic 后 retry_count == 3 = MAX_RETRIES。
        let mut s = RestartState::new(t0());
        // 第 1 次 panic → 等 5s spawn → retry_count=1
        s.record_crash(t0());
        assert!(s.decide_restart(t0() + Duration::from_secs(5)).is_some());
        s.on_respawned(t0() + Duration::from_secs(5));
        assert_eq!(s.retry_count, 1);
        // 第 2 次 panic → 等 30s spawn → retry_count=2
        s.record_crash(t0() + Duration::from_secs(10));
        assert!(s.decide_restart(t0() + Duration::from_secs(40)).is_some());
        s.on_respawned(t0() + Duration::from_secs(40));
        assert_eq!(s.retry_count, 2);
        // 第 3 次 panic → 等 5min spawn → retry_count=3
        s.record_crash(t0() + Duration::from_secs(50));
        assert!(
            s.decide_restart(t0() + Duration::from_secs(50) + Duration::from_secs(300))
                .is_some()
        );
        s.on_respawned(t0() + Duration::from_secs(50) + Duration::from_secs(300));
        assert_eq!(s.retry_count, 3);
        assert!(s.is_permanent_failure());
        // 第 4 次 panic → record_crash 返回 false（拒绝记录）
        assert!(!s.record_crash(t0() + Duration::from_secs(1000)));
    }

    #[test]
    fn restart_status_healthy_for_new_state() {
        let s = RestartState::new(t0());
        assert_eq!(RestartStatus::from_state(&s, t0()), RestartStatus::Healthy);
    }

    #[test]
    fn restart_status_restarting_shows_remaining_secs() {
        let mut s = RestartState::new(t0());
        s.record_crash(t0());
        // 2s passed → 3s remaining (backoff(0)=5s).
        let status = RestartStatus::from_state(&s, t0() + Duration::from_secs(2));
        assert_eq!(
            status,
            RestartStatus::Restarting {
                retry_count: 0,
                remaining_secs: 3
            }
        );
    }

    #[test]
    fn restart_status_permanent_failure_at_max_retries() {
        let mut s = RestartState::new(t0());
        s.retry_count = MAX_RETRIES;
        let status = RestartStatus::from_state(&s, t0());
        assert_eq!(
            status,
            RestartStatus::PermanentFailure {
                retry_count: MAX_RETRIES
            }
        );
    }

    #[test]
    fn restart_status_restarted_after_successful_respawn() {
        let mut s = RestartState::new(t0());
        s.record_crash(t0());
        let restart_time = t0() + Duration::from_secs(5);
        s.on_respawned(restart_time);
        // 2s after restart → Restarted with elapsed=2.
        let status = RestartStatus::from_state(&s, restart_time + Duration::from_secs(2));
        assert_eq!(
            status,
            RestartStatus::Restarted {
                retry_count: 1,
                elapsed_secs: 2
            }
        );
    }
}
