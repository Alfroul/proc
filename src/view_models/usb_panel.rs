use crossterm::event::{KeyCode, KeyEvent};

use crate::app_panel::{KeyResult, Panel, PanelContext};
use crate::eject::classify::HandleRisk;
use crate::eject::{HandleLock, RemovableDevice};

pub struct UsbPanel {
    pub devices: Vec<RemovableDevice>,
    pub device_cursor: usize,
    pub locks: Vec<(HandleLock, HandleRisk)>,
    pub lock_cursor: usize,
    pub status_message: Option<String>,
    pub focus_locks: bool,
}

impl Default for UsbPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbPanel {
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            device_cursor: 0,
            locks: Vec::new(),
            lock_cursor: 0,
            status_message: None,
            focus_locks: false,
        }
    }

    pub fn scan_devices(&mut self, processes: &[crate::collect::ProcessInfo]) {
        match crate::eject::scan_all_devices() {
            Ok(devices) => {
                self.devices = devices;
                self.device_cursor = 0;
                if !self.devices.is_empty() {
                    self.auto_select_first(processes);
                } else {
                    self.locks.clear();
                    self.status_message = Some("未检测到可移除设备".to_string());
                }
            }
            Err(_) => {
                self.devices.clear();
                self.status_message = Some("扫描设备失败".to_string());
            }
        }
    }

    fn auto_select_first(&mut self, processes: &[crate::collect::ProcessInfo]) {
        if let Some(dev) = self.devices.get(self.device_cursor) {
            let letter = dev.drive_letter;
            match crate::eject::scan_device_locks_with_processes(letter, processes) {
                Ok(locks) => {
                    let occupied = !locks.is_empty();
                    if let Some(dev) = self.devices.get_mut(self.device_cursor) {
                        dev.is_occupied = occupied;
                    }
                    if occupied {
                        self.locks = locks;
                    } else {
                        self.locks.clear();
                        self.status_message = Some(
                            "✅ 无占用进程，可以安全弹出 U 盘了（请手动在系统托盘或文件管理器中弹出）"
                                .to_string(),
                        );
                    }
                }
                Err(_) => {
                    if let Some(dev) = self.devices.get_mut(self.device_cursor) {
                        dev.is_occupied = false;
                    }
                }
            }
        }
    }

    pub fn refresh_device_list(&mut self) {
        if let Ok(mut new_devices) = crate::eject::scan_all_devices() {
            for new_dev in &mut new_devices {
                if let Some(old) = self
                    .devices
                    .iter()
                    .find(|d| d.drive_letter == new_dev.drive_letter)
                {
                    new_dev.is_occupied = old.is_occupied;
                }
            }
            self.devices = new_devices;
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        if self.focus_locks {
            let total = self.locks.len();
            if total == 0 {
                return;
            }
            let new = self.lock_cursor as i32 + delta;
            self.lock_cursor = if new < 0 {
                total - 1
            } else if new as usize >= total {
                0
            } else {
                new as usize
            };
        } else {
            let total = self.devices.len();
            if total == 0 {
                return;
            }
            let new = self.device_cursor as i32 + delta;
            self.device_cursor = if new < 0 {
                total - 1
            } else if new as usize >= total {
                0
            } else {
                new as usize
            };
        }
    }

    fn select_device(&mut self, processes: &[crate::collect::ProcessInfo]) -> Option<String> {
        let device = self.devices.get(self.device_cursor)?;
        let letter = device.drive_letter;
        match crate::eject::scan_device_locks_with_processes(letter, processes) {
            Ok(locks) => {
                let occupied = !locks.is_empty();
                if let Some(dev) = self.devices.get_mut(self.device_cursor) {
                    dev.is_occupied = occupied;
                }
                if occupied {
                    self.locks = locks;
                    self.focus_locks = true;
                    Some(format!("发现 {} 个锁定进程", self.locks.len()))
                } else {
                    self.locks.clear();
                    self.status_message = Some("✅ 无锁定进程，可以安全弹出".to_string());
                    Some(format!("弹出 {}", letter))
                }
            }
            Err(e) => Some(format!("扫描锁定失败: {}", e)),
        }
    }

    fn kill_safe(&mut self) -> Option<String> {
        if !self.focus_locks {
            return None;
        }
        let pid = self.locks.get(self.lock_cursor)?.0.pid;
        let name = self.locks.get(self.lock_cursor)?.0.process_name.clone();
        match crate::kill::kill_process(pid, false) {
            Ok(crate::kill::KillResult::Killed) => {
                self.locks.remove(self.lock_cursor);
                if self.lock_cursor >= self.locks.len() && self.lock_cursor > 0 {
                    self.lock_cursor -= 1;
                }
                if self.locks.is_empty() {
                    self.focus_locks = false;
                }
                Some(format!("进程 {} (PID {}) 已终止", name, pid))
            }
            Ok(crate::kill::KillResult::AlreadyGone) => {
                self.locks.remove(self.lock_cursor);
                Some("进程已不存在".to_string())
            }
            Ok(crate::kill::KillResult::AccessDenied) => {
                Some("权限不足 — 请以管理员身份重启 proc".to_string())
            }
            Ok(crate::kill::KillResult::Failed(e)) => Some(format!("终止失败: {}", e)),
            Err(e) => Some(format!("错误: {}", e)),
        }
    }

    fn refresh(&mut self, processes: &[crate::collect::ProcessInfo]) {
        self.scan_devices(processes);
        self.status_message = Some("已刷新设备列表".to_string());
    }

    fn wait_and_monitor(&mut self) -> Option<String> {
        let device = self.devices.get(self.device_cursor)?;
        let letter = device.drive_letter;
        self.status_message = Some(format!("等待 {} 盘插入...", letter));
        Some(format!("监控 {} 盘", letter))
    }

    fn toggle_focus(&mut self) {
        if !self.locks.is_empty() {
            self.focus_locks = !self.focus_locks;
        }
    }
}

impl Panel for UsbPanel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        let processes = ctx.cached_processes;
        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => {
                let _ = self.select_device(processes);
            }
            KeyCode::Char('k') => {
                let _ = self.kill_safe();
            }
            KeyCode::Char('r') => self.refresh(processes),
            KeyCode::Char('w') => {
                let _ = self.wait_and_monitor();
            }
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::Esc => {
                if self.focus_locks {
                    self.focus_locks = false;
                } else {
                    self.status_message = None;
                }
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    fn tick(&mut self, _ctx: &mut PanelContext) -> bool {
        self.refresh_device_list();
        false
    }

    fn cursor(&self) -> usize {
        if self.focus_locks {
            self.lock_cursor
        } else {
            self.device_cursor
        }
    }

    fn scroll(&self) -> usize {
        0
    }
}
