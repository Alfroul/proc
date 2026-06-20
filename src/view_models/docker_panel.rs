use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};

use crate::app_panel::{KeyResult, Panel, PanelContext};
use crate::docker::events::{self, DockerEvent, DockerEventReceiver};
use crate::docker::snapshot_worker::DockerSnapshotWorker;
use crate::docker::stats::ContainerStats;
use crate::docker::{ContainerInfo, DockerMonitor};

pub struct DockerPanel {
    pub monitor: Option<Arc<Mutex<DockerMonitor>>>,
    pub containers: Vec<ContainerInfo>,
    pub cursor: usize,
    pub scroll: usize,
    pub connected: bool,
    pub status_message: Option<String>,
    pub event_receiver: Option<DockerEventReceiver>,
    pub events: Vec<DockerEvent>,
    pub detail: Option<ContainerInfo>,
    pub detail_stats: Option<ContainerStats>,
    /// 后台快照 worker(在 monitor 初始化时 spawn,持有 Arc::clone)。
    /// 字段必须留在 panel 上,drop 时 worker 才会退出。
    pub snapshot_worker: Option<DockerSnapshotWorker>,
}

impl Default for DockerPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DockerPanel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            monitor: None,
            containers: Vec::new(),
            cursor: 0,
            scroll: 0,
            connected: false,
            status_message: None,
            event_receiver: None,
            events: Vec::new(),
            detail: None,
            detail_stats: None,
            snapshot_worker: None,
        }
    }

    /// 同步刷新:首次连接时初始化 `Arc<Mutex<DockerMonitor>>` 并 spawn 后台
    /// snapshot worker;无论何时都立即同步拉一次容器列表(用户按 r 触发,
    /// 期望立即响应)。后续周期性更新由 worker 异步推送,经 `poll_events`
    /// 的 `try_recv` 应用。
    pub fn refresh(&mut self) {
        if self.monitor.is_none() {
            match DockerMonitor::connect() {
                Ok(monitor) => {
                    let monitor_arc = Arc::new(Mutex::new(monitor));
                    let worker = crate::docker::snapshot_worker::spawn(Arc::clone(&monitor_arc));
                    self.monitor = Some(monitor_arc);
                    self.snapshot_worker = Some(worker);
                    self.connected = true;
                }
                Err(e) => {
                    self.connected = false;
                    self.status_message = Some(format!("❌ {}", e));
                    return;
                }
            }
        }
        if let Some(ref monitor) = self.monitor {
            let result: crate::error::Result<_> = monitor
                .lock()
                .map_err(|_| crate::error::ProcError::docker("DockerMonitor mutex poisoned"))
                .and_then(|m| m.list_containers(true));
            match result {
                Ok(containers) => {
                    self.containers = containers;
                    self.connected = true;
                    if self
                        .status_message
                        .as_ref()
                        .is_none_or(|m| !m.starts_with('✅'))
                    {
                        self.status_message = None;
                    }
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 获取容器列表失败: {}", e));
                }
            }
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let total = self.containers.len();
        if total == 0 {
            return;
        }
        let new = self.cursor as i32 + delta;
        self.cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
    }

    fn restart_selected(&mut self) {
        let name = self.containers.get(self.cursor).map(|c| c.name.clone());
        if let Some(name) = name
            && let Some(ref monitor) = self.monitor
        {
            let result: crate::error::Result<()> = monitor
                .lock()
                .map_err(|_| crate::error::ProcError::docker("DockerMonitor mutex poisoned"))
                .and_then(|m| m.restart_container(&name));
            match result {
                Ok(()) => {
                    self.status_message = Some(format!("✅ 容器 {} 已重启", name));
                    self.refresh();
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 重启失败: {}", e));
                }
            }
        }
    }

    fn stop_selected(&mut self) {
        let name = self.containers.get(self.cursor).map(|c| c.name.clone());
        if let Some(name) = name
            && let Some(ref monitor) = self.monitor
        {
            let result: crate::error::Result<()> = monitor
                .lock()
                .map_err(|_| crate::error::ProcError::docker("DockerMonitor mutex poisoned"))
                .and_then(|m| m.stop_container(&name));
            match result {
                Ok(()) => {
                    self.status_message = Some(format!("✅ 容器 {} 已停止", name));
                    self.refresh();
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 停止失败: {}", e));
                }
            }
        }
    }

    pub fn start_watching(&mut self) {
        if let Some(ref monitor) = self.monitor {
            // 持锁期间 clone docker client(bollard::Docker 内部 Arc,clone 廉价),
            // 立即释放锁,让事件监听线程独立持有。
            let docker_client = monitor.lock().ok().map(|m| m.docker());
            if let Some(docker_client) = docker_client {
                let receiver = events::spawn_event_watcher(docker_client);
                self.event_receiver = Some(receiver);
                self.status_message = Some("✅ 已开始监听容器事件".to_string());
            } else {
                self.status_message = Some("❌ Docker mutex poisoned".to_string());
            }
        } else {
            self.status_message = Some("❌ Docker 未连接，请先刷新".to_string());
        }
    }

    fn show_detail(&mut self) {
        if self.detail.is_some() {
            self.detail = None;
            self.detail_stats = None;
            return;
        }
        let container = self.containers.get(self.cursor).cloned();
        if let Some(c) = container {
            let name = c.name.clone();
            self.detail = Some(c);
            if let Some(ref monitor) = self.monitor {
                self.detail_stats = monitor.lock().ok().and_then(|m| m.get_stats(&name).ok());
            }
        }
    }

    pub fn poll_events(&mut self) {
        // 1) 处理事件流(docker::events::spawn_event_watcher 推送)
        if let Some(ref receiver) = self.event_receiver {
            let new_events: Vec<DockerEvent> = std::iter::from_fn(|| receiver.try_recv()).collect();
            for event in new_events {
                let action = event.action.clone();
                let container_name = event
                    .container_name
                    .clone()
                    .unwrap_or_else(|| event.container_id.clone());
                self.events.insert(0, event);
                if self.events.len() > 100 {
                    self.events.truncate(100);
                }
                match action.as_str() {
                    "die" | "stop" => {
                        crate::monitor::notify::send_toast(
                            "Docker 容器停止",
                            &format!("容器 {} 已停止", container_name),
                        )
                        .ok();
                    }
                    "start" => {
                        crate::monitor::notify::send_toast(
                            "Docker 容器启动",
                            &format!("容器 {} 已启动", container_name),
                        )
                        .ok();
                    }
                    "health_status" => {
                        crate::monitor::notify::send_toast(
                            "Docker 健康状态变化",
                            &format!("容器 {} 健康状态变化", container_name),
                        )
                        .ok();
                    }
                    _ => {}
                }
            }
        }

        // 2) 应用后台 snapshot worker 推送的容器列表(每 ~5s 一份)。
        //    **不再**调 self.refresh() — 旧实现每秒同步 block_on list_containers
        //    是 DockerPanel 卡顿的根因。
        if let Some(ref worker) = self.snapshot_worker
            && let Some(snap) = worker.try_recv_latest()
        {
            match snap.result {
                Ok(containers) => {
                    self.containers = containers;
                    self.connected = true;
                    // 容器列表变短时 clamp cursor,避免越界。
                    let total = self.containers.len();
                    if total == 0 {
                        self.cursor = 0;
                    } else if self.cursor >= total {
                        self.cursor = total - 1;
                    }
                    if self
                        .status_message
                        .as_ref()
                        .is_none_or(|m| !m.starts_with('✅'))
                    {
                        self.status_message = None;
                    }
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 获取容器列表失败: {}", e));
                }
            }
        }
    }
}

impl Panel for DockerPanel {
    fn handle_key(&mut self, key: KeyEvent, _ctx: &mut PanelContext) -> KeyResult {
        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => self.show_detail(),
            KeyCode::Char('r') => self.restart_selected(),
            KeyCode::Char('s') => self.stop_selected(),
            KeyCode::Char('a') => self.start_watching(),
            KeyCode::Esc => {
                if self.detail.is_some() {
                    self.detail = None;
                    self.detail_stats = None;
                } else {
                    self.status_message = None;
                }
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    fn tick(&mut self, _ctx: &mut PanelContext) -> bool {
        self.poll_events();
        // 容器消失后（被删 / 停 → list 不再返回）cursor 必须收紧，避免越界渲染。
        let total = self.containers.len();
        if total == 0 {
            self.cursor = 0;
        } else if self.cursor >= total {
            self.cursor = total - 1;
        }
        false
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn scroll(&self) -> usize {
        self.scroll
    }
}
