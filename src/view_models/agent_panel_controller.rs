//! v0.21 stage 1：AgentPanel 的 controller 骨架（ADR-0031 / ADR-0012 模式）。
//!
//! 面板状态机（Idle / Streaming / AwaitingConfirm）+ 输入缓冲声明。键位状态机
//! （Enter 发送 / Esc 中断 / y n confirm / PgUp PgDn 滚动）stage 3 接线——
//! stage 1 的 App 层对退出键（Ctrl+D / Esc）内联处理，不触达
//! [`AgentPanelController::handle_key`]。

use crossterm::event::KeyEvent;

use crate::app_panel::{PanelAction, PanelContext};

/// 面板内部状态机（决策 D3 / 风险 2：confirm 挂起生命周期）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentPanelMode {
    /// 空闲：输入框可编辑，Enter 发送。
    #[default]
    Idle,
    /// 生成中：streaming 进行，Esc 中断。
    Streaming,
    /// 等待写操作确认：`y` 执行 / `n` 拒绝。
    AwaitingConfirm,
}

/// Agent 面板状态（stage 1 只声明输入缓冲 + 状态机；messages 渲染缓冲 /
/// confirm pending / session handle stage 2/3 补）。
pub struct AgentPanel {
    /// 输入缓冲（字符 / Backspace / 中文输入，复用 SearchState 同款处理思路）。
    pub input: String,
    /// 面板状态机。
    pub mode: AgentPanelMode,
}

impl Default for AgentPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPanel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            input: String::new(),
            mode: AgentPanelMode::Idle,
        }
    }
}

/// AgentPanel 的 controller 包装（与既有 5 个 XxxPanelController 同款结构）。
pub struct AgentPanelController {
    pub panel: AgentPanel,
}

impl Default for AgentPanelController {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPanelController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            panel: AgentPanel::new(),
        }
    }

    #[must_use]
    pub fn panel(&self) -> &AgentPanel {
        &self.panel
    }

    #[must_use]
    pub fn panel_mut(&mut self) -> &mut AgentPanel {
        &mut self.panel
    }

    /// 键位分发（stage 3 实装：Enter 发送 / Esc 中断 / y n confirm / 滚动）。
    ///
    /// stage 1 不被调用——App 层对退出键内联处理（stage doc 风险 3：todo!()
    /// 不能被测试路径触达）。
    pub fn handle_key(&mut self, _key: KeyEvent, _ctx: &mut PanelContext) -> PanelAction {
        todo!("v0.21 stage 3 落地键位状态机")
    }
}
