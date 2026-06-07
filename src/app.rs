use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};

const SPARKLINE_LEN: usize = 30;
const MAX_TRACKED: usize = 50;
const MAX_OP_HISTORY: usize = 100;

pub struct OpRecord {
    pub time: String,
    pub message: String,
}

pub struct ProcHistory {
    pub cpu: VecDeque<u64>,
}

use crate::classify;
use crate::collect::{ProcessInfo, SortField, SystemSnapshot, REFRESH_INTERVAL, HEAVY_REFRESH_INTERVAL};
use crate::error::Result;
use crate::estats::EStatsCollector;
use crate::kill;
use crate::tree::{self, TreeFilter, TreeNode};
use crate::port_map::{self, PortEntry, ProcessNetGroup, Protocol, NetworkViewMode, ProcessSortField, ConnectionDiff, RemoteGroup, RemoteSortField};
use crate::eject;
use crate::eject::locks::HandleLock;
use crate::eject::classify::HandleRisk;
use crate::eject::device::RemovableDevice;
use crate::monitor::{MonitorManager, MonitorTarget, MonitorStatus, RestartPolicy};
use crate::monitor::watchdog::{self, WatchdogHandle, WatchdogEvent};
use crate::monitor::port_watcher::{self, PortWatchHandle, PortEvent};
use crate::docker::{ContainerInfo, DockerMonitor};
use crate::docker::events::{self, DockerEvent, DockerEventReceiver};
use crate::docker::stats::ContainerStats;
use crate::anomaly::{self, Anomaly, AnomalySeverity};
use crate::diag::{self, DiagnosticPhase, DiagnosticState, DiagnosticTool};
use crate::alert::AlertManager;
use crate::security::{SecurityScorer, SecurityScore};
use crate::record::Player;
use crate::record::frame::FrameTreeNode;
use crate::record::reader::frame_process_to_process_info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Menu,
    ProcessList,
    ProcessTree,
    PortMap,
    UsbAssistant,
    MonitorPanel,
    DockerPanel,
    ProcessDetail,
    Help,
    Replay,
}

pub struct App {
    pub mode: AppMode,
    pub snapshot: SystemSnapshot,
    pub cached_processes: Vec<ProcessInfo>,
    pub should_quit: bool,
    pub last_refresh: Instant,
    pub last_heavy_refresh: Instant,

    pub sort_field: SortField,
    pub cursor_index: usize,
    pub scroll_offset: usize,
    pub selected_indices: HashSet<usize>,
    pub search_active: bool,
    pub search_query: String,
    pub pending_redraw: bool,

    pub detail_process: Option<ProcessInfo>,
    pub detail_port_info: String,

    pub proc_history: HashMap<u32, ProcHistory>,
    pub global_cpu_history: VecDeque<u64>,
    pub global_mem_history: VecDeque<u64>,

    pub pending_kill: Option<KillRequest>,
    pub kill_confirm: bool,
    pub status_message: Option<String>,
    pub op_history: VecDeque<OpRecord>,

    // Process tree state
    pub tree_nodes: Vec<TreeNode>,
    pub tree_filter: TreeFilter,
    pub tree_cursor: usize,
    pub tree_scroll: usize,
    pub tree_search_active: bool,
    pub tree_search_query: String,
    pub tree_selected_indices: HashSet<usize>,

    // Port map state
    pub port_entries: Vec<PortEntry>,
    pub port_cursor: usize,
    pub port_scroll: usize,
    pub port_filter: Option<Protocol>,
    pub port_search_active: bool,
    pub port_search_query: String,
    pub port_detail: Option<PortEntry>,
    pub port_sort_field: port_map::PortSortField,
    pub port_state_filter: port_map::PortStateFilter,

    pub port_view_mode: NetworkViewMode,
    pub port_process_groups: Vec<ProcessNetGroup>,
    pub port_process_cursor: usize,
    pub port_process_scroll: usize,
    pub port_process_sort: ProcessSortField,
    pub port_expanded_pid: Option<u32>,
    pub port_is_admin: bool,
    pub estats_collector: Option<EStatsCollector>,
    pub port_process_speeds: HashMap<u32, (u64, u64, u64, u64)>,  // (down_speed, up_speed, total_down, total_up)
    pub prev_port_entries: Vec<PortEntry>,
    pub connection_diff: ConnectionDiff,
    pub connection_history: VecDeque<usize>,

    pub port_remote_groups: Vec<RemoteGroup>,
    pub port_remote_cursor: usize,
    pub port_remote_scroll: usize,
    pub port_remote_sort: RemoteSortField,

    // Anomaly detection state
    pub anomaly_detector: anomaly::AnomalyDetector,
    pub active_anomalies: Vec<Anomaly>,
    pub anomaly_dismissed: HashSet<String>,
    pub show_anomaly_detail: bool,
    pub anomaly_cursor: usize,
    pub prev_critical_ids: HashSet<String>,

    // USB assistant state
    pub usb_devices: Vec<RemovableDevice>,
    pub usb_device_cursor: usize,
    pub usb_locks: Vec<(HandleLock, HandleRisk)>,
    pub usb_lock_cursor: usize,
    pub usb_status_message: Option<String>,

    // Monitor panel state
    pub monitor_manager: MonitorManager,
    pub monitor_cursor: usize,
    pub monitor_add_submenu: Option<MonitorAddSubmenu>,
    pub monitor_watchdog_handles: Vec<WatchdogHandle>,
    pub monitor_port_handles: Vec<PortWatchHandle>,

    // Docker panel state
    pub docker_monitor: Option<DockerMonitor>,
    pub docker_containers: Vec<ContainerInfo>,
    pub docker_cursor: usize,
    pub docker_scroll: usize,
    pub docker_connected: bool,
    pub docker_status_message: Option<String>,
    pub docker_event_receiver: Option<DockerEventReceiver>,
    pub docker_events: Vec<DockerEvent>,
    pub docker_detail: Option<ContainerInfo>,
    pub docker_detail_stats: Option<ContainerStats>,

    // Diagnostic state
    pub show_diagnostics: bool,
    pub diagnostic: Option<DiagnosticState>,
    pub diagnostic_rx: Option<std::sync::mpsc::Receiver<String>>,
    pub diagnostic_thread: Option<std::thread::JoinHandle<()>>,

    // Alert & Security scoring
    pub alert_manager: AlertManager,
    pub alert_popup_open: bool,
    pub alert_scroll: usize,
    pub security_scorer: SecurityScorer,
    pub security_scores: HashMap<u32, SecurityScore>,
    scoring_cursor: usize,

    // Cached sorted/filtered process list (rebuilt only on data change)
    cached_sorted: Vec<(classify::ProcessClass, ProcessInfo)>,
    data_dirty: bool,

    // Replay state
    pub replay_player: Option<Player>,
    pub timeline_state: Option<TimelineState>,

    // Recording state (VT100 — managed by run_app, App only holds the flag)
    recording_wanted: bool,
    recording_elapsed_secs: u64,
}

