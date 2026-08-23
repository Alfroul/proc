use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

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
// v0.14 stage 4：扩 re-export `ReplayDirection`（与 ReplaySpeed 对称）。
pub use crate::replay::{ReplayDirection, ReplaySpeed, TimelineState};

use crate::agent::session::{ConfirmDecision, SessionHandle};
use crate::alert::AlertManager;
use crate::app_panel::{Panel, PanelAction, PanelContext};
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
use crate::record::{Bookmark, BookmarkFile};
use crate::replay::{ReplayAction, ReplayController};
use crate::security::{BackgroundScorer, SecurityScore};
use crate::tree::TreeNode;
use crate::tui::command_palette::{CommandAction, CommandPalette, PaletteHandleResult};
use crate::view_models::AgentAction;
use crate::view_models::AgentPanelController;
use crate::view_models::DockerPanel;
use crate::view_models::DockerPanelController;
use crate::view_models::MonitorPanel;
use crate::view_models::MonitorPanelController;
use crate::view_models::PortPanel;
use crate::view_models::PortPanelController;
use crate::view_models::ProcessPanel;
use crate::view_models::ProcessPanelController;
use crate::view_models::UsbPanel;
use crate::view_models::UsbPanelController;
use crate::view_models::short_provider_label;

/// v0.7.0 阶段 3：App 的按键分发层。
///
/// 决定一次按键优先派给「命令面板浮层 / 搜索框 / 当前面板」。Palette 激活时
/// 拦截所有按键；Search 是从现有 panel 的 search 状态派生出来的「逻辑层」（兼容
/// v0.6.0 测试，不破坏既有 `/` 进入搜索的行为）。
///
/// 详见 `docs/adr/0010-shell-completion-and-palette.md`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppLayer {
    /// 默认层：按键派给当前面板（ProcessPanel / PortPanel / ...）。
    #[default]
    Normal,
    /// 任一 panel 的 search 处于 active（`/` 进入）。逻辑层，无独立字段。
    Search,
    /// Ctrl+P 命令面板浮层激活，拦截所有按键。
    Palette,
}

pub struct App {
    pub mode: AppMode,
    pub snapshot: SystemSnapshot,
    pub cached_processes: Arc<Vec<ProcessInfo>>,
    pub should_quit: bool,
    pub last_refresh: Instant,
    pub last_heavy_refresh: Instant,
    pub pending_redraw: bool,

    // Panels
    // v0.7.0 阶段 5：5 个 panel 字段类型从 `XxxPanel` 改为对应 controller。
    // 字段名保留 `xxx_panel`（仅类型变），让 src/tui/* 的 `app.xxx_panel` 访问
    // 路径不破坏 —— 调用方多一层 `.panel`（或 `.panel()`）取 inner。详见
    // ADR-0012。
    pub process_panel: ProcessPanelController,
    pub port_panel: PortPanelController,
    pub usb_panel: UsbPanelController,
    pub monitor_panel: MonitorPanelController,
    pub docker_panel: DockerPanelController,
    // v0.21：AI Agent 面板（ADR-0031）。stage 3：controller 状态机 + streaming
    // 渲染消费 agent_session 事件流；session 生命周期 = 进面板建 / 退面板
    // teardown Drop kill llama-server（D5 按需 spawn 延续）。
    pub agent_panel: AgentPanelController,
    /// Agent 会话句柄（进入面板时 build_session；None = 构造失败或未进入）。
    pub agent_session: Option<SessionHandle>,

    // v0.6.0 阶段 5：后台采集 worker（port / usb / net_flow / dns_log）统一由
    // `WorkerManager` 持有，详见 `src/workers/manager.rs`。Docker logs worker
    // 仍由 `DockerPanel` 自管（生命周期与 panel 绑定）。
    pub workers: crate::workers::WorkerManager,
    // 阶段 8 D3：DNS 查询日志（worker drain 出来的最新 N 条）。worker 句柄在
    // `self.workers.dns_log_worker`；这里只存数据。cap=1000 FIFO；仅内存缓冲，
    // 录屏（record/）路径不序列化任何 DNS 数据（隐私）。
    pub dns_log_recent: VecDeque<crate::dns_log::DnsQuery>,
    // v0.7 阶段 8 / v0.12 阶段 2：Flow graph（Windows-only Schannel 路径，ADR-0022）。
    // schannel_etw worker 在 `self.workers.schannel_etw_worker`；这里存从 worker
    // drain 出的 SniRecord 关联构造的 ProcessFlow 快照（无 source 字段，全部
    // 来自 Schannel）。worker 为 None（非管理员 / x86）→ flows 保持空 Vec。
    pub flows: Vec<crate::flow::ProcessFlow>,

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

    // v0.14 stage 2：录屏书签系统（录制路径）。
    // 录制中按 `b` 触发 inline label 输入；Enter 提交书签；停止录制时 flush 到
    // `.prec.bookmarks.json` sidecar（tui::run_app 调 take_*_for_flush）。
    recording_bookmarks: Vec<Bookmark>,
    /// 当前录屏文件路径（VT100 recorder 启动时由 tui::run_app 注入；sidecar flush 用）。
    recording_path: Option<std::path::PathBuf>,
    /// 当前录屏已写出的帧数（VtRecorder 每次成功 capture 时由 tui::run_app 注入）。
    recording_frame_count: usize,
    /// `b` 键打开的 inline label 输入状态：None=未激活 / Some=激活中。
    pub pending_bookmark_label: Option<PendingBookmarkLabel>,

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
    /// v0.11.0 阶段 1（ADR-0019）：App 保留的 `crash_tx` 副本，用于 worker
    /// restart 路径给 respawn 的新 worker clone。`tick_poll_crashes` /
    /// `restart_tick` 通过此 sender 让新 worker 仍能把后续 panic 推给主线程。
    pub crash_tx: Option<std::sync::mpsc::Sender<crate::metrics::crash::WorkerCrash>>,

    // v0.7.0 阶段 3：命令面板 Ctrl+P + AppLayer 状态机。
    /// 当前显式激活的层。Normal / Search 是逻辑层（从 panel search 状态派生），
    /// Palette 是显式覆盖（Ctrl+P 打开 / Esc 关闭）。
    pub active_layer: AppLayer,
    /// Ctrl+P 命令面板状态。常驻 App（避免每次打开重建 nucleo Matcher）。
    pub command_palette: CommandPalette,
}

/// v0.14 stage 2：录制中按 `b` 触发的 inline label 输入状态。
///
/// 激活后 App::handle_key 把字符键推到 `input`（Enter 提交 / Esc 取消）。
/// label 可空 → 默认生成「书签 #N」。详见 `docs/stages/v0.14-stage-2.md`。
#[derive(Debug, Clone)]
pub struct PendingBookmarkLabel {
    /// 标记瞬间的帧索引（capture 时的 recording_frame_count 快照）。
    pub frame_idx: usize,
    /// 标记瞬间的 unix epoch 秒。
    pub timestamp_secs: u64,
    /// 用户输入 buffer（可空 → 默认 label）。
    pub input: String,
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
        let process_panel = ProcessPanelController::new(process_panel);

