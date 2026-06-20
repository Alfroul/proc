use std::collections::{HashMap, HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyEvent};

use crate::anomaly::{self, Anomaly, AnomalySeverity};
use crate::app_panel::{KeyResult, KillRequest, Panel, PanelContext};
use crate::collect::ProcessInfo;
use crate::diag::{self, DiagnosticPhase, DiagnosticState, DiagnosticTool};
use crate::estats::EStatsCollector;
use crate::kill;
use crate::port_map::{
    self, ConnectionDiff, NetworkViewMode, PortEntry, ProcessNetGroup, ProcessSortField, Protocol,
    RemoteGroup, RemoteSortField,
};

const PAGE_SIZE: usize = 20;
const SPARKLINE_LEN: usize = 30;

/// 数据缩水后把光标拉回有效区间。total == 0 时光标归零。
fn clamp_cursor(cursor: &mut usize, total: usize) {
    if total == 0 {
        *cursor = 0;
    } else if *cursor >= total {
        *cursor = total - 1;
    }
}

pub struct PortPanel {
    // Core port data
    pub port_entries: Vec<PortEntry>,
    pub prev_port_entries: Vec<PortEntry>,

    // Port view state
    pub port_cursor: usize,
    pub port_scroll: usize,
    pub port_filter: Option<Protocol>,
    pub port_search: crate::search::SearchState,
    pub port_detail: Option<PortEntry>,
    pub port_sort_field: port_map::PortSortField,
    pub port_state_filter: port_map::PortStateFilter,
    pub port_view_mode: NetworkViewMode,
    pub port_is_admin: bool,

    // Process view state
    pub port_process_groups: Vec<ProcessNetGroup>,
    pub port_process_cursor: usize,
    pub port_process_scroll: usize,
    pub port_process_sort: ProcessSortField,
    pub port_expanded_pid: Option<u32>,
    pub port_process_speeds: HashMap<u32, (u64, u64, u64, u64)>,
    pub estats_collector: Option<EStatsCollector>,

    // Remote view state
    pub port_remote_groups: Vec<RemoteGroup>,
    pub port_remote_cursor: usize,
    pub port_remote_scroll: usize,
    pub port_remote_sort: RemoteSortField,

    // Connection tracking
    pub connection_diff: ConnectionDiff,
    pub connection_history: VecDeque<usize>,

    // Filtered caches
    pub cached_filtered_ports: Vec<PortEntry>,
    pub cached_filtered_process_groups: Vec<ProcessNetGroup>,
    pub cached_filtered_remote_groups: Vec<RemoteGroup>,
    pub port_filter_dirty: bool,

    // Anomaly detection
    pub anomaly_detector: anomaly::AnomalyDetector,
    pub active_anomalies: Vec<Anomaly>,
    pub anomaly_dismissed: HashSet<String>,
    pub show_anomaly_detail: bool,
    pub anomaly_cursor: usize,
    pub prev_critical_ids: HashSet<String>,

    // Diagnostics
    pub show_diagnostics: bool,
    pub diagnostic: Option<DiagnosticState>,
    pub diagnostic_rx: Option<std::sync::mpsc::Receiver<String>>,
    pub diagnostic_thread: Option<std::thread::JoinHandle<()>>,

    // 后台 worker 推送的待处理 sockets(由 App::tick_panels 注入)。
    // `None` = 本 tick 无新数据,跳过 ports 重算。
    pub pending_sockets: Option<Vec<netstat2::SocketInfo>>,
}

