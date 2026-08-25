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
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;

use serde_json::Value;
use tokio::sync::oneshot;

use super::provider::LlmProvider;
use super::runner::{AgentOptions, AgentRunner, StopCause, StreamEvent};
use super::session_log::SessionRecorder;
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

/// agent 侧录制状态（v0.23 stage 2，ADR-0033 D5）——ADR-0029 record_handle
/// pattern 的 AgentSession 层实例，全部子进程管理薄包 MCP 既有 helper：
/// - `child` 槽跨 tool 调用保活（session 线程与 `SessionHandle` 共享 clone）
/// - `file_path` 槽记忆 start 时的落盘路径——agent catalog 的
///   `proc_record_stop` 是 no_params，stop / teardown 靠记忆值定位文件
#[derive(Clone, Default)]
pub struct RecordState {
    child: Arc<Mutex<Option<std::process::Child>>>,
    file_path: Arc<Mutex<Option<String>>>,
}

impl RecordState {
    /// `proc_record_start` 真实执行：spawn headless 录屏子进程（TestBackend
    /// 合成系统仪表盘，与 MCP `proc mcp` 路径同款语义）。ok 时记忆 file_path。
    pub fn start(&self, confirm: bool, output: &str, duration_secs: Option<u64>) -> Value {
        let v = crate::mcp::handler::record::make_record_start_json(
            confirm,
            output,
            duration_secs,
            &self.child,
        );
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            if let Ok(mut slot) = self.file_path.lock() {
                *slot = Some(output.to_string());
            }
        }
        v
    }

    /// `proc_record_stop` 真实执行：kill child + 等 flush + 读 metadata。
    /// 无 start 记忆时返业务错误（ok:false），is_error 不置位（与 kill 同款契约）。
    pub fn stop(&self) -> Value {
        let remembered = self
            .file_path
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_default();
        if remembered.is_empty() {
            return serde_json::json!({
                "ok": false,
                "error": "无录屏进行中；先调 proc_record_start",
            });
        }
        let v = crate::mcp::handler::record::make_record_stop_json(&remembered, &self.child);
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            if let Ok(mut slot) = self.file_path.lock() {
                *slot = None;
            }
        }
        v
    }

    /// teardown 自动 stop（D5 停止路径②③，防孤儿录制进程）：录制进行中才动，
    /// 幂等（child 已空返 None）。返回 Notice 文案「录屏已自动保存至 <path>」。
    pub fn teardown_stop(&self) -> Option<String> {
        let has_child = self.child.lock().ok().map(|g| g.is_some())?;
        if !has_child {
            return None;
        }
        let v = self.stop();
        let path = v
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("未知路径");
        Some(format!("录屏已自动保存至 {path}"))
    }
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
    ///
    /// v0.22 stage 3：`recorder` 是 session observability 旁路（ADR-0032 D5），
    /// 关日志时传 [`SessionRecorder::disabled`]。
    pub fn spawn(
        provider: Arc<dyn LlmProvider>,
        registry: ToolRegistry,
        options: AgentOptions,
        recorder: SessionRecorder,
        rag: Option<(Arc<super::rag::RagIndex>, super::rag::RagParams)>,
    ) -> SessionHandle {
        let (events_tx, events_rx) = std_mpsc::channel::<SessionEvent>();
        let (queries_tx, queries_rx) = std_mpsc::channel::<SessionCommand>();
        let cancel = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        // v0.23 stage 2（ADR-0033 D5）：录制状态在此内建（签名不变）——线程
        // clone 喂 runner + 循环退出兜底 teardown；Handle clone 供 App teardown
        // 自动 stop 落盘（防孤儿双保险，见 teardown_stop）。
        let record = RecordState::default();

        let thread_cancel = Arc::clone(&cancel);
        let thread_exited = Arc::clone(&exited);
        let thread_record = record.clone();
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
                    recorder,
                    thread_record,
                    rag,
                );
                thread_exited.store(true, Ordering::SeqCst);
            })
            .expect("spawn agent-session thread");

        SessionHandle {
            events: events_rx,
            queries: queries_tx,
            cancel,
            exited,
            record,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn session_loop(
    provider: Arc<dyn LlmProvider>,
    registry: ToolRegistry,
    options: AgentOptions,
    queries_rx: std_mpsc::Receiver<SessionCommand>,
    events_tx: std_mpsc::Sender<SessionEvent>,
    cancel: Arc<AtomicBool>,
    recorder: SessionRecorder,
    record: RecordState,
    rag: Option<(Arc<super::rag::RagIndex>, super::rag::RagParams)>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let ev = SessionEvent::Error(format!("tokio runtime 创建失败: {e}"));
            recorder.log(&ev);
            let _ = events_tx.send(ev);
            return;
        }
    };
    let mut runner =
        AgentRunner::new(provider, registry, options).with_record_state(record.clone());
    if let Some((index, params)) = rag {
        runner = runner.with_rag(index, params);
    }
    let mut history: Vec<Message> = Vec::new();

    loop {
        let query = match queries_rx.recv() {
            Ok(SessionCommand::Shutdown) | Err(_) => break,
            Ok(SessionCommand::Query(q)) => q,
        };
        if query.trim().is_empty() {
            let ev = SessionEvent::Error("query 不能为空".to_string());
            recorder.log(&ev);
            let _ = events_tx.send(ev);
            continue;
        }
        // 新 query 清掉上一轮的 interrupt 残留（cancel 只作用于当前 run）。
        cancel.store(false, Ordering::Relaxed);
        let started = SessionEvent::QueryStarted(query.clone());
        recorder.log(&started);
        let _ = events_tx.send(started);

        let sink_tx = events_tx.clone();
        let sink_recorder = recorder.clone();
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
            sink_recorder.log(&session_event);
            let _ = sink_tx.send(session_event);
        };
        let confirm_tx = events_tx.clone();
        let confirm_recorder = recorder.clone();
        let confirm = move |req: ConfirmRequest| {
            confirm_with_recorder(&confirm_tx, &confirm_recorder, req);
        };

        match rt.block_on(runner.run_streaming(&query, &history, &sink, Some(&confirm), &cancel)) {
            Ok(outcome) => {
                // Interrupted 丢弃当前 run（history 保留已完成轮，风险 2 语义）。
                if outcome.stop != StopCause::Interrupted {
                    history.push(Message::new(Role::User, query));
                    history.push(Message::new(Role::Assistant, outcome.final_text.clone()));
                    truncate_history(&mut history);
                }
                let finished = SessionEvent::SessionFinished {
                    final_text: outcome.final_text,
                    stop: outcome.stop,
                };
                recorder.log(&finished);
                let _ = events_tx.send(finished);
            }
            Err(e) => {
                let ev = SessionEvent::Error(e.to_string());
                recorder.log(&ev);
                let _ = events_tx.send(ev);
            }
        }
    }
    // v0.23 stage 2（ADR-0033 D5 停止路径②③兜底）：循环退出（Shutdown /
    // Sender 全 drop）时若录制仍进行，静默自动 stop 落盘。App 正常 teardown
    // 已先行调 SessionHandle::stop_orphan_recording（带 Notice 提示），此处
    // 覆盖 Handle 被直接 drop 未走 App teardown 的场景——幂等双保险。
    let _ = record.teardown_stop();
    // events_tx 在此 drop——UI 侧 drain_event 会感知 Disconnected（会话终结）。
}

