use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};

const SPARKLINE_LEN: usize = 30;
const MAX_TRACKED: usize = 50;
const MAX_OP_HISTORY: usize = 100;
/// DNS 查询日志内存缓冲上限（FIFO 丢弃最旧）。stage-8.md 验收要求 cap=1000。
/// 隐私：DNS 查询含敏感信息，永不持久化到磁盘（仅内存 + UI 实时显示）。
const DNS_LOG_BUFFER_CAP: usize = 1000;

// Re-export types that moved to app_panel
pub use crate::app_panel::{AppGroupSortField, AppMode, KillRequest, MonitorAddSubmenu, OpRecord};
// v0.6.0 阶段 5：ReplaySpeed / TimelineState 搬到 `crate::replay`，
// 这里 re-export 让 `crate::app::ReplaySpeed` 等旧路径继续可用（TUI 不动 import）。
pub use crate::replay::{ReplaySpeed, TimelineState};

use crate::alert::AlertManager;
use crate::app_panel::{KeyResult, Panel, PanelContext};
use crate::classify;
use crate::collect::{
    HEAVY_REFRESH_INTERVAL, ProcessInfo, ProcessViewMode, REFRESH_INTERVAL, SortField,
    SystemSnapshot,
};
use crate::docker::ContainerInfo;
use crate::docker::exec::ContainerExec;
use crate::eject::classify::HandleRisk;
use crate::eject::{HandleLock, RemovableDevice};
use crate::error::Result;
use crate::inspect::{InspectorAction, InspectorController};
use crate::port_map::{self, NetworkViewMode, PortEntry};
use crate::record::Player;
use crate::replay::{ReplayAction, ReplayController};
use crate::security::{BackgroundScorer, SecurityScore};
use crate::tree::TreeNode;
use crate::view_models::DockerPanel;
use crate::view_models::MonitorPanel;
use crate::view_models::PortPanel;
use crate::view_models::ProcessPanel;
use crate::view_models::UsbPanel;

pub struct App {
    pub mode: AppMode,
    pub snapshot: SystemSnapshot,
    pub cached_processes: Arc<Vec<ProcessInfo>>,
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

    // v0.6.0 阶段 5：后台采集 worker（port / usb / net_flow / dns_log）统一由
    // `WorkerManager` 持有，详见 `src/workers/manager.rs`。Docker logs worker
    // 仍由 `DockerPanel` 自管（生命周期与 panel 绑定）。
    pub workers: crate::workers::WorkerManager,
    // 阶段 8 D3：DNS 查询日志（worker drain 出来的最新 N 条）。worker 句柄在
    // `self.workers.dns_log_worker`；这里只存数据。cap=1000 FIFO；仅内存缓冲，
    // 录屏（record/）路径不序列化任何 DNS 数据（隐私）。
    pub dns_log_recent: VecDeque<crate::dns_log::DnsQuery>,

    // 阶段 9 E2：容器 exec 嵌入式 PTY。
    // container_exec = Some 仅在 AppMode::ContainerExec 时；其他模式必须为 None。
    // vt100 parser 同生命周期，避免重新创建丢历史输出。
    pub container_exec: Option<ContainerExec>,
    pub container_exec_vt: Option<vt100::Parser>,
    /// DockerPanel 按 `e` 时设置目标容器名；App::switch_mode(ContainerExec) 取出启动。
    pub pending_container_exec_target: Option<String>,
    /// exec 模式退出时给用户的一次性提示（如「容器已退出」），渲染一次后清空。
    pub container_exec_exit_msg: Option<String>,

    // Global state
    pub status_message: Option<String>,
    pub kill_confirm: bool,
    pub pending_kill: Option<KillRequest>,

    // Inspector (v0.6.0 阶段 5：从 App 上帝对象拆出，集中持详情页状态)。
    pub inspector: InspectorController,

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

    // Replay state (v0.6.0 阶段 5：从 App 上帝对象拆出，集中在 ReplayController)。
    // 字段名保持 `replay_player` / `timeline_state`，嵌套多一层 `.replay.` 前缀。
    pub replay: ReplayController,

    // Recording
    recording_wanted: bool,
    recording_elapsed_secs: u64,
    /// v0.6.0 阶段 2：用户按 `R` 启动录屏时进入「待确认」状态，等 `y/n` 决定。
    pub pending_record_confirm: bool,

    // Throttle
    pub throttle_info: Option<crate::throttle::ThrottleInfo>,
    pub throttle_reason: crate::throttle::ThrottleReason,

    // Per-process disk speed
    // 键 = (pid, start_time)：避免 PID 复用后把死进程的累计 IO 算到新进程头上（ADR-0003）。
    prev_process_disk: HashMap<(u32, u64), (u64, u64)>,
    prev_process_disk_time: Instant,

    // Self-monitoring：proc 自身的 ProcessInfo 快照（从 cached_processes 按 PID 找）
    pub self_proc: Option<ProcessInfo>,

    // Platform
    pub is_windows: bool,

    // Sidebar 折叠/展开状态：阶段 2 B2，按 `c` 切换；持久化到 ui.toml。
    // 折叠 = 现有 sidebar 视觉；展开 = per-core CPU 频率/温度表格。
    pub sidebar_expanded: bool,

