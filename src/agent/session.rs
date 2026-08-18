//! v0.21 TUI AgentPanel 会话层（ADR-0031 D1）。
//!
//! 专用 `std::thread` + 自有 tokio Runtime + `std::sync::mpsc` 事件通道桥接
//! 同步 TUI 主循环（WorkerManager 同款模式）。本文件 stage 1 只落类型骨架
//! （SessionEvent / ConfirmRequest / ConfirmDecision / 空 struct），实装在
//! stage 2（会话线程 + run_streaming 接线）。

use serde_json::Value;
use tokio::sync::oneshot;

use super::runner::StopCause;

/// history 滑动窗口：system prompt + 最近 N 轮（决策 D4，常量不预加配置）。
pub const MAX_HISTORY_TURNS: usize = 12;

/// 写操作确认决策（面板 `y` → Approved / `n` → Denied）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmDecision {
    /// 用户确认执行——dispatch 以 `confirm: true` 真执行（ADR-0008/0029 契约）。
    Approved,
    /// 用户拒绝——返 blocked JSON，模型转向解释 + 给等价命令行。
    Denied,
}

/// 写操作确认请求（dispatch 遇 [`crate::agent::tools::dispatch::WRITE_TOOL_NAMES`]
/// tool 时经 `SessionEvent::ConfirmRequested` 发出，面板 y/n 后经 reply 回传决策）。
///
/// derive Debug 可行：`oneshot::Sender` 实现了 Debug。
#[derive(Debug)]
pub struct ConfirmRequest {
    pub tool_name: String,
    pub arguments: Value,
    /// 影响摘要（kill → 目标进程列表 / docker_rm → 容器名 / usb_release → 盘符）。
    pub summary: String,
    /// 一次性回复通道（TUI y/n 后 send）。
    pub reply: oneshot::Sender<ConfirmDecision>,
}

/// TUI 渲染的唯一数据源（tick 内 `try_recv` drain）。
///
/// 不 derive Clone：`ConfirmRequested` 含 `oneshot::Sender` 不可 Clone。
#[derive(Debug)]
pub enum SessionEvent {
    /// 用户 query 开始处理。
    QueryStarted(String),
    /// assistant 流式文本增量（D6：TUI 按 tick 批量 append）。
    TextDelta(String),
    /// 开始执行一个 tool call。
    ToolStart { name: String, arguments: Value },
    /// 一个 tool call 执行完成。
    ToolFinished {
        name: String,
        is_error: bool,
        result_chars: usize,
    },
    /// 写操作请求确认（面板进入 AwaitingConfirm 态）。
    ConfirmRequested(ConfirmRequest),
    /// 一轮 LLM 调用结束（多轮 ReAct 的轮边界）。
    TurnFinished,
    /// 整个 query 处理完成。
    SessionFinished { final_text: String, stop: StopCause },
    /// 会话错误（网络 / ctx 溢出 / provider 错误）。
    Error(String),
}

/// AgentSession：跨 query 的会话容器（多轮 history + provider + 事件通道）。
///
/// stage 2 实装：专用 `std::thread` + 自有 tokio Runtime + `spawn` /
/// `send_query` / `interrupt`。stage 1 不写方法（空 struct 无字段，方法签名
/// stage 2 随字段一并落）。
pub struct AgentSession {
    /// 字段 stage 2 实装时补全（provider / registry / history / events_tx / cancel）。
    _private: (),
}

/// TUI 侧持有的会话句柄（drain 事件 + 中断 + Drop 收尾）。
///
/// stage 2 实装：`interrupt()`（cancel flag）+ `drain_event()`（try_recv）+
/// Drop 时对 pending ConfirmRequest 发 Denied。
pub struct SessionHandle {
    /// 字段 stage 2 实装时补全（events_rx / cancel flag / join handle）。
    _private: (),
}
