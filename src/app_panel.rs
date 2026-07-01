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

impl std::fmt::Debug for KillRequest {
    /// v0.7.0 阶段 5：手动 impl Debug，避免在 `PanelAction::Kill` derive 时炸。
    /// pids 列表可能含几十个 PID，截断到前 8 个避免日志膨胀。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pids_str = if self.pids.len() > 8 {
            format!("{:?}...+{}]", self.pids[..8].to_vec(), self.pids.len() - 8)
        } else {
            format!("{:?}", self.pids)
        };
        f.debug_struct("KillRequest")
            .field("pids", &pids_str)
            .field("force", &self.force)
            .finish()
    }
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

/// v0.7.0 阶段 5：PanelController 的 handle_key 返回值。
///
/// 与 v0.6.0 `KeyResult` 共存（surgical：v0.8 评估合并）。`KeyResult` 是
/// `Panel` trait 的返回值（v0.6 旧路径），`PanelAction` 是 `XxxPanelController`
/// 的返回值（v0.7 新路径）。当前 5 个 controller 都只是把 inner panel 的
/// `KeyResult` 翻译成 `PanelAction`；未来 controller 主动产副作用（如 docker
/// exec spawn）时直接 emit `PanelAction::Kill` / `Clipboard` 等变体，不再走
/// `PanelContext` mutable ref。
///
/// **对应 ADR-0012**。
#[derive(Debug)]
pub enum PanelAction {
    /// 默认：无副作用（controller 已消化完状态变更）。
    Noop,
    /// 全局退出（q 键）。
    Quit,
    /// 切面板（1-6 / Tab）。App 调 `switch_mode`。
    SwitchMode(AppMode),
    /// 录屏开关（R 键）。
    ToggleRecording,
    /// 写一行 status_message（Docker / Inspector 等刷新结果）。
    StatusMessage(String),
    /// 请求 kill 进程。App 弹 kill_confirm dialog。
    Kill(KillRequest),
    /// 复制到剪贴板（Inspector y / Docker copy-id 等）。
    Clipboard(String),
}

impl From<KeyResult> for PanelAction {
    /// v0.7 阶段 5：旧 `KeyResult` 翻译为新 `PanelAction`。controller 包了 panel
    /// 后用此 fn 转换；v0.8 评估彻底废掉 `KeyResult` 时移除。
    fn from(r: KeyResult) -> Self {
        match r {
            KeyResult::Quit => Self::Quit,
            KeyResult::SwitchMode(m) => Self::SwitchMode(m),
            KeyResult::ToggleRecording => Self::ToggleRecording,
            // Consumed / Ignored 都对 App 无副作用，统一映射 Noop。
            KeyResult::Consumed | KeyResult::Ignored => Self::Noop,
        }
    }
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

    /// v0.11 阶段 3：当前 ProcessFlow 快照（`App::flows` 的引用）。Flow 子视图
    /// 在 FilterExpr 模式下用此切片走 `apply_network` 过滤；substring 模式下
    /// 走 sni/dns_name/comm/remote_addr 子串匹配。
    pub flows: &'a [crate::ebpf::flow::ProcessFlow],
}

/// Trait for a TUI panel that owns its own state.
pub trait Panel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult;
    fn tick(&mut self, ctx: &mut PanelContext) -> bool;
    fn cursor(&self) -> usize;
    fn scroll(&self) -> usize;
}
