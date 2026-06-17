use std::collections::{HashMap, VecDeque};

use crossterm::event::KeyEvent;

use crate::alert::AlertManager;
use crate::classify;
use crate::collect::{ProcessInfo, SystemSnapshot};
use crate::security::SecurityScore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    ProcessList,
    PortMap,
    UsbAssistant,
    MonitorPanel,
    DockerPanel,
    ProcessDetail,
    Replay,
    Help,
}

/// Inspector 内部 Tab（阶段 13，ADR-0004）。
///
/// v1 范围：Summary / Env / Network / Dlls（4 个）。
/// v2 计划追加 Handles / Windows（在 0.5.0+）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectionTab {
    #[default]
    Summary,
    Env,
    Network,
    Dlls,
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
        }
    }

    /// `Tab` 键正向切换。next 是循环的：Dlls → Summary。
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Summary => Self::Env,
            Self::Env => Self::Network,
            Self::Network => Self::Dlls,
            Self::Dlls => Self::Summary,
        }
    }

    /// `Shift+Tab`（BackTab）逆向切换。prev 也是循环的：Summary → Dlls。
    #[must_use]
    pub fn prev(self) -> Self {
        match self {
            Self::Summary => Self::Dlls,
            Self::Env => Self::Summary,
            Self::Network => Self::Env,
            Self::Dlls => Self::Network,
        }
    }

    /// 列举全部 v1 Tab —— Tab 栏渲染用。
    #[must_use]
    pub fn all() -> &'static [InspectionTab] {
        const ALL: &[InspectionTab] = &[
            InspectionTab::Summary,
            InspectionTab::Env,
            InspectionTab::Network,
            InspectionTab::Dlls,
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
}

/// Trait for a TUI panel that owns its own state.
pub trait Panel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult;
    fn tick(&mut self, ctx: &mut PanelContext) -> bool;
    fn cursor(&self) -> usize;
    fn scroll(&self) -> usize;
}
