use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};

const SPARKLINE_LEN: usize = 30;
const MAX_TRACKED: usize = 50;
const MAX_OP_HISTORY: usize = 100;

// Re-export types that moved to app_panel
pub use crate::app_panel::{AppGroupSortField, AppMode, KillRequest, MonitorAddSubmenu, OpRecord};

use crate::alert::AlertManager;
use crate::app_panel::{KeyResult, Panel, PanelContext};
use crate::classify;
use crate::collect::{
    HEAVY_REFRESH_INTERVAL, ProcessInfo, ProcessViewMode, REFRESH_INTERVAL, SortField,
    SystemSnapshot,
};
use crate::docker::ContainerInfo;
use crate::eject::classify::HandleRisk;
use crate::eject::device::RemovableDevice;
use crate::eject::locks::HandleLock;
use crate::error::Result;
use crate::port_map::{self, NetworkViewMode, PortEntry};
use crate::record::Player;
use crate::security::{BackgroundScorer, SecurityScore};
use crate::tree::TreeNode;
use crate::view_models::DockerPanel;
use crate::view_models::MonitorPanel;
use crate::view_models::PortPanel;
use crate::view_models::ProcessPanel;
use crate::view_models::UsbPanel;

#[derive(Debug, Clone, Copy)]
pub enum ReplaySpeed {
    Half,
    Normal,
    Double,
    Quad,
}

impl ReplaySpeed {
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Half => 0.5,
            Self::Normal => 1.0,
            Self::Double => 2.0,
            Self::Quad => 4.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimelineState {
    pub current_frame: usize,
    pub total_frames: usize,
    pub speed: ReplaySpeed,
    pub playing: bool,
    pub half_tick: u32,
}

pub struct App {
    pub mode: AppMode,
    pub snapshot: SystemSnapshot,
    pub cached_processes: Vec<ProcessInfo>,
    pub should_quit: bool,
    pub last_refresh: Instant,
    pub last_heavy_refresh: Instant,
    pub pending_redraw: bool,

    // Panels
    pub process_panel: ProcessPanel,
    pub port_panel: PortPanel,
    pub usb_panel: UsbPanel,
    pub monitor_panel: MonitorPanel,
    pub docker_panel: DockerPanel,

    // Global state
    pub detail_process: Option<ProcessInfo>,
    pub detail_port_info: String,
    pub status_message: Option<String>,
    pub kill_confirm: bool,
    pub pending_kill: Option<KillRequest>,

    // History tracking
    pub proc_history: HashMap<u32, ProcHistory>,
    pub global_cpu_history: VecDeque<u64>,
    pub global_mem_history: VecDeque<u64>,
    pub op_history: VecDeque<OpRecord>,

    // Security scoring (background thread)
    pub background_scorer: BackgroundScorer,
    pub security_scores: HashMap<u32, SecurityScore>,
    scoring_pending: bool,

    // Cached sorted process list
    cached_sorted: Vec<(usize, classify::ProcessClass)>,
    data_dirty: bool,

    // Alert
    pub alert_manager: AlertManager,
    pub alert_popup_open: bool,
    pub alert_scroll: usize,

    // Help page
    pub help_scroll: usize,

    // Replay state
    pub replay_player: Option<Player>,
    pub timeline_state: Option<TimelineState>,

    // Recording
    recording_wanted: bool,
    recording_elapsed_secs: u64,

    // Throttle
    pub throttle_info: Option<crate::throttle::ThrottleInfo>,
    pub throttle_reason: crate::throttle::ThrottleReason,

    // Per-process disk speed
    prev_process_disk: HashMap<u32, (u64, u64)>,
    prev_process_disk_time: Instant,

    // Platform
    pub is_windows: bool,
}

pub struct ProcHistory {
    pub cpu: VecDeque<u64>,
}

impl App {
    pub fn new() -> Result<Self> {
        crate::tui::theme::init_persisted_theme();

        let mut snapshot = SystemSnapshot::new()?;
        let _ = snapshot.refresh_heavy_incremental();
        let processes = snapshot.cached_processes_vec();
        let (_, mem_total) = snapshot.memory_usage();
        let port_entries = port_map::scan_ports().unwrap_or_default();

        let mut process_panel = ProcessPanel::new(&processes);
        process_panel.init_tree(&processes, mem_total);

        let mut port_panel = PortPanel::new();
        port_panel.port_entries = port_entries;

        let is_windows = cfg!(target_os = "windows");
        let status_message = if is_windows {
            None
        } else {
            Some(
                "Linux/macOS 模式：以下功能已禁用 — 安全评分签名验证、降频检测、U盘句柄枚举、Toast 通知、EStats 带宽。详见 README。"
                    .to_string(),
            )
        };

        Ok(Self {
            mode: AppMode::ProcessList,
            snapshot,
            cached_processes: processes.clone(),
            should_quit: false,
            last_refresh: Instant::now(),
            last_heavy_refresh: Instant::now() - HEAVY_REFRESH_INTERVAL,
            pending_redraw: true,
            process_panel,
            port_panel,
            usb_panel: UsbPanel::new(),
            monitor_panel: MonitorPanel::new(),
            docker_panel: DockerPanel::new(),
            detail_process: None,
            detail_port_info: String::new(),
            status_message,
            kill_confirm: false,
            pending_kill: None,
            proc_history: HashMap::new(),
            global_cpu_history: VecDeque::new(),
            global_mem_history: VecDeque::new(),
            op_history: VecDeque::new(),
            background_scorer: BackgroundScorer::new(),
            security_scores: HashMap::new(),
            scoring_pending: false,
            cached_sorted: Vec::new(),
            data_dirty: true,
            alert_manager: AlertManager::load_or_default(),
            alert_popup_open: false,
            alert_scroll: 0,
            help_scroll: 0,
            replay_player: None,
            timeline_state: None,
            recording_wanted: false,
            recording_elapsed_secs: 0,
            throttle_info: None,
            throttle_reason: crate::throttle::ThrottleReason::None,
            prev_process_disk: HashMap::new(),
            prev_process_disk_time: Instant::now(),
            is_windows,
        })
    }

