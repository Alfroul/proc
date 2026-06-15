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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppGroupSortField {
    Cpu,
    Memory,
    ProcessCount,
}

impl AppGroupSortField {
    pub fn next(&self) -> Self {
        match self {
            Self::Cpu => Self::Memory,
            Self::Memory => Self::ProcessCount,
            Self::ProcessCount => Self::Cpu,
        }
    }

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
