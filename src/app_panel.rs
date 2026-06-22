use std::collections::{HashMap, VecDeque};

use crossterm::event::KeyEvent;

use crate::alert::AlertManager;
use crate::classify;
use crate::collect::{ProcessInfo, SystemSnapshot};
use crate::dns_log::DnsQuery;
use crate::security::SecurityScore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    ProcessList,
    PortMap,
    UsbAssistant,
    MonitorPanel,
    DockerPanel,
    ProcessDetail,
    /// 阶段 9 E2：容器 exec 嵌入式 PTY 视图。从 DockerPanel 按 `e` 进入，
    /// `Ctrl+D` / 输入 `exit` / `Ctrl+\` / `Esc` 退出回 DockerPanel。
    ContainerExec,
    Replay,
    Help,
}

/// Inspector 内部 Tab（阶段 13，ADR-0004；阶段 1 扩为 6 变体）。
///
/// v1：Summary / Env / Network / Dlls（4 个，已上线）。
/// v2：追加 Handles / Memory（声明 + 占位 UI；实现在阶段 4 上线）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectionTab {
    #[default]
    Summary,
    Env,
    Network,
    Dlls,
    /// 进程打开的所有句柄（文件 / 注册表 / 事件 / 信号量等）。阶段 4 上线。
    Handles,
    /// VirtualQueryEx / /proc/<pid>/maps 内存映射。阶段 4 上线。
    Memory,
}

impl InspectionTab {
    /// Tab 栏上的中文显示文字（也是测试的稳定 anchor）。
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Summary => "概要",
            Self::Env => "环境",
            Self::Network => "网络",
            Self::Dlls => "DLL",
            Self::Handles => "句柄",
            Self::Memory => "内存",
        }
    }

    /// `Tab` 键正向切换。next 是循环的：Memory → Summary。
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Summary => Self::Env,
            Self::Env => Self::Network,
            Self::Network => Self::Dlls,
            Self::Dlls => Self::Handles,
            Self::Handles => Self::Memory,
            Self::Memory => Self::Summary,
        }
    }

    /// `Shift+Tab`（BackTab）逆向切换。prev 也是循环的：Summary → Memory。
    #[must_use]
    pub fn prev(self) -> Self {
        match self {
            Self::Summary => Self::Memory,
            Self::Env => Self::Summary,
            Self::Network => Self::Env,
            Self::Dlls => Self::Network,
            Self::Handles => Self::Dlls,
            Self::Memory => Self::Handles,
        }
    }

    /// 列举全部 6 个 Tab —— Tab 栏渲染用。顺序 = next 循环顺序。
    #[must_use]
    pub fn all() -> &'static [InspectionTab] {
        const ALL: &[InspectionTab] = &[
            InspectionTab::Summary,
            InspectionTab::Env,
            InspectionTab::Network,
            InspectionTab::Dlls,
            InspectionTab::Handles,
            InspectionTab::Memory,
        ];
        ALL
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppGroupSortField {
    Cpu,
    Memory,
    ProcessCount,
}

impl AppGroupSortField {
    #[must_use]
    pub fn next(&self) -> Self {
        match self {
            Self::Cpu => Self::Memory,
            Self::Memory => Self::ProcessCount,
            Self::ProcessCount => Self::Cpu,
        }
    }

    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "内存",
            Self::ProcessCount => "进程数",
        }
    }
}

pub struct KillRequest {
    pub pids: Vec<u32>,
    pub force: bool,
}

pub struct OpRecord {
    pub time: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum MonitorAddSubmenu {
    SelectType,
    EnterPid {
        input: String,
    },
    EnterPort {
        input: String,
    },
    EnterCommand {
        cmd_input: String,
        args_input: String,
        cwd_input: String,
        retries_input: String,
    },
}

/// Result of a panel handling a key event.
#[derive(Debug)]
pub enum KeyResult {
    /// Key was consumed by the panel.
    Consumed,
    /// Key was not handled by the panel.
    Ignored,
    /// Application should quit.
    Quit,
    /// Switch to a different mode.
    SwitchMode(AppMode),
    /// Toggle recording on/off.
    ToggleRecording,
}

/// Shared context passed to panels during tick and key handling.
pub struct PanelContext<'a> {
    // Read-only shared data
    pub snapshot: &'a SystemSnapshot,
    pub cached_processes: &'a [ProcessInfo],
    pub cached_sorted: &'a [(usize, classify::ProcessClass)],
    pub security_scores: &'a HashMap<u32, SecurityScore>,

    // Mutable shared state — panels can write feedback here
    pub status_message: &'a mut Option<String>,
    pub detail_process: &'a mut Option<ProcessInfo>,
    pub pending_kill: &'a mut Option<KillRequest>,
    pub data_dirty: &'a mut bool,
    pub pending_redraw: &'a mut bool,
    pub alert_manager: &'a mut AlertManager,
    pub op_history: &'a mut VecDeque<OpRecord>,

    /// 阶段 8 D3 DNS 查询日志（仅内存缓冲，cap=1000 FIFO）。
    /// PortPanel 在 DNS 子视图中按 `c` 清空。详见 [`crate::dns_log`]。
    pub dns_log_recent: &'a mut VecDeque<DnsQuery>,

    /// 阶段 9 E2：DockerPanel 按 `e` 进入容器 exec 模式时设置容器名，
    /// App::handle_key 看到 `SwitchMode(ContainerExec)` 后取出启动 PTY。
    pub pending_container_exec: &'a mut Option<String>,
}

/// Trait for a TUI panel that owns its own state.
pub trait Panel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult;
    fn tick(&mut self, ctx: &mut PanelContext) -> bool;
    fn cursor(&self) -> usize;
    fn scroll(&self) -> usize;
}