/// confirm hook 的 session 层包装（v0.22 stage 3 observability，ADR-0032 D5）：
/// 换出 `req.reply` 包一层转发线程——面板决策到达时先记录
/// `LogEvent::ConfirmDecision` 再转发原通道给 runner（先记录后转发保证
/// 日志序：confirm_decision 条目先于 runner 恢复后的任何后续事件）。
///
/// 面板 drop 未决策（通道断开）→ 线程干净退出不转发——runner 侧
/// RecvError → Denied 语义不变。
fn confirm_with_recorder(
    confirm_tx: &std_mpsc::Sender<SessionEvent>,
    recorder: &SessionRecorder,
    mut req: ConfirmRequest,
) {
    let (wrap_tx, wrap_rx) = oneshot::channel();
    let orig_reply = std::mem::replace(&mut req.reply, wrap_tx);
    let decision_recorder = recorder.clone();
    std::thread::spawn(move || {
        if let Ok(decision) = wrap_rx.blocking_recv() {
            decision_recorder.log_confirm_decision(matches!(decision, ConfirmDecision::Approved));
            let _ = orig_reply.send(decision);
        }
        // Err = 面板 drop 未决策——通道断开，不转发（runner 侧 RecvError → Denied）。
    });
    let event = SessionEvent::ConfirmRequested(req);
    recorder.log(&event);
    let _ = confirm_tx.send(event);
}