        let mut port_panel = PortPanel::new();
        port_panel.port_entries = port_entries;
        let port_panel = PortPanelController::new(port_panel);

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
        // v0.11.0 阶段 1：App 保留 crash_tx 副本用于 worker restart 路径
        // （respawn 的新 worker 需 clone 这份 sender），详见 ADR-0019。
        let (crash_tx, crash_rx) = crate::metrics::crash::channel();
        let workers = crate::workers::WorkerManager::new(Some(&crash_tx));
        let mut docker_panel = DockerPanel::new();
        docker_panel.crash_tx = Some(crash_tx.clone());
        let docker_panel = DockerPanelController::new(docker_panel);

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
            usb_panel: UsbPanelController::new(UsbPanel::new()),
            monitor_panel: MonitorPanelController::new(MonitorPanel::new()),
            docker_panel,
            agent_panel: AgentPanelController::new(),
            agent_session: None,
            workers,
            dns_log_recent: VecDeque::new(),
            flows: Vec::new(),
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
            recording_bookmarks: Vec::new(),
            recording_path: None,
            recording_frame_count: 0,
            pending_bookmark_label: None,
            throttle_info: None,
            throttle_reason: crate::throttle::ThrottleReason::None,
            prev_process_disk: HashMap::new(),
            prev_process_disk_time: Instant::now(),
            self_proc: None,
            is_windows,
            sidebar_expanded: crate::ui_state::load_sidebar_expanded(),
            crash_rx: Some(crash_rx),
            active_crashes: Vec::new(),
            crash_tx: Some(crash_tx),
            active_layer: AppLayer::Normal,
            command_palette: CommandPalette::new(),
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

    // --- v0.14 stage 2：录屏书签 ---

    /// 当前录屏已写出的帧数（由 tui::run_app 每 tick 注入）。
    pub fn recording_frame_count(&self) -> usize {
        self.recording_frame_count
    }
    pub fn set_recording_frame_count(&mut self, n: usize) {
        self.recording_frame_count = n;
    }

    /// 当前录屏文件路径（VT100 recorder 启动时由 tui::run_app 注入）。
    pub fn set_recording_path(&mut self, p: std::path::PathBuf) {
        self.recording_path = Some(p);
    }

    /// 录屏书签（录制中累积；停止录制时 tui::run_app 调 take_recording_bookmarks flush）。
    pub fn recording_bookmarks(&self) -> &[Bookmark] {
        &self.recording_bookmarks
    }

    /// 取走累积的书签（录制停止时 tui::run_app 调，flush 到 sidecar）。
    pub fn take_recording_bookmarks(&mut self) -> Vec<Bookmark> {
        std::mem::take(&mut self.recording_bookmarks)
    }

    /// 取走录屏路径（与 take_recording_bookmarks 配套使用）。
    pub fn take_recording_path(&mut self) -> Option<std::path::PathBuf> {
        self.recording_path.take()
    }

    /// 触发 inline label 输入（录制中按 `b` 调）。
    /// `now` 是当前 unix epoch 秒（让测试可注入固定时间）。
    fn start_bookmark_label_input(&mut self, now: u64) {
        let frame_idx = self.recording_frame_count;
        self.pending_bookmark_label = Some(PendingBookmarkLabel {
            frame_idx,
            timestamp_secs: now,
            input: String::new(),
        });
        self.status_message = Some(format!(
            "标记书签（帧 {frame_idx}）：输入 label，Enter 提交 / Esc 取消"
        ));
    }

