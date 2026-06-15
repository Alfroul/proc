use crossterm::event::{KeyCode, KeyEvent};

use crate::app_panel::{KeyResult, Panel, PanelContext};
use crate::docker::events::{self, DockerEvent, DockerEventReceiver};
use crate::docker::stats::ContainerStats;
use crate::docker::{ContainerInfo, DockerMonitor};

pub struct DockerPanel {
    pub monitor: Option<DockerMonitor>,
    pub containers: Vec<ContainerInfo>,
    pub cursor: usize,
    pub scroll: usize,
    pub connected: bool,
    pub status_message: Option<String>,
    pub event_receiver: Option<DockerEventReceiver>,
    pub events: Vec<DockerEvent>,
    pub detail: Option<ContainerInfo>,
    pub detail_stats: Option<ContainerStats>,
}

impl Default for DockerPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DockerPanel {
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
        }
    }

    pub fn refresh(&mut self) {
        if self.monitor.is_none() {
            match DockerMonitor::connect() {
                Ok(monitor) => {
                    self.monitor = Some(monitor);
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
            match monitor.list_containers(true) {
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
            match monitor.restart_container(&name) {
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
            match monitor.stop_container(&name) {
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
            let docker_client = monitor.docker();
            let receiver = events::spawn_event_watcher(docker_client);
            self.event_receiver = Some(receiver);
            self.status_message = Some("✅ 已开始监听容器事件".to_string());
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
                self.detail_stats = monitor.get_stats(&name).ok();
            }
        }
    }

    pub fn poll_events(&mut self) {
        let new_events: Vec<DockerEvent> = if let Some(ref receiver) = self.event_receiver {
            std::iter::from_fn(|| receiver.try_recv()).collect()
        } else {
            return;
        };
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
        self.refresh();
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
        false
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn scroll(&self) -> usize {
        self.scroll
    }
}