    // v0.6.0 阶段 3：worker 可观测性 + crash 报告。
    // crash_rx 接收所有 SnapshotWorker（port/usb/net_flow/dns_log/docker）
    // 在 catch_unwind 后 best-effort 发送的 WorkerCrash。CLI 模式（如 `proc
    // diag`）不消费 → 字段为 None，避免无人 recv 堆积。
    pub crash_rx: Option<std::sync::mpsc::Receiver<crate::metrics::crash::WorkerCrash>>,
    /// worker 崩溃后保留的最近 N 条 crash，TUI 在顶部渲染 banner。
    /// `tick()` 时 drain `crash_rx` 追加；用户按 `D` 在 `handle_key` 里清空。
    pub active_crashes: Vec<crate::metrics::crash::WorkerCrash>,
}

pub struct ProcHistory {
    pub cpu: VecDeque<u64>,
}

impl App {
    pub fn new() -> Result<Self> {
        crate::tui::theme::init_persisted_theme();

        let mut snapshot = SystemSnapshot::new()?;
        let _ = snapshot.refresh_heavy_incremental();
        let processes = snapshot.cached_processes_arc();
        let (_, mem_total) = snapshot.memory_usage();
        let port_entries = port_map::scan_ports().unwrap_or_default();

        let mut process_panel = ProcessPanel::new(&processes[..]);
        process_panel.init_tree(&processes[..], mem_total);

        let mut port_panel = PortPanel::new();
        port_panel.port_entries = port_entries;

        let is_windows = cfg!(target_os = "windows");
        let status_message = if crate::ui_state::load_first_run() {
            // 首次启动：Windows 只提示帮助入口；Linux/macOS 把降级清单也带上，
            // 否则用户按 ? 写盘 first_run=false 后再也看不到降级清单了。
            if is_windows {
                Some("首次使用？按 ? 查看快捷键".to_string())
            } else {
                Some(
                    "首次使用？按 ? 查看快捷键 — Linux/macOS 模式：以下功能已降级 — 安全评分签名验证/DLL检查/特权检查、降频检测、U盘助手、Toast 通知、EStats 带宽、GPU；进程分类走启发式（按路径推断，无 Service Cache）。详见 README 平台支持表。"
                        .to_string(),
                )
            }
        } else if is_windows {
            None
        } else {
            Some(
                "Linux/macOS 模式：以下功能已降级 — 安全评分签名验证/DLL检查/特权检查、降频检测、U盘助手、Toast 通知、EStats 带宽、GPU；进程分类走启发式（按路径推断，无 Service Cache）。详见 README 平台支持表。"
                    .to_string(),
            )
        };

        // v0.6.0 阶段 3：crash channel —— 给所有 SnapshotWorker 共享。
        // v0.6.0 阶段 5：4 个直管 worker 的 spawn 统一进 WorkerManager。
        let (crash_tx, crash_rx) = crate::metrics::crash::channel();
        let workers = crate::workers::WorkerManager::new(Some(&crash_tx));
        let mut docker_panel = DockerPanel::new();
        docker_panel.crash_tx = Some(crash_tx);

        Ok(Self {
            mode: AppMode::ProcessList,
            snapshot,
            cached_processes: Arc::clone(&processes),
            should_quit: false,
            last_refresh: Instant::now(),
            last_heavy_refresh: Instant::now() - HEAVY_REFRESH_INTERVAL,
            pending_redraw: true,
            process_panel,
            port_panel,
            usb_panel: UsbPanel::new(),
            monitor_panel: MonitorPanel::new(),
            docker_panel,
            workers,
            dns_log_recent: VecDeque::new(),
            container_exec: None,
            container_exec_vt: None,
            pending_container_exec_target: None,
            container_exec_exit_msg: None,
            status_message,
            kill_confirm: false,
            pending_kill: None,
            inspector: InspectorController::new(),
            proc_history: HashMap::new(),
            global_cpu_history: VecDeque::new(),
            global_mem_history: VecDeque::new(),
            op_history: VecDeque::new(),
            background_scorer: BackgroundScorer::new(),
            security_scores: HashMap::new(),
            scoring_pending: false,
            cached_sorted: Vec::new(),
            data_dirty: true,
            alert_manager: AlertManager::try_load().unwrap_or_else(|e| {
                tracing::warn!("加载告警配置失败，使用默认规则: {}", e);
                AlertManager::default()
            }),
            alert_popup_open: false,
            alert_scroll: 0,
            help_scroll: 0,
            replay: ReplayController::new(),
            recording_wanted: false,
            recording_elapsed_secs: 0,
            pending_record_confirm: false,
            throttle_info: None,
            throttle_reason: crate::throttle::ThrottleReason::None,
            prev_process_disk: HashMap::new(),
            prev_process_disk_time: Instant::now(),
            self_proc: None,
            is_windows,
            sidebar_expanded: crate::ui_state::load_sidebar_expanded(),
            crash_rx: Some(crash_rx),
            active_crashes: Vec::new(),
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

    // --- v0.6.0 阶段 3：worker 可观测性 ---

    /// 聚合所有 SnapshotWorker 的 metrics 快照。`proc diag` + `?` 帮助页消费。
    ///
    /// Light/Heavy/Smart worker 由 `SystemSnapshot` 内部持有（非 SnapshotWorker
    /// 模板），阶段 5 WorkerManager 重构时统一接入；当前阶段不暴露。
    #[must_use]
    pub fn worker_metrics(&self) -> Vec<crate::metrics::NamedWorkerStats> {
        // v0.6.0 阶段 5：4 个直管 worker 的 metrics 走 WorkerManager 聚合。
        let mut out = self.workers.metrics_snapshot();
        if let Some(w) = self.docker_panel.snapshot_worker.as_ref() {
            out.push(crate::metrics::NamedWorkerStats {
                name: "docker",
                stats: w.metrics.snapshot(),
            });
        }
        out
    }

    /// 主循环 tick 调一次：drain `crash_rx`，新到的 `WorkerCrash` 追加到
    /// `active_crashes`，触发 TUI 顶部 banner 渲染。
    pub fn poll_crashes(&mut self) {
        let Some(rx) = &self.crash_rx else {
            return;
        };
        while let Ok(crash) = rx.try_recv() {
            tracing::error!(
                worker = crash.worker,
                panic = %crash.message,
                "worker crashed (banner shown)"
            );
            self.active_crashes.push(crash);
            // 上限 10 条防止失忆式增长 —— 用户按 D 清空。
            if self.active_crashes.len() > 10 {
                self.active_crashes.drain(0..1);
            }
            self.pending_redraw = true;
        }
    }

    /// 清空所有 banner（用户按 D 触发）。
    pub fn dismiss_all_crashes(&mut self) {
        if !self.active_crashes.is_empty() {
            self.active_crashes.clear();
            self.pending_redraw = true;
        }
    }

    fn toggle_recording(&mut self) {
        if self.recording_wanted {
            // 录屏中再按 R → 直接停止（原行为）。
            self.recording_wanted = false;
            self.pending_record_confirm = false;
            self.status_message = Some("录屏已停止".to_string());
            return;
        }
        // 启动前弹确认 — 录屏会捕获屏幕所有内容（DNS 域名 / 进程 cmd / env 真值
        // 切换虽会强制复位但仍可能漏一帧），需要用户显式同意。
        self.pending_record_confirm = true;
        self.status_message =
            Some("⚠ 录屏会捕获屏幕所有内容（含 DNS 域名 / 进程 cmd）。y 确认 / n 取消".to_string());
    }

    /// v0.6.0 阶段 2：处理 `pending_record_confirm` 状态下的按键。
    /// 返回 `true` 表示已消费（调用方应 short-circuit）。
    fn handle_record_confirm(&mut self, key: KeyEvent) -> bool {
        if !self.pending_record_confirm {
            return false;
        }
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.pending_record_confirm = false;
                self.recording_wanted = true;
                // 录屏启动同时强制复位 env_reveal，防录到切换前残留的 reveal 帧。
                self.inspector.env_reveal = false;
                self.status_message = Some("录屏中… (R 停止)".to_string());
            }
            KeyCode::Char('n')
            | KeyCode::Char('N')
            | KeyCode::Esc
            | KeyCode::Char('q')
            | KeyCode::Char('Q') => {
                self.pending_record_confirm = false;
                self.status_message = Some("录屏已取消".to_string());
            }
            _ => {
                // 其他键吞掉，等用户选 y/n；不传递给下层 panel 防误触。
            }
        }
        true
    }

    pub fn replay_frame_mode(&self) -> AppMode {
        self.replay.frame_mode()
    }

    pub fn start_replay(&mut self, player: Player) {
        self.replay.start(player);
        self.mode = AppMode::Replay;
        self.apply_replay_frame();
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
            KeyCode::Char('c') => {
                // Sidebar 折叠/展开：阶段 2 B2。v0.6.0 阶段 6 起，详情页复制
                // 迁移到 `y`（vim yank），`c` 在所有模式下统一为侧边栏折叠，
                // 不再有「详情页 vs 全局」双语义冲突。
                self.sidebar_expanded = !self.sidebar_expanded;
                crate::ui_state::save_sidebar_expanded(self.sidebar_expanded);
                self.status_message = Some(if self.sidebar_expanded {
                    "侧边栏：展开（per-core 频率/温度）".to_string()
                } else {
                    "侧边栏：折叠".to_string()
                });
                true
            }
            KeyCode::Char('A') => {
                self.alert_popup_open = !self.alert_popup_open;
                self.alert_scroll = 0;
                true
            }
            KeyCode::Char('?') => {
                // 进入 Help 视为"看过帮助"，立即写盘 first_run=false，下次启动不再显示引导。
                crate::ui_state::mark_first_run_done();
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

        // v0.6.0 阶段 3：worker 崩溃 banner — 按 D 清空，优先级高于其他绑定。
        if key.code == KeyCode::Char('D') && !self.active_crashes.is_empty() {
            self.dismiss_all_crashes();
            return;
        }

        // v0.6.0 阶段 2：录屏待确认状态 — 拦截所有按键，等 y/n
        if self.pending_record_confirm {
            // 唯一例外：再按 R 视为取消（用户改主意）
            if key.code == KeyCode::Char('R') {
                self.pending_record_confirm = false;
                self.status_message = Some("录屏已取消".to_string());
                return;
            }
            self.handle_record_confirm(key);
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
            || self.port_panel.port_search.is_active()
            || self.inspector.inspection_search.is_active();
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
                cached_processes: &self.cached_processes[..],
                cached_sorted: &self.cached_sorted,
                security_scores: &self.security_scores,
                status_message: &mut self.status_message,
                detail_process: &mut self.inspector.detail_process,
                pending_kill: &mut self.pending_kill,
                data_dirty: &mut self.data_dirty,
                pending_redraw: &mut self.pending_redraw,
                alert_manager: &mut self.alert_manager,
                op_history: &mut self.op_history,
                dns_log_recent: &mut self.dns_log_recent,
                pending_container_exec: &mut self.pending_container_exec_target,
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
                AppMode::ContainerExec => {
                    let _ = ctx;
                    self.handle_container_exec_key(key);
                    return;
                }
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
        // 退出 ContainerExec 时必须 drop PTY + parser，避免 fd 泄漏。
        if self.mode == AppMode::ContainerExec && mode != AppMode::ContainerExec {
            let exit_msg = self
                .container_exec
                .as_ref()
                .map(|ce| format!("✅ 已退出容器 {}", ce.container));
            self.container_exec = None;
            self.container_exec_vt = None;
            if let Some(m) = exit_msg {
                self.container_exec_exit_msg = Some(m);
            }
        }
        if mode == AppMode::ProcessList {
            self.process_panel.process_view_mode = ProcessViewMode::List;
            self.process_panel.cursor_index = 0;
            self.process_panel.scroll_offset = 0;
        }
        if mode == AppMode::UsbAssistant && self.mode != AppMode::UsbAssistant {
            self.usb_panel.scan_devices(&self.cached_processes[..]);
        }
        if mode == AppMode::DockerPanel && self.mode != AppMode::DockerPanel {
            self.docker_panel.refresh();
            if self.docker_panel.connected && self.docker_panel.event_receiver.is_none() {
                self.docker_panel.start_watching();
            }
        }
        // 进入详情页时预加载 Inspector 数据：env/dlls/handles/memory 一次采集（同步）。
        // net 复用 port_panel.port_entries（后台 worker 每 ~3s 已推新），
        // 避免再调一次 scan_ports 的几百毫秒 syscall 卡帧。
        // 失败的子项会退化为空 Vec，TUI 层在 Tab 内显示「无数据」。
        if mode == AppMode::ProcessDetail {
            let ports_snapshot: Vec<crate::port_map::PortEntry> =
                self.port_panel.port_entries.clone();
            // v0.6.0 阶段 5：详情页初始化整体封装到 InspectorController::open。
            self.inspector.open(&ports_snapshot);
        }
        self.mode = mode;
        self.process_panel.search.clear();
        self.status_message = None;
        self.data_dirty = true;

        // 进入 ContainerExec：从 pending_container_exec_target 启动 PTY + vt100 parser。
        if mode == AppMode::ContainerExec {
            self.enter_container_exec();
        }
    }

    /// 从 `pending_container_exec_target` 启动 ContainerExec + vt100 Parser。
    /// 失败（Docker 未连接 / spawn 失败）时回退到 DockerPanel 并提示错误。
    fn enter_container_exec(&mut self) {
        let target = match self.pending_container_exec_target.take() {
            Some(t) => t,
            None => {
                self.container_exec_exit_msg = Some("❌ 未指定 exec 目标容器".to_string());
                self.mode = AppMode::DockerPanel;
                return;
            }
        };

        // 拿 image 用于 detect_default_shell（用户没显式传 cmd 时）。
        let image = self
            .docker_panel
            .containers
            .iter()
            .find(|c| c.name == target || c.id.starts_with(&target))
            .map(|c| c.image.clone());

        // 检查容器在运行状态（docker exec 需要容器 running）。
        let is_running = self
            .docker_panel
            .containers
            .iter()
            .find(|c| c.name == target || c.id.starts_with(&target))
            .is_some_and(|c| c.state == "running");

        if !is_running {
            self.container_exec_exit_msg = Some(format!("❌ 容器 {target} 未运行，无法 exec"));
            self.mode = AppMode::DockerPanel;
            return;
        }

        let exec = match ContainerExec::start(&target, &[], image.as_deref()) {
            Ok(e) => e,
            Err(e) => {
                self.container_exec_exit_msg = Some(format!("❌ exec 启动失败: {e}"));
                self.mode = AppMode::DockerPanel;
                return;
            }
        };
        let (cols, rows) = exec.size();
        // vt100 parser scrollback = 0：嵌入式视图不滚动历史（按 Esc 退出查日志）。
        let parser = vt100::Parser::new(rows, cols, 0);
        self.container_exec = Some(exec);
        self.container_exec_vt = Some(parser);
        self.container_exec_exit_msg = None;
    }

    /// 容器 exec 模式按键处理（不走 panel）。
    ///
    /// 把 [`KeyEvent`] 转 ANSI 字节序列（[`crate::tui::container_exec_view::key_event_to_pty_bytes`]）
    /// 写进 PTY writer。Ctrl+C / Ctrl+D / Ctrl+\\ 在 exec 模式下转发到容器
    /// （raw mode 下 crossterm 不传 SIGINT，ctrlc handler 不触发）。
    fn handle_container_exec_key(&mut self, key: KeyEvent) {
        let Some(bytes) = crate::tui::container_exec_view::key_event_to_pty_bytes(key) else {
            return;
        };
        // 先在 if let 内拿到错误信息（&mut ce 借用期间不能调 switch_mode），
        // 退出 if let 块后再切 mode。switch_mode 会用「✅ 已退出容器 xxx」
        // 覆盖 exit_msg，所以这里在 switch 之后再覆盖为带错误原因的提示
        // —— 否则用户看不到具体写入失败原因（P1-X2）。
        let mut switch_with_err: Option<String> = None;
        if let Some(ce) = self.container_exec.as_mut() {
            if let Err(e) = ce.write_all(&bytes) {
                switch_with_err = Some(format!(
                    "❌ 容器 {} 写入失败，已切回 Docker 面板: {e}",
                    ce.container
                ));
            }
        }
        if let Some(err_msg) = switch_with_err {
            self.switch_mode(AppMode::DockerPanel);
            self.container_exec_exit_msg = Some(err_msg);
        }
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
        // v0.6.0 阶段 5：键盘路由整体封装到 InspectorController::handle_key；
        // 副作用（status_message / kill / record_op / clipboard / monitor）通过
        // InspectorAction 派发回 App 处理。
        let ports_snapshot: Vec<crate::port_map::PortEntry> = self.port_panel.port_entries.clone();
        let action = self
            .inspector
            .handle_key(key, &ports_snapshot, self.recording_wanted);
        match action {
            InspectorAction::Noop => {}
            InspectorAction::StatusMsg(msg) => {
                self.status_message = Some(msg);
            }
            InspectorAction::Close => {
                self.process_panel.process_view_mode = ProcessViewMode::List;
                self.mode = AppMode::ProcessList;
            }
            InspectorAction::BumpPriority { pid, up } => {
                self.bump_priority(pid, up);
            }
            InspectorAction::KillPid(pid) => {
                let proc_name = self
                    .inspector
                    .detail_process
                    .as_ref()
                    .map(|p| p.name.to_string())
                    .unwrap_or_default();
                let result = crate::kill::kill_process(pid, false);
                match result {
                    Ok(crate::kill::KillResult::Killed) => {
                        let msg = format!("终止 {} (PID {})", proc_name, pid);
                        self.status_message = Some(format!("{} 已终止", msg));
                        self.record_op(msg);
                        self.mode = AppMode::ProcessList;
                    }
                    Ok(crate::kill::KillResult::AlreadyGone) => {
                        self.status_message = Some("进程已不存在".to_string());
                        self.mode = AppMode::ProcessList;
                    }
                    Ok(crate::kill::KillResult::AccessDenied) => {
                        self.status_message =
                            Some("权限不足，无法终止进程 — 请以管理员身份重启 proc".to_string());
                    }
                    Ok(crate::kill::KillResult::Failed(e)) => {
                        self.status_message = Some(format!("终止失败: {}", e));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("终止失败: {}", e));
                    }
                }
            }
            InspectorAction::AddMonitor(pid) => {
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
            InspectorAction::CopyInfo(info) => {
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
    }

    fn handle_replay_key(&mut self, key: KeyEvent) {
        let action = self.replay.handle_key(key);
        self.dispatch_replay_action(action);
    }

    /// `ReplayController::handle_key` / `tick` 返回 [`ReplayAction`] 后，
    /// App 在此派发副作用：`Quit` → `should_quit`；`ApplyFrame` → 把当前帧
    /// 应用到 15+ panel / metrics 字段；`Noop` 不动。
    fn dispatch_replay_action(&mut self, action: ReplayAction) {
        match action {
            ReplayAction::Noop => {}
            ReplayAction::Quit => self.should_quit = true,
            ReplayAction::ApplyFrame => self.apply_replay_frame(),
        }
    }

    /// 把 controller 当前帧应用到 panels / metrics / histories / 导航状态。
    /// 触发点：`start_replay`（首帧）/ `dispatch_replay_action(ApplyFrame)`
    /// （Left/Right/Home/End / tick 自动步进）。原 `replay_load_current_frame`。
    fn apply_replay_frame(&mut self) {
        // Clone the frame so we release the immutable borrow on `self.replay`
        // before we start mutating panel state below.
        let Some(frame) = self.replay.current_frame() else {
            return;
        };

        self.restore_replay_panel_data(&frame);
        self.restore_replay_metrics(&frame);
        self.restore_replay_view_mode(&frame);
        self.restore_replay_nav(frame.nav);

        self.data_dirty = true;
    }

    fn restore_replay_panel_data(&mut self, frame: &crate::record::frame::UiFrame) {
        self.cached_processes = Arc::new(frame.processes.iter().map(ProcessInfo::from).collect());
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
                &self.cached_processes[..],
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

    fn handle_kill_confirm(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(req) = self.pending_kill.take() {
                    let pid_to_name: HashMap<u32, String> = self
                        .cached_processes
                        .iter()
                        .map(|p| (p.pid, (*p.name).to_string()))
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
                            Ok(crate::kill::KillResult::AccessDenied) => results.push(format!(
                                "{} (PID {}) 权限不足 — 请以管理员身份重启 proc",
                                name, pid
                            )),
                            Ok(crate::kill::KillResult::Failed(e)) => {
                                results.push(format!("{} (PID {}) 失败: {}", name, pid, e))
                            }
                            Err(e) => results.push(format!("{} (PID {}) 失败: {}", name, pid, e)),
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
                        .refresh_tree(&self.cached_processes[..], self.snapshot.memory_usage().1);
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

        // v0.6.0 阶段 3：drain worker crash → banner。每帧都跑，第一时间反馈。
        self.poll_crashes();

        // Replay mode
        if self.mode == AppMode::Replay {
            return self.tick_replay();
        }

        // Container exec 模式：drain PTY → vt100 parser；child 退出则切回 DockerPanel。
        // 每帧都跑（不等 REFRESH_INTERVAL），保证 docker exec 输出流畅。
        if self.mode == AppMode::ContainerExec {
            self.tick_container_exec();
            return true;
        }

        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            self.last_refresh = Instant::now();
            let had_heavy = self.tick_light_refresh();
            self.tick_throttle_check();
            self.tick_history_sample(had_heavy);
            self.tick_alert_evaluate();
            self.tick_dns_log();
            self.tick_panels();
            self.tick_usb_monitor_docker();
            self.tick_self_monitor();
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
            let action = self.replay.tick();
            self.dispatch_replay_action(action);
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
                    self.cached_processes = self.snapshot.cached_processes_arc();
                    self.update_disk_speeds();
                    self.update_net_rates();

                    let alive_pids: HashSet<u32> =
                        self.cached_processes.iter().map(|p| p.pid).collect();
                    self.security_scores
                        .retain(|pid, _| alive_pids.contains(pid));

                    // v0.6.0 阶段 5：detail_process 维护 + priority/affinity 缓存
                    // 刷新封装到 InspectorController。
                    self.inspector.sync_detail(&self.cached_processes);
                    self.inspector.refresh_detail_priority();

                    self.data_dirty = true;

                    if !self.scoring_pending {
                        let procs = Arc::clone(&self.cached_processes);
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
        // Arc::make_mut: refcount==1 时 zero-cost 原地修改；scoring 持有旧 Arc 时
        // 触发一次 COW 复制（每 heavy refresh 至多一次，可接受）。
        let procs = Arc::make_mut(&mut self.cached_processes);
        for proc in procs {
            let key = (proc.pid, proc.start_time);
            if let Some(&(prev_r, prev_w)) = self.prev_process_disk.get(&key) {
                proc.disk_read_speed =
                    ((proc.disk_usage.0.saturating_sub(prev_r)) as f64 / elapsed) as u64;
                proc.disk_write_speed =
                    ((proc.disk_usage.1.saturating_sub(prev_w)) as f64 / elapsed) as u64;
            }
        }
        self.prev_process_disk = self
            .cached_processes
            .iter()
            .map(|p| ((p.pid, p.start_time), p.disk_usage))
            .collect();
        self.prev_process_disk_time = now;
    }

    /// 阶段 7 D1：从 [`net_flow_worker`] drain 最新一份 per-PID 速率，贴回
    /// `cached_processes.net_sent_rate` / `net_recv_rate`。
    ///
    /// 设计：
    /// - worker 1s 推一份；主线程 tick 50ms，heavy refresh 2s。每次 heavy 都 drain，
    ///   拿到的总是「最新一份」（worker 内部 sync_channel(1) 已 dedup）
    /// - 没 worker / 没 snapshot → 全部置 0（之前可能有值但 worker 关闭了）
    /// - 按 PID 匹配 cached_processes。PID 复用风险：worker 端 collector 已做
    ///   「累计回退」检测；这里只贴当前 PID 即可
    fn update_net_rates(&mut self) {
        let snapshot = if let Some(worker) = &self.workers.net_flow_worker {
            worker.try_recv_latest()
        } else {
            None
        };

        let procs = Arc::make_mut(&mut self.cached_processes);

        match snapshot {
            Some(s) => {
                let by_pid: HashMap<u32, (u64, u64)> = s
                    .rates
                    .iter()
                    .map(|r| (r.pid, (r.bytes_sent_per_sec, r.bytes_recv_per_sec)))
                    .collect();
                for proc in procs {
                    if let Some(&(sent, recv)) = by_pid.get(&proc.pid) {
                        proc.net_sent_rate = sent;
                        proc.net_recv_rate = recv;
                    } else {
                        proc.net_sent_rate = 0;
                        proc.net_recv_rate = 0;
                    }
                }
            }
            None => {
                // 无 worker / 无新帧：保留当前值，不强制清零。
                // 进程列表里新加入的 ProcessInfo 默认就是 0。
            }
        }
    }

    /// 阶段 8 D3：从 [`dns_log_worker`] drain 最新一份 DNS 查询日志，
    /// 追加到 `dns_log_recent`（cap=1000 FIFO）。worker 不存在时跳过。
    ///
    /// 隐私：DNS 查询不持久化；本方法只动内存 VecDeque，无 IO。
    fn tick_dns_log(&mut self) {
        let Some(worker) = &self.workers.dns_log_worker else {
            return;
        };
        let Some(snap) = worker.try_recv_latest() else {
            return;
        };
        for q in snap.queries {
            self.dns_log_recent.push_back(q);
            if self.dns_log_recent.len() > DNS_LOG_BUFFER_CAP {
                self.dns_log_recent.pop_front();
            }
        }
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

    /// 容器 exec tick：drain PTY 输出 → vt100 parser；child 退出则切回 DockerPanel。
    fn tick_container_exec(&mut self) {
        // 1) drain 所有 reader chunk → vt100 parser
        let bytes: Vec<u8> = self
            .container_exec
            .as_ref()
            .map(|ce| ce.drain())
            .unwrap_or_default();
        if !bytes.is_empty()
            && let Some(parser) = self.container_exec_vt.as_mut()
        {
            parser.process(&bytes);
        }

        // 2) 检测 child 是否退出（用户输入 exit / Ctrl+D / Ctrl+\）
        let exited = self
            .container_exec
            .as_mut()
            .is_some_and(|ce| ce.is_finished());
        if exited {
            let name = self
                .container_exec
                .as_ref()
                .map(|ce| ce.container.clone())
                .unwrap_or_default();
            self.container_exec_exit_msg = Some(format!("✅ 容器 {name} 会话已结束"));
            self.switch_mode(AppMode::DockerPanel);
        }
    }

    /// 终端 resize 事件：触发 ContainerExec PTY + vt100 parser 同步尺寸。
    /// 实际尺寸从 ratatui draw 的 area 拿（run_app 在 draw 后调 resize_container_exec）。
    pub fn notify_terminal_resized(&mut self) {
        if self.mode == AppMode::ContainerExec {
            self.pending_redraw = true;
        }
    }

    /// 用 ratatui 渲染尺寸调整 PTY + vt100 parser。ContainerExec 模式下 run_app 调用。
    pub fn resize_container_exec(&mut self, cols: u16, rows: u16) {
        // 减去顶部 1 行 header + 底部 1 行 footer（layout.rs::draw 的 vertical split）。
        let effective_rows = rows.saturating_sub(2);
        if effective_rows == 0 || cols == 0 {
            return;
        }
        if let Some(ce) = self.container_exec.as_mut() {
            if let Err(e) = ce.resize(cols, effective_rows) {
                tracing::warn!("PTY resize 失败: {e}");
            }
        }
        if let Some(parser) = self.container_exec_vt.as_mut() {
            // vt100 parser set_size(rows, cols)：注意顺序与 portable-pty 相反。
            parser.set_size(effective_rows, cols);
        }
    }

    /// 从 cached_processes 按 PID 取 proc 自身的 ProcessInfo 快照，供 sidebar 显示。
    fn tick_self_monitor(&mut self) {
        let self_pid = std::process::id();
        self.self_proc = self
            .cached_processes
            .iter()
            .find(|p| p.pid == self_pid)
            .cloned();
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
            let processes: &[ProcessInfo] = &self.cached_processes[..];
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
            .evaluate(&self.snapshot, &self.cached_processes[..]);
        for event in &alert_events {
            if let crate::alert::AlertEventType::Fired = event.event_type
                && let crate::alert::AlertSeverity::Critical = event.severity
            {
                let _ = crate::monitor::notify::send_toast("proc - Critical Alert", &event.message);
            }
        }
    }

    fn tick_panels(&mut self) {
        // 先从后台 worker 取最新 sockets(若有),注入 PortPanel 待处理队列。
        // 把这步放在 ctx 构造之前,避免在 ctx 借用 self 期间再借 self.workers.port_worker。
        let new_sockets = self
            .workers
            .port_worker
            .try_recv_latest()
            .map(|s| s.sockets);

        let mut ctx = PanelContext {
            snapshot: &self.snapshot,
            cached_processes: &self.cached_processes[..],
            cached_sorted: &self.cached_sorted,
            security_scores: &self.security_scores,
            status_message: &mut self.status_message,
            detail_process: &mut self.inspector.detail_process,
            pending_kill: &mut self.pending_kill,
            data_dirty: &mut self.data_dirty,
            pending_redraw: &mut self.pending_redraw,
            alert_manager: &mut self.alert_manager,
            op_history: &mut self.op_history,
            dns_log_recent: &mut self.dns_log_recent,
            pending_container_exec: &mut self.pending_container_exec_target,
        };
        if self.mode == AppMode::ProcessList {
            self.process_panel.tick(&mut ctx);
        } else if self.mode == AppMode::PortMap {
            if let Some(sockets) = new_sockets {
                self.port_panel.pending_sockets = Some(sockets);
            }
            self.port_panel.tick(&mut ctx);
        }
    }

    fn tick_usb_monitor_docker(&mut self) {
        // USB 设备列表由后台 UsbSnapshotWorker 每 ~5s 推一次;主线程只 try_recv
        // + 合并 is_occupied 状态。设备锁查询(scan_device_locks_with_processes)
        // 仍按需在 UsbPanel 内同步触发(用户按 r / Enter)。
        if self.mode == AppMode::UsbAssistant
            && let Some(snap) = self.workers.usb_worker.try_recv_latest()
        {
            self.usb_panel.merge_devices(snap.devices);
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
        let processes: &[ProcessInfo] = &self.cached_processes[..];
        let search = &self.process_panel.search;
        let query = search.query();
        let filtered: Vec<&ProcessInfo> = if query.is_empty() {
            processes.iter().collect()
        } else {
            // v0.6.0 阶段 4：搜索 query 一次性 lowercase（O(query.len())），
            // 进程名匹配走 `ProcessInfo::name_lower` 预算字段，避免每进程每按键
            // `to_lowercase` 分配。
            let q_lower = search.query_lower();
            processes
                .iter()
                .filter(|p| p.name_lower.contains(q_lower) || p.pid.to_string().contains(query))
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
        // P1.1/P1.2: tie-breaker 加 pid 防止同分进程顺序抖动；
        // Name 路径直接复用预计算的 name_lower，省掉 N 次 to_lowercase。
        if sort_field == SortField::Name {
            let mut keyed: Vec<(std::sync::Arc<str>, classify::ProcessClass, &ProcessInfo)> =
                result
                    .into_iter()
                    .map(|(class, p)| (std::sync::Arc::clone(&p.name_lower), class, p))
                    .collect();
            keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.pid.cmp(&b.2.pid)));
            self.cached_sorted = keyed
                .iter()
                .map(|(_, class, p)| (*pid_to_idx.get(&p.pid).unwrap_or(&0), *class))
                .collect();
            self.data_dirty = false;
            return;
        }

        result.sort_by(|a, b| match sort_field {
            SortField::Cpu => {
                b.1.cpu_usage
                    .partial_cmp(&a.1.cpu_usage)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.1.pid.cmp(&b.1.pid))
            }
            SortField::Memory => b.1.memory.cmp(&a.1.memory).then(a.1.pid.cmp(&b.1.pid)),
            SortField::Pid => a.1.pid.cmp(&b.1.pid),
            SortField::Name => unreachable!("Name 路径在 sort_field 分支前已处理"),
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
                sa.cmp(&sb).then(a.1.pid.cmp(&b.1.pid))
            }
            SortField::DiskRead => {
                let sa = a.1.disk_read_speed;
                let sb = b.1.disk_read_speed;
                sb.cmp(&sa).then(a.1.pid.cmp(&b.1.pid))
            }
            SortField::DiskWrite => {
                let sa = a.1.disk_write_speed;
                let sb = b.1.disk_write_speed;
                sb.cmp(&sa).then(a.1.pid.cmp(&b.1.pid))
            }
            SortField::NetSent => {
                let sa = a.1.net_sent_rate;
                let sb = b.1.net_sent_rate;
                sb.cmp(&sa).then(a.1.pid.cmp(&b.1.pid))
            }
            SortField::NetRecv => {
                let sa = a.1.net_recv_rate;
                let sb = b.1.net_recv_rate;
                sb.cmp(&sa).then(a.1.pid.cmp(&b.1.pid))
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
        // 折叠：13 行（基线）。
        // 展开：基线 + per-core 表头 + 最多 8 核 + 1 行间隔 = 13 + 1 + 8 + 1 = 23。
        if self.sidebar_expanded {
            13 + 1 + 8 + 1
        } else {
            13
        }
    }

    pub fn usb_scan_devices(&mut self) {
        self.usb_panel.scan_devices(&self.cached_processes[..]);
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

    /// A4：进程优先级 +1 / -1 档（详情页 `+`/`-`、列表页 `+`/`-` 都走这里）。
    /// 失败时把错误塞到 status_message，不退出详情页。
    fn bump_priority(&mut self, pid: u32, up: bool) {
        let current = match crate::process_control::get_priority(pid) {
            Ok(c) => c,
            Err(e) => {
                self.status_message = Some(format!("读取优先级失败: {}", e));
                return;
            }
        };
        let next = if up {
            current.bump_up()
        } else {
            current.bump_down()
        };
        if next == current {
            self.status_message = Some(format!(
                "已到达 {} 端，无法继续{}",
                current.label(),
                if up { "调高" } else { "调低" }
            ));
            return;
        }
        match crate::process_control::set_priority(pid, next) {
            Ok(()) => {
                let verb = if up { "调高至" } else { "调低至" };
                self.status_message = Some(format!("PID {} 优先级 {} {}", pid, verb, next.label()));
                self.record_op(format!(
                    "PID {} 优先级 {} {} → {}",
                    pid,
                    if up { "调高" } else { "调低" },
                    current.label(),
                    next.label()
                ));
                // 阶段 11 P1-A3：调整成功后刷新缓存，让用户立即看到新值。
                self.inspector.refresh_detail_priority();
            }
            Err(e) => {
                self.status_message = Some(format!(
                    "设置优先级失败 ({} → {}): {}",
                    current.label(),
                    next.label(),
                    e
                ));
            }
        }
    }

    /// A4：进程列表 `+`/`-` 公开入口 —— ProcessPanel 直接调，免去重复样板。
    pub fn bump_selected_priority(&mut self, pid: u32, up: bool) {
        self.bump_priority(pid, up);
    }
}
