//! v0.21 stage 3：AgentPanel 的 controller（ADR-0031 / ADR-0012 模式）。
//!
//! 面板状态机（Idle / Streaming / AwaitingConfirm）+ 输入缓冲 + 对话流
//! [`ChatEntry`] 缓冲。[`AgentPanel::apply_event`] 是纯状态迁移（App tick
//! drain `SessionEvent` 逐个喂入），键位状态机消费三态并经
//! `PanelAction::Agent(AgentAction)` 出带副作用（App 持 SessionHandle 执行）。
//!
//! confirm 生命周期（风险 2）：`pending_confirm` 持 reply 通道——y/n 在
//! [`AgentPanel::resolve_confirm`] 内直接 send 并回填 entry；退出面板由
//! App teardown 对 pending 发 Denied。

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::agent::session::{ConfirmDecision, ConfirmRequest, SessionEvent};
use crate::app_panel::{PanelAction, PanelContext};

/// 面板内部状态机（决策 D3 / 风险 2：confirm 挂起生命周期）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentPanelMode {
    /// 空闲：输入框可编辑，Enter 发送。
    #[default]
    Idle,
    /// 生成中：streaming 进行，Esc 中断（输入锁定）。
    Streaming,
    /// 等待写操作确认：`y` 执行 / `n` 拒绝。
    AwaitingConfirm,
}

/// 对话流渲染单元（entries 缓冲的元素）。
#[derive(Debug)]
pub enum ChatEntry {
    /// 用户 query 行。
    User(String),
    /// assistant 流式段落（TextDelta append 目标；tool 步骤后的下一轮自然分段）。
    AssistantStreaming(String),
    /// 最终回答（proc_finish answer——不经 TextDelta 透传，SessionFinished 落段）。
    AssistantFinal(String),
    /// tool 步骤行（ToolStart 建 is_error=None，ToolFinished 回填）。
    ToolCall {
        name: String,
        arguments: serde_json::Value,
        is_error: Option<bool>,
        result_chars: usize,
    },
    /// 写操作确认块（decision 在 y/n 后回填）。
    Confirm {
        tool_name: String,
        summary: String,
        decision: Option<ConfirmDecision>,
    },
    /// 会话错误（provider / ctx 溢出 / 构造失败）。
    Error(String),
    /// 系统提示行（中断 / 会话终止）。
    Notice(String),
}

/// controller 出带副作用（App 持 SessionHandle 执行；`PanelAction::Agent` 载体）。
#[derive(Debug)]
pub enum AgentAction {
    /// 发送 query（session.send_query）。
    SendQuery(String),
    /// 中断当前 run（session.interrupt；Esc 在 Streaming 态）。
    Interrupt,
    /// 退出面板回 ProcessList（teardown 由 switch_mode 出 Agent 分支做）。
    ExitPanel,
}