    /// 处理 inline label 输入态下的按键。返回 true 表示已消费。
    fn handle_bookmark_label_input(&mut self, key: KeyEvent, now: u64) -> bool {
        let Some(pending) = self.pending_bookmark_label.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                self.pending_bookmark_label = None;
                self.status_message = Some("书签标记取消".to_string());
            }
            KeyCode::Enter => {
                // 移出 pending（解除借用），再 push 到 recording_bookmarks
                let pending = self.pending_bookmark_label.take().unwrap();
                let next_id = self
                    .recording_bookmarks
                    .iter()
                    .map(|b| b.id)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let label = if pending.input.trim().is_empty() {
                    format!("书签 #{next_id}")
                } else {
                    pending.input.clone()
                };
                let frame_idx = pending.frame_idx;
                self.recording_bookmarks.push(Bookmark {
                    id: next_id,
                    frame_idx,
                    timestamp_secs: pending.timestamp_secs,
                    label,
                    created_at: now,
                });
                self.status_message = Some(format!("已加书签 #{next_id}（帧 {frame_idx}）"));
            }
            KeyCode::Backspace => {
                pending.input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                pending.input.push(c);
            }
            _ => {
                // 其他键吞掉
            }
        }
        true
    }

    /// 录制停止 hook：把累积书签 flush 到 sidecar。tui::run_app 调用。
    /// 调用方负责在录制真正停止（recorder.stop 完成）后再调此方法。
    pub fn flush_recording_bookmarks(&mut self) {
        let Some(path) = self.recording_path.take() else {
            // 无录屏路径（用户未真正启动 recorder，或路径已被取走）— 清书签防泄漏
            self.recording_bookmarks.clear();
            return;
        };
        if self.recording_bookmarks.is_empty() {
            // 无书签 — 不写 sidecar（避免空文件污染用户目录）
            return;
        }
        let mut file = BookmarkFile::load_or_empty(&path);
        let bookmarks = std::mem::take(&mut self.recording_bookmarks);
        for bm in bookmarks {
            file.bookmarks.push(bm);
        }
        file.sort_by_frame();
        file.write(&path);
    }

    // --- v0.6.0 阶段 3：worker 可观测性 ---

    /// 聚合所有 SnapshotWorker 的 metrics 快照。`proc diag` + `?` 帮助页消费。
    ///
    /// Light/Heavy/Smart worker 由 `SystemSnapshot` 内部持有（非 SnapshotWorker
    /// 模板），阶段 5 WorkerManager 重构时统一接入；当前阶段不暴露。
    #[must_use]
    pub fn worker_metrics(&self) -> Vec<crate::metrics::NamedWorkerStats> {
        // v0.6.0 阶段 5：4 个直管 worker 的 metrics 走 WorkerManager 聚合。
        // v0.7.0 阶段 1 TD-5：Docker snapshot + logs worker 由 DockerPanel::metrics 聚合。
        let mut out = self.workers.metrics_snapshot();
        out.extend(self.docker_panel.panel.metrics());
        out
    }

    /// 主循环 tick 调一次：drain `crash_rx`，新到的 `WorkerCrash` 追加到
    /// `active_crashes`，触发 TUI 顶部 banner 渲染。
    ///
    /// v0.11.0 阶段 1（ADR-0019）：drain 同时调 `workers.restart(name, now, crash_tx)`
    /// 记录 restart 状态 + 触发指数退避 respawn 决策。
    pub fn poll_crashes(&mut self) {
        let Some(rx) = &self.crash_rx else {
            return;
        };
        let now = std::time::SystemTime::now();
        let mut new_crashes: Vec<crate::metrics::crash::WorkerCrash> = Vec::new();
        while let Ok(crash) = rx.try_recv() {
            tracing::error!(
                worker = crash.worker,
                panic = %crash.message,
                "worker crashed (banner shown)"
            );
            // v0.11.0 阶段 1：通知 WorkerManager 记录 + 尝试 respawn。
            // crash_tx 必须从 self.crash_tx 借，不能从 self.crash_rx（不同字段）。
            let crash_tx_ref = self
                .crash_tx
                .as_ref()
                .map(|s| s as &std::sync::mpsc::Sender<crate::metrics::crash::WorkerCrash>);
            let _ = self.workers.restart(crash.worker, now, crash_tx_ref);
            new_crashes.push(crash);
        }
        if !new_crashes.is_empty() {
            self.active_crashes.extend(new_crashes);
            // 上限 10 条防止失忆式增长 —— 用户按 D 清空。
            while self.active_crashes.len() > 10 {
                self.active_crashes.drain(0..1);
            }
            self.pending_redraw = true;
        }
    }

    /// v0.11.0 阶段 1（ADR-0019）：每 1s 调一次。检查 `restart_history` 中
    /// pending crash 的 worker，backoff 到期就 respawn。返回值是本 tick 触发
    /// respawn 的 worker 列表（thread_name），用于 status_message 反馈。
    pub fn restart_tick(&mut self) -> Vec<&'static str> {
        let now = std::time::SystemTime::now();
        let crash_tx_ref = self
            .crash_tx
            .as_ref()
            .map(|s| s as &std::sync::mpsc::Sender<crate::metrics::crash::WorkerCrash>);
        let restarted = self.workers.restart_tick(now, crash_tx_ref);
        if !restarted.is_empty() {
            self.pending_redraw = true;
        }
        restarted
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
                self.process_panel.panel.process_view_mode = ProcessViewMode::Tree;
                self.process_panel.panel.cursor_index = 0;
                self.process_panel.panel.scroll_offset = 0;
                self.process_panel.panel.tree_cursor = 0;
                self.process_panel.panel.tree_scroll = 0;
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
                //
                // v0.6.0 阶段 8（REVIEW-7.md P1-6）：详情页内 'c' 改为显示 deprecation
                // warning 指引用户到 'y'（旧 0.5.0 用户肌肉记忆按 'c' 复制 → 给句话提示）。
                // 让 'c' 落入 InspectorController::handle_key，那里返回 StatusMsg。
                // v0.7.0 计划移除该 deprecation 分支，'c' 重新统一为侧边栏折叠。
                if self.mode == AppMode::ProcessDetail {
                    return false;
                }
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

        // v0.14 stage 2：书签 inline label 输入态 — 拦截所有按键
        // （仅次于 kill_confirm 的优先级，让 Esc / Enter 不会被下层吃掉）
        if self.pending_bookmark_label.is_some() {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            self.handle_bookmark_label_input(key, now);
            return;
        }

        // v0.6.0 阶段 3：worker 崩溃 banner — 按 D 清空。
        // v0.7.0 阶段 3：palette 激活时让位 —— 'D' 喂给 palette 输入框（fuzzy 搜
        // "dismiss" 仍可触发 DismissCrashes action）。
        // v0.21 stage 3：Agent 面板让位 —— 'D' 是输入框字符。
        if key.code == KeyCode::Char('D')
            && !self.active_crashes.is_empty()
            && self.active_layer != AppLayer::Palette
            && self.mode != AppMode::Agent
        {
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

        // v0.7.0 阶段 3：命令面板浮层 —— 拦截所有按键（除上方 modal 外）。
        // Stay = 输入字符 / 上下选择；Close = Esc 取消；Execute = Enter 触发 action。
        if self.active_layer == AppLayer::Palette {
            let result = self.command_palette.handle_key(key);
            match result {
                PaletteHandleResult::Stay => {}
                PaletteHandleResult::Close => self.active_layer = AppLayer::Normal,
                PaletteHandleResult::Execute(action) => {
                    self.active_layer = AppLayer::Normal;
                    self.dispatch_command_action(action);
                }
            }
            return;
        }

        // v0.7.0 阶段 3：Ctrl+P 打开命令面板。Replay / ContainerExec 内核被
        // 内部模式捕获（容器 PTY / 回放时间轴），不拦截。
        if is_ctrl_p(&key) && !self.keyboard_captured_by_inner_mode() {
            self.active_layer = AppLayer::Palette;
            self.command_palette.reset();
            return;
        }

        // Global recording toggle
        // v0.21 stage 3：Agent 面板内 'R' 是输入框字符（录屏开关退面板再按）。
        if key.code == KeyCode::Char('R') && self.mode != AppMode::Agent {
            self.toggle_recording();
            return;
        }

        // Global tab switching (only when no search/submenu active)
        // v0.21 stage 3：Agent 面板豁免 —— 输入框必须能打数字 1-6 / t / c / A / ?
        // （否则含数字的 query 会把面板切走，E2B 实测踩坑）。
        let any_search = self.process_panel.panel.search.is_active()
            || self.process_panel.panel.tree_search.is_active()
            || self.process_panel.panel.app_group_search.is_active()
            || self.port_panel.panel.port_search.is_active()
            || self.inspector.inspection_search.is_active();
        if !any_search
            && !self.kill_confirm
            && self.mode != AppMode::Agent
            && self.monitor_panel.panel.add_submenu.is_none()
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

        // v0.14 stage 2：录制中按 `b` 添加书签
        // 激活条件：录制中 + 普通面板模式（非 Replay / ContainerExec / ProcessDetail）+ 无 search 激活
        let bookmark_panel_modes = matches!(
            self.mode,
            AppMode::ProcessList
                | AppMode::PortMap
                | AppMode::UsbAssistant
                | AppMode::MonitorPanel
                | AppMode::DockerPanel
        );
        let any_search = self.process_panel.panel.search.is_active()
            || self.process_panel.panel.tree_search.is_active()
            || self.process_panel.panel.app_group_search.is_active()
            || self.port_panel.panel.port_search.is_active()
            || self.inspector.inspection_search.is_active();
        if self.recording_wanted
            && self.pending_bookmark_label.is_none()
            && bookmark_panel_modes
            && !any_search
            && key.code == KeyCode::Char('b')
        {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            self.start_bookmark_label_input(now);
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
                flows: &self.flows,
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
                AppMode::Help => PanelAction::Noop,
                // v0.21 stage 3：controller 键位状态机（Enter 发送 / Esc 中断或
                // 退出 / y·n confirm / PgUp PgDn 滚动），副作用经 AgentAction 出带。
                AppMode::Agent => self.agent_panel.handle_key(key, &mut ctx),
            }
        };

        // v0.7.0 阶段 5：dispatch PanelAction 副作用。controller 内部已通过 ctx
        // mutable ref 处理了大部分状态变更；此处只翻译"出带"的 PanelAction
        // 变体（Quit / SwitchMode / 状态消息 / kill / 剪贴板）。ToggleRecording
        // 现无 controller emit，预留分支。
        match result {
            PanelAction::Quit => self.should_quit = true,
            PanelAction::SwitchMode(mode) => self.switch_mode(mode),
            PanelAction::StatusMessage(s) => self.status_message = Some(s),
            PanelAction::Kill(req) => {
                self.pending_kill = Some(req);
                self.kill_confirm = true;
            }
            PanelAction::Clipboard(s) => {
                let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(s));
            }
            PanelAction::Agent(a) => self.dispatch_agent_action(a),
            PanelAction::Noop | PanelAction::ToggleRecording => {}
        }

        // If pending_kill was set by a panel via ctx, enable kill_confirm
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
            self.process_panel.panel.process_view_mode = ProcessViewMode::List;
            self.process_panel.panel.cursor_index = 0;
            self.process_panel.panel.scroll_offset = 0;
        }
        if mode == AppMode::UsbAssistant && self.mode != AppMode::UsbAssistant {
            self.usb_panel
                .panel
                .scan_devices(&self.cached_processes[..]);
        }
        if mode == AppMode::DockerPanel && self.mode != AppMode::DockerPanel {
            self.docker_panel.panel.refresh();
            if self.docker_panel.panel.connected && self.docker_panel.panel.event_receiver.is_none()
            {
                self.docker_panel.panel.start_watching();
            }
        }
        // 进入详情页时预加载 Inspector 数据：env/dlls/handles/memory 一次采集（同步）。
        // net 复用 port_panel.port_entries（后台 worker 每 ~3s 已推新），
        // 避免再调一次 scan_ports 的几百毫秒 syscall 卡帧。
        // 失败的子项会退化为空 Vec，TUI 层在 Tab 内显示「无数据」。
        if mode == AppMode::ProcessDetail {
            let ports_snapshot: Vec<crate::port_map::PortEntry> =
                self.port_panel.panel.port_entries.clone();
            // v0.6.0 阶段 5：详情页初始化整体封装到 InspectorController::open。
            self.inspector.open(&ports_snapshot);
        }
        // v0.21 stage 3：Agent 会话生命周期——进面板建 session（D5 惰性
        // spawn 延续），出面板 teardown Drop kill llama-server。
        if mode == AppMode::Agent && self.mode != AppMode::Agent {
            self.enter_agent_session();
        }
        if self.mode == AppMode::Agent && mode != AppMode::Agent {
            self.teardown_agent_session();
        }
        self.mode = mode;
        self.process_panel.panel.search.clear();
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
            .panel
            .containers
            .iter()
            .find(|c| c.name == target || c.id.starts_with(&target))
            .map(|c| c.image.clone());

        // 检查容器在运行状态（docker exec 需要容器 running）。
        let is_running = self
            .docker_panel
            .panel
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

    /// v0.21 stage 3：AgentAction 副作用执行（controller 是纯状态机，App 持
    /// SessionHandle）。ExitPanel 的 teardown 由 switch_mode 的「出 Agent」分支做。
    fn dispatch_agent_action(&mut self, action: AgentAction) {
        match action {
            AgentAction::SendQuery(q) => {
                match self.agent_session.as_ref() {
                    Some(s) if s.send_query(&q) => {}
                    Some(_) => {
                        self.agent_panel.panel_mut().entries.push(
                            crate::view_models::ChatEntry::Error(
                                "会话已终结，无法发送（退出面板重进）".to_string(),
                            ),
                        );
                    }
                    None => {
                        self.agent_panel.panel_mut().entries.push(
                            crate::view_models::ChatEntry::Error(
                                "会话不可用（provider 构造失败，见上方错误）".to_string(),
                            ),
                        );
                    }
                }
                self.pending_redraw = true;
            }
            AgentAction::Interrupt => {
                if let Some(s) = self.agent_session.as_ref() {
                    s.interrupt();
                }
            }
            AgentAction::ExitPanel => self.switch_mode(AppMode::ProcessList),
        }
    }

    /// v0.21 stage 3：进入面板建会话（D5：llama-server 仍惰性 spawn 于首次
    /// query——这里只起 session 线程）。构造失败降级 Error entry 不阻塞进入。
    fn enter_agent_session(&mut self) {
        match crate::agent::build_session(None, None, 10) {
            Ok((handle, spec)) => {
                self.agent_session = Some(handle);
                self.agent_panel
                    .panel_mut()
                    .reset_for_new_session(short_provider_label(&spec.name, &spec.detail));
            }
            Err(e) => {
                self.agent_session = None;
                self.agent_panel
                    .panel_mut()
                    .reset_for_new_session(String::new());
                self.agent_panel
                    .panel_mut()
                    .entries
                    .push(crate::view_models::ChatEntry::Error(format!(
                        "会话构造失败：{e}"
                    )));
            }
        }
    }

    /// v0.21 stage 3：拆会话——pending confirm 发 Denied（风险 2 收尾语义）→
    /// interrupt + shutdown + 有界等线程退出（cancel 检查点密集通常 <100ms；
    /// ≤3s 上限防 UI 卡死）→ Drop kill llama-server。幂等（None 安全）。
    fn teardown_agent_session(&mut self) {
        if self.agent_panel.panel.pending_confirm.is_some() {
            self.agent_panel
                .panel_mut()
                .resolve_confirm(ConfirmDecision::Denied);
        }
        if let Some(s) = self.agent_session.take() {
            // v0.23 stage 2（ADR-0033 D5 停止路径②③）：录制进行中自动 stop
            // 落盘（防孤儿），提示双落点——面板 Notice（仍在面板时）+ App
            // status_message（面板退出后 status bar 仍可见）。置于 interrupt/
            // shutdown 之前（kill + flush 完成后提示才真实）。
            if let Some(notice) = s.stop_orphan_recording() {
                self.agent_panel
                    .panel_mut()
                    .entries
                    .push(crate::view_models::ChatEntry::Notice(notice.clone()));
                self.status_message = Some(notice);
            }
            s.interrupt();
            s.shutdown();
            let deadline = Instant::now() + std::time::Duration::from_secs(3);
            while !s.is_exited() && Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        self.agent_panel.panel_mut().mode = crate::view_models::AgentPanelMode::Idle;
        self.pending_redraw = true;
    }

    /// v0.21 stage 3：Agent 面板每帧 drain（不进 REFRESH_INTERVAL 门——流式
    /// 响应性）。D6 节流：本 tick 全部事件批量 apply 后单次 pending_redraw。
    fn tick_agent(&mut self) {
        let Some(session) = self.agent_session.as_ref() else {
            return;
        };
        let mut changed = false;
        while let Some(ev) = session.drain_event() {
            changed |= self.agent_panel.panel_mut().apply_event(ev);
        }
        // 会话死亡感知（stage 2 注记 4）：事件排空后线程已退出但状态仍挂着。
        if session.is_exited()
            && self.agent_panel.panel.mode != crate::view_models::AgentPanelMode::Idle
        {
            self.agent_panel.panel_mut().mark_session_dead();
            changed = true;
        }
        if changed {
            self.pending_redraw = true;
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
        let ports_snapshot: Vec<crate::port_map::PortEntry> =
            self.port_panel.panel.port_entries.clone();
        let action = self
            .inspector
            .handle_key(key, &ports_snapshot, self.recording_wanted);
        match action {
            InspectorAction::Noop => {}
            InspectorAction::StatusMsg(msg) => {
                self.status_message = Some(msg);
            }
            InspectorAction::Close => {
                self.process_panel.panel.process_view_mode = ProcessViewMode::List;
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
                match self.monitor_panel.panel.manager.add_monitor(
                    crate::monitor::MonitorTarget::ByPid { pid },
                    crate::monitor::RestartPolicy::NotifyOnly,
                ) {
                    Ok(monitor_id) => {
                        self.monitor_panel.panel.manager.add_notification(format!(
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
            InspectorAction::ToggleEcoQoS { pid, make_eco } => {
                // v0.7 阶段 6：派发 EcoQoS 切换（ADR-0014）。非 Windows 平台
                // set_throttle 返回错误，写入 status_message；不退出详情页。
                match crate::throttle::set_throttle(pid, make_eco) {
                    Ok(()) => {
                        self.status_message = Some(format!(
                            "PID {} EcoQoS 已切换为 {}",
                            pid,
                            if make_eco { "Eco (🍃)" } else { "Normal" }
                        ));
                        // 立即刷新 detail_process 的 throttled 字段，避免
                        // 用户等下一个 heavy tick（~2s）才看到 UI 更新。
                        if let Some(p) = self.inspector.detail_process.as_mut() {
                            p.throttled = if make_eco {
                                crate::throttle::EcoQoSState::Eco
                            } else {
                                crate::throttle::EcoQoSState::Normal
                            };
                        }
                    }
                    Err(e) => {
                        tracing::warn!("EcoQoS 切换失败 (PID {}): {}", pid, e);
                        self.status_message = Some(format!("EcoQoS 切换失败: {}", e));
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
    /// 应用到 15+ panel / metrics 字段；`BookmarkPanelToggled` → 设 status 提示；
    /// `SearchInputToggled` / `SearchMatchesUpdated` → 设 status 提示命中数；
    /// `DirectionToggled` → 设 status 提示当前方向；`Noop` 不动。
    fn dispatch_replay_action(&mut self, action: ReplayAction) {
        match action {
            ReplayAction::Noop => {}
            ReplayAction::Quit => self.should_quit = true,
            ReplayAction::ApplyFrame => self.apply_replay_frame(),
            ReplayAction::BookmarkPanelToggled => {
                let open = self.replay.bookmark_panel.is_some();
                self.status_message = Some(if open {
                    "书签面板：Up/Down 选择 · Enter 跳转 · e 编辑 · d 删除 · Esc 关闭 / 搜索"
                        .to_string()
                } else {
                    "书签面板已关闭".to_string()
                });
            }
            ReplayAction::SearchInputToggled => {
                let active = self.replay.search_input_active;
                self.status_message = Some(if active {
                    "搜索输入：输入表达式 / substring，Enter 提交 / Esc 取消（n/N 跳转）"
                        .to_string()
                } else {
                    "搜索输入已退出（n/N 仍可跳转命中帧）".to_string()
                });
            }
            ReplayAction::SearchMatchesUpdated => {
                let n = self.replay.search.matches.len();
                let error_msg = self
                    .replay
                    .search
                    .error
                    .as_ref()
                    .map(|e| format!("  ⚠ {}", e.msg))
                    .unwrap_or_default();
                self.status_message = Some(format!("搜索命中 {n} 帧{error_msg}"));
            }
            ReplayAction::DirectionToggled => {
                let dir = self.replay.timeline_state.as_ref().map(|ts| ts.direction);
                self.status_message = Some(match dir {
                    Some(ReplayDirection::Reverse) => "倒放中（再按 r 切回正向）".to_string(),
                    Some(ReplayDirection::Forward) | None => {
                        "正向播放（再按 r 切倒放）".to_string()
                    }
                });
            }
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
        self.process_panel.panel.tree_nodes = frame.tree_nodes.iter().map(TreeNode::from).collect();
        self.port_panel.panel.port_entries =
            frame.port_entries.iter().map(PortEntry::from).collect();
        self.port_panel.panel.port_view_mode =
            NetworkViewMode::from_frame_code(frame.port_view_mode);
        self.usb_panel.panel.devices = frame
            .usb_devices
            .iter()
            .map(RemovableDevice::from)
            .collect();
        self.usb_panel.panel.locks = frame
            .usb_locks
            .iter()
            .map(|l| (HandleLock::from(l), HandleRisk::from(l)))
            .collect();
        self.docker_panel.panel.containers = frame
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
        self.process_panel.panel.process_view_mode = match frame.process_view_mode {
            1 => ProcessViewMode::Tree,
            2 => ProcessViewMode::AppGroup,
            _ => ProcessViewMode::List,
        };

        if self.process_panel.panel.process_view_mode == ProcessViewMode::AppGroup {
            self.process_panel.panel.app_groups = crate::app_group::compute_groups(
                &self.cached_processes[..],
                &mut self.process_panel.panel.version_info_cache,
            );
            self.process_panel.panel.app_group_sort_groups();
        }
    }

    fn restore_replay_nav(&mut self, nav: crate::record::frame::FrameNav) {
        self.process_panel.panel.cursor_index = nav.cursor;
        self.process_panel.panel.scroll_offset = nav.scroll;
        self.process_panel.panel.selected_pids = nav.selected.into_iter().collect();
        self.process_panel.panel.tree_cursor = nav.tree_cursor;
        self.process_panel.panel.tree_scroll = nav.tree_scroll;
        self.process_panel.panel.tree_selected_pids = nav.tree_selected.into_iter().collect();
        self.port_panel.panel.port_cursor = nav.port_cursor;
        self.port_panel.panel.port_scroll = nav.port_scroll;
        self.port_panel.panel.port_process_cursor = nav.port_process_cursor;
        self.port_panel.panel.port_process_scroll = nav.port_process_scroll;
        self.port_panel.panel.port_remote_cursor = nav.port_remote_cursor;
        self.port_panel.panel.port_remote_scroll = nav.port_remote_scroll;
        self.usb_panel.panel.device_cursor = nav.usb_device_cursor;
        self.monitor_panel.panel.cursor = nav.monitor_cursor;
        self.docker_panel.panel.cursor = nav.docker_cursor;
        self.docker_panel.panel.scroll = nav.docker_scroll;
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
                    self.process_panel.panel.selected_pids.clear();
                    self.process_panel.panel.tree_selected_pids.clear();
                    if let Err(e) = self.snapshot.refresh() {
                        tracing::warn!("刷新进程列表失败: {}", e);
                    }
                    self.process_panel
                        .panel
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
            || self.process_panel.panel.process_view_mode != ProcessViewMode::List
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
        let clicked_index = data_row as usize + self.process_panel.panel.scroll_offset;

        use crossterm::event::{MouseButton, MouseEventKind};
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) if clicked_index < self.cached_sorted.len() => {
                self.process_panel.panel.cursor_index = clicked_index;
                // Toggle select
                if let Some((idx, _)) = self.cached_sorted.get(clicked_index) {
                    let pid = self.cached_processes[*idx].pid;
                    if self.process_panel.panel.selected_pids.contains(&pid) {
                        self.process_panel.panel.selected_pids.remove(&pid);
                    } else {
                        self.process_panel.panel.selected_pids.insert(pid);
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
                if self.process_panel.panel.process_view_mode == ProcessViewMode::Tree {
                    self.process_panel
                        .panel
                        .tree_move_cursor(lines, &self.cached_processes[..]);
                } else if self.process_panel.panel.process_view_mode == ProcessViewMode::AppGroup {
                    self.process_panel
                        .panel
                        .app_group_move_cursor(lines, &self.cached_processes[..]);
                } else {
                    self.process_panel
                        .panel
                        .move_cursor(lines, &self.cached_sorted);
                }
            }
            AppMode::PortMap => {
                let total = self.port_panel.panel.visible_port_count();
                if total == 0 {
                    return;
                }
                let new = self.port_panel.panel.port_cursor as i32 + lines;
                self.port_panel.panel.port_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::DockerPanel => {
                let total = self.docker_panel.panel.containers.len();
                if total == 0 {
                    return;
                }
                let new = self.docker_panel.panel.cursor as i32 + lines;
                self.docker_panel.panel.cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::UsbAssistant => {
                let total = self.usb_panel.panel.devices.len();
                if total == 0 {
                    return;
                }
                let new = self.usb_panel.panel.device_cursor as i32 + lines;
                self.usb_panel.panel.device_cursor = new.clamp(0, (total - 1) as i32) as usize;
            }
            AppMode::MonitorPanel => {
                let total = self.monitor_panel.panel.manager.list_monitors().len();
                if total == 0 {
                    return;
                }
                let new = self.monitor_panel.panel.cursor as i32 + lines;
                self.monitor_panel.panel.cursor = new.clamp(0, (total - 1) as i32) as usize;
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
        self.process_panel.panel.get_selected_pids()
    }

    // --- Tick ---

    pub fn tick(&mut self) -> bool {
        let mut needs_draw = self.data_dirty;

        // v0.6.0 阶段 3：drain worker crash → banner。每帧都跑，第一时间反馈。
        self.poll_crashes();

        // v0.21 stage 3：Agent 面板每帧 drain SessionEvent（流式响应性，D6）。
        if self.mode == AppMode::Agent {
            self.tick_agent();
        }

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
            self.overlay_flow_sni_schannel();
            // v0.11.0 阶段 1（ADR-0019）：每 1s 检查 restart_history，
            // backoff 到期的 worker 触发 respawn。restart 状态变化触发 banner
            // 重绘（pending_redraw）。
            self.restart_tick();
            self.tick_panels();
            self.tick_usb_monitor_docker();
            self.tick_self_monitor();
            needs_draw = true;
        }

        if self.data_dirty {
            self.rebuild_sorted_cache();
            needs_draw = true;
        }

        if self.port_panel.panel.port_filter_dirty {
            self.port_panel.panel.rebuild_port_filters();
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
                    self.overlay_disk_speeds_etw();
                    self.update_net_rates();

                    let alive_pids: HashSet<u32> =
                        self.cached_processes.iter().map(|p| p.pid).collect();
                    self.security_scores
                        .retain(|pid, _| alive_pids.contains(pid));

                    // v0.10 阶段 4（REVIEW-11 P1-1）：Schannel-only flow 退出感知。
                    // Schannel event 自带 PID 但无进程退出事件——所有 flow 都直接
                    // 在 App::flows 里（v0.12 移除 ebpf 路径后无 aggregator）。
                    // 这里在 heavy refresh 拿到 alive_pids 时同步打 exit_time，
                    // 后续 overlay_flow_sni_schannel 内的 reaper 段按 GHOST_FLOW_TTL
                    // 移除。schannel_etw_worker 为 None 时跳过。
                    if self.workers.schannel_etw_worker.is_some() {
                        let now = std::time::SystemTime::now();
                        crate::flow::mark_dead_flows(&mut self.flows, &alive_pids, now);
                    }

                    // v0.6.0 阶段 5：detail_process 维护 + priority/affinity 缓存
                    // 刷新封装到 InspectorController。
                    self.inspector.sync_detail(&self.cached_processes);
                    self.inspector.refresh_detail_priority();

                    self.data_dirty = true;

                    if !self.scoring_pending {
                        let procs = Arc::clone(&self.cached_processes);
                        let ports = std::sync::Arc::new(self.port_panel.panel.port_entries.clone());
                        let flows = std::sync::Arc::new(self.flows.clone());
                        self.background_scorer.request(procs, ports, flows);
                        self.scoring_pending = true;
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!("刷新进程列表失败: {}", e),
            }
        }

        if let Some(scores) = self.background_scorer.poll_results() {
            // v0.11 阶段 4（ADR-0021）：poll 后把 score.signature 反向同步到
            // cached_processes[*].signature_status，让 UI（进程列表 emoji /
            // Inspector Summary）能显示最新结果而非 ProcessInfo 默认值 Pending。
            // Arc::make_mut：scoring 持有旧 Arc 时触发一次 COW 复制（每 heavy
            // refresh 至多一次），与 update_disk_speeds 同款 zero-cost 路径。
            let procs = Arc::make_mut(&mut self.cached_processes);
            for proc in procs {
                if let Some(score) = scores.get(&proc.pid)
                    && score.signature != proc.signature_status
                {
                    proc.signature_status = score.signature;
                }
            }
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

    /// v0.7 阶段 7：从 [`disk_io_etw_worker`] drain 最新一份 per-PID BPS，
    /// **覆盖** `update_disk_speeds` 写入的 sysinfo delta（ETW 更准）。
    ///
    /// 设计：
    /// - 先跑 `update_disk_speeds`（sysinfo delta 填充），再用 ETW 数据覆写匹配 PID
    /// - ETW 缺失的 PID（thread_map 没刷到 / 未知 kernel thread）保留 sysinfo 值
    /// - 无 worker（非 Windows / 非管理员 / session 占用）→ 整个方法 no-op
    /// - drain 后 worker 内部 sync_channel(1) 空，下一次 tick 不会有「旧帧」污染
    fn overlay_disk_speeds_etw(&mut self) {
        let Some(worker) = &self.workers.disk_io_etw_worker else {
            return;
        };
        let Some(map) = worker.try_recv_latest() else {
            return;
        };

        let procs = Arc::make_mut(&mut self.cached_processes);
        for proc in procs.iter_mut() {
            if let Some(stats) = map.get(&proc.pid) {
                proc.disk_read_speed = stats.read_bps;
                proc.disk_write_speed = stats.write_bps;
            }
        }
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

    /// v0.10 阶段 3 / v0.12 阶段 2：从 [`schannel_etw_worker`] drain 最新一份
    /// `Vec<SniRecord>`，把 SNI 覆盖到匹配 pid 的 `ProcessFlow.sni` 上。
    /// 没匹配上的 record 直接 push 一条新的 flow（Windows-only 后 Schannel
    /// 是唯一来源）。
    ///
    /// 设计：
    /// - **匹配键：pid**（Schannel event 自带 `EVENT_HEADER.ProcessId`，与
    ///   `ProcessFlow.pid` 直接对齐，不需要 thread_map）。同一 pid 多条 flow 时
    ///   **全部覆盖**——Schannel event 没给 remote_addr，无法精确关联到具体 flow。
    /// - **PID 复用**：从 `cached_processes` 查 pid → (start_time, comm)；
    ///   找不到的 record（进程已退 / sysinfo 未刷到）start_time 留 0、comm 留空。
    /// - drain 后 worker 内部 sync_channel(1) 空，下一次 tick 不会有「旧帧」污染。
    fn overlay_flow_sni_schannel(&mut self) {
        let Some(worker) = &self.workers.schannel_etw_worker else {
            return;
        };
        let Some(records) = worker.try_recv_latest() else {
            return;
        };
        if records.is_empty() {
            return;
        }

        // 用 cached_processes 建 pid → (start_time, comm) map（O(1) 查询，
        // 避免每个 record 全量 scan）。
        let pid_info: HashMap<u32, (u64, String)> = self
            .cached_processes
            .iter()
            .map(|p| (p.pid, (p.start_time, p.name.to_string())))
            .collect();

        for rec in records {
            let mut matched = false;
            for flow in self.flows.iter_mut() {
                if flow.pid == rec.pid {
                    flow.sni = Some(rec.sni.clone());
                    // Schannel event 时间戳更精确（来自 EVENT_HEADER.TimeStamp），
                    // 刷新 last_seen。
                    flow.last_seen = rec.ts;
                    matched = true;
                }
            }
            if !matched {
                // 新建 flow。remote_addr / remote_port / bytes / dns_name 留空 /
                // None（Schannel event 不给 socket 元数据 + 不参与 DNS 关联）。
                let (start_time, comm) = pid_info
                    .get(&rec.pid)
                    .map(|(st, n)| (*st, n.clone()))
                    .unwrap_or((0, String::new()));
                self.flows.push(crate::flow::ProcessFlow {
                    pid: rec.pid,
                    start_time,
                    comm,
                    local_addr: String::new(),
                    remote_addr: String::new(),
                    remote_port: 0,
                    bytes_out: 0,
                    bytes_in: 0,
                    dns_name: None,
                    sni: Some(rec.sni.clone()),
                    first_seen: rec.ts,
                    last_seen: rec.ts,
                    exit_time: None,
                });
            }
        }

        // 新 push 后需重排保持 last_seen 倒序。
        self.flows.sort_by_key(|f| std::cmp::Reverse(f.last_seen));

        // v0.10 阶段 4（REVIEW-11 P1-1）：reaper expired ghost flows。
        // exit_time 由 tick_light_refresh 在 alive_pids 不含其 pid 时打上；
        // 30s 后这里 retain 移除。
        let now = std::time::SystemTime::now();
        crate::flow::reap_expired_flows(&mut self.flows, now);

        // v0.11 阶段 3：clamp 到过滤后总数，搜索 / FilterExpr 收窄后光标不越界。
        let total = self
            .port_panel
            .panel
            .flow_filtered_indices(&self.flows)
            .len();
        self.port_panel.panel.flow_clamp_cursor(total);
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
            flows: &self.flows,
        };
        if self.mode == AppMode::ProcessList {
            self.process_panel.panel.tick(&mut ctx);
        } else if self.mode == AppMode::PortMap {
            if let Some(sockets) = new_sockets {
                self.port_panel.panel.pending_sockets = Some(sockets);
            }
            self.port_panel.panel.tick(&mut ctx);
        }
    }

    fn tick_usb_monitor_docker(&mut self) {
        // USB 设备列表由后台 UsbSnapshotWorker 每 ~5s 推一次;主线程只 try_recv
        // + 合并 is_occupied 状态。设备锁查询(scan_device_locks_with_processes)
        // 仍按需在 UsbPanel 内同步触发(用户按 r / Enter)。
        if self.mode == AppMode::UsbAssistant
            && let Some(snap) = self.workers.usb_worker.try_recv_latest()
        {
            self.usb_panel.panel.merge_devices(snap.devices);
        }
        self.monitor_panel.panel.poll_events();
        if self.mode == AppMode::DockerPanel {
            self.docker_panel.panel.poll_events();
        }
    }

    fn clamp_cursors(&mut self) {
        let total = self.cached_sorted.len();
        if self.process_panel.panel.cursor_index >= total && total > 0 {
            self.process_panel.panel.cursor_index = total - 1;
        }
    }

    fn rebuild_sorted_cache(&mut self) {
        let processes: &[ProcessInfo] = &self.cached_processes[..];
        let search = &self.process_panel.panel.search;
        // v0.7.0 阶段 4：按 QueryMode 分支。
        // - Substring：v0.6 行为 100% 保留（name_lower.contains(q_lower) || pid.contains）。
        // - FilterExpr：用 search.filter_expr（None 时无过滤；parse 失败时保留上一次成功 AST，
        //   见 SearchState::reparse_filter）。security_score 从 self.security_scores 查，
        //   ProcessInfo 本身不持分数。
        let filtered: Vec<&ProcessInfo> = match search.mode {
            crate::search::QueryMode::Substring => {
                let query = search.query();
                if query.is_empty() {
                    processes.iter().collect()
                } else {
                    // v0.6.0 阶段 4：搜索 query 一次性 lowercase（O(query.len())），
                    // 进程名匹配走 `ProcessInfo::name_lower` 预算字段，避免每进程每按键
                    // `to_lowercase` 分配。
                    let q_lower = search.query_lower();
                    processes
                        .iter()
                        .filter(|p| {
                            p.name_lower.contains(q_lower) || p.pid.to_string().contains(query)
                        })
                        .collect()
                }
            }
            crate::search::QueryMode::FilterExpr => match &search.filter_expr {
                Some(expr) => {
                    let security_scores = &self.security_scores;
                    let total_memory = self.snapshot.memory_usage().1;
                    processes
                        .iter()
                        .filter(|p| {
                            let score = security_scores.get(&p.pid).map(|s| s.score);
                            let ctx = crate::filter::EvalCtx {
                                process: p,
                                security_score: score,
                                total_memory,
                            };
                            expr.apply(&ctx)
                        })
                        .collect()
                }
                None => processes.iter().collect(),
            },
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

        let sort_field = self.process_panel.panel.sort_field;
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
        // v0.21 stage 3：Agent 会话有界收尾（mid-run 退出防 llama-server 孤儿——
        // session 线程被进程退出强杀时 LlamaServerHandle::Drop 不会执行）。
        self.teardown_agent_session();
        for handle in &self.monitor_panel.panel.watchdog_handles {
            handle.stop();
        }
        for handle in &self.monitor_panel.panel.port_handles {
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
        self.usb_panel
            .panel
            .scan_devices(&self.cached_processes[..]);
    }

    pub fn docker_refresh(&mut self) {
        self.docker_panel.panel.refresh();
    }

    pub fn monitor_poll_events(&mut self) {
        self.monitor_panel.panel.poll_events();
    }

    pub fn docker_poll_events(&mut self) {
        self.docker_panel.panel.poll_events();
    }

    pub fn filtered_ports(&self) -> &[PortEntry] {
        self.port_panel.panel.filtered_ports()
    }

    pub fn filtered_process_groups(&self) -> &[crate::port_map::ProcessNetGroup] {
        self.port_panel.panel.filtered_process_groups()
    }

    pub fn filtered_remote_groups(&self) -> &[crate::port_map::RemoteGroup] {
        self.port_panel.panel.filtered_remote_groups()
    }

    pub fn anomaly_count(&self) -> usize {
        self.port_panel.panel.anomaly_count()
    }

    pub fn visible_anomalies(&self) -> Vec<&crate::anomaly::Anomaly> {
        self.port_panel.panel.visible_anomalies()
    }

    pub fn dismiss_anomaly(&mut self, id: &str) {
        self.port_panel.panel.dismiss_anomaly(id);
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

    // ---- v0.7.0 阶段 3：命令面板 ----

    /// 当前生效的层（Normal / Search / Palette）。Search 是从 panel search 状态
    /// 派生出来的逻辑层；Palette 由 `active_layer` 字段显式覆盖。
    #[must_use]
    pub fn current_layer(&self) -> AppLayer {
        if self.active_layer == AppLayer::Palette {
            AppLayer::Palette
        } else if self.any_search_active() {
            AppLayer::Search
        } else {
            AppLayer::Normal
        }
    }

    /// 是否打开了命令面板浮层（语义上等价于 `active_layer == Palette`，但名字
    /// 更直观，给测试 / 渲染层用）。
    #[must_use]
    pub fn is_palette_open(&self) -> bool {
        self.active_layer == AppLayer::Palette
    }

    fn any_search_active(&self) -> bool {
        self.process_panel.panel.search.is_active()
            || self.process_panel.panel.tree_search.is_active()
            || self.process_panel.panel.app_group_search.is_active()
            || self.port_panel.panel.port_search.is_active()
            || self.inspector.inspection_search.is_active()
    }

    /// Replay / ContainerExec 模式下，按键被内部时间轴 / PTY 完全捕获，命令面板
    /// 不应该拦截。Help 模式同理（用户在读帮助，按 ?/Esc 退出）。
    #[must_use]
    fn keyboard_captured_by_inner_mode(&self) -> bool {
        matches!(
            self.mode,
            AppMode::Replay | AppMode::ContainerExec | AppMode::Help | AppMode::Agent
        )
    }

    /// 命令面板执行选中项。各 CommandAction 映射到既有的副作用路径 ——
    /// 简单状态切换直接改字段，复杂操作（kill / docker restart）调用既有 panel 方法。
    fn dispatch_command_action(&mut self, action: CommandAction) {
        match action {
            CommandAction::Quit => self.should_quit = true,
            CommandAction::SwitchPanel(mode) => self.switch_mode(mode),
            CommandAction::SetProcessViewMode(view) => {
                self.switch_mode(AppMode::ProcessList);
                self.process_panel.panel.process_view_mode = view;
                self.process_panel.panel.cursor_index = 0;
                self.process_panel.panel.scroll_offset = 0;
                self.status_message = Some(format!("视图: {}", view_mode_label(view)));
            }
            CommandAction::SortBy(field) => {
                self.process_panel.panel.sort_field = field;
                crate::ui_state::save_sort_field(field);
                self.data_dirty = true;
                self.status_message = Some(format!("排序: {}", sort_field_label(field)));
            }
            CommandAction::SwitchInspectionTab(tab) => {
                if self.mode == AppMode::ProcessDetail {
                    self.inspector.inspection_tab = tab;
                    self.inspector.inspection_scroll = 0;
                    self.status_message = Some(format!("详情 Tab: {}", tab.label()));
                } else {
                    self.status_message = Some("请先进入详情页（选中进程按 Enter）".to_string());
                }
            }
            CommandAction::RefreshInspector => {
                if self.mode == AppMode::ProcessDetail {
                    let ports = self.port_panel.panel.port_entries.clone();
                    self.inspector.open(&ports);
                    self.status_message = Some("详情页已刷新".to_string());
                }
            }
            CommandAction::EnterDetail => {
                if self.mode == AppMode::ProcessList {
                    self.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                }
            }
            CommandAction::KillCursor => {
                if self.mode == AppMode::ProcessList {
                    self.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
                }
            }
            CommandAction::ForceKillCursor => {
                if self.mode == AppMode::ProcessList {
                    self.handle_key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT));
                }
            }
            CommandAction::SelectAllVisible => {
                if self.mode == AppMode::ProcessList {
                    self.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
                }
            }
            CommandAction::CycleTheme => {
                crate::tui::theme::cycle_theme();
                self.status_message = Some(format!("主题: {}", crate::tui::theme::theme_name()));
            }
            CommandAction::SetTheme(idx) => {
                crate::tui::theme::set_theme_index(idx);
                self.status_message = Some(format!("主题: {}", crate::tui::theme::theme_name()));
            }
            CommandAction::ToggleSidebar => {
                self.sidebar_expanded = !self.sidebar_expanded;
                crate::ui_state::save_sidebar_expanded(self.sidebar_expanded);
                self.status_message = Some(if self.sidebar_expanded {
                    "侧边栏：展开（per-core 频率/温度）".to_string()
                } else {
                    "侧边栏：折叠".to_string()
                });
            }
            CommandAction::ToggleHelp => {
                crate::ui_state::mark_first_run_done();
                self.mode = AppMode::Help;
                self.help_scroll = 0;
            }
            CommandAction::ToggleAlertPopup => {
                self.alert_popup_open = !self.alert_popup_open;
                self.alert_scroll = 0;
            }
            CommandAction::ToggleRecording => self.toggle_recording(),
            CommandAction::DismissCrashes => self.dismiss_all_crashes(),
            CommandAction::DockerStartEvents => {
                if self.mode == AppMode::DockerPanel
                    && self.docker_panel.panel.connected
                    && self.docker_panel.panel.event_receiver.is_none()
                {
                    self.docker_panel.panel.start_watching();
                }
            }
            CommandAction::DockerStopContainer => {
                if self.mode == AppMode::DockerPanel {
                    self.docker_panel.panel.palette_stop_selected();
                }
            }
            CommandAction::DockerRestartContainer => {
                if self.mode == AppMode::DockerPanel {
                    self.docker_panel.panel.palette_restart_selected();
                }
            }
        }
    }
}

/// Ctrl+P 检测：crossterm 把 Ctrl+P 编码为 `Char('p')` + `Modifiers::CONTROL`
/// （Shift 状态用大小写区分，所以也兼容 `Char('P')`）。
fn is_ctrl_p(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('p') | KeyCode::Char('P'))
}

fn view_mode_label(m: ProcessViewMode) -> &'static str {
    match m {
        ProcessViewMode::List => "列表",
        ProcessViewMode::Tree => "进程树",
        ProcessViewMode::AppGroup => "应用分组",
    }
}

fn sort_field_label(f: SortField) -> &'static str {
    match f {
        SortField::Cpu => "CPU",
        SortField::Memory => "内存",
        SortField::Pid => "PID",
        SortField::Name => "名称",
        SortField::Security => "安全分",
        SortField::DiskRead => "磁盘读",
        SortField::DiskWrite => "磁盘写",
        SortField::NetSent => "上行",
        SortField::NetRecv => "下行",
    }
}