impl Default for PortPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl PortPanel {
    pub fn new() -> Self {
        let is_admin = crate::collect::is_elevated();
        let port_entries = port_map::scan_ports().unwrap_or_default();

        Self {
            port_entries: port_entries.clone(),
            prev_port_entries: Vec::new(),
            port_cursor: 0,
            port_scroll: 0,
            port_filter: None,
            port_search: crate::search::SearchState::new(),
            port_detail: None,
            port_sort_field: port_map::PortSortField::LocalPort,
            port_state_filter: port_map::PortStateFilter::All,
            port_view_mode: NetworkViewMode::Port,
            port_is_admin: is_admin,
            port_process_groups: Vec::new(),
            port_process_cursor: 0,
            port_process_scroll: 0,
            port_process_sort: ProcessSortField::ConnectionCount,
            port_expanded_pid: None,
            port_process_speeds: HashMap::new(),
            estats_collector: {
                if is_admin {
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
            port_remote_groups: Vec::new(),
            port_remote_cursor: 0,
            port_remote_scroll: 0,
            port_remote_sort: RemoteSortField::ConnectionCount,
            connection_diff: ConnectionDiff::default(),
            connection_history: VecDeque::new(),
            cached_filtered_ports: Vec::new(),
            cached_filtered_process_groups: Vec::new(),
            cached_filtered_remote_groups: Vec::new(),
            port_filter_dirty: true,
            anomaly_detector: anomaly::AnomalyDetector::new(),
            active_anomalies: Vec::new(),
            anomaly_dismissed: HashSet::new(),
            show_anomaly_detail: false,
            anomaly_cursor: 0,
            prev_critical_ids: HashSet::new(),
            show_diagnostics: false,
            diagnostic: None,
            diagnostic_rx: None,
            diagnostic_thread: None,
            pending_sockets: None,
        }
    }

    #[must_use]
    pub fn filtered_ports(&self) -> &[PortEntry] {
        &self.cached_filtered_ports
    }

    #[must_use]
    pub fn filtered_process_groups(&self) -> &[ProcessNetGroup] {
        &self.cached_filtered_process_groups
    }

    #[must_use]
    pub fn filtered_remote_groups(&self) -> &[RemoteGroup] {
        &self.cached_filtered_remote_groups
    }

    #[must_use]
    pub fn anomaly_count(&self) -> usize {
        self.active_anomalies
            .iter()
            .filter(|a| !self.anomaly_dismissed.contains(&a.id()))
            .count()
    }

    #[must_use]
    pub fn visible_anomalies(&self) -> Vec<&Anomaly> {
        self.active_anomalies
            .iter()
            .filter(|a| !self.anomaly_dismissed.contains(&a.id()))
            .collect()
    }

    pub fn dismiss_anomaly(&mut self, id: &str) {
        self.anomaly_dismissed.insert(id.to_string());
    }

    #[must_use]
    pub fn visible_port_count(&self) -> usize {
        match self.port_view_mode {
            NetworkViewMode::Port => self.filtered_ports().len(),
            NetworkViewMode::Process => self.port_process_groups.len(),
            NetworkViewMode::Remote => self.port_remote_groups.len(),
        }
    }

    pub fn refresh_ports(&mut self, name_map: &HashMap<u32, String>) {
        // 仅供 CLI / 测试路径同步使用;TUI 主循环改用后台 PortSnapshotWorker。
        let af_flags = netstat2::AddressFamilyFlags::IPV4 | netstat2::AddressFamilyFlags::IPV6;
        let proto_flags = netstat2::ProtocolFlags::TCP | netstat2::ProtocolFlags::UDP;
        if let Ok(sockets) = netstat2::get_sockets_info(af_flags, proto_flags) {
            self.port_entries =
                port_map::scan_ports_with_names(&sockets, name_map).unwrap_or_default();
        }
    }

    pub fn rebuild_port_filters(&mut self) {
        // Filtered ports
        let mut entries: Vec<PortEntry> = self
            .port_entries
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
                if !self.port_search.query().is_empty() {
                    let query = self.port_search.query().to_lowercase();
                    let matches_port = e.local_port.to_string().contains(self.port_search.query())
                        || e.remote_port
                            .map(|p| p.to_string().contains(self.port_search.query()))
                            .unwrap_or(false);
                    let matches_name = e.process_name.to_lowercase().contains(&query);
                    return matches_port || matches_name;
                }
                true
            })
            .cloned()
            .collect();
        port_map::sort_entries(&mut entries, self.port_sort_field);
        self.cached_filtered_ports = entries;

        // Filtered process groups
        let filtered: Vec<PortEntry> = self
            .port_entries
            .iter()
            .filter(|e| port_map::matches_filter(e, &self.port_state_filter))
            .cloned()
            .collect();
        let mut groups = ProcessNetGroup::from_entries(&filtered);
        if !self.port_search.query().is_empty() {
            let query = self.port_search.query().to_lowercase();
            groups.retain(|g| g.process_name.to_lowercase().contains(&query));
        }
        port_map::sort_process_groups(&mut groups, self.port_process_sort);
        for group in &mut groups {
            if let Some(&(ds, us, td, tu)) = self.port_process_speeds.get(&group.pid) {
                group.down_speed = ds;
                group.up_speed = us;
                group.total_down = td;
                group.total_up = tu;
            }
        }
        self.cached_filtered_process_groups = groups;

        // Filtered remote groups
        let filtered_remote: Vec<PortEntry> = self
            .port_entries
            .iter()
            .filter(|e| port_map::matches_filter(e, &self.port_state_filter))
            .cloned()
            .collect();
        let mut remote_groups = RemoteGroup::from_entries(&filtered_remote);
        if !self.port_search.query().is_empty() {
            let query = self.port_search.query().to_lowercase();
            remote_groups.retain(|g| {
                g.remote_addr.to_string().to_lowercase().contains(&query)
                    || g.process_names
                        .iter()
                        .any(|n| n.to_lowercase().contains(&query))
            });
        }
        port_map::sort_remote_groups(&mut remote_groups, self.port_remote_sort);
        self.cached_filtered_remote_groups = remote_groups;

        self.port_filter_dirty = false;
    }