/// Agent 面板状态（纯 view model：不持 session，App 层接线）。
pub struct AgentPanel {
    /// 输入缓冲（字符 / Backspace / 中文输入——crossterm Char 事件天然 UTF-8）。
    pub input: String,
    /// 面板状态机。
    pub mode: AgentPanelMode,
    /// 对话流缓冲（渲染唯一数据源）。
    pub entries: Vec<ChatEntry>,
    /// 挂起的写操作确认（reply 通道由面板持有，y/n / 退出时 send）。
    pub pending_confirm: Option<ConfirmRequest>,
    /// 距底滚动行数（0 = 钉底跟随新内容；PgUp 增 PgDn 减——控制器无需知渲染宽度）。
    pub scroll_from_bottom: usize,
    /// 状态行 provider/model 短标签（进入面板时从 ProviderSpec 提取）。
    pub provider_detail: String,
    /// 当前 query 开始时刻（状态行用时）。
    pub query_started: Option<Instant>,
    /// 结束后冻结的用时（Streaming/AwaitingConfirm 期间 None = 实时计算）。
    pub finished_after: Option<Duration>,
    /// 当前 query 已执行 tool 步骤数（状态行）。
    pub tool_steps: u32,
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
            entries: Vec::new(),
            pending_confirm: None,
            scroll_from_bottom: 0,
            provider_detail: String::new(),
            query_started: None,
            finished_after: None,
            tool_steps: 0,
        }
    }

    /// 进入面板时复位（D5：会话历史随退出即弃，残留显示会误导）。
    pub fn reset_for_new_session(&mut self, provider_detail: String) {
        self.input.clear();
        self.mode = AgentPanelMode::Idle;
        self.entries.clear();
        self.pending_confirm = None;
        self.scroll_from_bottom = 0;
        self.provider_detail = provider_detail;
        self.query_started = None;
        self.finished_after = None;
        self.tool_steps = 0;
    }

    /// App tick drain 喂入一个 SessionEvent，返回是否有可见变化（→ pending_redraw）。
    pub fn apply_event(&mut self, ev: SessionEvent) -> bool {
        match ev {
            SessionEvent::QueryStarted(q) => {
                self.entries.push(ChatEntry::User(q));
                self.mode = AgentPanelMode::Streaming;
                self.query_started = Some(Instant::now());
                self.finished_after = None;
                self.tool_steps = 0;
                true
            }
            SessionEvent::TextDelta(t) => {
                match self.entries.last_mut() {
                    Some(ChatEntry::AssistantStreaming(s)) => s.push_str(&t),
                    _ => self.entries.push(ChatEntry::AssistantStreaming(t)),
                }
                true
            }
            SessionEvent::ToolStart { name, arguments } => {
                self.entries.push(ChatEntry::ToolCall {
                    name,
                    arguments,
                    is_error: None,
                    result_chars: 0,
                });
                self.tool_steps += 1;
                true
            }
            SessionEvent::ToolFinished {
                name,
                is_error,
                result_chars,
            } => {
                // 从尾往前找同名未回填条目（同名 tool 连续调用时更新最近的）。
                for e in self.entries.iter_mut().rev() {
                    if let ChatEntry::ToolCall {
                        name: n,
                        is_error: slot,
                        result_chars: rc,
                        ..
                    } = e
                    {
                        if *n == name && slot.is_none() {
                            *slot = Some(is_error);
                            *rc = result_chars;
                            break;
                        }
                    }
                }
                true
            }
            SessionEvent::ConfirmRequested(req) => {
                self.entries.push(ChatEntry::Confirm {
                    tool_name: req.tool_name.clone(),
                    summary: req.summary.clone(),
                    decision: None,
                });
                self.pending_confirm = Some(req);
                self.mode = AgentPanelMode::AwaitingConfirm;
                true
            }
            SessionEvent::TurnFinished => false,
            SessionEvent::SessionFinished { final_text, stop } => {
                use crate::agent::runner::StopCause;
                match stop {
                    // interrupted_outcome 的 final_text 是占位文案，不进对话流。
                    StopCause::Interrupted => self.entries.push(ChatEntry::Notice(format!(
                        "⏹ 已中断（已完成 {} 个 tool 步骤）",
                        self.tool_steps
                    ))),
                    _ => self.entries.push(ChatEntry::AssistantFinal(final_text)),
                }
                self.finish_run();
                true
            }
            SessionEvent::Error(e) => {
                self.entries.push(ChatEntry::Error(e));
                self.finish_run();
                true
            }
        }
    }

    /// run 结束收尾（SessionFinished / Error 共用）：Idle + 冻结用时 + 清挂起。
    fn finish_run(&mut self) {
        self.mode = AgentPanelMode::Idle;
        self.finished_after = self.query_started.map(|t| t.elapsed());
        // cancel 收尾链里 reply 由 runner 侧 Sender-drop 兜底，这里只清引用。
        self.pending_confirm = None;
    }

    /// y/n 决策回传：send reply + 回填 entry + 回到 Streaming（run 继续）。
    /// 退出面板对 pending 发 Denied 也走这里（风险 2 收尾语义）。
    pub fn resolve_confirm(&mut self, decision: ConfirmDecision) {
        if let Some(req) = self.pending_confirm.take() {
            let _ = req.reply.send(decision);
            if let Some(ChatEntry::Confirm { decision: slot, .. }) = self.entries.last_mut() {
                *slot = Some(decision);
            }
            if self.mode == AgentPanelMode::AwaitingConfirm {
                self.mode = AgentPanelMode::Streaming;
            }
        }
    }

    /// 会话线程死亡感知（stage 2 注记 4）：Streaming 态下 is_exited 时由 App 调用。
    pub fn mark_session_dead(&mut self) {
        self.entries
            .push(ChatEntry::Notice("⚠ 会话已终止（线程退出）".to_string()));
        self.finish_run();
    }
}