/// TUI 侧持有的会话句柄（drain 事件 + 发 query + 中断 + Drop 收尾）。
pub struct SessionHandle {
    events: std_mpsc::Receiver<SessionEvent>,
    queries: std_mpsc::Sender<SessionCommand>,
    cancel: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    record: RecordState,
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

    /// teardown 钩子（v0.23 stage 2，ADR-0033 D5 停止路径②③）：录制进行中
    /// 则自动 stop 落盘（防孤儿），返回 Notice 文案。App 的
    /// `teardown_agent_session` 在 interrupt/shutdown **之前**调用（用户还能
    /// 看到提示）；无录制返 None。
    pub fn stop_orphan_recording(&self) -> Option<String> {
        self.record.teardown_stop()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造带注入 child + file_path 的 RecordState（私有字段直填，孤儿清理
    /// 路径不依赖真实 spawn 链路）。fake child 用 `ping -n 30`（Windows 自带、
    /// 长 sleep、kill 即退）。
    fn state_with_fake_child(file_path: &str) -> RecordState {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .spawn()
            .expect("spawn ping 失败");
        // 立即确认 spawn 成功且未退出（30s sleep 内不会自然退出）。
        assert!(child.try_wait().unwrap().is_none());
        RecordState {
            child: Arc::new(Mutex::new(Some(child))),
            file_path: Arc::new(Mutex::new(Some(file_path.to_string()))),
        }
    }

    #[test]
    fn stop_kills_running_child_and_reports_exit() {
        let state = state_with_fake_child("Z:/no-such-dir/orphan-test.prec");
        let v = state.stop();
        assert_eq!(v["ok"], serde_json::json!(true), "{v}");
        assert_eq!(v["action"], serde_json::json!("stop"));
        assert!(v.get("exit_code").is_some(), "kill 后 wait 到退出码: {v}");
        // .prec 不存在 → metadata_warning 容错分支（ok 仍 true）。
        assert!(v.get("metadata_warning").is_some(), "{v}");
        // ok 后记忆槽清空：二次 stop 是业务错误。
        let v2 = state.stop();
        assert_eq!(v2["ok"], serde_json::json!(false), "{v2}");
        assert!(
            v2["error"].as_str().unwrap().contains("无录屏进行中"),
            "{v2}"
        );
    }

    #[test]
    fn teardown_stop_without_recording_returns_none() {
        let state = RecordState::default();
        assert!(state.teardown_stop().is_none());
    }

    #[test]
    fn teardown_stop_kills_child_and_is_idempotent() {
        let state = state_with_fake_child("Z:/no-such-dir/teardown-test.prec");
        let notice = state.teardown_stop().expect("录制中应返 Notice");
        assert!(
            notice.contains("录屏已自动保存至") && notice.contains("teardown-test.prec"),
            "{notice}"
        );
        // 幂等：child 槽已被 take，二次调用返 None。
        assert!(state.teardown_stop().is_none());
    }
}