    // --- Recording ---

    pub fn recording_wanted(&self) -> bool {
        self.recording_wanted
    }
    pub fn set_recording_wanted(&mut self, wanted: bool) {
        self.recording_wanted = wanted;
    }
    pub fn set_recording_elapsed(&mut self, secs: u64) {
        self.recording_elapsed_secs = secs;
    }
    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some(msg);
        self.pending_redraw = true;
    }
    pub fn is_recording(&self) -> bool {
        self.recording_wanted
    }
    pub fn recording_elapsed(&self) -> u64 {
        self.recording_elapsed_secs
    }

    fn toggle_recording(&mut self) {
        self.recording_wanted = !self.recording_wanted;
    }

    pub fn replay_frame_mode(&self) -> AppMode {
        let frame_index = self
            .timeline_state
            .as_ref()
            .map(|ts| ts.current_frame)
            .unwrap_or(0);
        if let Some(ref player) = self.replay_player
            && let Some(frame) = player.frame_at(frame_index)
        {
            return match frame.mode.as_str() {
                "ProcessTree" | "ProcessList" => AppMode::ProcessList,
                "PortMap" => AppMode::PortMap,
                "UsbAssistant" => AppMode::UsbAssistant,
                "MonitorPanel" => AppMode::MonitorPanel,
                "DockerPanel" => AppMode::DockerPanel,
                _ => AppMode::ProcessList,
            };
        }
        AppMode::ProcessList
    }

    pub fn start_replay(&mut self, player: Player) {
        let total = player.total_frames();
        self.replay_player = Some(player);
        self.timeline_state = Some(TimelineState {
            current_frame: 0,
            total_frames: total,
            speed: ReplaySpeed::Normal,
            playing: false,
            half_tick: 0,
        });
        self.mode = AppMode::Replay;
        self.replay_load_current_frame();
    }

    // --- Key dispatch ---

