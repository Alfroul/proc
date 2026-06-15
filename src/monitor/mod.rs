pub mod notify;
pub mod port_watcher;
pub mod snapshot;
pub mod watchdog;

use std::path::PathBuf;
use std::time::SystemTime;

use crate::error::{ProcError, Result};

/// 监控目标类型
#[derive(Debug, Clone)]
pub enum MonitorTarget {
    /// 按 PID 监控进程（仅通知模式）
    ByPid { pid: u32 },
    /// 按端口号监控（检测端口占用变化）
    ByPort { port: u16 },
    /// 按命令监控（Watchdog 自动重启）
    ByCommand {
        cmd: String,
        args: Vec<String>,
        cwd: Option<PathBuf>,
    },
}

/// 重启策略
#[derive(Debug, Clone, Default)]
pub enum RestartPolicy {
    /// 仅通知，不自动重启
    #[default]
    NotifyOnly,
    /// 自动重启
    AutoRestart {
        max_retries: u32,
        base_backoff: u64, // 秒
        max_backoff: u64,  // 秒
    },
}

/// 监控条目状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorStatus {
    Running,
    Stopped,
    Crashed,
    Paused,
}

impl std::fmt::Display for MonitorStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "运行中"),
            Self::Stopped => write!(f, "已停止"),
            Self::Crashed => write!(f, "已崩溃"),
            Self::Paused => write!(f, "已暂停"),
        }
    }
}

/// 监控条目
#[derive(Debug, Clone)]
pub struct MonitorEntry {
    pub id: u32,
    pub target: MonitorTarget,
    pub pid: Option<u32>,
    pub status: MonitorStatus,
    pub crash_count: u32,
    pub last_crash_time: Option<SystemTime>,
    pub restart_policy: RestartPolicy,
    pub created_at: SystemTime,
}

/// 通知记录
#[derive(Debug, Clone)]
pub struct NotificationRecord {
    pub timestamp: SystemTime,
    pub message: String,
}

/// 监控管理器
#[derive(Default)]
pub struct MonitorManager {
    entries: Vec<MonitorEntry>,
    next_id: u32,
    notifications: Vec<NotificationRecord>,
}

impl MonitorManager {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
            notifications: Vec::new(),
        }
    }

    pub fn add_monitor(
        &mut self,
        target: MonitorTarget,
        restart_policy: RestartPolicy,
    ) -> Result<u32> {
        let id = self.next_id;
        self.next_id += 1;

        let pid = match &target {
            MonitorTarget::ByPid { pid } => Some(*pid),
            MonitorTarget::ByPort { .. } => None,
            MonitorTarget::ByCommand { .. } => None,
        };

        let entry = MonitorEntry {
            id,
            target,
            pid,
            status: MonitorStatus::Running,
            crash_count: 0,
            last_crash_time: None,
            restart_policy,
            created_at: SystemTime::now(),
        };

        self.entries.push(entry);
        Ok(id)
    }

    pub fn remove_monitor(&mut self, id: u32) -> Result<()> {
        let idx = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| ProcError::NotFound(format!("监控条目 {} 不存在", id)))?;
        self.entries.remove(idx);
        Ok(())
    }

    pub fn list_monitors(&self) -> &[MonitorEntry] {
        &self.entries
    }

    pub fn get_monitor(&self, id: u32) -> Option<&MonitorEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_monitor_mut(&mut self, id: u32) -> Option<&mut MonitorEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn add_notification(&mut self, message: String) {
        self.notifications.push(NotificationRecord {
            timestamp: SystemTime::now(),
            message,
        });
        // 保留最近 100 条通知
        if self.notifications.len() > 100 {
            let excess = self.notifications.len() - 100;
            self.notifications.drain(..excess);
        }
    }

    pub fn notifications(&self) -> &[NotificationRecord] {
        &self.notifications
    }

    /// 计算指数退避时间（秒）
    pub fn calc_backoff(base: u64, crash_count: u32, max_backoff: u64) -> u64 {
        let exp = crash_count.saturating_sub(1);
        let delay = base.saturating_mul(2u64.saturating_pow(exp));
        delay.min(max_backoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_monitor_by_pid() {
        let mut mgr = MonitorManager::new();
        let id = mgr
            .add_monitor(
                MonitorTarget::ByPid { pid: 1234 },
                RestartPolicy::NotifyOnly,
            )
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(mgr.list_monitors().len(), 1);
        assert_eq!(mgr.list_monitors()[0].pid, Some(1234));
    }

    #[test]
    fn test_add_monitor_by_port() {
        let mut mgr = MonitorManager::new();
        let id = mgr
            .add_monitor(
                MonitorTarget::ByPort { port: 8080 },
                RestartPolicy::NotifyOnly,
            )
            .unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn test_add_monitor_by_command() {
        let mut mgr = MonitorManager::new();
        let id = mgr
            .add_monitor(
                MonitorTarget::ByCommand {
                    cmd: "cargo run".to_string(),
                    args: vec![],
                    cwd: None,
                },
                RestartPolicy::AutoRestart {
                    max_retries: 5,
                    base_backoff: 1,
                    max_backoff: 30,
                },
            )
            .unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn test_remove_monitor() {
        let mut mgr = MonitorManager::new();
        let id = mgr
            .add_monitor(
                MonitorTarget::ByPid { pid: 1234 },
                RestartPolicy::NotifyOnly,
            )
            .unwrap();
        mgr.remove_monitor(id).unwrap();
        assert!(mgr.list_monitors().is_empty());
    }

    #[test]
    fn test_remove_nonexistent_monitor() {
        let mut mgr = MonitorManager::new();
        assert!(mgr.remove_monitor(999).is_err());
    }

    #[test]
    fn test_calc_backoff() {
        assert_eq!(MonitorManager::calc_backoff(1, 1, 30), 1); // 1*2^0 = 1
        assert_eq!(MonitorManager::calc_backoff(1, 2, 30), 2); // 1*2^1 = 2
        assert_eq!(MonitorManager::calc_backoff(1, 3, 30), 4); // 1*2^2 = 4
        assert_eq!(MonitorManager::calc_backoff(1, 4, 30), 8); // 1*2^3 = 8
        assert_eq!(MonitorManager::calc_backoff(1, 5, 30), 16); // 1*2^4 = 16
        assert_eq!(MonitorManager::calc_backoff(1, 6, 30), 30); // 1*2^5=32, capped at 30
    }

    #[test]
    fn test_add_notification() {
        let mut mgr = MonitorManager::new();
        mgr.add_notification("test notification".to_string());
        assert_eq!(mgr.notifications().len(), 1);
    }
}