    #[must_use]
    pub fn compute_group_visual_positions(
        groups: &[ProcessNetGroup],
        expanded_pid: Option<u32>,
    ) -> Vec<usize> {
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

    pub fn close_diagnostic(&mut self) {
        self.diagnostic = None;
        self.diagnostic_rx = None;
        self.diagnostic_thread = None;
        self.show_diagnostics = false;
    }

    pub fn start_diagnostic(&mut self, tool: DiagnosticTool) {
        let Some(ref mut diag) = self.diagnostic else {
            return;
        };
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

    pub fn poll_diagnostic(&mut self) -> bool {
        let mut new_lines = false;
        if let Some(ref rx) = self.diagnostic_rx {
            const MAX_CONTENT_LINES: usize = 500;
            const MAX_RECV_PER_TICK: usize = 100;
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
            if new_lines
                && let Some(ref mut diag) = self.diagnostic
                && diag.auto_scroll
            {
                let visible_rows = 16usize;
                diag.scroll = diag.content.len().saturating_sub(visible_rows) as u16;
            }
        }
        new_lines
    }

    // --- Cursor/clamp helpers ---

    fn port_move_cursor(&mut self, delta: i32) {
        let total = self.filtered_ports().len();
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
        self.port_clamp_scroll(PAGE_SIZE);
    }

    fn port_show_detail(&mut self) {
        if self.port_detail.is_some() {
            self.port_detail = None;
            return;
        }
        if let Some(entry) = self.filtered_ports().get(self.port_cursor) {
            self.port_detail = Some(entry.clone());
        }
    }

    fn port_kill_selected(&mut self, cached_processes: &[ProcessInfo]) -> Option<String> {
        let pid = self.filtered_ports().get(self.port_cursor).map(|e| e.pid)?;
        if pid == 0 {
            return Some("无法确定占用进程".to_string());
        }
        match kill::kill_process(pid, false) {
            Ok(kill::KillResult::Killed) => {
                let name = cached_processes
                    .iter()
                    .find(|p| p.pid == pid)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "?".to_string());
                Some(format!("{} (PID {}) 已终止", name, pid))
            }
            Ok(kill::KillResult::AlreadyGone) => Some("进程已不存在".to_string()),
            Ok(kill::KillResult::AccessDenied) => {
                Some("权限不足 — 请以管理员身份重启 proc".to_string())
            }
            Ok(kill::KillResult::Failed(e)) => Some(format!("终止失败: {}", e)),
            Err(e) => Some(format!("错误: {}", e)),
        }
    }

    fn port_process_move_cursor(&mut self, delta: i32) {
        let total = self.filtered_process_groups().len();
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
        self.port_process_clamp_scroll(PAGE_SIZE);
    }

    fn port_process_clamp_scroll(&mut self, page_size: usize) {
        let groups = self.filtered_process_groups();
        let positions = Self::compute_group_visual_positions(groups, self.port_expanded_pid);
        if let Some(&cursor_visual) = positions.get(self.port_process_cursor) {
            if cursor_visual < self.port_process_scroll {
                self.port_process_scroll = cursor_visual;
            } else if cursor_visual >= self.port_process_scroll + page_size {
                self.port_process_scroll = cursor_visual - page_size + 1;
            }
        }
    }

    fn port_process_toggle_expand(&mut self) {
        if let Some(group) = self.filtered_process_groups().get(self.port_process_cursor) {
            self.port_expanded_pid = if self.port_expanded_pid == Some(group.pid) {
                None
            } else {
                Some(group.pid)
            };
        }
    }

    fn port_process_kill_selected(&mut self) -> Option<String> {
        let groups = self.filtered_process_groups().to_vec();
        let group = groups.get(self.port_process_cursor)?;
        let pid = group.pid;
        if pid == 0 {
            return Some("无法确定占用进程".to_string());
        }
        match kill::kill_process(pid, false) {
            Ok(kill::KillResult::Killed) => {
                Some(format!("{} (PID {}) 已终止", group.process_name, pid))
            }
            Ok(kill::KillResult::AlreadyGone) => Some("进程已不存在".to_string()),
            Ok(kill::KillResult::AccessDenied) => {
                Some("权限不足 — 请以管理员身份重启 proc".to_string())
            }
            Ok(kill::KillResult::Failed(e)) => Some(format!("终止失败: {}", e)),
            Err(e) => Some(format!("错误: {}", e)),
        }
    }

    fn port_remote_move_cursor(&mut self, delta: i32) {
        let total = self.filtered_remote_groups().len();
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
        self.port_remote_clamp_scroll(PAGE_SIZE);
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
        let groups = self.filtered_remote_groups().to_vec();
        if let Some(group) = groups.get(self.port_remote_cursor)
            && let Some(conn) = group.connections.first()
        {
            self.port_detail = Some(conn.clone());
        }
    }

    fn port_remote_kill_selected(&mut self) -> Option<KillRequest> {
        let groups = self.filtered_remote_groups().to_vec();
        let group = groups.get(self.port_remote_cursor)?;
        let pids: Vec<u32> = group
            .connections
            .iter()
            .map(|c| c.pid)
            .filter(|&p| p > 0)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if pids.is_empty() {
            return None;
        }
        Some(KillRequest { pids, force: false })
    }

    fn port_clamp_scroll(&mut self, page_size: usize) {
        if self.port_cursor < self.port_scroll {
            self.port_scroll = self.port_cursor;
        } else if self.port_cursor >= self.port_scroll + page_size {
            self.port_scroll = self.port_cursor - page_size + 1;
        }
    }

    fn handle_anomaly_detail_key(&mut self, key: KeyEvent) -> KeyResult {
        let visible = self.visible_anomalies();
        match key.code {
            KeyCode::Char('a') | KeyCode::Esc => {
                self.show_anomaly_detail = false;
            }
            KeyCode::Up if self.anomaly_cursor > 0 => {
                self.anomaly_cursor -= 1;
            }
            KeyCode::Down if self.anomaly_cursor + 1 < visible.len() => {
                self.anomaly_cursor += 1;
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
        KeyResult::Consumed
    }

    fn handle_diagnostic_key(&mut self, key: KeyEvent) -> KeyResult {
        let Some(ref mut diag) = self.diagnostic else {
            return KeyResult::Consumed;
        };
        match key.code {
            KeyCode::Esc => {
                self.close_diagnostic();
            }
            KeyCode::Up => {
                diag.scroll = diag.scroll.saturating_sub(3);
                diag.auto_scroll = false;
            }
            KeyCode::Down => {
                diag.scroll += 3;
                diag.auto_scroll = false;
            }
            KeyCode::Char('p') | KeyCode::Char('P') if diag.phase == DiagnosticPhase::Menu => {
                self.start_diagnostic(DiagnosticTool::Ping);
            }
            KeyCode::Char('d') | KeyCode::Char('D') if diag.phase == DiagnosticPhase::Menu => {
                self.start_diagnostic(DiagnosticTool::DnsReverse);
            }
            KeyCode::Char('w') | KeyCode::Char('W') if diag.phase == DiagnosticPhase::Menu => {
                self.start_diagnostic(DiagnosticTool::Whois);
            }
            KeyCode::Char('t') | KeyCode::Char('T') if diag.phase == DiagnosticPhase::Menu => {
                self.start_diagnostic(DiagnosticTool::Traceroute);
            }
            KeyCode::Char('s') | KeyCode::Char('S') if diag.phase == DiagnosticPhase::Menu => {
                self.start_diagnostic(DiagnosticTool::PortScan);
            }
            KeyCode::Enter
                if diag.phase == DiagnosticPhase::Completed
                    || diag.phase == DiagnosticPhase::Failed =>
            {
                self.close_diagnostic();
            }
            _ => {}
        }
        KeyResult::Consumed
    }
}

impl Panel for PortPanel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        // Handle search
        if self.port_search.is_active() {
            if self.port_search.handle_input(key) {
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_process_cursor = 0;
                self.port_process_scroll = 0;
                self.port_remote_cursor = 0;
                self.port_remote_scroll = 0;
                self.port_filter_dirty = true;
            }
            return KeyResult::Consumed;
        }

        // Anomaly detail overlay
        if self.show_anomaly_detail {
            return self.handle_anomaly_detail_key(key);
        }

        // Diagnostic overlay
        if self.show_diagnostics {
            return self.handle_diagnostic_key(key);
        }

        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
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
            KeyCode::Char('/') => {
                self.port_search.active = true;
            }
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
                self.port_filter_dirty = true;
            }
            KeyCode::Esc => {
                if self.show_anomaly_detail {
                    self.show_anomaly_detail = false;
                } else {
                    self.port_search.clear();
                    self.port_detail = None;
                    self.port_expanded_pid = None;
                    *ctx.status_message = None;
                    self.port_filter_dirty = true;
                }
            }
            KeyCode::Up => match self.port_view_mode {
                NetworkViewMode::Process => self.port_process_move_cursor(-1),
                NetworkViewMode::Remote => self.port_remote_move_cursor(-1),
                NetworkViewMode::Port => self.port_move_cursor(-1),
            },
            KeyCode::Down => match self.port_view_mode {
                NetworkViewMode::Process => self.port_process_move_cursor(1),
                NetworkViewMode::Remote => self.port_remote_move_cursor(1),
                NetworkViewMode::Port => self.port_move_cursor(1),
            },
            KeyCode::Enter => match self.port_view_mode {
                NetworkViewMode::Process => self.port_process_toggle_expand(),
                NetworkViewMode::Remote => self.port_remote_show_detail(),
                NetworkViewMode::Port => self.port_show_detail(),
            },
            KeyCode::Char('k') => match self.port_view_mode {
                NetworkViewMode::Port => {
                    if let Some(msg) = self.port_kill_selected(ctx.cached_processes) {
                        *ctx.status_message = Some(msg);
                    }
                }
                NetworkViewMode::Process => {
                    if let Some(msg) = self.port_process_kill_selected() {
                        *ctx.status_message = Some(msg);
                    }
                }
                NetworkViewMode::Remote => {
                    if let Some(req) = self.port_remote_kill_selected() {
                        *ctx.pending_kill = Some(req);
                    }
                }
            },
            KeyCode::Char('x') => {
                if let Some(entry) = self.filtered_ports().get(self.port_cursor)
                    && let Some(ip) = entry.remote_addr
                    && !ip.is_unspecified()
                {
                    self.diagnostic = Some(DiagnosticState::new(ip));
                    self.show_diagnostics = true;
                }
            }
            KeyCode::Char('c') => {
                let text = match self.port_view_mode {
                    NetworkViewMode::Port => self.filtered_ports().get(self.port_cursor).map(|e| {
                        format!(
                            "{}:{} → {}:{} ({}) [{}]",
                            e.local_addr,
                            e.local_port,
                            e.remote_addr.map(|a| a.to_string()).unwrap_or_default(),
                            e.remote_port.map(|p| p.to_string()).unwrap_or_default(),
                            e.protocol,
                            e.process_name
                        )
                    }),
                    NetworkViewMode::Process => self
                        .filtered_process_groups()
                        .get(self.port_process_cursor)
                        .map(|g| {
                            format!(
                                "{} (PID {}): {} connections",
                                g.process_name,
                                g.pid,
                                g.connections.len()
                            )
                        }),
                    NetworkViewMode::Remote => self
                        .filtered_remote_groups()
                        .get(self.port_remote_cursor)
                        .map(|g| format!("{}: {} connections", g.remote_addr, g.connections.len())),
                };
                if let Some(t) = text {
                    let _ = arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&t));
                    *ctx.status_message = Some("已复制到剪贴板".to_string());
                }
            }
            KeyCode::Char('s') => {
                self.port_sort_field = self.port_sort_field.next();
                self.port_cursor = 0;
                self.port_scroll = 0;
                self.port_filter_dirty = true;
                *ctx.status_message = Some(format!("排序: {}", self.port_sort_field.label()));
            }
            KeyCode::PageUp => match self.port_view_mode {
                NetworkViewMode::Port => {
                    self.port_cursor = self.port_cursor.saturating_sub(PAGE_SIZE);
                    self.port_clamp_scroll(PAGE_SIZE);
                }
                NetworkViewMode::Process => {
                    self.port_process_cursor = self.port_process_cursor.saturating_sub(PAGE_SIZE);
                    self.port_process_clamp_scroll(PAGE_SIZE);
                }
                NetworkViewMode::Remote => {
                    self.port_remote_cursor = self.port_remote_cursor.saturating_sub(PAGE_SIZE);
                    self.port_remote_clamp_scroll(PAGE_SIZE);
                }
            },
            KeyCode::PageDown => match self.port_view_mode {
                NetworkViewMode::Port => {
                    let total = self.filtered_ports().len();
                    self.port_cursor = (self.port_cursor + PAGE_SIZE).min(total.saturating_sub(1));
                    self.port_clamp_scroll(PAGE_SIZE);
                }
                NetworkViewMode::Process => {
                    let total = self.filtered_process_groups().len();
                    self.port_process_cursor =
                        (self.port_process_cursor + PAGE_SIZE).min(total.saturating_sub(1));
                    self.port_process_clamp_scroll(PAGE_SIZE);
                }
                NetworkViewMode::Remote => {
                    let total = self.filtered_remote_groups().len();
                    self.port_remote_cursor =
                        (self.port_remote_cursor + PAGE_SIZE).min(total.saturating_sub(1));
                    self.port_remote_clamp_scroll(PAGE_SIZE);
                }
            },
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    fn tick(&mut self, ctx: &mut PanelContext) -> bool {
        let mut needs_draw = false;

        // 仅在拿到新 sockets(后台 worker 每 ~3s 推一次)时做重算;
        // 无新数据时跳过 diff/group/anomaly —— 这些计算依赖 port_entries,
        // 数据没变时重算毫无意义。
        if let Some(sockets) = self.pending_sockets.take() {
            self.prev_port_entries = self.port_entries.clone();
            let name_map = ctx.snapshot.process_name_map();
            self.port_entries =
                port_map::scan_ports_with_names(&sockets, &name_map).unwrap_or_default();
            self.connection_diff =
                port_map::diff_connections(&self.prev_port_entries, &self.port_entries);

            if self.connection_history.len() >= SPARKLINE_LEN {
                self.connection_history.pop_front();
            }
            self.connection_history
                .push_back(self.connection_diff.active_count);

            // Process groups
            self.port_process_groups = ProcessNetGroup::from_entries(&self.port_entries);
            port_map::sort_process_groups(&mut self.port_process_groups, self.port_process_sort);

            // Remote groups
            self.port_remote_groups = RemoteGroup::from_entries(&self.port_entries);
            port_map::sort_remote_groups(&mut self.port_remote_groups, self.port_remote_sort);

            // EStats speeds
            self.port_process_speeds.clear();
            if let Some(ref mut collector) = self.estats_collector {
                collector.sample();
                for group in &self.port_process_groups {
                    let (ds, us, td, tu) = collector.process_speed(group.pid, &group.connections);
                    self.port_process_speeds.insert(group.pid, (ds, us, td, tu));
                }
            }

            self.port_filter_dirty = true;

            // Anomaly detection
            let new_anomalies = self.anomaly_detector.detect(
                &self.port_entries,
                &self.connection_diff,
                &self.port_process_groups,
                &self.port_remote_groups,
            );

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

            needs_draw = true;
        }

        // Poll diagnostic
        if self.poll_diagnostic() {
            needs_draw = true;
        }

        // Rebuild filters
        if self.port_filter_dirty {
            self.rebuild_port_filters();
            needs_draw = true;
        }

        // Clamp cursors so 端口消失 / 搜索过滤后光标不至于越界。各 move_cursor
        // 走 wraparound，不 clamp 时第一次按键会产生奇怪跳变。
        // 先把长度算出来，避免借 `&mut self.port_cursor` 的同时再借 `&self`。
        let port_total = self.filtered_ports().len();
        let proc_total = self.filtered_process_groups().len();
        let remote_total = self.filtered_remote_groups().len();
        clamp_cursor(&mut self.port_cursor, port_total);
        clamp_cursor(&mut self.port_process_cursor, proc_total);
        clamp_cursor(&mut self.port_remote_cursor, remote_total);

        needs_draw
    }

    fn cursor(&self) -> usize {
        match self.port_view_mode {
            NetworkViewMode::Port => self.port_cursor,
            NetworkViewMode::Process => self.port_process_cursor,
            NetworkViewMode::Remote => self.port_remote_cursor,
        }
    }

    fn scroll(&self) -> usize {
        match self.port_view_mode {
            NetworkViewMode::Port => self.port_scroll,
            NetworkViewMode::Process => self.port_process_scroll,
            NetworkViewMode::Remote => self.port_remote_scroll,
        }
    }
}