/// AgentPanel 的 controller 包装（与既有 5 个 XxxPanelController 同款结构）。
pub struct AgentPanelController {
    pub panel: AgentPanel,
    /// 键位产生了可见变化（handle_key 出口统一转 pending_redraw 后清零）。
    visible_dirty: bool,
}

/// 状态行 provider 短标签：llama-cpp 的 detail 含双完整路径太长，取
/// `model: ` 段的文件 stem；其余 provider 直接用 detail（mock / 模型名都短）。
#[must_use]
pub fn short_provider_label(name: &str, detail: &str) -> String {
    if let Some(model_part) = detail.split("model: ").nth(1) {
        let stem = model_part.rsplit(['/', '\\']).next().unwrap_or(model_part);
        let stem = stem.strip_suffix(".gguf").unwrap_or(stem);
        return format!("{name}: {stem}");
    }
    if detail.is_empty() {
        return name.to_string();
    }
    format!("{name}: {detail}")
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
            visible_dirty: false,
        }
    }

    /// 用既有 panel 状态构造（测试驱动 AwaitingConfirm 等预设态）。
    #[must_use]
    pub fn with_panel(panel: AgentPanel) -> Self {
        Self {
            panel,
            visible_dirty: false,
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

    /// 键位状态机（stage 3 实装；stage 1 的退出键内联处理由 App 分支删除）。
    pub fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> PanelAction {
        let action = self.dispatch_key(key);
        if self.visible_dirty {
            self.visible_dirty = false;
            *ctx.pending_redraw = true;
        }
        action
    }

    fn dispatch_key(&mut self, key: KeyEvent) -> PanelAction {
        use AgentPanelMode::{AwaitingConfirm, Idle, Streaming};

        // 全局键：Ctrl+D 退出（任意态）。
        if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return PanelAction::Agent(AgentAction::ExitPanel);
        }
        // 滚动键（任意态）。
        match key.code {
            KeyCode::PageUp => {
                self.panel.scroll_from_bottom = self.panel.scroll_from_bottom.saturating_add(10);
                self.visible_dirty = true;
                return PanelAction::Noop;
            }
            KeyCode::PageDown => {
                self.panel.scroll_from_bottom = self.panel.scroll_from_bottom.saturating_sub(10);
                self.visible_dirty = true;
                return PanelAction::Noop;
            }
            _ => {}
        }

        match self.panel.mode {
            AwaitingConfirm => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.panel.resolve_confirm(ConfirmDecision::Approved);
                    self.visible_dirty = true;
                    PanelAction::Noop
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.panel.resolve_confirm(ConfirmDecision::Denied);
                    self.visible_dirty = true;
                    PanelAction::Noop
                }
                _ => PanelAction::Noop,
            },
            Streaming => match key.code {
                KeyCode::Esc => PanelAction::Agent(AgentAction::Interrupt),
                // 输入锁定（生成中）：字符 / Enter / Backspace 忽略。
                _ => PanelAction::Noop,
            },
            Idle => match key.code {
                KeyCode::Enter => {
                    let q = self.panel.input.trim().to_string();
                    if q.is_empty() {
                        return PanelAction::Noop;
                    }
                    self.panel.input.clear();
                    self.visible_dirty = true;
                    PanelAction::Agent(AgentAction::SendQuery(q))
                }
                KeyCode::Esc => PanelAction::Agent(AgentAction::ExitPanel),
                KeyCode::Backspace => {
                    self.panel.input.pop();
                    self.visible_dirty = true;
                    PanelAction::Noop
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.panel.input.push(c);
                    self.visible_dirty = true;
                    PanelAction::Noop
                }
                _ => PanelAction::Noop,
            },
        }
    }
}
