use crossterm::event::{KeyCode, KeyEvent};

use crate::app_panel::{KeyResult, MonitorAddSubmenu, Panel, PanelContext};
use crate::monitor::port_watcher::{self, PortEvent, PortWatchHandle};
use crate::monitor::watchdog::{WatchdogEvent, WatchdogHandle};
use crate::monitor::{MonitorManager, MonitorStatus, MonitorTarget, RestartPolicy};

pub struct MonitorPanel {
    pub manager: MonitorManager,
    pub cursor: usize,
    pub add_submenu: Option<MonitorAddSubmenu>,
    pub watchdog_handles: Vec<WatchdogHandle>,
    pub port_handles: Vec<PortWatchHandle>,
}

impl Default for MonitorPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl MonitorPanel {
    pub fn new() -> Self {
        Self {
            manager: MonitorManager::new(),
            cursor: 0,
            add_submenu: None,
            watchdog_handles: Vec::new(),
            port_handles: Vec::new(),
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let total = self.manager.list_monitors().len();
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

    fn delete_selected(&mut self) -> Option<String> {
        let id = self
            .manager
            .list_monitors()
            .get(self.cursor)
            .map(|m| m.id)?;
        self.manager.remove_monitor(id).ok()?;
        if self.cursor >= self.manager.list_monitors().len() && self.cursor > 0 {
            self.cursor -= 1;
        }
        Some(format!("已删除监控 (ID: {})", id))
    }

    fn toggle_pause(&mut self) -> Option<String> {
        let monitors = self.manager.list_monitors();
        let monitor = monitors.get(self.cursor)?;
        let id = monitor.id;
        let is_running = monitor.status == MonitorStatus::Running;
        if let Some(entry) = self.manager.get_monitor_mut(id) {
            entry.status = if is_running {
                MonitorStatus::Paused
            } else {
                MonitorStatus::Running
            };
        }
        Some(if is_running { "已暂停" } else { "已恢复" }.to_string())
    }

    fn handle_submenu_input(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        let submenu = match &self.add_submenu {
            Some(s) => s.clone(),
            None => return KeyResult::Consumed,
        };

        match submenu {
            MonitorAddSubmenu::SelectType => match key.code {
                KeyCode::Char('1') => {
                    self.add_submenu = Some(MonitorAddSubmenu::EnterPid {
                        input: String::new(),
                    });
                }
                KeyCode::Char('2') => {
                    self.add_submenu = Some(MonitorAddSubmenu::EnterPort {
                        input: String::new(),
                    });
                }
                KeyCode::Char('3') => {
                    self.add_submenu = Some(MonitorAddSubmenu::EnterCommand {
                        cmd_input: String::new(),
                        args_input: String::new(),
                        cwd_input: String::new(),
                        retries_input: "5".to_string(),
                    });
                }
                KeyCode::Esc => {
                    self.add_submenu = None;
                }
                _ => {}
            },
            MonitorAddSubmenu::EnterPid { input } => match key.code {
                KeyCode::Enter => {
                    if let Ok(pid) = input.parse::<u32>() {
                        match self
                            .manager
                            .add_monitor(MonitorTarget::ByPid { pid }, RestartPolicy::NotifyOnly)
                        {
                            Ok(id) => {
                                self.manager.add_notification(format!(
                                    "已添加 PID {} 监控 (ID: {})",
                                    pid, id
                                ));
                                *ctx.status_message = Some(format!("已添加 PID {} 监控", pid));
                            }
                            Err(e) => {
                                tracing::warn!("添加监控失败: {}", e);
                                *ctx.status_message = Some(format!("添加监控失败: {}", e));
                            }
                        }
                    }
                    self.add_submenu = None;
                }
                KeyCode::Esc => {
                    self.add_submenu = None;
                }
                KeyCode::Backspace => {
                    if let Some(MonitorAddSubmenu::EnterPid { input }) = &mut self.add_submenu {
                        input.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(MonitorAddSubmenu::EnterPid { input }) = &mut self.add_submenu {
                        input.push(c);
                    }
                }
                _ => {}
            },
            MonitorAddSubmenu::EnterPort { input } => match key.code {
                KeyCode::Enter => {
                    if let Ok(port) = input.parse::<u16>() {
                        match self
                            .manager
                            .add_monitor(MonitorTarget::ByPort { port }, RestartPolicy::NotifyOnly)
                        {
                            Ok(id) => {
                                let handle = port_watcher::spawn_port_watcher(port, 5);
                                self.port_handles.push(handle);
                                self.manager.add_notification(format!(
                                    "已添加端口 {} 监控 (ID: {})",
                                    port, id
                                ));
                                *ctx.status_message = Some(format!("已添加端口 {} 监控", port));
                            }
                            Err(e) => {
                                tracing::warn!("添加监控失败: {}", e);
                                *ctx.status_message = Some(format!("添加监控失败: {}", e));
                            }
                        }
                    }
                    self.add_submenu = None;
                }
                KeyCode::Esc => {
                    self.add_submenu = None;
                }
                KeyCode::Backspace => {
                    if let Some(MonitorAddSubmenu::EnterPort { input }) = &mut self.add_submenu {
                        input.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(MonitorAddSubmenu::EnterPort { input }) = &mut self.add_submenu {
                        input.push(c);
                    }
                }
                _ => {}
            },
            MonitorAddSubmenu::EnterCommand { .. } => match key.code {
                KeyCode::Enter => {
                    if let Some(MonitorAddSubmenu::EnterCommand {
                        cmd_input,
                        args_input,
                        cwd_input,
                        retries_input,
                    }) = &self.add_submenu
                        && !cmd_input.is_empty()
                    {
                        let args: Vec<String> = if args_input.is_empty() {
                            Vec::new()
                        } else {
                            args_input
                                .split_whitespace()
                                .map(|s| s.to_string())
                                .collect()
                        };
                        let cwd = if cwd_input.is_empty() {
                            None
                        } else {
                            Some(std::path::PathBuf::from(cwd_input))
                        };
                        let max_retries = retries_input.parse::<u32>().unwrap_or(5);
                        match self.manager.add_monitor(
                            MonitorTarget::ByCommand {
                                cmd: cmd_input.clone(),
                                args: args.clone(),
                                cwd: cwd.clone(),
                            },
                            RestartPolicy::AutoRestart {
                                max_retries,
                                base_backoff: 1,
                                max_backoff: 30,
                            },
                        ) {
                            Ok(id) => {
                                self.manager
                                    .add_notification(format!("已添加命令监控 (ID: {})", id));
                                *ctx.status_message = Some(format!("已添加命令监控 (ID: {})", id));
                            }
                            Err(e) => {
                                tracing::warn!("添加监控失败: {}", e);
                                *ctx.status_message = Some(format!("添加监控失败: {}", e));
                            }
                        }
                    }
                    self.add_submenu = None;
                }
                KeyCode::Esc => {
                    self.add_submenu = None;
                }
                KeyCode::Backspace => {
                    if let Some(MonitorAddSubmenu::EnterCommand {
                        cmd_input,
                        args_input,
                        cwd_input,
                        retries_input,
                    }) = &mut self.add_submenu
                    {
                        if !retries_input.is_empty() {
                            retries_input.pop();
                        } else if !cwd_input.is_empty() {
                            cwd_input.pop();
                        } else if !args_input.is_empty() {
                            args_input.pop();
                        } else {
                            cmd_input.pop();
                        }
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(MonitorAddSubmenu::EnterCommand { cmd_input, .. }) =
                        &mut self.add_submenu
                    {
                        cmd_input.push(c);
                    }
                }
                KeyCode::Tab => {
                    // Tab between fields (simplified - real impl would cycle focus)
                }
                _ => {}
            },
        }
        KeyResult::Consumed
    }

    pub fn poll_events(&mut self) {
        // Poll watchdog events
        for handle in &self.watchdog_handles {
            while let Some(event) = handle.try_recv() {
                match event {
                    WatchdogEvent::Crashed {
                        monitor_id,
                        exit_code,
                        attempt,
                        restarting,
                    } => {
                        self.manager.add_notification(format!(
                            "监控 {} 崩溃 (code: {:?}, attempt: {}, restarting: {})",
                            monitor_id, exit_code, attempt, restarting
                        ));
                    }
                    WatchdogEvent::Started { monitor_id, pid } => {
                        self.manager
                            .add_notification(format!("监控 {} 启动 PID {}", monitor_id, pid));
                    }
                    WatchdogEvent::Stopped { monitor_id, reason } => {
                        self.manager
                            .add_notification(format!("监控 {} 停止: {}", monitor_id, reason));
                    }
                    WatchdogEvent::Running { monitor_id, pid } => {
                        self.manager
                            .add_notification(format!("监控 {} 运行中 PID {}", monitor_id, pid));
                    }
                }
            }
        }
        // Poll port watcher events
        for handle in &self.port_handles {
            while let Some(event) = handle.try_recv() {
                match event {
                    PortEvent::Occupied {
                        port,
                        pid,
                        process_name,
                    } => {
                        self.manager.add_notification(format!(
                            "端口 {} 被 {} (PID {}) 占用",
                            port, process_name, pid
                        ));
                    }
                    PortEvent::Released { port } => {
                        self.manager
                            .add_notification(format!("端口 {} 已释放", port));
                    }
                }
            }
        }
    }
}

impl Panel for MonitorPanel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        // Submenu input takes priority
        if self.add_submenu.is_some() {
            return self.handle_submenu_input(key, ctx);
        }

        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('a') => {
                self.add_submenu = Some(MonitorAddSubmenu::SelectType);
            }
            KeyCode::Char('d') => {
                if let Some(msg) = self.delete_selected() {
                    *ctx.status_message = Some(msg);
                }
            }
            KeyCode::Char('s') => {
                if let Some(msg) = self.toggle_pause() {
                    *ctx.status_message = Some(msg);
                }
            }
            KeyCode::Esc => {
                *ctx.status_message = None;
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
        0
    }
}