pub struct KillRequest {
    pub pids: Vec<u32>,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub enum MonitorAddSubmenu {
    SelectType,
    EnterPid { input: String },
    EnterPort { input: String },
    EnterCommand {
        cmd_input: String,
        args_input: String,
        cwd_input: String,
        retries_input: String,
    },
}

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

impl App {
    pub fn new() -> Result<Self> {
        let snapshot = SystemSnapshot::new()?;
        let processes = snapshot.processes();
        let tree_nodes = tree::build_process_tree(&processes);
        let port_entries = port_map::scan_ports().unwrap_or_default();

        Ok(Self {
            mode: AppMode::ProcessList,
            snapshot,
            cached_processes: processes.clone(),
            should_quit: false,
            last_refresh: Instant::now(),
            last_heavy_refresh: Instant::now() - HEAVY_REFRESH_INTERVAL,
            sort_field: SortField::Cpu,
            cursor_index: 0,
            scroll_offset: 0,
            selected_indices: HashSet::new(),
            search_active: false,
            search_query: String::new(),
            pending_redraw: true,
            detail_process: None,
            detail_port_info: String::new(),
            proc_history: HashMap::new(),
            global_cpu_history: VecDeque::new(),
            global_mem_history: VecDeque::new(),
            pending_kill: None,
            kill_confirm: false,
            status_message: None,
            op_history: VecDeque::new(),
            tree_nodes,
            tree_filter: TreeFilter::All,
            tree_cursor: 0,
            tree_scroll: 0,
            tree_search_active: false,
            tree_search_query: String::new(),
            tree_selected_indices: HashSet::new(),
            port_entries,
            port_cursor: 0,
            port_scroll: 0,
            port_filter: None,
            port_search_active: false,
            port_search_query: String::new(),
            port_detail: None,
            port_sort_field: port_map::PortSortField::LocalPort,
            port_state_filter: port_map::PortStateFilter::All,
            port_view_mode: NetworkViewMode::Port,
            port_process_groups: Vec::new(),
            port_process_cursor: 0,
            port_process_scroll: 0,
            port_process_sort: ProcessSortField::ConnectionCount,
            port_expanded_pid: None,
            port_is_admin: crate::collect::is_elevated(),
            estats_collector: {
                if crate::collect::is_elevated() {
                    match EStatsCollector::new() {
                        Ok(c) => Some(c),
                        Err(e) => {
                            tracing::warn!("EStats 初始化失败，降级到基础模式: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            },
            port_process_speeds: HashMap::new(),
            prev_port_entries: Vec::new(),
            connection_diff: ConnectionDiff::default(),
            connection_history: VecDeque::new(),
            port_remote_groups: Vec::new(),
            port_remote_cursor: 0,
            port_remote_scroll: 0,
            port_remote_sort: RemoteSortField::ConnectionCount,
            anomaly_detector: anomaly::AnomalyDetector::new(),
            active_anomalies: Vec::new(),
            anomaly_dismissed: HashSet::new(),
            show_anomaly_detail: false,
            anomaly_cursor: 0,
            prev_critical_ids: HashSet::new(),
            usb_devices: Vec::new(),
            usb_device_cursor: 0,
            usb_locks: Vec::new(),
            usb_lock_cursor: 0,
            usb_status_message: None,
            monitor_manager: MonitorManager::new(),
            monitor_cursor: 0,
            monitor_add_submenu: None,
            monitor_watchdog_handles: Vec::new(),
            monitor_port_handles: Vec::new(),
            docker_monitor: None,
            docker_containers: Vec::new(),
            docker_cursor: 0,
            docker_scroll: 0,
            docker_connected: false,
            docker_status_message: None,
            docker_event_receiver: None,
            docker_events: Vec::new(),
            docker_detail: None,
            docker_detail_stats: None,
            show_diagnostics: false,
            diagnostic: None,
            diagnostic_rx: None,
            diagnostic_thread: None,
            alert_manager: AlertManager::load_or_default(),
            alert_popup_open: false,
            alert_scroll: 0,
            security_scorer: SecurityScorer::new(),
            security_scores: HashMap::new(),
            scoring_cursor: 0,
            cached_sorted: Vec::new(),
            data_dirty: true,
            replay_player: None,
            timeline_state: None,
            recording_wanted: false,
            recording_elapsed_secs: 0,
        })
    }

    /// Handle global tab-switching keys. Returns true if the key was consumed.
    fn try_handle_tab_switch(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('1') => { self.switch_mode(AppMode::ProcessList); true }
            KeyCode::Char('2') => { self.switch_mode(AppMode::ProcessTree); true }
            KeyCode::Char('3') => { self.switch_mode(AppMode::PortMap); true }
            KeyCode::Char('4') => { self.switch_mode(AppMode::UsbAssistant); true }
            KeyCode::Char('5') => { self.switch_mode(AppMode::MonitorPanel); true }
            KeyCode::Char('6') => { self.switch_mode(AppMode::DockerPanel); true }
            KeyCode::Char('t') => { crate::tui::theme::cycle_theme(); true }
            KeyCode::Char('A') => {
                self.alert_popup_open = !self.alert_popup_open;
                self.alert_scroll = 0;
                true
            }
            _ => false,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.pending_redraw = true;
        if self.search_active {
            self.handle_search_input(key);
            return;
        }
        if self.tree_search_active {
            self.handle_tree_search_input(key);
            return;
        }
        if self.port_search_active {
            self.handle_port_search_input(key);
            return;
        }

        if self.kill_confirm {
            self.handle_kill_confirm(key);
            return;
        }

        if self.monitor_add_submenu.is_some() {
            self.handle_monitor_submenu_input(key);
            return;
        }

        // Shift+R: toggle recording (global, works in all modes)
        if key.code == KeyCode::Char('R') {
            self.toggle_recording();
            return;
        }

        if !self.search_active && !self.tree_search_active && !self.port_search_active
            && !self.kill_confirm && self.monitor_add_submenu.is_none()
            && self.try_handle_tab_switch(key)
        {
            return;
        }

        // Alert popup handling
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

        match self.mode {
            AppMode::ProcessList | AppMode::Menu => self.handle_process_list_key(key),
            AppMode::ProcessDetail => self.handle_detail_key(key),
            AppMode::ProcessTree => self.handle_tree_key(key),
            AppMode::PortMap => self.handle_port_key(key),
            AppMode::UsbAssistant => self.handle_usb_key(key),
            AppMode::MonitorPanel => self.handle_monitor_key(key),
            AppMode::DockerPanel => self.handle_docker_key(key),
            AppMode::Replay => self.handle_replay_key(key),
            _ => self.handle_global_key(key),
        }
    }

    fn handle_process_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Left => {
                self.sort_field = self.sort_field.prev();
                self.cursor_index = 0;
                self.scroll_offset = 0;
                self.data_dirty = true;
            }
            KeyCode::Right => {
                self.sort_field = self.sort_field.next();
                self.cursor_index = 0;
                self.scroll_offset = 0;
                self.data_dirty = true;
            }
            KeyCode::Char(' ') => self.toggle_select(),
            KeyCode::Char('a') => self.select_all(),
            KeyCode::Char('A') => self.deselect_all(),
            KeyCode::Char('/') => {
                self.search_active = true;
            }
            KeyCode::Enter => self.enter_detail(),
            KeyCode::Char('k') => self.initiate_kill(false),
            KeyCode::Char('K') => self.initiate_kill(true),
            KeyCode::Char('S') => {
                self.sort_field = SortField::Security;
                self.cursor_index = 0;
                self.scroll_offset = 0;
                self.data_dirty = true;
            }
            KeyCode::PageUp => self.page_up(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::Esc => {
                if self.mode == AppMode::Menu {
                    self.mode = AppMode::ProcessList;
                }
                self.search_query.clear();
                self.status_message = None;
            }
            _ => {}
        }
    }

    fn handle_detail_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => self.mode = AppMode::ProcessList,
            KeyCode::Char('k') => {
                if let Some(ref proc) = self.detail_process {
                    let result = kill::kill_process(proc.pid, false);
                    match result {
                        Ok(kill::KillResult::Killed) => {
                            let msg = format!("终止 {} (PID {})", proc.name, proc.pid);
                            self.status_message = Some(format!("{} 已终止", msg));
                            self.record_op(msg);
                            self.mode = AppMode::ProcessList;
                        }
                        Ok(kill::KillResult::AlreadyGone) => {
                            self.status_message = Some("进程已不存在".to_string());
                            self.mode = AppMode::ProcessList;
                        }
                        Ok(kill::KillResult::AccessDenied) => {
                            self.status_message = Some("权限不足，无法终止进程".to_string());
                        }
                        Ok(kill::KillResult::Failed(e)) => {
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
                    match self.monitor_manager.add_monitor(
                        MonitorTarget::ByPid { pid },
                        RestartPolicy::NotifyOnly,
                    ) {
                        Ok(monitor_id) => {
                            self.monitor_manager.add_notification(
                                format!("已添加 PID {} 监控 (ID: {})", pid, monitor_id)
                            );
                            self.status_message = Some(format!("已添加 PID {} 监控 (ID: {})", pid, monitor_id));
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
                    let info = tree::format_process_info(proc);
                    match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&info)) {
                        Ok(()) => {
                            self.status_message = Some(format!("已复制进程信息到剪贴板 ({} bytes)", info.len()));
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

    fn handle_tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Up => self.tree_move_cursor(-1),
            KeyCode::Down => self.tree_move_cursor(1),
            KeyCode::Enter => self.tree_toggle_expand(),
            KeyCode::Char('/') => self.tree_search_active = true,
            KeyCode::Char(' ') => self.tree_toggle_select(),
            KeyCode::Char('a') => self.tree_select_all(),
            KeyCode::Char('A') => self.tree_deselect_all(),
            KeyCode::Char('k') => self.tree_initiate_kill(false),
            KeyCode::Char('K') => self.tree_initiate_kill(true),
            KeyCode::Char('o') => self.tree_select_orphans(),
            KeyCode::Char('z') => self.tree_select_stale(),
            KeyCode::Char('f') => self.tree_cycle_filter(),
            KeyCode::Esc => {
                self.tree_search_query.clear();
                self.status_message = None;
            }
            _ => {}
        }
    }

    fn handle_port_key(&mut self, key: KeyEvent) {
        if self.show_anomaly_detail {
            let visible = self.visible_anomalies();
            match key.code {
                KeyCode::Char('a') | KeyCode::Esc => {
                    self.show_anomaly_detail = false;
                }
                KeyCode::Up => {
                    if self.anomaly_cursor > 0 {
                        self.anomaly_cursor -= 1;
                    }
                }
                KeyCode::Down => {
                    if self.anomaly_cursor + 1 < visible.len() {
                        self.anomaly_cursor += 1;
                    }
                }
                KeyCode::Char('d') => {
                    if let Some(a) = visible.get(self.anomaly_cursor) {
                        let id = a.id();
                        self.dismiss_anomaly(&id);
                        let new_visible = self.visible_anomalies();
                        if self.anomaly_cursor >= new_visible.len() && self.anomaly_cursor > 0 {
                            self.anomaly_cursor -= 1;
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        if self.show_diagnostics {
            self.handle_diagnostic_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('g') => {
                self.port_view_mode = self.port_view_mode.toggle();
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_process_cursor = 0;
                self.port_process_scroll = 0;
                self.port_expanded_pid = None;
                self.port_remote_cursor = 0;
                self.port_remote_scroll = 0;
            }
            KeyCode::Char('/') => self.port_search_active = true,
            KeyCode::Char('a') => {
                self.show_anomaly_detail = !self.show_anomaly_detail;
                if self.show_anomaly_detail {
                    self.anomaly_cursor = 0;
                }
            }
            KeyCode::Char('f') => {
                self.port_state_filter = self.port_state_filter.next();
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_process_cursor = 0;
                self.port_process_scroll = 0;
                self.port_remote_cursor = 0;
                self.port_remote_scroll = 0;
            }
            KeyCode::Esc => {
                if self.show_anomaly_detail {
                    self.show_anomaly_detail = false;
                } else {
                    self.port_search_query.clear();
                    self.port_detail = None;
                    self.port_expanded_pid = None;
                    self.status_message = None;
                }
            }
            KeyCode::Up => {
                match self.port_view_mode {
                    NetworkViewMode::Process => self.port_process_move_cursor(-1),
                    NetworkViewMode::Remote => self.port_remote_move_cursor(-1),
                    NetworkViewMode::Port => self.port_move_cursor(-1),
                }
            }
            KeyCode::Down => {
                match self.port_view_mode {
                    NetworkViewMode::Process => self.port_process_move_cursor(1),
                    NetworkViewMode::Remote => self.port_remote_move_cursor(1),
                    NetworkViewMode::Port => self.port_move_cursor(1),
                }
            }
            KeyCode::Enter => {
                match self.port_view_mode {
                    NetworkViewMode::Process => self.port_process_toggle_expand(),
                    NetworkViewMode::Remote => self.port_remote_show_detail(),
                    NetworkViewMode::Port => self.port_show_detail(),
                }
            }
            KeyCode::Char('k') => {
                match self.port_view_mode {
                    NetworkViewMode::Process => self.port_process_kill_selected(),
                    NetworkViewMode::Remote => self.port_remote_kill_selected(),
                    NetworkViewMode::Port => self.port_kill_selected(),
                }
            }
            KeyCode::Char('s') => {
                match self.port_view_mode {
                    NetworkViewMode::Process => {
                        self.port_process_sort = self.port_process_sort.next();
                    }
                    NetworkViewMode::Remote => {
                        self.port_remote_sort = self.port_remote_sort.next();
                    }
                    NetworkViewMode::Port => {
                        self.port_sort_field = self.port_sort_field.next();
                    }
                }
            }
            KeyCode::Char('d') => {
                if self.port_view_mode == NetworkViewMode::Remote {
                    let groups = self.get_filtered_remote_groups();
                    if let Some(group) = groups.get(self.port_remote_cursor) {
                        let ip = group.remote_addr;
                        if ip.is_unspecified() {
                            self.status_message = Some("无效 IP（0.0.0.0 / ::），无法诊断".to_string());
                        } else {
                            self.diagnostic = Some(DiagnosticState::new(ip));
                            self.show_diagnostics = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn close_diagnostic(&mut self) {
        self.show_diagnostics = false;
        self.diagnostic = None;
        self.diagnostic_rx = None;
        self.diagnostic_thread = None;
    }

    fn handle_diagnostic_key(&mut self, key: KeyEvent) {
        let Some(ref mut diag) = self.diagnostic else { return };
        match diag.phase {
            DiagnosticPhase::Menu => match key.code {
                KeyCode::Up => {
                    if diag.tool_index > 0 {
                        diag.tool_index -= 1;
                    }
                }
                KeyCode::Down => {
                    let tools = DiagnosticState::tool_list();
                    if diag.tool_index + 1 < tools.len() {
                        diag.tool_index += 1;
                    }
                }
                KeyCode::Enter => {
                    let tools = DiagnosticState::tool_list();
                    if let Some(tool) = tools.get(diag.tool_index) {
                        if DiagnosticState::tool_unavailable_for_private(tool)
                            && diag::is_private_or_loopback(&diag.target_ip)
                        {
                            return;
                        }
                        let tool = match diag.tool_index {
                            0 => DiagnosticTool::Ping,
                            1 => DiagnosticTool::DnsReverse,
                            2 => DiagnosticTool::Whois,
                            3 => DiagnosticTool::Traceroute,
                            _ => DiagnosticTool::PortScan,
                        };
                        self.start_diagnostic(tool);
                    }
                }
                KeyCode::Esc => self.close_diagnostic(),
                _ => {}
            },
            DiagnosticPhase::Running => match key.code {
                KeyCode::Up => {
                    diag.auto_scroll = false;
                    diag.scroll = diag.scroll.saturating_sub(1);
                }
                KeyCode::Down => {
                    diag.auto_scroll = false;
                    diag.scroll += 1;
                }
                KeyCode::Esc => self.close_diagnostic(),
                _ => {}
            },
            DiagnosticPhase::Completed | DiagnosticPhase::Failed => match key.code {
                KeyCode::Up => {
                    diag.auto_scroll = false;
                    diag.scroll = diag.scroll.saturating_sub(1);
                }
                KeyCode::Down => {
                    diag.auto_scroll = false;
                    diag.scroll += 1;
                }
                KeyCode::Enter => {
                    diag.phase = DiagnosticPhase::Menu;
                    diag.scroll = 0;
                }
                KeyCode::Esc => self.close_diagnostic(),
                _ => {}
            },
        }
    }

    fn start_diagnostic(&mut self, tool: DiagnosticTool) {
        let Some(ref mut diag) = self.diagnostic else { return };

        // Backend guard: reject Whois/Traceroute for private IPs
        if DiagnosticState::tool_unavailable_for_private(&tool)
            && diag::is_private_or_loopback(&diag.target_ip)
        {
            diag.phase = DiagnosticPhase::Failed;
            diag.error_msg = Some("内网 IP 不支持此工具".to_string());
            diag.content.push("内网 IP 不支持此工具".to_string());
            return;
        }

        diag.phase = DiagnosticPhase::Running;
        diag.content.clear();
        diag.scroll = 0;
        diag.auto_scroll = true;

        let target_ip = diag.target_ip;
        match tool {
            DiagnosticTool::Ping => {
                let (handle, rx) = diag::run_ping(target_ip);
                self.diagnostic_rx = Some(rx);
                self.diagnostic_thread = Some(handle);
            }
            DiagnosticTool::DnsReverse => {
                let (handle, rx) = diag::run_dns_reverse(target_ip);
                self.diagnostic_rx = Some(rx);
                self.diagnostic_thread = Some(handle);
            }
            DiagnosticTool::Whois => {
                let (_, rx) = diag::run_whois(target_ip);
                self.diagnostic_rx = Some(rx);
            }
            DiagnosticTool::Traceroute => {
                let (_, rx) = diag::run_traceroute(target_ip);
                self.diagnostic_rx = Some(rx);
            }
            DiagnosticTool::PortScan => {
                let (_, rx) = diag::run_port_scan(target_ip);
                self.diagnostic_rx = Some(rx);
            }
        }
    }

    fn handle_usb_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Up => self.usb_move_cursor(-1),
            KeyCode::Down => self.usb_move_cursor(1),
            KeyCode::Enter => self.usb_select_device(),
            KeyCode::Char('k') => self.usb_kill_safe(),
            KeyCode::Char('r') => self.usb_refresh(),
            KeyCode::Char('w') => self.usb_wait_and_monitor(),
            KeyCode::Tab => self.usb_toggle_focus(),
            KeyCode::Esc => {
                self.usb_status_message = None;
            }
            _ => {}
        }
    }

    fn handle_monitor_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Up => self.monitor_move_cursor(-1),
            KeyCode::Down => self.monitor_move_cursor(1),
            KeyCode::Char('a') => {
                self.monitor_add_submenu = Some(MonitorAddSubmenu::SelectType);
            }
            KeyCode::Char('d') => self.monitor_delete_selected(),
            KeyCode::Char('r') => self.monitor_restart_selected(),
            KeyCode::Char('s') => self.monitor_toggle_pause(),
            KeyCode::Esc => {
                self.status_message = None;
            }
            _ => {}
        }
    }

    fn handle_monitor_submenu_input(&mut self, key: KeyEvent) {
        let submenu = match &self.monitor_add_submenu {
            Some(s) => s.clone(),
            None => return,
        };

        match submenu {
            MonitorAddSubmenu::SelectType => match key.code {
                KeyCode::Char('1') => {
                    self.monitor_add_submenu = Some(MonitorAddSubmenu::EnterPid { input: String::new() });
                }
                KeyCode::Char('2') => {
                    self.monitor_add_submenu = Some(MonitorAddSubmenu::EnterPort { input: String::new() });
                }
                KeyCode::Char('3') => {
                    self.monitor_add_submenu = Some(MonitorAddSubmenu::EnterCommand {
                        cmd_input: String::new(),
                        args_input: String::new(),
                        cwd_input: String::new(),
                        retries_input: "5".to_string(),
                    });
                }
                KeyCode::Esc => {
                    self.monitor_add_submenu = None;
                }
                _ => {}
            },
            MonitorAddSubmenu::EnterPid { input } => match key.code {
                KeyCode::Enter => {
                    if let Ok(pid) = input.parse::<u32>() {
                        match self.monitor_manager.add_monitor(
                            MonitorTarget::ByPid { pid },
                            RestartPolicy::NotifyOnly,
                        ) {
                            Ok(id) => {
                                self.monitor_manager.add_notification(format!("已添加 PID {} 监控 (ID: {})", pid, id));
                                self.status_message = Some(format!("已添加 PID {} 监控", pid));
                                self.record_op(format!("添加 PID {} 监控", pid));
                            }
                            Err(e) => {
                                tracing::warn!("添加监控失败: {}", e);
                                self.status_message = Some(format!("添加监控失败: {}", e));
                            }
                        }
                    }
                    self.monitor_add_submenu = None;
                }
                KeyCode::Esc => {
                    self.monitor_add_submenu = None;
                }
                KeyCode::Backspace => {
                    if let Some(MonitorAddSubmenu::EnterPid { input }) = &mut self.monitor_add_submenu {
                        input.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(MonitorAddSubmenu::EnterPid { input }) = &mut self.monitor_add_submenu {
                        input.push(c);
                    }
                }
                _ => {}
            },
            MonitorAddSubmenu::EnterPort { input } => match key.code {
                KeyCode::Enter => {
                    if let Ok(port) = input.parse::<u16>() {
                        match self.monitor_manager.add_monitor(
                            MonitorTarget::ByPort { port },
                            RestartPolicy::NotifyOnly,
                        ) {
                            Ok(id) => {
                                let handle = port_watcher::spawn_port_watcher(port, 5);
                                self.monitor_port_handles.push(handle);
                                self.monitor_manager.add_notification(format!("已添加端口 {} 监控 (ID: {})", port, id));
                                self.status_message = Some(format!("已添加端口 {} 监控", port));
                                self.record_op(format!("添加端口 {} 监控", port));
                            }
                            Err(e) => {
                                tracing::warn!("添加监控失败: {}", e);
                                self.status_message = Some(format!("添加监控失败: {}", e));
                            }
                        }
                    }
                    self.monitor_add_submenu = None;
                }
                KeyCode::Esc => {
                    self.monitor_add_submenu = None;
                }
                KeyCode::Backspace => {
                    if let Some(MonitorAddSubmenu::EnterPort { input }) = &mut self.monitor_add_submenu {
                        input.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(MonitorAddSubmenu::EnterPort { input }) = &mut self.monitor_add_submenu {
                        input.push(c);
                    }
                }
                _ => {}
            },
            MonitorAddSubmenu::EnterCommand { cmd_input, args_input, cwd_input, retries_input } => match key.code {
                KeyCode::Enter => {
                    if !cmd_input.is_empty() {
                        let args: Vec<String> = if args_input.is_empty() {
                            Vec::new()
                        } else {
                            args_input.split_whitespace().map(|s| s.to_string()).collect()
                        };
                        let cwd = if cwd_input.is_empty() { None } else { Some(std::path::PathBuf::from(&cwd_input)) };
                        let max_retries = retries_input.parse::<u32>().unwrap_or(5);
                        match self.monitor_manager.add_monitor(
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
                                let handle = watchdog::spawn_watchdog(
                                    id,
                                    &cmd_input,
                                    &args,
                                    cwd.as_deref(),
                                    RestartPolicy::AutoRestart {
                                        max_retries,
                                        base_backoff: 1,
                                        max_backoff: 30,
                                    },
                                );
                                self.monitor_watchdog_handles.push(handle);
                                self.monitor_manager.add_notification(format!("已添加命令监控: {} (ID: {})", cmd_input, id));
                                self.status_message = Some(format!("已添加命令监控: {}", cmd_input));
                                self.record_op(format!("添加命令监控: {}", cmd_input));
                            }
                            Err(e) => {
                                tracing::warn!("添加监控失败: {}", e);
                                self.status_message = Some(format!("添加监控失败: {}", e));
                            }
                        }
                    }
                    self.monitor_add_submenu = None;
                }
                KeyCode::Esc => {
                    self.monitor_add_submenu = None;
                }
                KeyCode::Backspace => {
                    if let Some(MonitorAddSubmenu::EnterCommand { cmd_input, args_input, cwd_input, retries_input }) = &mut self.monitor_add_submenu {
                        if !retries_input.is_empty() && cmd_input.is_empty() && args_input.is_empty() && cwd_input.is_empty() {
                            retries_input.pop();
                        } else if !cwd_input.is_empty() && cmd_input.is_empty() && args_input.is_empty() {
                            cwd_input.pop();
                        } else if !args_input.is_empty() && cmd_input.is_empty() {
                            args_input.pop();
                        } else {
                            cmd_input.pop();
                        }
                    }
                }
                KeyCode::Tab => {
                    if let Some(MonitorAddSubmenu::EnterCommand { .. }) = &mut self.monitor_add_submenu {
                        // Tab 切换输入字段（简化处理）
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(MonitorAddSubmenu::EnterCommand { cmd_input, .. }) = &mut self.monitor_add_submenu {
                        cmd_input.push(c);
                    }
                }
                _ => {}
            },
        }
    }

    fn handle_global_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.mode = AppMode::ProcessList,
            _ => {}
        }
    }

    // --- Recording state (VT100) ---

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

    /// Returns the recorded AppMode of the current replay frame (for rendering).
    pub fn replay_frame_mode(&self) -> AppMode {
        let frame_index = self.timeline_state.as_ref().map(|ts| ts.current_frame).unwrap_or(0);
        if let Some(ref player) = self.replay_player {
            if let Some(frame) = player.frame_at(frame_index) {
                return match frame.mode.as_str() {
                    "ProcessTree" => AppMode::ProcessTree,
                    "PortMap" => AppMode::PortMap,
                    "UsbAssistant" => AppMode::UsbAssistant,
                    "MonitorPanel" => AppMode::MonitorPanel,
                    "DockerPanel" => AppMode::DockerPanel,
                    _ => AppMode::ProcessList,
                };
            }
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
        // Load initial frame
        self.replay_load_current_frame();
    }

    fn handle_replay_key(&mut self, key: KeyEvent) {
        let Some(ref mut ts) = self.timeline_state else { return };
        match key.code {
            KeyCode::Char('q') => {
                self.should_quit = true;
            }
            KeyCode::Char(' ') => {
                ts.playing = !ts.playing;
            }
            KeyCode::Left => {
                if key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                    ts.current_frame = ts.current_frame.saturating_sub(10);
                } else {
                    ts.current_frame = ts.current_frame.saturating_sub(1);
                }
                ts.playing = false;
                self.replay_load_current_frame();
            }
            KeyCode::Right => {
                if key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT) {
                    ts.current_frame = (ts.current_frame + 10).min(ts.total_frames.saturating_sub(1));
                } else {
                    ts.current_frame = (ts.current_frame + 1).min(ts.total_frames.saturating_sub(1));
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
        let frame_index = self.timeline_state.as_ref().map(|ts| ts.current_frame).unwrap_or(0);
        if let Some(ref player) = self.replay_player {
            if let Some(frame) = player.frame_at(frame_index) {
                // Restore process list
                self.cached_processes = frame.processes.iter().map(frame_process_to_process_info).collect();

                // Restore tree nodes
                self.tree_nodes = frame.tree_nodes.iter().map(frame_tree_node_to_tree_node).collect();

                // Restore port data
                self.port_entries = frame.port_entries.iter().map(|e| PortEntry {
                    protocol: match e.protocol.as_str() {
                        "Udp" => Protocol::Udp,
                        _ => Protocol::Tcp,
                    },
                    local_addr: e.local_addr.parse().unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
                    local_port: e.local_port,
                    remote_addr: e.remote_addr.as_ref().and_then(|a| a.parse().ok()),
                    remote_port: e.remote_port,
                    state: e.state.clone(),
                    pid: e.pid,
                    process_name: e.process_name.clone(),
                }).collect();

                // Restore port view mode
                self.port_view_mode = match frame.port_view_mode {
                    1 => NetworkViewMode::Process,
                    2 => NetworkViewMode::Remote,
                    _ => NetworkViewMode::Port,
                };

                // Restore USB data
                self.usb_devices = frame.usb_devices.iter().map(|d| RemovableDevice {
                    drive_letter: d.drive_letter,
                    label: d.label.clone(),
                    total_size: d.total_size,
                    used_size: d.used_size,
                    file_system: d.file_system.clone(),
                    is_occupied: d.is_occupied,
                    device_path: String::new(),
                }).collect();
                self.usb_locks = frame.usb_locks.iter().map(|l| {
                    (HandleLock {
                        pid: l.pid,
                        process_name: l.process_name.clone(),
                        exe_path: l.exe_path.clone(),
                        process_class: classify::ProcessClass::Unknown,
                        port_info: Vec::new(),
                    }, match l.risk.as_str() {
                        "Critical" => HandleRisk::Critical,
                        "Warning" => HandleRisk::Warning,
                        "Safe" => HandleRisk::Safe,
                        _ => HandleRisk::Harmless,
                    })
                }).collect();

                // Restore Docker data
                self.docker_containers = frame.docker_containers.iter().map(|c| ContainerInfo {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    image: c.image.clone(),
                    status: c.status.clone(),
                    state: c.state.clone(),
                    health: match c.health.as_str() {
                        "Healthy" => crate::docker::HealthStatus::Healthy,
                        "Unhealthy" => crate::docker::HealthStatus::Unhealthy,
                        "Starting" => crate::docker::HealthStatus::Starting,
                        _ => crate::docker::HealthStatus::NotConfigured,
                    },
                    cpu_percent: c.cpu_percent,
                    memory_usage: c.memory_usage,
                    network_in: c.network_in,
                    network_out: c.network_out,
                    running_since: None,
                    ports: c.ports.clone(),
                }).collect();

                // Restore op history (take the frame's ops, keep recent)
                self.op_history = frame.ops.iter().rev().take(MAX_OP_HISTORY).rev().map(|o| OpRecord {
                    time: o.time.clone(),
                    message: o.message.clone(),
                }).collect();

                // Restore status message
                self.status_message = frame.status_message.clone();

                // Restore navigation state
                self.cursor_index = frame.nav.cursor;
                self.scroll_offset = frame.nav.scroll;
                self.selected_indices = frame.nav.selected.iter().copied().collect();
                self.tree_cursor = frame.nav.tree_cursor;
                self.tree_scroll = frame.nav.tree_scroll;
                self.tree_selected_indices = frame.nav.tree_selected.iter().copied().collect();
                self.port_cursor = frame.nav.port_cursor;
                self.port_scroll = frame.nav.port_scroll;
                self.port_process_cursor = frame.nav.port_process_cursor;
                self.port_process_scroll = frame.nav.port_process_scroll;
                self.port_remote_cursor = frame.nav.port_remote_cursor;
                self.port_remote_scroll = frame.nav.port_remote_scroll;
                self.usb_device_cursor = frame.nav.usb_device_cursor;
                self.monitor_cursor = frame.nav.monitor_cursor;
                self.docker_cursor = frame.nav.docker_cursor;
                self.docker_scroll = frame.nav.docker_scroll;

                // Restore system metrics for sidebar display
                self.snapshot.set_replay_metrics(
                    frame.cpu_usage,
                    frame.memory_used,
                    frame.memory_total,
                    frame.net_down,
                    frame.net_up,
                );
                self.global_cpu_history = frame.cpu_history.iter().copied().collect();
                self.global_mem_history = frame.mem_history.iter().copied().collect();

                self.data_dirty = true;
            }
        }
    }

    fn replay_tick(&mut self) {
        let step = {
            let Some(ref ts) = self.timeline_state else { return };
            if !ts.playing || ts.total_frames == 0 {
                return;
            }
            let speed = ts.speed;
            match speed {
                ReplaySpeed::Half => {
                    let Some(ref mut ts) = self.timeline_state else { return };
                    ts.half_tick += 1;
                    if ts.half_tick >= 2 {
                        ts.half_tick = 0;
                        1
                    } else {
                        0
                    }
                }
                ReplaySpeed::Normal => 1,
                ReplaySpeed::Double => 2,
                ReplaySpeed::Quad => 4,
            }
        };

        if step > 0 {
            let (_, at_end) = {
                let Some(ref mut ts) = self.timeline_state else { return };
                let total = ts.total_frames;
                ts.current_frame = (ts.current_frame + step).min(total.saturating_sub(1));
                (ts.current_frame, ts.current_frame >= total.saturating_sub(1))
            };
            self.replay_load_current_frame();
            if at_end {
                let Some(ref mut ts) = self.timeline_state else { return };
                ts.playing = false;
            }
        }
    }

    // --- Mouse handling ---

    pub fn handle_mouse(&mut self, event: MouseEvent) {
        self.pending_redraw = true;
        if self.mode == AppMode::Replay {
            return;
        }

        let Ok((term_width, _)) = crossterm::terminal::size() else { return };

        // REC button: rightmost area of toolbar
        let rec_w: u16 = 12;
        let rec_start = term_width.saturating_sub(rec_w + 1);
        if event.row < 3 && event.column >= rec_start {
            self.toggle_recording();
            return;
        }

        // Only handle clicks in the process table area (ProcessList mode)
        if self.mode != AppMode::ProcessList {
            return;
        }

        // Process table is in main_area: row >= 3, column < (term_width - 60)
        let table_right = term_width.saturating_sub(60);
        if event.row < 3 || event.column >= table_right {
            return;
        }

        // Row 3 is the table header, data rows start at row 4
        let data_row = event.row as isize - 4;
        if data_row < 0 {
            return;
        }
        let clicked_index = data_row as usize + self.scroll_offset;

        use crossterm::event::{MouseButton, MouseEventKind};
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if (clicked_index as usize) < self.filtered_count() {
                    self.cursor_index = clicked_index as usize;
                    self.toggle_select();
                }
            }
            _ => {}
        }
    }

    fn handle_search_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search_active = false;
                self.search_query.clear();
                self.cursor_index = 0;
                self.scroll_offset = 0;
                self.data_dirty = true;
            }
            KeyCode::Enter => {
                self.search_active = false;
                self.cursor_index = 0;
                self.scroll_offset = 0;
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.cursor_index = 0;
                self.scroll_offset = 0;
                self.data_dirty = true;
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.cursor_index = 0;
                self.scroll_offset = 0;
                self.data_dirty = true;
            }
            _ => {}
        }
    }

    fn handle_tree_search_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.tree_search_active = false;
                self.tree_search_query.clear();
                self.tree_cursor = 0;
                self.tree_scroll = 0;
            }
            KeyCode::Enter => {
                self.tree_search_active = false;
                self.tree_cursor = 0;
                self.tree_scroll = 0;
            }
            KeyCode::Backspace => {
                self.tree_search_query.pop();
                self.tree_cursor = 0;
                self.tree_scroll = 0;
            }
            KeyCode::Char(c) => {
                self.tree_search_query.push(c);
                self.tree_cursor = 0;
                self.tree_scroll = 0;
            }
            _ => {}
        }
    }

    fn handle_port_search_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.port_search_active = false;
                self.port_search_query.clear();
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_process_cursor = 0;
                self.port_process_scroll = 0;
                self.port_remote_cursor = 0;
                self.port_remote_scroll = 0;
            }
            KeyCode::Enter => {
                self.port_search_active = false;
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_process_cursor = 0;
                self.port_process_scroll = 0;
                self.port_remote_cursor = 0;
                self.port_remote_scroll = 0;
            }
            KeyCode::Backspace => {
                self.port_search_query.pop();
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_process_cursor = 0;
                self.port_process_scroll = 0;
                self.port_remote_cursor = 0;
                self.port_remote_scroll = 0;
            }
            KeyCode::Char(c) => {
                self.port_search_query.push(c);
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_process_cursor = 0;
                self.port_process_scroll = 0;
                self.port_remote_cursor = 0;
                self.port_remote_scroll = 0;
            }
            _ => {}
        }
    }

    fn handle_kill_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(req) = self.pending_kill.take() {
                    let pid_to_name: std::collections::HashMap<u32, String> = self.cached_processes
                        .iter().map(|p| (p.pid, p.name.clone())).collect();
                    let mut results = Vec::new();
                    for pid in req.pids {
                        let name = pid_to_name.get(&pid).map(|s| s.as_str()).unwrap_or("?");
                        match kill::kill_process(pid, req.force) {
                            Ok(kill::KillResult::Killed) => results.push(format!("终止 {} (PID {})", name, pid)),
                            Ok(kill::KillResult::AlreadyGone) => results.push(format!("{} (PID {}) 已不存在", name, pid)),
                            Ok(kill::KillResult::AccessDenied) => results.push(format!("{} (PID {}) 权限不足", name, pid)),
                            Ok(kill::KillResult::Failed(e)) => results.push(format!("{} (PID {}) 失败: {}", name, pid, e)),
                            Err(e) => results.push(format!("{} (PID {}) 错误: {}", name, pid, e)),
                        }
                    }
                    self.status_message = Some(results.join("; "));
                    self.record_op(results.join("; "));
                    self.selected_indices.clear();
                    self.tree_selected_indices.clear();
                    if let Err(e) = self.snapshot.refresh() {
                        tracing::warn!("刷新进程列表失败: {}", e);
                    }
                    self.refresh_tree();
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
        // UTC+8
        let local_secs = secs + 8 * 3600;
        let h = ((local_secs / 3600) % 24) as u8;
        let m = ((local_secs / 60) % 60) as u8;
        self.op_history.push_back(OpRecord {
            time: format!("{:02}:{:02}", h, m),
            message,
        });
        if self.op_history.len() > MAX_OP_HISTORY {
            self.op_history.pop_front();
        }
    }

    fn switch_mode(&mut self, mode: AppMode) {
        if mode == AppMode::UsbAssistant && self.mode != AppMode::UsbAssistant {
            self.usb_scan_devices();
        }
        if mode == AppMode::DockerPanel && self.mode != AppMode::DockerPanel {
            self.docker_refresh();
            if self.docker_connected && self.docker_event_receiver.is_none() {
                self.docker_start_watching();
            }
        }
        self.mode = mode;
        self.search_active = false;
        self.search_query.clear();
        self.status_message = None;
        self.data_dirty = true;
    }

    fn move_cursor(&mut self, delta: i32) {
        let total = self.filtered_count();
        if total == 0 {
            return;
        }
        let new = self.cursor_index as i32 + delta;
        self.cursor_index = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.clamp_scroll(20);
    }

    pub fn handle_scroll(&mut self, lines: i32) {
        self.pending_redraw = true;
        match self.mode {
            AppMode::ProcessList | AppMode::ProcessDetail => {
                self.move_cursor(lines);
            }
            AppMode::ProcessTree => {
                self.tree_move_cursor(lines);
            }
            AppMode::PortMap => {
                let total = self.visible_port_count();
                if total == 0 { return; }
                let new = self.port_cursor as i32 + lines;
                self.port_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::DockerPanel => {
                let total = self.docker_containers.len();
                if total == 0 { return; }
                let new = self.docker_cursor as i32 + lines;
                self.docker_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::UsbAssistant => {
                let total = self.usb_devices.len();
                if total == 0 { return; }
                let new = self.usb_device_cursor as i32 + lines;
                self.usb_device_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::MonitorPanel => {
                let total = self.monitor_manager.list_monitors().len();
                if total == 0 { return; }
                let new = self.monitor_cursor as i32 + lines;
                self.monitor_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            _ => {}
        }
    }

    fn visible_port_count(&self) -> usize {
        match self.port_view_mode {
            port_map::NetworkViewMode::Port => self.get_filtered_port_entries().len(),
            port_map::NetworkViewMode::Process => self.port_process_groups.len(),
            port_map::NetworkViewMode::Remote => self.port_remote_groups.len(),
        }
    }

    fn toggle_select(&mut self) {
        if self.selected_indices.contains(&self.cursor_index) {
            self.selected_indices.remove(&self.cursor_index);
        } else {
            self.selected_indices.insert(self.cursor_index);
        }
    }

    fn select_all(&mut self) {
        let total = self.filtered_count();
        self.selected_indices = (0..total).collect();
    }

    fn deselect_all(&mut self) {
        self.selected_indices.clear();
    }

    fn enter_detail(&mut self) {
        let proc_info = self.get_filtered_sorted_processes()
            .get(self.cursor_index)
            .map(|(_, p)| p.clone());
        if let Some(proc) = proc_info {
            self.detail_port_info = Self::format_port_info_cached(proc.pid);
            self.detail_process = Some(proc);
            self.mode = AppMode::ProcessDetail;
        }
    }

    fn format_port_info_cached(pid: u32) -> String {
        match port_map::find_ports_by_pid(pid) {
            Ok(ports) if ports.is_empty() => "无".to_string(),
            Ok(ports) => ports
                .iter()
                .map(|p| format!("{}:{} ({})", p.local_addr, p.local_port, p.protocol))
                .collect::<Vec<_>>()
                .join(", "),
            Err(_) => "扫描失败".to_string(),
        }
    }

    fn initiate_kill(&mut self, force: bool) {
        let pids: Vec<u32> = if self.selected_indices.is_empty() {
            let processes = self.get_filtered_sorted_processes();
            if let Some((_, proc)) = processes.get(self.cursor_index) {
                vec![proc.pid]
            } else {
                return;
            }
        } else {
            let processes = self.get_filtered_sorted_processes();
            self.selected_indices
                .iter()
                .filter_map(|&i| processes.get(i).map(|(_, p)| p.pid))
                .collect()
        };

        if pids.is_empty() {
            return;
        }

        self.pending_kill = Some(KillRequest { pids, force });
        self.kill_confirm = true;
    }

    fn page_up(&mut self) {
        let page_size = 20;
        self.cursor_index = self.cursor_index.saturating_sub(page_size);
        self.clamp_scroll(page_size);
    }

    fn page_down(&mut self) {
        let page_size = 20;
        let total = self.filtered_count();
        self.cursor_index = (self.cursor_index + page_size).min(total.saturating_sub(1));
        self.clamp_scroll(page_size);
    }

    // --- Process tree actions ---

    fn tree_move_cursor(&mut self, delta: i32) {
        let visible = self.get_filtered_tree_visible();
        let total = visible.len();
        if total == 0 {
            return;
        }
        let new = self.tree_cursor as i32 + delta;
        self.tree_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.tree_clamp_scroll(20);
    }

    fn tree_toggle_expand(&mut self) {
        let pid = {
            let filtered = tree::filter_tree(&self.tree_nodes, self.tree_filter);
            let visible = tree::flatten_visible(&filtered);
            visible.get(self.tree_cursor).map(|n| n.pid)
        };
        if let Some(pid) = pid {
            tree::toggle_node_by_pid(&mut self.tree_nodes, pid);
        }
    }

    fn tree_toggle_select(&mut self) {
        if self.tree_selected_indices.contains(&self.tree_cursor) {
            self.tree_selected_indices.remove(&self.tree_cursor);
        } else {
            self.tree_selected_indices.insert(self.tree_cursor);
        }
    }

    fn tree_select_all(&mut self) {
        let visible = self.get_filtered_tree_visible();
        self.tree_selected_indices = (0..visible.len()).collect();
    }

    fn tree_deselect_all(&mut self) {
        self.tree_selected_indices.clear();
    }

    fn tree_initiate_kill(&mut self, force: bool) {
        let pids: Vec<u32> = if self.tree_selected_indices.is_empty() {
            let visible = self.get_filtered_tree_visible();
            visible.get(self.tree_cursor).map(|n| n.pid).into_iter().collect()
        } else {
            self.get_tree_selected_pids()
        };

        if pids.is_empty() {
            return;
        }

        self.pending_kill = Some(KillRequest { pids, force });
        self.kill_confirm = true;
    }

    fn tree_select_orphans(&mut self) {
        let visible = self.get_filtered_tree_visible();
        let indices: Vec<usize> = visible
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_orphan)
            .map(|(i, _)| i)
            .collect();

        if indices.is_empty() {
            self.status_message = Some("无孤儿进程".to_string());
            return;
        }

        self.tree_selected_indices = indices.into_iter().collect();

        let total_mem: u64 = visible
            .iter()
            .enumerate()
            .filter(|(i, _)| self.tree_selected_indices.contains(i))
            .filter(|(_, n)| n.is_orphan)
            .map(|(_, n)| n.memory)
            .sum();
        let safe_count = visible
            .iter()
            .enumerate()
            .filter(|(i, _)| self.tree_selected_indices.contains(i))
            .filter(|(_, n)| n.is_orphan && n.children.is_empty())
            .count();

        self.status_message = Some(format!(
            "{}个孤儿 | 可直接杀:{} | 共{} | Space取消 | k终止",
            self.tree_selected_indices.len(),
            safe_count,
            crate::format::format_bytes(total_mem),
        ));
    }

    fn tree_select_stale(&mut self) {
        let visible = self.get_filtered_tree_visible();
        let indices: Vec<usize> = visible
            .iter()
            .enumerate()
            .filter(|(_, n)| n.is_zombie || n.is_stale)
            .map(|(i, _)| i)
            .collect();

        if indices.is_empty() {
            self.status_message = Some("无僵尸/残存进程".to_string());
            return;
        }

        self.tree_selected_indices = indices.into_iter().collect();

        let total_mem: u64 = visible
            .iter()
            .enumerate()
            .filter(|(i, _)| self.tree_selected_indices.contains(i))
            .filter(|(_, n)| n.is_zombie || n.is_stale)
            .map(|(_, n)| n.memory)
            .sum();
        let safe_count = visible
            .iter()
            .enumerate()
            .filter(|(i, _)| self.tree_selected_indices.contains(i))
            .filter(|(_, n)| (n.is_zombie || n.is_stale) && n.children.is_empty())
            .count();

        self.status_message = Some(format!(
            "{}个残存 | 可直接杀:{} | 共{} | Space取消 | k终止",
            self.tree_selected_indices.len(),
            safe_count,
            crate::format::format_bytes(total_mem),
        ));
    }

    fn get_tree_selected_pids(&self) -> Vec<u32> {
        let visible = self.get_filtered_tree_visible();
        self.tree_selected_indices
            .iter()
            .filter_map(|&i| visible.get(i).map(|n| n.pid))
            .collect()
    }

    fn tree_cycle_filter(&mut self) {
        self.tree_filter = match self.tree_filter {
            TreeFilter::All => TreeFilter::MyProcesses,
            TreeFilter::MyProcesses => TreeFilter::SystemProcesses,
            TreeFilter::SystemProcesses => TreeFilter::All,
        };
        self.tree_cursor = 0;
        self.tree_scroll = 0;
    }

    fn tree_clamp_scroll(&mut self, page_size: usize) {
        if self.tree_cursor < self.tree_scroll {
            self.tree_scroll = self.tree_cursor;
        } else if self.tree_cursor >= self.tree_scroll + page_size {
            self.tree_scroll = self.tree_cursor - page_size + 1;
        }
    }

    fn refresh_tree(&mut self) {
        let expanded_pids = tree::collect_expanded_pids(&self.tree_nodes);
        self.tree_nodes = tree::build_process_tree(&self.cached_processes);
        tree::restore_expanded_pids(&mut self.tree_nodes, &expanded_pids);
    }

    pub fn get_filtered_tree_visible(&self) -> Vec<TreeNode> {
        let filtered = tree::filter_tree(&self.tree_nodes, self.tree_filter);
        let visible = tree::flatten_visible(&filtered);
        if self.tree_search_query.is_empty() {
            visible.into_iter().cloned().collect()
        } else {
            let query_lower = self.tree_search_query.to_lowercase();
            visible
                .into_iter()
                .filter(|n| {
                    n.name.to_lowercase().contains(&query_lower)
                        || n.pid.to_string().contains(&self.tree_search_query)
                })
                .cloned()
                .collect()
        }
    }

    // --- Port map actions ---

    fn port_move_cursor(&mut self, delta: i32) {
        let total = self.get_filtered_port_entries().len();
        if total == 0 {
            return;
        }
        let new = self.port_cursor as i32 + delta;
        self.port_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.port_clamp_scroll(20);
    }

    fn port_show_detail(&mut self) {
        if self.port_detail.is_some() {
            self.port_detail = None;
            return;
        }
        let entries = self.get_filtered_port_entries();
        if let Some(entry) = entries.get(self.port_cursor) {
            self.port_detail = Some((*entry).clone());
        }
    }

    fn port_kill_selected(&mut self) {
        let pid = {
            let entries = self.get_filtered_port_entries();
            entries.get(self.port_cursor).map(|e| e.pid)
        };
        if let Some(pid) = pid {
            if pid == 0 {
                self.status_message = Some("无法确定占用进程".to_string());
                return;
            }
            match kill::kill_process(pid, false) {
                Ok(kill::KillResult::Killed) => {
                    let name = self.cached_processes.iter().find(|p| p.pid == pid).map(|p| p.name.clone()).unwrap_or_else(|| "?".to_string());
                    self.status_message = Some(format!("{} (PID {}) 已终止", name, pid));
                    self.record_op(format!("终止 {} (PID {})", name, pid));
                    self.refresh_ports();
                }
                Ok(kill::KillResult::AlreadyGone) => {
                    self.status_message = Some("进程已不存在".to_string());
                }
                Ok(kill::KillResult::AccessDenied) => {
                    self.status_message = Some("权限不足".to_string());
                }
                Ok(kill::KillResult::Failed(e)) => {
                    self.status_message = Some(format!("终止失败: {}", e));
                }
                Err(e) => {
                    self.status_message = Some(format!("错误: {}", e));
                }
            }
        }
    }

    fn port_process_move_cursor(&mut self, delta: i32) {
        let groups = self.get_filtered_process_groups();
        let total = groups.len();
        if total == 0 {
            return;
        }
        let new = self.port_process_cursor as i32 + delta;
        self.port_process_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.port_process_clamp_scroll(20);
    }

    fn port_process_clamp_scroll(&mut self, page_size: usize) {
        let groups = self.get_filtered_process_groups();
        let positions = Self::compute_group_visual_positions(&groups, self.port_expanded_pid);
        if let Some(&cursor_visual) = positions.get(self.port_process_cursor) {
            if cursor_visual < self.port_process_scroll {
                self.port_process_scroll = cursor_visual;
            } else if cursor_visual >= self.port_process_scroll + page_size {
                self.port_process_scroll = cursor_visual - page_size + 1;
            }
        }
    }

    fn port_process_toggle_expand(&mut self) {
        let groups = self.get_filtered_process_groups();
        if let Some(group) = groups.get(self.port_process_cursor) {
            if self.port_expanded_pid == Some(group.pid) {
                self.port_expanded_pid = None;
            } else {
                self.port_expanded_pid = Some(group.pid);
            }
        }
    }

    fn port_process_kill_selected(&mut self) {
        let groups = self.get_filtered_process_groups();
        if let Some(group) = groups.get(self.port_process_cursor) {
            let pid = group.pid;
            if pid == 0 {
                self.status_message = Some("无法确定占用进程".to_string());
                return;
            }
            match kill::kill_process(pid, false) {
                Ok(kill::KillResult::Killed) => {
                    self.status_message = Some(format!("{} (PID {}) 已终止", group.process_name, pid));
                    self.record_op(format!("终止 {} (PID {})", group.process_name, pid));
                    self.refresh_ports();
                }
                Ok(kill::KillResult::AlreadyGone) => {
                    self.status_message = Some("进程已不存在".to_string());
                }
                Ok(kill::KillResult::AccessDenied) => {
                    self.status_message = Some("权限不足".to_string());
                }
                Ok(kill::KillResult::Failed(e)) => {
                    self.status_message = Some(format!("终止失败: {}", e));
                }
                Err(e) => {
                    self.status_message = Some(format!("错误: {}", e));
                }
            }
        }
    }

    fn port_remote_move_cursor(&mut self, delta: i32) {
        let groups = self.get_filtered_remote_groups();
        let total = groups.len();
        if total == 0 {
            return;
        }
        let new = self.port_remote_cursor as i32 + delta;
        self.port_remote_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.port_remote_clamp_scroll(20);
    }

    fn port_remote_clamp_scroll(&mut self, page_size: usize) {
        if self.port_remote_cursor < self.port_remote_scroll {
            self.port_remote_scroll = self.port_remote_cursor;
        } else if self.port_remote_cursor >= self.port_remote_scroll + page_size {
            self.port_remote_scroll = self.port_remote_cursor - page_size + 1;
        }
    }

    fn port_remote_show_detail(&mut self) {
        if self.port_detail.is_some() {
            self.port_detail = None;
            return;
        }
        let groups = self.get_filtered_remote_groups();
        if let Some(group) = groups.get(self.port_remote_cursor) {
            if let Some(conn) = group.connections.first() {
                self.port_detail = Some(conn.clone());
            }
        }
    }

    fn port_remote_kill_selected(&mut self) {
        let groups = self.get_filtered_remote_groups();
        if let Some(group) = groups.get(self.port_remote_cursor) {
            let pids: Vec<u32> = group.connections.iter().map(|c| c.pid).filter(|&p| p > 0).collect::<HashSet<_>>().into_iter().collect();
            if pids.is_empty() {
                self.status_message = Some("无可终止进程".to_string());
                return;
            }
            self.pending_kill = Some(KillRequest { pids, force: false });
            self.kill_confirm = true;
        }
    }

    pub fn get_filtered_remote_groups(&self) -> Vec<RemoteGroup> {
        let filtered: Vec<PortEntry> = self.port_entries
            .iter()
            .filter(|e| port_map::matches_filter(e, &self.port_state_filter))
            .cloned()
            .collect();

        let mut groups = RemoteGroup::from_entries(&filtered);

        if !self.port_search_query.is_empty() {
            let query = self.port_search_query.to_lowercase();
            groups.retain(|g| {
                g.remote_addr.to_string().to_lowercase().contains(&query)
                    || g.process_names.iter().any(|n| n.to_lowercase().contains(&query))
            });
        }

        port_map::sort_remote_groups(&mut groups, self.port_remote_sort);
        groups
    }

    fn port_clamp_scroll(&mut self, page_size: usize) {
        if self.port_cursor < self.port_scroll {
            self.port_scroll = self.port_cursor;
        } else if self.port_cursor >= self.port_scroll + page_size {
            self.port_scroll = self.port_cursor - page_size + 1;
        }
    }

    fn refresh_ports(&mut self) {
        let name_map = self.snapshot.process_name_map();
        let af_flags = netstat2::AddressFamilyFlags::IPV4 | netstat2::AddressFamilyFlags::IPV6;
        let proto_flags = netstat2::ProtocolFlags::TCP | netstat2::ProtocolFlags::UDP;
        match netstat2::get_sockets_info(af_flags, proto_flags) {
            Ok(sockets) => {
                self.port_entries = port_map::scan_ports_with_names(&sockets, &name_map).unwrap_or_default();
            }
            Err(_) => {}
        }
    }

    pub fn get_filtered_port_entries(&self) -> Vec<&PortEntry> {
        self.port_entries
            .iter()
            .filter(|e| {
                if let Some(ref proto) = self.port_filter
                    && e.protocol != *proto
                {
                    return false;
                }
                if !port_map::matches_filter(e, &self.port_state_filter) {
                    return false;
                }
                if !self.port_search_query.is_empty() {
                    let query = self.port_search_query.to_lowercase();
                    let matches_port = e.local_port.to_string().contains(&self.port_search_query)
                        || e.remote_port
                            .map(|p| p.to_string().contains(&self.port_search_query))
                            .unwrap_or(false);
                    let matches_name = e.process_name.to_lowercase().contains(&query);
                    return matches_port || matches_name;
                }
                true
            })
            .collect()
    }

    pub fn get_filtered_port_entries_owned(&self) -> Vec<PortEntry> {
        let mut entries: Vec<PortEntry> = self.get_filtered_port_entries()
            .into_iter()
            .cloned()
            .collect();
        port_map::sort_entries(&mut entries, self.port_sort_field);
        entries
    }

    pub fn get_filtered_process_groups(&self) -> Vec<ProcessNetGroup> {
        let filtered: Vec<PortEntry> = self.port_entries
            .iter()
            .filter(|e| port_map::matches_filter(e, &self.port_state_filter))
            .cloned()
            .collect();

        let mut groups = ProcessNetGroup::from_entries(&filtered);

        if !self.port_search_query.is_empty() {
            let query = self.port_search_query.to_lowercase();
            groups.retain(|g| g.process_name.to_lowercase().contains(&query));
        }

        port_map::sort_process_groups(&mut groups, self.port_process_sort);

        // Apply EStats speed data from tick()
        for group in &mut groups {
            if let Some(&(ds, us, td, tu)) = self.port_process_speeds.get(&group.pid) {
                group.down_speed = ds;
                group.up_speed = us;
                group.total_down = td;
                group.total_up = tu;
            }
        }

        groups
    }

    fn compute_group_visual_positions(groups: &[ProcessNetGroup], expanded_pid: Option<u32>) -> Vec<usize> {
        let mut positions = Vec::with_capacity(groups.len());
        let mut visual_row = 0;
        for group in groups {
            positions.push(visual_row);
            visual_row += 1;
            if expanded_pid == Some(group.pid) {
                visual_row += group.connections.len();
            }
        }
        positions
    }

    pub fn anomaly_count(&self) -> usize {
        self.active_anomalies
            .iter()
            .filter(|a| !self.anomaly_dismissed.contains(&a.id()))
            .count()
    }

    pub fn visible_anomalies(&self) -> Vec<&Anomaly> {
        self.active_anomalies
            .iter()
            .filter(|a| !self.anomaly_dismissed.contains(&a.id()))
            .collect()
    }

    pub fn dismiss_anomaly(&mut self, id: &str) {
        self.anomaly_dismissed.insert(id.to_string());
    }

    fn clamp_scroll(&mut self, page_size: usize) {
        if self.cursor_index < self.scroll_offset {
            self.scroll_offset = self.cursor_index;
        } else if self.cursor_index >= self.scroll_offset + page_size {
            self.scroll_offset = self.cursor_index - page_size + 1;
        }
    }

    pub fn tick(&mut self) -> bool {
        let mut needs_draw = self.data_dirty;

        // Replay mode: skip snapshot refresh, advance frames
        if self.mode == AppMode::Replay {
            if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
                self.last_refresh = Instant::now();
                self.replay_tick();
            }
            if self.data_dirty {
                self.rebuild_sorted_cache();
            }
            return true;
        }

        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            self.last_refresh = Instant::now();
            self.snapshot.refresh_light();

            let need_heavy = self.last_heavy_refresh.elapsed() >= HEAVY_REFRESH_INTERVAL;
            if need_heavy {
                self.last_heavy_refresh = Instant::now();
                if let Err(e) = self.snapshot.refresh_heavy() {
                    tracing::warn!("刷新进程列表失败: {}", e);
                }
                self.cached_processes = self.snapshot.processes();

                self.scoring_cursor = 0;

                let alive_pids: HashSet<u32> = self.cached_processes.iter().map(|p| p.pid).collect();
                self.security_scorer.evict_expired();
                self.security_scorer.invalidate_dead(&alive_pids);
                self.security_scores.retain(|pid, _| alive_pids.contains(pid));

                // 详情页实时更新：按 PID 从最新进程列表刷新
                if let Some(ref mut detail) = self.detail_process {
                    let pid = detail.pid;
                    if let Some(latest) = self.cached_processes.iter().find(|p| p.pid == pid) {
                        *detail = latest.clone();
                    } else {
                        self.detail_process = None;
                    }
                }

                self.data_dirty = true;
            }

            // P2: Batch security scoring — process a limited number per tick
            self.security_scorer.reset_budget();
            let scoring_batch = 50;
            let processes = &self.cached_processes;
            let end = (self.scoring_cursor + scoring_batch).min(processes.len());
            if self.scoring_cursor < processes.len() {
                let port_entries = &self.port_entries;
                for proc in &processes[self.scoring_cursor..end] {
                    let score = self.security_scorer.score(proc, processes, port_entries);
                    self.security_scores.insert(proc.pid, score);
                }
                self.scoring_cursor = end;
                if self.scoring_cursor >= processes.len() {
                    self.scoring_cursor = processes.len();
                }
            }

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

            let mut sorted: Vec<&ProcessInfo> = processes.iter().collect();
            sorted.sort_by(|a, b| {
                b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap_or(std::cmp::Ordering::Equal)
            });
            let alive_pids: HashSet<u32> = processes.iter().map(|p| p.pid).collect();
            for proc in sorted.iter().take(MAX_TRACKED) {
                let entry = self.proc_history.entry(proc.pid).or_insert_with(|| {
                    ProcHistory { cpu: VecDeque::new() }
                });
                if entry.cpu.len() >= SPARKLINE_LEN {
                    entry.cpu.pop_front();
                }
                entry.cpu.push_back((proc.cpu_usage * 10.0) as u64);
            }
            self.proc_history.retain(|pid, _| alive_pids.contains(pid));

            // Alert evaluation
            let alert_events = self.alert_manager.evaluate(&self.snapshot, &self.cached_processes);
            for event in &alert_events {
                if let crate::alert::AlertEventType::Fired = event.event_type {
                    match event.severity {
                        crate::alert::AlertSeverity::Critical => {
                            let _ = crate::monitor::notify::send_toast(
                                "proc - Critical Alert",
                                &event.message,
                            );
                        }
                        crate::alert::AlertSeverity::Warning | crate::alert::AlertSeverity::Info => {}
                    }
                }
            }

            if self.mode == AppMode::ProcessTree {
                self.refresh_tree();
            } else if self.mode == AppMode::PortMap {
                self.prev_port_entries = self.port_entries.clone();
                self.refresh_ports();
                self.connection_diff = port_map::diff_connections(&self.prev_port_entries, &self.port_entries);

                if self.connection_history.len() >= SPARKLINE_LEN {
                    self.connection_history.pop_front();
                }
                self.connection_history.push_back(self.connection_diff.active_count);

                self.port_process_groups = ProcessNetGroup::from_entries(&self.port_entries);
                port_map::sort_process_groups(&mut self.port_process_groups, self.port_process_sort);

                self.port_remote_groups = RemoteGroup::from_entries(&self.port_entries);
                port_map::sort_remote_groups(&mut self.port_remote_groups, self.port_remote_sort);

                self.port_process_speeds.clear();
                if let Some(ref mut collector) = self.estats_collector {
                    collector.sample();
                    for group in &self.port_process_groups {
                        let (ds, us, td, tu) = collector.process_speed(group.pid, &group.connections);
                        self.port_process_speeds.insert(group.pid, (ds, us, td, tu));
                    }
                }

                // Anomaly detection
                let new_anomalies = self.anomaly_detector.detect(
                    &self.port_entries,
                    &self.connection_diff,
                    &self.port_process_groups,
                    &self.port_remote_groups,
                );

                // Toast for new Critical anomalies
                let critical_ids: HashSet<String> = new_anomalies
                    .iter()
                    .filter(|a| a.severity == AnomalySeverity::Critical)
                    .map(|a| a.id())
                    .collect();
                for a in &new_anomalies {
                    if a.severity == AnomalySeverity::Critical
                        && !self.prev_critical_ids.contains(&a.id())
                    {
                        let _ = crate::monitor::notify::notify_anomaly("严重", &a.title);
                    }
                }
                self.prev_critical_ids = critical_ids;

                self.active_anomalies = new_anomalies;
            }

            if self.mode == AppMode::UsbAssistant || !self.usb_devices.is_empty() {
                self.usb_refresh_device_list();
            }
            needs_draw = true;
        }

        // Rebuild sorted cache only when data actually changed
        if self.data_dirty {
            self.rebuild_sorted_cache();
            needs_draw = true;
        }

        let total = self.filtered_count();
        if self.cursor_index >= total && total > 0 {
            self.cursor_index = total - 1;
        }

        self.monitor_poll_events();

        if self.mode == AppMode::DockerPanel {
            self.docker_poll_events();
        }

        // Poll diagnostic channel
        if let Some(ref rx) = self.diagnostic_rx {
            const MAX_CONTENT_LINES: usize = 500;
            const MAX_RECV_PER_TICK: usize = 100;
            let mut new_lines = false;
            let mut received = 0;
            loop {
                match rx.try_recv() {
                    Ok(line) => {
                        if let Some(ref mut diag) = self.diagnostic {
                            if diag.content.len() < MAX_CONTENT_LINES {
                                diag.content.push(line);
                            } else if diag.content.len() == MAX_CONTENT_LINES {
                                diag.content.push("... (输出过多，已截断)".to_string());
                            }
                            new_lines = true;
                        }
                        received += 1;
                        if received >= MAX_RECV_PER_TICK {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(ref mut diag) = self.diagnostic {
                            diag.phase = DiagnosticPhase::Completed;
                        }
                        self.diagnostic_rx = None;
                        self.diagnostic_thread = None;
                        new_lines = true;
                        break;
                    }
                }
            }
            // Auto-scroll to bottom if user hasn't manually scrolled
            if new_lines {
                if let Some(ref mut diag) = self.diagnostic {
                    if diag.auto_scroll {
                        // Popup height is 20, minus 4 for borders/title/footer
                        let visible_rows = 16usize;
                        diag.scroll = diag.content.len().saturating_sub(visible_rows) as u16;
                    }
                }
                needs_draw = true;
            }
        }

        needs_draw
    }

    fn filtered_count(&self) -> usize {
        self.cached_sorted.len()
    }

    pub fn get_filtered_sorted_processes(&self) -> &[(classify::ProcessClass, ProcessInfo)] {
        &self.cached_sorted
    }

    fn rebuild_sorted_cache(&mut self) {
        let processes = &self.cached_processes;
        let filtered: Vec<&ProcessInfo> = if self.search_query.is_empty() {
            processes.iter().collect()
        } else {
            let query = self.search_query.to_lowercase();
            processes
                .iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&query)
                        || p.pid.to_string().contains(&self.search_query)
                })
                .collect()
        };

        let mut result: Vec<(classify::ProcessClass, ProcessInfo)> = filtered
            .into_iter()
            .map(|p| (classify::classify_process(p), p.clone()))
            .collect();

        result.sort_by(|a, b| match self.sort_field {
            SortField::Cpu => b.1.cpu_usage.partial_cmp(&a.1.cpu_usage).unwrap_or(std::cmp::Ordering::Equal),
            SortField::Memory => b.1.memory.cmp(&a.1.memory),
            SortField::Pid => a.1.pid.cmp(&b.1.pid),
            SortField::Name => a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()),
            SortField::Security => {
                let sa = self.security_scores.get(&a.1.pid).map(|s| s.score).unwrap_or(100);
                let sb = self.security_scores.get(&b.1.pid).map(|s| s.score).unwrap_or(100);
                sa.cmp(&sb)
            }
        });

        self.cached_sorted = result;
        self.data_dirty = false;
    }

    pub fn get_selected_pids(&self) -> Vec<u32> {
        self.selected_indices
            .iter()
            .filter_map(|&i| self.cached_sorted.get(i).map(|(_, p)| p.pid))
            .collect()
    }

    pub fn sidebar_height(&self) -> u16 {
        let mut h: u16 = 3;
        let (cpu_temp, gpu_temp) = self.snapshot.temperatures();
        if cpu_temp.is_some() || gpu_temp.is_some() {
            h += 1;
        }
        h += 1;
        h += self.snapshot.all_disks().len() as u16;
        h += 1;
        h += 1;
        h += 2;
        let adapters = self.snapshot.net_adapters()
            .iter()
            .filter(|a| a.ipv4.is_some())
            .count() as u16;
        h += adapters;
        h += 1;
        h += 1;
        h += 3;
        h
    }

    // --- USB assistant actions ---

    pub fn usb_scan_devices(&mut self) {
        match eject::scan_all_devices() {
            Ok(devices) => {
                self.usb_devices = devices;
                self.usb_device_cursor = 0;
                if !self.usb_devices.is_empty() {
                    self.usb_select_device();
                } else {
                    self.usb_locks.clear();
                    self.usb_status_message = Some("未检测到可移除设备".to_string());
                }
            }
            Err(e) => {
                self.usb_status_message = Some(format!("扫描失败: {}", e));
            }
        }
    }

    fn usb_refresh_device_list(&mut self) {
        if let Ok(devices) = eject::scan_all_devices() {
            self.usb_devices = devices;
        }
    }

    fn usb_select_device(&mut self) {
        if let Some(dev) = self.usb_devices.get(self.usb_device_cursor) {
            let letter = dev.drive_letter;
            let processes = &self.cached_processes;
            match eject::scan_device_locks_with_processes(letter, processes) {
                Ok(locks) => {
                    let occupied = !locks.is_empty();
                    self.usb_locks = locks;
                    self.usb_lock_cursor = 0;
                    if let Some(dev) = self.usb_devices.get_mut(self.usb_device_cursor) {
                        dev.is_occupied = occupied;
                    }
                    if !occupied {
                        self.usb_status_message = Some(
                            "✅ 无占用进程，可以安全弹出 U 盘了（请手动在系统托盘或文件管理器中弹出）"
                                .to_string(),
                        );
                    } else {
                        self.usb_status_message = None;
                    }
                }
                Err(e) => {
                    self.usb_locks.clear();
                    self.usb_status_message = Some(format!("扫描占用失败: {}", e));
                }
            }
        }
    }

    fn usb_move_cursor(&mut self, delta: i32) {
        let locks = &self.usb_locks;
        let devices = &self.usb_devices;

        if !locks.is_empty() {
            let total = locks.len();
            let new = self.usb_lock_cursor as i32 + delta;
            self.usb_lock_cursor = if new < 0 {
                total - 1
            } else if new as usize >= total {
                0
            } else {
                new as usize
            };
        } else if !devices.is_empty() {
            let total = devices.len();
            let new = self.usb_device_cursor as i32 + delta;
            self.usb_device_cursor = if new < 0 {
                total - 1
            } else if new as usize >= total {
                0
            } else {
                new as usize
            };
        }
    }

    fn usb_kill_safe(&mut self) {
        if let Some(dev) = self.usb_devices.get(self.usb_device_cursor) {
            let letter = dev.drive_letter;
            match eject::kill_safe_processes(letter) {
                Ok((killed, skipped, errors)) => {
                    if killed > 0 {
                        self.usb_status_message = Some(format!(
                            "已终止 {} 个进程，跳过 {} 个{}",
                            killed,
                            skipped,
                            if errors.is_empty() {
                                String::new()
                            } else {
                                format!("，错误: {}", errors.join("; "))
                            }
                        ));
                    } else if skipped > 0 {
                        self.usb_status_message = Some(
                            "无可终止的用户进程，剩余进程为系统/关键进程".to_string(),
                        );
                    } else {
                        self.usb_status_message = Some("无需终止的进程".to_string());
                    }
                    self.usb_select_device();
                }
                Err(e) => {
                    self.usb_status_message = Some(format!("终止失败: {}", e));
                }
            }
        }
    }

    fn usb_refresh(&mut self) {
        self.usb_status_message = Some("重新扫描...".to_string());
        self.usb_scan_devices();
    }

    fn usb_wait_and_monitor(&mut self) {
        if let Some(dev) = self.usb_devices.get(self.usb_device_cursor) {
            self.usb_status_message = Some(
                "持续监测模式已启动（每 5 秒扫描一次）".to_string(),
            );
            let letter = dev.drive_letter;
            match eject::cache::wait_and_monitor(letter, 5, 12) {
                Ok(true) => {
                    self.usb_status_message = Some(
                        "✅ 已无占用进程，可以安全弹出 U 盘了（请手动在系统托盘或文件管理器中弹出）"
                            .to_string(),
                    );
                    self.usb_select_device();
                }
                Ok(false) => {
                    self.usb_status_message = Some(
                        "监测超时，仍有进程占用".to_string(),
                    );
                    self.usb_select_device();
                }
                Err(e) => {
                    self.usb_status_message = Some(format!("监测失败: {}", e));
                }
            }
        }
    }

    fn usb_toggle_focus(&mut self) {
        if !self.usb_devices.is_empty() && !self.usb_locks.is_empty() {
            self.usb_device_cursor = if self.usb_device_cursor < self.usb_devices.len() - 1 {
                self.usb_device_cursor + 1
            } else {
                0
            };
            self.usb_select_device();
        }
    }

    // --- Monitor panel actions ---

    fn monitor_move_cursor(&mut self, delta: i32) {
        let total = self.monitor_manager.list_monitors().len();
        if total == 0 {
            return;
        }
        let new = self.monitor_cursor as i32 + delta;
        self.monitor_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
    }

    fn monitor_delete_selected(&mut self) {
        let id = {
            let monitors = self.monitor_manager.list_monitors();
            monitors.get(self.monitor_cursor).map(|e| e.id)
        };
        if let Some(id) = id {
            // 停止相关的 watchdog 和 port watcher
            self.monitor_watchdog_handles.retain(|h| h.monitor_id != id);
            self.monitor_port_handles.retain(|h| {
                // 找到对应的监控条目
                self.monitor_manager.list_monitors()
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| match &e.target {
                        MonitorTarget::ByPort { port } => h.port != *port,
                        _ => true,
                    })
                    .unwrap_or(true)
            });

            if self.monitor_manager.remove_monitor(id).is_ok() {
                self.monitor_manager.add_notification(format!("已删除监控 ID {}", id));
                self.status_message = Some(format!("已删除监控 ID {}", id));
                self.record_op(format!("删除监控 ID {}", id));
                let total = self.monitor_manager.list_monitors().len();
                if self.monitor_cursor >= total && total > 0 {
                    self.monitor_cursor = total - 1;
                }
            }
        }
    }

    fn monitor_restart_selected(&mut self) {
        let info = {
            let monitors = self.monitor_manager.list_monitors();
            monitors.get(self.monitor_cursor).map(|e| (e.id, e.target.clone(), e.restart_policy.clone()))
        };
        if let Some((id, target, policy)) = info {
            match &target {
                MonitorTarget::ByCommand { cmd, args, cwd } => {
                    let handle = watchdog::spawn_watchdog(
                        id,
                        cmd,
                        args,
                        cwd.as_deref(),
                        policy,
                    );
                    self.monitor_watchdog_handles.push(handle);
                    if let Some(entry) = self.monitor_manager.get_monitor_mut(id) {
                        entry.status = MonitorStatus::Running;
                        entry.crash_count = 0;
                    }
                    self.monitor_manager.add_notification(format!("手动重启命令监控: {}", cmd));
                    self.status_message = Some(format!("已重启命令监控: {}", cmd));
                    self.record_op(format!("重启命令监控: {}", cmd));
                }
                _ => {
                    self.status_message = Some("仅命令监控支持手动重启".to_string());
                }
            }
        }
    }

    fn monitor_toggle_pause(&mut self) {
        let id = {
            let monitors = self.monitor_manager.list_monitors();
            monitors.get(self.monitor_cursor).map(|e| (e.id, e.status))
        };
        if let Some((id, status)) = id
            && let Some(entry) = self.monitor_manager.get_monitor_mut(id)
        {
            entry.status = match status {
                MonitorStatus::Paused => MonitorStatus::Running,
                _ => MonitorStatus::Paused,
            };
            let new_status = entry.status;
            self.monitor_manager.add_notification(format!("监控 ID {} 状态: {}", id, new_status));
            self.status_message = Some(format!("监控 ID {} 状态: {}", id, new_status));
        }
    }

    pub fn monitor_poll_events(&mut self) {
        for handle in &self.monitor_watchdog_handles {
            while let Some(event) = handle.try_recv() {
                match event {
                    WatchdogEvent::Started { monitor_id, pid } => {
                        if let Some(entry) = self.monitor_manager.get_monitor_mut(monitor_id) {
                            entry.pid = Some(pid);
                            entry.status = MonitorStatus::Running;
                        }
                        self.monitor_manager.add_notification(format!("监控进程已启动 (PID {})", pid));
                    }
                    WatchdogEvent::Crashed { monitor_id, exit_code, attempt, restarting } => {
                        if let Some(entry) = self.monitor_manager.get_monitor_mut(monitor_id) {
                            entry.crash_count = attempt;
                            entry.last_crash_time = Some(std::time::SystemTime::now());
                            entry.status = if restarting { MonitorStatus::Running } else { MonitorStatus::Crashed };
                        }
                        self.monitor_manager.add_notification(format!(
                            "进程崩溃 (code: {:?})，第 {} 求重启",
                            exit_code, attempt
                        ));
                    }
                    WatchdogEvent::Stopped { monitor_id, reason } => {
                        if let Some(entry) = self.monitor_manager.get_monitor_mut(monitor_id) {
                            entry.status = MonitorStatus::Stopped;
                        }
                        self.monitor_manager.add_notification(format!("监控停止: {}", reason));
                    }
                    WatchdogEvent::Running { monitor_id, pid } => {
                        if let Some(entry) = self.monitor_manager.get_monitor_mut(monitor_id) {
                            entry.pid = Some(pid);
                        }
                    }
                }
            }
        }

        for handle in &self.monitor_port_handles {
            while let Some(event) = handle.try_recv() {
                match event {
                    PortEvent::Occupied { port, pid, process_name } => {
                        self.monitor_manager.add_notification(format!(
                            "端口 {} 被 {} (PID {}) 占用",
                            port, process_name, pid
                        ));
                    }
                    PortEvent::Released { port } => {
                        self.monitor_manager.add_notification(format!("端口 {} 已释放", port));
                    }
                }
            }
        }
    }
}

// --- Docker panel actions ---

impl App {
    fn handle_docker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Up => self.docker_move_cursor(-1),
            KeyCode::Down => self.docker_move_cursor(1),
            KeyCode::Enter => self.docker_show_detail(),
            KeyCode::Char('r') => self.docker_restart_selected(),
            KeyCode::Char('s') => self.docker_stop_selected(),
            KeyCode::Char('a') => self.docker_start_watching(),
            KeyCode::Esc => {
                if self.docker_detail.is_some() {
                    self.docker_detail = None;
                    self.docker_detail_stats = None;
                } else {
                    self.docker_status_message = None;
                }
            }
            _ => {}
        }
    }

    fn docker_move_cursor(&mut self, delta: i32) {
        let total = self.docker_containers.len();
        if total == 0 {
            return;
        }
        let new = self.docker_cursor as i32 + delta;
        self.docker_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
    }

    pub fn docker_refresh(&mut self) {
        if self.docker_monitor.is_none() {
            match DockerMonitor::connect() {
                Ok(monitor) => {
                    self.docker_monitor = Some(monitor);
                    self.docker_connected = true;
                }
                Err(e) => {
                    self.docker_connected = false;
                    self.docker_status_message = Some(format!("❌ {}", e));
                    return;
                }
            }
        }

        if let Some(ref monitor) = self.docker_monitor {
            match monitor.list_containers(true) {
                Ok(containers) => {
                    self.docker_containers = containers;
                    self.docker_connected = true;
                    if self.docker_status_message
                        .as_ref()
                        .is_none_or(|m| !m.starts_with('✅'))
                    {
                        self.docker_status_message = None;
                    }
                }
                Err(e) => {
                    self.docker_status_message = Some(format!("❌ 获取容器列表失败: {}", e));
                }
            }
        }
    }

    fn docker_restart_selected(&mut self) {
        let name = self
            .docker_containers
            .get(self.docker_cursor)
            .map(|c| c.name.clone());
        if let Some(name) = name
            && let Some(ref monitor) = self.docker_monitor
        {
            match monitor.restart_container(&name) {
                Ok(()) => {
                    self.docker_status_message =
                        Some(format!("✅ 容器 {} 已重启", name));
                    self.docker_refresh();
                }
                Err(e) => {
                    self.docker_status_message =
                        Some(format!("❌ 重启失败: {}", e));
                }
            }
        }
    }

    fn docker_stop_selected(&mut self) {
        let name = self
            .docker_containers
            .get(self.docker_cursor)
            .map(|c| c.name.clone());
        if let Some(name) = name
            && let Some(ref monitor) = self.docker_monitor
        {
            match monitor.stop_container(&name) {
                Ok(()) => {
                    self.docker_status_message =
                        Some(format!("✅ 容器 {} 已停止", name));
                    self.docker_refresh();
                }
                Err(e) => {
                    self.docker_status_message =
                        Some(format!("❌ 停止失败: {}", e));
                }
            }
        }
    }

    fn docker_start_watching(&mut self) {
        if let Some(ref monitor) = self.docker_monitor {
            let docker_client = monitor.docker();
            let receiver = events::spawn_event_watcher(docker_client);
            self.docker_event_receiver = Some(receiver);
            self.docker_status_message = Some("✅ 已开始监听容器事件".to_string());
        } else {
            self.docker_status_message = Some("❌ Docker 未连接，请先刷新".to_string());
        }
    }

    fn docker_show_detail(&mut self) {
        if self.docker_detail.is_some() {
            self.docker_detail = None;
            self.docker_detail_stats = None;
            return;
        }

        let container = self.docker_containers.get(self.docker_cursor).cloned();
        if let Some(c) = container {
            let name = c.name.clone();
            self.docker_detail = Some(c);

            if let Some(ref monitor) = self.docker_monitor {
                self.docker_detail_stats = monitor.get_stats(&name).ok();
            }
        }
    }

    pub fn docker_poll_events(&mut self) {
        let new_events: Vec<DockerEvent> = if let Some(ref receiver) = self.docker_event_receiver {
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

            self.docker_events.insert(0, event);
            if self.docker_events.len() > 100 {
                self.docker_events.truncate(100);
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

        self.docker_refresh();
    }
}

fn frame_tree_node_to_tree_node(f: &FrameTreeNode) -> TreeNode {
    TreeNode {
        pid: f.pid,
        name: f.name.clone(),
        cpu: f.cpu,
        memory: f.memory,
        depth: f.depth,
        children: f.children.iter().map(frame_tree_node_to_tree_node).collect(),
        expanded: f.expanded,
        class: match f.class.as_str() {
            "UserApp" => classify::ProcessClass::UserApp,
            "SystemProcess" => classify::ProcessClass::SystemProcess,
            "WindowsService" => classify::ProcessClass::WindowsService,
            "Kernel" => classify::ProcessClass::Kernel,
            _ => classify::ProcessClass::Unknown,
        },
        is_orphan: f.is_orphan,
        is_zombie: f.is_zombie,
        is_stale: false,
        kill_safety: None,
    }
}

