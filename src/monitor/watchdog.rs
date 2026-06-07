use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crate::monitor::notify;
use crate::monitor::{MonitorManager, RestartPolicy};

/// Watchdog 发送给主线程的状态更新消息
#[derive(Debug)]
pub enum WatchdogEvent {
    Started { monitor_id: u32, pid: u32 },
    Crashed {
        monitor_id: u32,
        exit_code: Option<i32>,
        attempt: u32,
        restarting: bool,
    },
    Stopped { monitor_id: u32, reason: String },
    Running { monitor_id: u32, pid: u32 },
}

/// Watchdog 后台句柄
pub struct WatchdogHandle {
    pub monitor_id: u32,
    pub events: Receiver<WatchdogEvent>,
    pub shutdown: Sender<()>,
}

impl WatchdogHandle {
    pub fn try_recv(&self) -> Option<WatchdogEvent> {
        self.events.try_recv().ok()
    }

    pub fn stop(&self) {
        let _ = self.shutdown.send(());
    }
}

/// 启动 watchdog 后台线程
pub fn spawn_watchdog(
    monitor_id: u32,
    cmd: &str,
    args: &[String],
    cwd: Option<&Path>,
    restart_policy: RestartPolicy,
) -> WatchdogHandle {
    let (event_tx, event_rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    let cmd = cmd.to_string();
    let args = args.to_vec();
    let cwd = cwd.map(|p| p.to_path_buf());

    std::thread::spawn(move || {
        let (max_retries, base_backoff, max_backoff) = match &restart_policy {
            RestartPolicy::AutoRestart {
                max_retries,
                base_backoff,
                max_backoff,
            } => (*max_retries, *base_backoff, *max_backoff),
            RestartPolicy::NotifyOnly => (0, 1, 1),
        };

        let mut crash_count: u32 = 0;

        loop {
            if shutdown_rx.try_recv().is_ok() {
                let _ = event_tx.send(WatchdogEvent::Stopped {
                    monitor_id,
                    reason: "手动停止".to_string(),
                });
                return;
            }

            let mut child = match Command::new(&cmd).args(&args).current_dir(cwd.as_deref().unwrap_or(Path::new("."))).spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = event_tx.send(WatchdogEvent::Stopped {
                        monitor_id,
                        reason: format!("启动失败: {}", e),
                    });
                    return;
                }
            };

            let pid = child.id();
            let _ = event_tx.send(WatchdogEvent::Started {
                monitor_id,
                pid,
            });

            let status = match child.wait() {
                Ok(s) => s,
                Err(e) => {
                    let _ = event_tx.send(WatchdogEvent::Stopped {
                        monitor_id,
                        reason: format!("wait 失败: {}", e),
                    });
                    return;
                }
            };

            if shutdown_rx.try_recv().is_ok() {
                let _ = event_tx.send(WatchdogEvent::Stopped {
                    monitor_id,
                    reason: "手动停止".to_string(),
                });
                return;
            }

            let exit_code = status.code();

            if status.success() {
                let _ = event_tx.send(WatchdogEvent::Stopped {
                    monitor_id,
                    reason: "正常退出".to_string(),
                });
                return;
            }

            crash_count += 1;
            let attempt = crash_count;

            if max_retries > 0 && crash_count > max_retries {
                let _ = event_tx.send(WatchdogEvent::Crashed {
                    monitor_id,
                    exit_code,
                    attempt,
                    restarting: false,
                });
                let _ = event_tx.send(WatchdogEvent::Stopped {
                    monitor_id,
                    reason: format!("超过最大重试次数 ({})", max_retries),
                });
                notify::send_toast(
                    "进程监控",
                    &format!(
                        "监控进程 {} 崩溃，已超过最大重试次数 ({})",
                        cmd, max_retries
                    ),
                )
                .ok();
                return;
            }

            let _ = event_tx.send(WatchdogEvent::Crashed {
                monitor_id,
                exit_code,
                attempt,
                restarting: true,
            });

            notify::send_toast(
                "进程崩溃",
                &format!(
                    "监控进程 {} (PID {}) 异常退出 (code: {:?})，第 {} 次重启",
                    cmd, pid, exit_code, attempt
                ),
            )
            .ok();

            let backoff = MonitorManager::calc_backoff(base_backoff, crash_count, max_backoff);
            std::thread::sleep(Duration::from_secs(backoff));
        }
    });

    WatchdogHandle {
        monitor_id,
        events: event_rx,
        shutdown: shutdown_tx,
    }
}