    fn try_handle_tab_switch(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('1') => {
                self.switch_mode(AppMode::ProcessList);
                true
            }
            KeyCode::Char('2') => {
                self.switch_mode(AppMode::ProcessList);
                self.process_panel.process_view_mode = ProcessViewMode::Tree;
                self.process_panel.cursor_index = 0;
                self.process_panel.scroll_offset = 0;
                self.process_panel.tree_cursor = 0;
                self.process_panel.tree_scroll = 0;
                self.status_message = Some("视图: 进程树".to_string());
                true
            }
            KeyCode::Char('3') => {
                self.switch_mode(AppMode::PortMap);
                true
            }
            KeyCode::Char('4') => {
                self.switch_mode(AppMode::UsbAssistant);
                true
            }
            KeyCode::Char('5') => {
                self.switch_mode(AppMode::MonitorPanel);
                true
            }
            KeyCode::Char('6') => {
                self.switch_mode(AppMode::DockerPanel);
                true
            }
            KeyCode::Char('t') => {
                crate::tui::theme::cycle_theme();
                true
            }
            KeyCode::Char('A') => {
                self.alert_popup_open = !self.alert_popup_open;
                self.alert_scroll = 0;
                true
            }
            KeyCode::Char('?') => {
                self.mode = AppMode::Help;
                self.help_scroll = 0;
                true
            }
            _ => false,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.pending_redraw = true;

        // Global kill confirm dialog
        if self.kill_confirm {
            self.handle_kill_confirm(key);
            return;
        }

        // Global recording toggle
        if key.code == KeyCode::Char('R') {
            self.toggle_recording();
            return;
        }

        // Global tab switching (only when no search/submenu active)
        let any_search = self.process_panel.search.is_active()
            || self.process_panel.tree_search.is_active()
            || self.process_panel.app_group_search.is_active()
            || self.port_panel.port_search.is_active();
        if !any_search
            && !self.kill_confirm
            && self.monitor_panel.add_submenu.is_none()
            && self.try_handle_tab_switch(key)
        {
            return;
        }

        // Alert popup
        if self.alert_popup_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('A') => {
                    self.alert_popup_open = false;
                }
                KeyCode::Up => {
                    self.alert_scroll = self.alert_scroll.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.alert_scroll += 1;
                }
                _ => {}
            }
            return;
        }

        // Help page — handle navigation/scroll directly, Esc/?/q exits
        if self.mode == AppMode::Help {
            self.handle_help_key(key);
            return;
        }

        // Dispatch to panels — build context once before the match
        let result = {
            let mut ctx = PanelContext {
                snapshot: &self.snapshot,
                cached_processes: &self.cached_processes,
                cached_sorted: &self.cached_sorted,
                security_scores: &self.security_scores,
                status_message: &mut self.status_message,
                detail_process: &mut self.detail_process,
                pending_kill: &mut self.pending_kill,
                data_dirty: &mut self.data_dirty,
                pending_redraw: &mut self.pending_redraw,
                alert_manager: &mut self.alert_manager,
                op_history: &mut self.op_history,
            };
            match self.mode {
                AppMode::ProcessList => self.process_panel.handle_key(key, &mut ctx),
                AppMode::ProcessDetail => {
                    // Can't delegate to panel — handle directly
                    let _ = ctx;
                    self.handle_detail_key(key);
                    return;
                }
                AppMode::PortMap => self.port_panel.handle_key(key, &mut ctx),
                AppMode::UsbAssistant => self.usb_panel.handle_key(key, &mut ctx),
                AppMode::MonitorPanel => self.monitor_panel.handle_key(key, &mut ctx),
                AppMode::DockerPanel => self.docker_panel.handle_key(key, &mut ctx),
                AppMode::Replay => {
                    let _ = ctx;
                    self.handle_replay_key(key);
                    return;
                }
                AppMode::Help => KeyResult::Ignored,
            }
        };

        // Handle KeyResult
        match result {
            KeyResult::Quit => self.should_quit = true,
            KeyResult::SwitchMode(mode) => self.switch_mode(mode),
            KeyResult::Consumed | KeyResult::Ignored | KeyResult::ToggleRecording => {}
        }

        // If pending_kill was set by a panel, enable kill_confirm
        if self.pending_kill.is_some() {
            self.kill_confirm = true;
        }
    }

    fn switch_mode(&mut self, mode: AppMode) {
        if mode == AppMode::ProcessList {
            self.process_panel.process_view_mode = ProcessViewMode::List;
            self.process_panel.cursor_index = 0;
            self.process_panel.scroll_offset = 0;
        }
        if mode == AppMode::UsbAssistant && self.mode != AppMode::UsbAssistant {
            self.usb_panel.scan_devices(&self.cached_processes);
        }
        if mode == AppMode::DockerPanel && self.mode != AppMode::DockerPanel {
            self.docker_panel.refresh();
            if self.docker_panel.connected && self.docker_panel.event_receiver.is_none() {
                self.docker_panel.start_watching();
            }
        }
        self.mode = mode;
        self.process_panel.search.clear();
        self.status_message = None;
        self.data_dirty = true;
    }

    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.switch_mode(AppMode::ProcessList);
            }
            KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                self.help_scroll = self.help_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.help_scroll = self.help_scroll.saturating_add(10);
            }
            KeyCode::Home => {
                self.help_scroll = 0;
            }
            KeyCode::End => {
                self.help_scroll = usize::MAX / 2;
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => {
                self.process_panel.process_view_mode = ProcessViewMode::List;
                self.mode = AppMode::ProcessList;
            }
            KeyCode::Char('k') => {
                if let Some(ref proc) = self.detail_process {
                    let result = crate::kill::kill_process(proc.pid, false);
                    match result {
                        Ok(crate::kill::KillResult::Killed) => {
                            let msg = format!("终止 {} (PID {})", proc.name, proc.pid);
                            self.status_message = Some(format!("{} 已终止", msg));
                            self.record_op(msg);
                            self.mode = AppMode::ProcessList;
                        }
                        Ok(crate::kill::KillResult::AlreadyGone) => {
                            self.status_message = Some("进程已不存在".to_string());
                            self.mode = AppMode::ProcessList;
                        }
                        Ok(crate::kill::KillResult::AccessDenied) => {
                            self.status_message = Some("权限不足，无法终止进程".to_string());
                        }
                        Ok(crate::kill::KillResult::Failed(e)) => {
                            self.status_message = Some(format!("终止失败: {}", e));
                        }
                        Err(e) => {
                            self.status_message = Some(format!("错误: {}", e));
                        }
                    }
                }
            }
            KeyCode::Char('w') => {
                if let Some(ref proc) = self.detail_process {
                    let pid = proc.pid;
                    match self.monitor_panel.manager.add_monitor(
                        crate::monitor::MonitorTarget::ByPid { pid },
                        crate::monitor::RestartPolicy::NotifyOnly,
                    ) {
                        Ok(monitor_id) => {
                            self.monitor_panel.manager.add_notification(format!(
                                "已添加 PID {} 监控 (ID: {})",
                                pid, monitor_id
                            ));
                            self.status_message =
                                Some(format!("已添加 PID {} 监控 (ID: {})", pid, monitor_id));
                        }
                        Err(e) => {
                            tracing::warn!("添加监控失败: {}", e);
                            self.status_message = Some(format!("添加监控失败: {}", e));
                        }
                    }
                }
            }
            KeyCode::Char('c') => {
                if let Some(ref proc) = self.detail_process {
                    let info = crate::tree::format_process_info(proc);
                    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&info)) {
                        Ok(()) => {
                            self.status_message =
                                Some(format!("已复制进程信息到剪贴板 ({} bytes)", info.len()));
                        }
                        Err(e) => {
                            tracing::warn!("剪贴板复制失败: {}", e);
                            self.status_message = Some(format!("复制失败: {}", e));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_replay_key(&mut self, key: KeyEvent) {
        let Some(ref mut ts) = self.timeline_state else {
            return;
        };
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char(' ') => {
                ts.playing = !ts.playing;
            }
            KeyCode::Left => {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT)
                {
                    ts.current_frame = ts.current_frame.saturating_sub(10);
                } else {
                    ts.current_frame = ts.current_frame.saturating_sub(1);
                }
                ts.playing = false;
                self.replay_load_current_frame();
            }
            KeyCode::Right => {
                if key
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT)
                {
                    ts.current_frame =
                        (ts.current_frame + 10).min(ts.total_frames.saturating_sub(1));
                } else {
                    ts.current_frame =
                        (ts.current_frame + 1).min(ts.total_frames.saturating_sub(1));
                }
                ts.playing = false;
                self.replay_load_current_frame();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                ts.speed = match ts.speed {
                    ReplaySpeed::Half => ReplaySpeed::Normal,
                    ReplaySpeed::Normal => ReplaySpeed::Double,
                    ReplaySpeed::Double => ReplaySpeed::Quad,
                    ReplaySpeed::Quad => ReplaySpeed::Quad,
                };
            }
            KeyCode::Char('-') => {
                ts.speed = match ts.speed {
                    ReplaySpeed::Half => ReplaySpeed::Half,
                    ReplaySpeed::Normal => ReplaySpeed::Half,
                    ReplaySpeed::Double => ReplaySpeed::Normal,
                    ReplaySpeed::Quad => ReplaySpeed::Double,
                };
            }
            KeyCode::Home => {
                ts.current_frame = 0;
                ts.playing = false;
                self.replay_load_current_frame();
            }
            KeyCode::End => {
                ts.current_frame = ts.total_frames.saturating_sub(1);
                ts.playing = false;
                self.replay_load_current_frame();
            }
            _ => {}
        }
    }

    fn replay_load_current_frame(&mut self) {
        let frame_index = self
            .timeline_state
            .as_ref()
            .map(|ts| ts.current_frame)
            .unwrap_or(0);
        // Clone the frame so we release the immutable borrow on `self.replay_player`
        // before we start mutating panel state below.
        let Some(frame) = self
            .replay_player
            .as_ref()
            .and_then(|p| p.frame_at(frame_index).cloned())
        else {
            return;
        };

        self.restore_replay_panel_data(&frame);
        self.restore_replay_metrics(&frame);
        self.restore_replay_view_mode(&frame);
        self.restore_replay_nav(frame.nav);

        self.data_dirty = true;
    }

    fn restore_replay_panel_data(&mut self, frame: &crate::record::frame::UiFrame) {
        self.cached_processes = frame.processes.iter().map(ProcessInfo::from).collect();
        self.process_panel.tree_nodes = frame.tree_nodes.iter().map(TreeNode::from).collect();
        self.port_panel.port_entries = frame.port_entries.iter().map(PortEntry::from).collect();
        self.port_panel.port_view_mode = NetworkViewMode::from_frame_code(frame.port_view_mode);
        self.usb_panel.devices = frame
            .usb_devices
            .iter()
            .map(RemovableDevice::from)
            .collect();
        self.usb_panel.locks = frame
            .usb_locks
            .iter()
            .map(|l| (HandleLock::from(l), HandleRisk::from(l)))
            .collect();
        self.docker_panel.containers = frame
            .docker_containers
            .iter()
            .map(ContainerInfo::from)
            .collect();

        // Last MAX_OP_HISTORY ops in chronological order.
        let start = frame.ops.len().saturating_sub(MAX_OP_HISTORY);
        self.op_history = frame.ops[start..].iter().map(OpRecord::from).collect();
        self.status_message = frame.status_message.clone();
    }

    fn restore_replay_metrics(&mut self, frame: &crate::record::frame::UiFrame) {
        self.snapshot.set_replay_metrics(
            frame.cpu_usage,
            frame.memory_used,
            frame.memory_total,
            frame.net_down,
            frame.net_up,
        );
        self.global_cpu_history = frame.cpu_history.iter().copied().collect();
        self.global_mem_history = frame.mem_history.iter().copied().collect();
    }

    fn restore_replay_view_mode(&mut self, frame: &crate::record::frame::UiFrame) {
        self.process_panel.process_view_mode = match frame.process_view_mode {
            1 => ProcessViewMode::Tree,
            2 => ProcessViewMode::AppGroup,
            _ => ProcessViewMode::List,
        };

        if self.process_panel.process_view_mode == ProcessViewMode::AppGroup {
            self.process_panel.app_groups = crate::app_group::compute_groups(
                &self.cached_processes,
                &mut self.process_panel.version_info_cache,
            );
            self.process_panel.app_group_sort_groups();
        }
    }

    fn restore_replay_nav(&mut self, nav: crate::record::frame::FrameNav) {
        self.process_panel.cursor_index = nav.cursor;
        self.process_panel.scroll_offset = nav.scroll;
        self.process_panel.selected_pids = nav.selected.into_iter().collect();
        self.process_panel.tree_cursor = nav.tree_cursor;
        self.process_panel.tree_scroll = nav.tree_scroll;
        self.process_panel.tree_selected_pids = nav.tree_selected.into_iter().collect();
        self.port_panel.port_cursor = nav.port_cursor;
        self.port_panel.port_scroll = nav.port_scroll;
        self.port_panel.port_process_cursor = nav.port_process_cursor;
        self.port_panel.port_process_scroll = nav.port_process_scroll;
        self.port_panel.port_remote_cursor = nav.port_remote_cursor;
        self.port_panel.port_remote_scroll = nav.port_remote_scroll;
        self.usb_panel.device_cursor = nav.usb_device_cursor;
        self.monitor_panel.cursor = nav.monitor_cursor;
        self.docker_panel.cursor = nav.docker_cursor;
        self.docker_panel.scroll = nav.docker_scroll;
    }

    fn replay_tick(&mut self) {
        let at_end = {
            let Some(ts) = self.timeline_state.as_mut() else {
                return;
            };
            if !ts.playing || ts.total_frames == 0 {
                return;
            }
            // Compute frame step for this tick based on playback speed.
            // Half speed steps every other tick; the rest advance by N frames per tick.
            let step = match ts.speed {
                ReplaySpeed::Half => {
                    ts.half_tick = (ts.half_tick + 1) % 2;
                    usize::from(ts.half_tick == 0)
                }
                ReplaySpeed::Normal => 1,
                ReplaySpeed::Double => 2,
                ReplaySpeed::Quad => 4,
            };
            if step == 0 {
                return;
            }
            let last = ts.total_frames.saturating_sub(1);
            ts.current_frame = (ts.current_frame + step).min(last);
            ts.current_frame >= last
        };

        self.replay_load_current_frame();

        if at_end && let Some(ts) = self.timeline_state.as_mut() {
            ts.playing = false;
        }
    }

    fn handle_kill_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(req) = self.pending_kill.take() {
                    let pid_to_name: HashMap<u32, String> = self
                        .cached_processes
                        .iter()
                        .map(|p| (p.pid, p.name.clone()))
                        .collect();
                    let mut results = Vec::new();
                    for pid in req.pids {
                        let name = pid_to_name.get(&pid).map(|s| s.as_str()).unwrap_or("?");
                        match crate::kill::kill_process(pid, req.force) {
                            Ok(crate::kill::KillResult::Killed) => {
                                results.push(format!("终止 {} (PID {})", name, pid))
                            }
                            Ok(crate::kill::KillResult::AlreadyGone) => {
                                results.push(format!("{} (PID {}) 已不存在", name, pid))
                            }
                            Ok(crate::kill::KillResult::AccessDenied) => {
                                results.push(format!("{} (PID {}) 权限不足", name, pid))
                            }
                            Ok(crate::kill::KillResult::Failed(e)) => {
                                results.push(format!("{} (PID {}) 失败: {}", name, pid, e))
                            }
                            Err(e) => results.push(format!("{} (PID {}) 错误: {}", name, pid, e)),
                        }
                    }
                    self.status_message = Some(results.join("; "));
                    self.record_op(results.join("; "));
                    self.process_panel.selected_pids.clear();
                    self.process_panel.tree_selected_pids.clear();
                    if let Err(e) = self.snapshot.refresh() {
                        tracing::warn!("刷新进程列表失败: {}", e);
                    }
                    self.process_panel
                        .refresh_tree(&self.cached_processes, self.snapshot.memory_usage().1);
                }
                self.kill_confirm = false;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.pending_kill = None;
                self.kill_confirm = false;
            }
            _ => {}
        }
    }

    fn record_op(&mut self, message: String) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let offset_secs = crate::local_offset_hours() * 3600;
        let local_secs = (secs as i64 + offset_secs) as u64;
        let (_, month, day) = crate::epoch_secs_to_ymd(local_secs);
        let h = ((local_secs / 3600) % 24) as u8;
        let m = ((local_secs / 60) % 60) as u8;
        self.op_history.push_back(OpRecord {
            time: format!("{:02}-{:02} {:02}:{:02}", month, day, h, m),
            message,
        });
        if self.op_history.len() > MAX_OP_HISTORY {
            self.op_history.pop_front();
        }
    }

    // --- Mouse handling ---

    pub fn handle_mouse(&mut self, event: MouseEvent) {
        self.pending_redraw = true;
        if self.mode == AppMode::Replay {
            return;
        }

        let Ok((term_width, _)) = crossterm::terminal::size() else {
            return;
        };

        let rec_w: u16 = 12;
        let rec_start = term_width.saturating_sub(rec_w + 1);
        if event.row < 3 && event.column >= rec_start {
            self.toggle_recording();
            return;
        }

        if self.mode != AppMode::ProcessList
            || self.process_panel.process_view_mode != ProcessViewMode::List
        {
            return;
        }

        let table_right = term_width.saturating_sub(60);
        if event.row < 3 || event.column >= table_right {
            return;
        }

        let data_row = event.row as isize - 4;
        if data_row < 0 {
            return;
        }
        let clicked_index = data_row as usize + self.process_panel.scroll_offset;

        use crossterm::event::{MouseButton, MouseEventKind};
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if clicked_index < self.cached_sorted.len() => {
                self.process_panel.cursor_index = clicked_index;
                // Toggle select
                if let Some((idx, _)) = self.cached_sorted.get(clicked_index) {
                    let pid = self.cached_processes[*idx].pid;
                    if self.process_panel.selected_pids.contains(&pid) {
                        self.process_panel.selected_pids.remove(&pid);
                    } else {
                        self.process_panel.selected_pids.insert(pid);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn handle_scroll(&mut self, lines: i32) {
        self.pending_redraw = true;
        match self.mode {
            AppMode::ProcessList | AppMode::ProcessDetail => {
                if self.process_panel.process_view_mode == ProcessViewMode::Tree {
                    self.process_panel.tree_move_cursor(lines);
                } else if self.process_panel.process_view_mode == ProcessViewMode::AppGroup {
                    self.process_panel.app_group_move_cursor(lines);
                } else {
                    self.process_panel.move_cursor(lines, &self.cached_sorted);
                }
            }
            AppMode::PortMap => {
                let total = self.port_panel.visible_port_count();
                if total == 0 {
                    return;
                }
                let new = self.port_panel.port_cursor as i32 + lines;
                self.port_panel.port_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::DockerPanel => {
                let total = self.docker_panel.containers.len();
                if total == 0 {
                    return;
                }
                let new = self.docker_panel.cursor as i32 + lines;
                self.docker_panel.cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::UsbAssistant => {
                let total = self.usb_panel.devices.len();
                if total == 0 {
                    return;
                }
                let new = self.usb_panel.device_cursor as i32 + lines;
                self.usb_panel.device_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::MonitorPanel => {
                let total = self.monitor_panel.manager.list_monitors().len();
                if total == 0 {
                    return;
                }
                let new = self.monitor_panel.cursor as i32 + lines;
                self.monitor_panel.cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            _ => {}
        }
    }

    // --- Public accessors for TUI rendering ---

    pub fn get_filtered_sorted_processes(&self) -> &[(usize, classify::ProcessClass)] {
        &self.cached_sorted
    }

    pub fn get_sorted_process(&self, sorted_idx: usize) -> Option<&ProcessInfo> {
        self.cached_sorted
            .get(sorted_idx)
            .map(|(i, _)| &self.cached_processes[*i])
    }

    pub fn get_sorted_process_class(&self, sorted_idx: usize) -> Option<classify::ProcessClass> {
        self.cached_sorted.get(sorted_idx).map(|(_, c)| *c)
    }

    pub fn get_selected_pids(&self) -> Vec<u32> {
        self.process_panel.get_selected_pids()
    }

    // --- Tick ---

    pub fn tick(&mut self) -> bool {
        let mut needs_draw = self.data_dirty;

        // Replay mode
        if self.mode == AppMode::Replay {
            return self.tick_replay();
        }

        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            self.last_refresh = Instant::now();
            let had_heavy = self.tick_light_refresh();
            self.tick_throttle_check();
            self.tick_history_sample(had_heavy);
            self.tick_alert_evaluate();
            self.tick_panels();
            self.tick_usb_monitor_docker();
            needs_draw = true;
        }

        if self.data_dirty {
            self.rebuild_sorted_cache();
            needs_draw = true;
        }

        if self.port_panel.port_filter_dirty {
            self.port_panel.rebuild_port_filters();
            needs_draw = true;
        }

        self.clamp_cursors();
        needs_draw
    }

    fn tick_replay(&mut self) -> bool {
        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            self.last_refresh = Instant::now();
            self.replay_tick();
        }
        if self.data_dirty {
            self.rebuild_sorted_cache();
        }
        true
    }

    /// Refresh light snapshot + (optionally) heavy refresh; dispatch scoring request
    /// and poll previous scoring results. Returns true if heavy refresh happened this tick.
    fn tick_light_refresh(&mut self) -> bool {
        self.snapshot.refresh_light();

        let need_heavy = self.last_heavy_refresh.elapsed() >= HEAVY_REFRESH_INTERVAL;
        if need_heavy {
            self.last_heavy_refresh = Instant::now();
            match self.snapshot.refresh_heavy_incremental() {
                Ok(true) => {
                    self.cached_processes = self.snapshot.cached_processes_vec();
                    self.update_disk_speeds();

                    let alive_pids: HashSet<u32> =
                        self.cached_processes.iter().map(|p| p.pid).collect();
                    self.security_scores
                        .retain(|pid, _| alive_pids.contains(pid));

                    if let Some(ref mut detail) = self.detail_process {
                        let pid = detail.pid;
                        if let Some(latest) = self.cached_processes.iter().find(|p| p.pid == pid) {
                            *detail = latest.clone();
                        } else {
                            self.detail_process = None;
                        }
                    }

                    self.data_dirty = true;

                    if !self.scoring_pending {
                        let procs = std::sync::Arc::new(self.cached_processes.clone());
                        let ports = std::sync::Arc::new(self.port_panel.port_entries.clone());
                        self.background_scorer.request(procs, ports);
                        self.scoring_pending = true;
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("刷新进程列表失败: {}", e),
            }
        }

        if let Some(scores) = self.background_scorer.poll_results() {
            self.security_scores.extend(scores);
            self.scoring_pending = false;
        }

        need_heavy
    }

    fn update_disk_speeds(&mut self) {
        let now = Instant::now();
        let elapsed = now
            .duration_since(self.prev_process_disk_time)
            .as_secs_f64()
            .max(0.001);
        for proc in &mut self.cached_processes {
            if let Some(&(prev_r, prev_w)) = self.prev_process_disk.get(&proc.pid) {
                proc.disk_read_speed =
                    ((proc.disk_usage.0.saturating_sub(prev_r)) as f64 / elapsed) as u64;
                proc.disk_write_speed =
                    ((proc.disk_usage.1.saturating_sub(prev_w)) as f64 / elapsed) as u64;
            }
        }
        self.prev_process_disk = self
            .cached_processes
            .iter()
            .map(|p| (p.pid, p.disk_usage))
            .collect();
        self.prev_process_disk_time = now;
    }

    fn tick_throttle_check(&mut self) {
        self.throttle_info = self.snapshot.throttle_info().cloned();
        self.throttle_reason = if let Some(ref ti) = self.throttle_info {
            let (cpu_temp, _) = self.snapshot.temperatures();
            crate::throttle::classify_throttle(ti, self.snapshot.cpu_usage(), cpu_temp)
        } else {
            crate::throttle::ThrottleReason::None
        };
    }

    /// Sample global CPU/mem sparkline every tick; sample proc_history only on heavy frames.
    fn tick_history_sample(&mut self, had_heavy: bool) {
        let cpu_pct = (self.snapshot.cpu_usage() * 10.0) as u64;
        let (mem_used, mem_total) = self.snapshot.memory_usage();
        let mem_pct = if mem_total > 0 {
            (mem_used as f64 / mem_total as f64 * 1000.0) as u64
        } else {
            0
        };
        if self.global_cpu_history.len() >= SPARKLINE_LEN {
            self.global_cpu_history.pop_front();
        }
        self.global_cpu_history.push_back(cpu_pct);
        if self.global_mem_history.len() >= SPARKLINE_LEN {
            self.global_mem_history.pop_front();
        }
        self.global_mem_history.push_back(mem_pct);

        // retain every tick — clear dead entries to bound memory.
        let alive_pids: HashSet<u32> = self.cached_processes.iter().map(|p| p.pid).collect();
        self.proc_history.retain(|pid, _| alive_pids.contains(pid));

        if had_heavy {
            let processes = &self.cached_processes;
            let mut sorted: Vec<&ProcessInfo> = processes.iter().collect();
            // 只对前 MAX_TRACKED 排序，剩余不排序（O(N) 而非 O(N log N)）。
            let cmp_cpu = |a: &&ProcessInfo, b: &&ProcessInfo| {
                b.cpu_usage
                    .partial_cmp(&a.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            };
            if sorted.len() > MAX_TRACKED {
                let (left, _, _) = sorted.select_nth_unstable_by(MAX_TRACKED, cmp_cpu);
                left.sort_by(cmp_cpu);
            } else {
                sorted.sort_by(cmp_cpu);
            }
            for proc in sorted.iter().take(MAX_TRACKED) {
                let entry = self
                    .proc_history
                    .entry(proc.pid)
                    .or_insert_with(|| ProcHistory {
                        cpu: VecDeque::new(),
                    });
                if entry.cpu.len() >= SPARKLINE_LEN {
                    entry.cpu.pop_front();
                }
                entry.cpu.push_back((proc.cpu_usage * 10.0) as u64);
            }
        }
    }

    fn tick_alert_evaluate(&mut self) {
        let alert_events = self
            .alert_manager
            .evaluate(&self.snapshot, &self.cached_processes);
        for event in &alert_events {
            if let crate::alert::AlertEventType::Fired = event.event_type
                && let crate::alert::AlertSeverity::Critical = event.severity
            {
                let _ = crate::monitor::notify::send_toast("proc - Critical Alert", &event.message);
            }
        }
    }

    fn tick_panels(&mut self) {
        let mut ctx = PanelContext {
            snapshot: &self.snapshot,
            cached_processes: &self.cached_processes,
            cached_sorted: &self.cached_sorted,
            security_scores: &self.security_scores,
            status_message: &mut self.status_message,
            detail_process: &mut self.detail_process,
            pending_kill: &mut self.pending_kill,
            data_dirty: &mut self.data_dirty,
            pending_redraw: &mut self.pending_redraw,
            alert_manager: &mut self.alert_manager,
            op_history: &mut self.op_history,
        };
        if self.mode == AppMode::ProcessList {
            self.process_panel.tick(&mut ctx);
        } else if self.mode == AppMode::PortMap {
            self.port_panel.tick(&mut ctx);
        }
    }

    fn tick_usb_monitor_docker(&mut self) {
        if self.mode == AppMode::UsbAssistant || !self.usb_panel.devices.is_empty() {
            self.usb_panel.refresh_device_list();
        }
        self.monitor_panel.poll_events();
        if self.mode == AppMode::DockerPanel {
            self.docker_panel.poll_events();
        }
    }

    fn clamp_cursors(&mut self) {
        let total = self.cached_sorted.len();
        if self.process_panel.cursor_index >= total && total > 0 {
            self.process_panel.cursor_index = total - 1;
        }
    }

    fn rebuild_sorted_cache(&mut self) {
        let processes = &self.cached_processes;
        let query = self.process_panel.search.query();
        let filtered: Vec<&ProcessInfo> = if query.is_empty() {
            processes.iter().collect()
        } else {
            let q = query.to_lowercase();
            processes
                .iter()
                .filter(|p| p.name.to_lowercase().contains(&q) || p.pid.to_string().contains(query))
                .collect()
        };

        // 一次性建索引，避免 N 次 O(N) position 查找 → O(N²)。
        // 同时把 clone(ProcessInfo) 改为借引用，省去全字段深拷贝。
        let pid_to_idx: HashMap<u32, usize> = processes
            .iter()
            .enumerate()
            .map(|(i, p)| (p.pid, i))
            .collect();

        let mut result: Vec<(classify::ProcessClass, &ProcessInfo)> = filtered
            .into_iter()
            .map(|p| (classify::classify_process(p), p))
            .collect();

        let sort_field = self.process_panel.sort_field;
        result.sort_by(|a, b| match sort_field {
            SortField::Cpu => {
                b.1.cpu_usage
                    .partial_cmp(&a.1.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
            SortField::Memory => b.1.memory.cmp(&a.1.memory),
            SortField::Pid => a.1.pid.cmp(&b.1.pid),
            SortField::Name => a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()),
            SortField::Security => {
                let sa = self
                    .security_scores
                    .get(&a.1.pid)
                    .map(|s| s.score)
                    .unwrap_or(100);
                let sb = self
                    .security_scores
                    .get(&b.1.pid)
                    .map(|s| s.score)
                    .unwrap_or(100);
                sa.cmp(&sb)
            }
            SortField::DiskRead => {
                let sa = a.1.disk_read_speed;
                let sb = b.1.disk_read_speed;
                sb.cmp(&sa)
            }
            SortField::DiskWrite => {
                let sa = a.1.disk_write_speed;
                let sb = b.1.disk_write_speed;
                sb.cmp(&sa)
            }
        });

        // 转换为 (idx, class) 索引形式；PID 查 O(1) 而非 O(N)
        self.cached_sorted = result
            .iter()
            .map(|(class, p)| (*pid_to_idx.get(&p.pid).unwrap_or(&0), *class))
            .collect();

        self.data_dirty = false;
    }

    pub fn shutdown(&mut self) {
        for handle in &self.monitor_panel.watchdog_handles {
            handle.stop();
        }
        for handle in &self.monitor_panel.port_handles {
            handle.stop();
        }
    }

    pub fn sidebar_height(&self) -> u16 {
        13
    }

    pub fn usb_scan_devices(&mut self) {
        self.usb_panel.scan_devices(&self.cached_processes);
    }

    pub fn docker_refresh(&mut self) {
        self.docker_panel.refresh();
    }

    pub fn monitor_poll_events(&mut self) {
        self.monitor_panel.poll_events();
    }

    pub fn docker_poll_events(&mut self) {
        self.docker_panel.poll_events();
    }

    pub fn filtered_ports(&self) -> &[PortEntry] {
        self.port_panel.filtered_ports()
    }

    pub fn filtered_process_groups(&self) -> &[crate::port_map::ProcessNetGroup] {
        self.port_panel.filtered_process_groups()
    }

    pub fn filtered_remote_groups(&self) -> &[crate::port_map::RemoteGroup] {
        self.port_panel.filtered_remote_groups()
    }

    pub fn anomaly_count(&self) -> usize {
        self.port_panel.anomaly_count()
    }

    pub fn visible_anomalies(&self) -> Vec<&crate::anomaly::Anomaly> {
        self.port_panel.visible_anomalies()
    }

    pub fn dismiss_anomaly(&mut self, id: &str) {
        self.port_panel.dismiss_anomaly(id);
    }
}
