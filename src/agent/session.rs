//! v0.21 TUI AgentPanel 会话层（ADR-0031 D1）。
//!
//! 专用 `std::thread` + 自有 tokio Runtime + `std::sync::mpsc` 事件通道桥接
//! 同步 TUI 主循环（WorkerManager 同款模式）：async 全部封闭在 session 线程
//! 内，TUI 只经 [`SessionHandle`]（drain_event / send_query / interrupt）
//! 交互，零 runtime 纠缠。
//!
//! 多轮 history（D4）：session 线程持 `Vec<Message>`（user+assistant 交替），
//! 滑动窗口 `MAX_HISTORY_TURNS` 轮，超限截断最旧轮。
//!
//! confirm 生命周期（风险 2）：`SessionHandle::drop` 置 cancel——run_streaming
//! 的 cancel 检查点（turn 开头 / 每 delta / confirm await select 轮询）触发
//! Interrupted 收尾，线程主循环 recv Err 退出，任何时序 drop 都不挂死。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;

use serde_json::Value;
use tokio::sync::oneshot;

use super::provider::LlmProvider;
use super::runner::{AgentOptions, AgentRunner, StopCause, StreamEvent};
use super::tool_registry::ToolRegistry;
use super::types::{Message, Role};

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

/// TUI → session 线程指令（std mpsc；`Query` 之外还有显式退出通道）。
pub enum SessionCommand {
    /// 处理一个用户 query（追加进 conversation history）。
    Query(String),
    /// 线程退出（`SessionHandle` drop 时 Sender 断开也会触发同款收尾）。
    Shutdown,
}

/// history 滑动窗口截断：1 轮 = user + assistant 两条，超 `MAX_HISTORY_TURNS`
/// 轮 drain 最旧（system prompt 不在 history 内，由 runner 每次组装）。
pub fn truncate_history(history: &mut Vec<Message>) {
    let max = MAX_HISTORY_TURNS * 2;
    if history.len() > max {
        history.drain(..history.len() - max);
    }
}

/// AgentSession：跨 query 的会话容器（多轮 history + provider + 事件通道）。
///
/// `spawn` 后业务全在 session 线程内，调用方只持 [`SessionHandle`]。
pub struct AgentSession;

impl AgentSession {
    /// 起会话线程（专用 thread + 线程内自建 current_thread tokio Runtime——
    /// llama-server 句柄跨调用复用需要常驻 async 上下文，ADR-0031 备选 B
    /// 否决理由）。
    pub fn spawn(
        provider: Arc<dyn LlmProvider>,
        registry: ToolRegistry,
        options: AgentOptions,
    ) -> SessionHandle {
        let (events_tx, events_rx) = std_mpsc::channel::<SessionEvent>();
        let (queries_tx, queries_rx) = std_mpsc::channel::<SessionCommand>();
        let cancel = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));

        let thread_cancel = Arc::clone(&cancel);
        let thread_exited = Arc::clone(&exited);
        std::thread::Builder::new()
            .name("agent-session".to_string())
            .spawn(move || {
                session_loop(
                    provider,
                    registry,
                    options,
                    queries_rx,
                    events_tx,
                    thread_cancel,
                );
                thread_exited.store(true, Ordering::SeqCst);
            })
            .expect("spawn agent-session thread");

        SessionHandle {
            events: events_rx,
            queries: queries_tx,
            cancel,
            exited,
        }
    }
}

fn session_loop(
    provider: Arc<dyn LlmProvider>,
    registry: ToolRegistry,
    options: AgentOptions,
    queries_rx: std_mpsc::Receiver<SessionCommand>,
    events_tx: std_mpsc::Sender<SessionEvent>,
    cancel: Arc<AtomicBool>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = events_tx.send(SessionEvent::Error(format!("tokio runtime 创建失败: {e}")));
            return;
        }
    };
    let runner = AgentRunner::new(provider, registry, options);
    let mut history: Vec<Message> = Vec::new();

    loop {
        let query = match queries_rx.recv() {
            Ok(SessionCommand::Shutdown) | Err(_) => break,
            Ok(SessionCommand::Query(q)) => q,
        };
        if query.trim().is_empty() {
            let _ = events_tx.send(SessionEvent::Error("query 不能为空".to_string()));
            continue;
        }
        // 新 query 清掉上一轮的 interrupt 残留（cancel 只作用于当前 run）。
        cancel.store(false, Ordering::Relaxed);
        let _ = events_tx.send(SessionEvent::QueryStarted(query.clone()));

        let sink_tx = events_tx.clone();
        let sink = move |ev: StreamEvent| {
            let session_event = match ev {
                StreamEvent::TextDelta(t) => SessionEvent::TextDelta(t),
                StreamEvent::ToolStart { name, arguments } => {
                    SessionEvent::ToolStart { name, arguments }
                }
                StreamEvent::ToolFinished {
                    name,
                    is_error,
                    result_chars,
                } => SessionEvent::ToolFinished {
                    name,
                    is_error,
                    result_chars,
                },
                StreamEvent::TurnFinished => SessionEvent::TurnFinished,
            };
            let _ = sink_tx.send(session_event);
        };
        let confirm_tx = events_tx.clone();
        let confirm = move |req: ConfirmRequest| {
            let _ = confirm_tx.send(SessionEvent::ConfirmRequested(req));
        };

        match rt.block_on(runner.run_streaming(&query, &history, &sink, Some(&confirm), &cancel)) {
            Ok(outcome) => {
                // Interrupted 丢弃当前 run（history 保留已完成轮，风险 2 语义）。
                if outcome.stop != StopCause::Interrupted {
                    history.push(Message::new(Role::User, query));
                    history.push(Message::new(Role::Assistant, outcome.final_text.clone()));
                    truncate_history(&mut history);
                }
                let _ = events_tx.send(SessionEvent::SessionFinished {
                    final_text: outcome.final_text,
                    stop: outcome.stop,
                });
            }
            Err(e) => {
                let _ = events_tx.send(SessionEvent::Error(e.to_string()));
            }
        }
    }
    // events_tx 在此 drop——UI 侧 drain_event 会感知 Disconnected（会话终结）。
}

/// TUI 侧持有的会话句柄（drain 事件 + 发 query + 中断 + Drop 收尾）。
pub struct SessionHandle {
    events: std_mpsc::Receiver<SessionEvent>,
    queries: std_mpsc::Sender<SessionCommand>,
    cancel: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
}

impl SessionHandle {
    /// 发送一个 query（`false` = 会话线程已终结）。
    pub fn send_query(&self, query: impl Into<String>) -> bool {
        self.queries
            .send(SessionCommand::Query(query.into()))
            .is_ok()
    }

    /// 中断当前 run（cancel flag；session 线程收到下一个 query 时自动重置）。
    pub fn interrupt(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// 非阻塞取一个事件（TUI tick 内循环 drain；`None` = 暂无事件或会话终结）。
    pub fn drain_event(&self) -> Option<SessionEvent> {
        self.events.try_recv().ok()
    }

    /// 会话线程是否已退出（风险 2 的 drop-not-hang 测试断言用）。
    pub fn is_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// 显式退出指令（与 drop 等效，但保留 handle 供继续 drain 残留事件）。
    pub fn shutdown(&self) {
        let _ = self.queries.send(SessionCommand::Shutdown);
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        // confirm 挂起 / 流式进行中 drop：置 cancel 让 run_streaming 的 3 个
        // 检查点 + confirm await select 轮询触发 Interrupted 收尾；随后
        // queries Sender drop → 线程主循环 recv Err 退出。任何时序不挂死。
        self.cancel.store(true, Ordering::Relaxed);
    }
}
