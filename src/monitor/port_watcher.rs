use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crate::monitor::notify;
use crate::port_map;

/// 端口状态变化事件
#[derive(Debug)]
pub enum PortEvent {
    /// 端口被新进程占用
    Occupied {
        port: u16,
        pid: u32,
        process_name: String,
    },
    /// 端口已释放（之前占用的进程退出）
    Released { port: u16 },
}

/// 端口监视器后台句柄
pub struct PortWatchHandle {
    pub port: u16,
    pub events: Receiver<PortEvent>,
    pub shutdown: Sender<()>,
}

impl PortWatchHandle {
    pub fn try_recv(&self) -> Option<PortEvent> {
        self.events.try_recv().ok()
    }

    pub fn stop(&self) {
        let _ = self.shutdown.send(());
    }
}

/// 启动端口监视后台线程
pub fn spawn_port_watcher(port: u16, interval_secs: u64) -> PortWatchHandle {
    let (event_tx, event_rx) = mpsc::channel();
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    std::thread::spawn(move || {
        let mut last_pid: Option<u32> = None;

        loop {
            if shutdown_rx.try_recv().is_ok() {
                return;
            }

            let current = port_map::find_pid_by_port(port).ok();

            let current_entry = current
                .as_ref()
                .and_then(|entries| entries.first())
                .map(|e| (e.pid, e.process_name.clone()));

            let current_pid = current_entry.as_ref().map(|(pid, _)| *pid);

            match (last_pid, current_pid) {
                (Some(_old_pid), None) => {
                    let _ = event_tx.send(PortEvent::Released { port });
                    notify::notify_port_change(port, "occupied", "released").ok();
                }
                (None, Some(new_pid)) => {
                    let name = current_entry.as_ref().map(|(_, n)| n.as_str()).unwrap_or("-");
                    let _ = event_tx.send(PortEvent::Occupied {
                        port,
                        pid: new_pid,
                        process_name: name.to_string(),
                    });
                    notify::notify_port_change(port, "released", &format!("occupied by {} (PID {})", name, new_pid)).ok();
                }
                (Some(old_pid), Some(new_pid)) if old_pid != new_pid => {
                    let name = current_entry.as_ref().map(|(_, n)| n.as_str()).unwrap_or("-");
                    let _ = event_tx.send(PortEvent::Released { port });
                    let _ = event_tx.send(PortEvent::Occupied {
                        port,
                        pid: new_pid,
                        process_name: name.to_string(),
                    });
                    notify::notify_port_change(
                        port,
                        &format!("occupied by PID {}", old_pid),
                        &format!("occupied by {} (PID {})", name, new_pid),
                    ).ok();
                }
                _ => {}
            }

            last_pid = current_pid;

            std::thread::sleep(Duration::from_secs(interval_secs));
        }
    });

    PortWatchHandle {
        port,
        events: event_rx,
        shutdown: shutdown_tx,
    }
}
