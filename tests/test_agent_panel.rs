//! v0.21 stage 3 集成测试 — TUI AgentPanel（controller 状态机 + 渲染 + App 集成）。
//!
//! 覆盖：
//! 1. A 组：controller 键位状态机（Idle 输入 / Enter 发送 / Esc 退出·中断 /
//!    AwaitingConfirm y·n / PgUp PgDn 滚动 / Ctrl+D）
//! 2. B 组：`AgentPanel::apply_event` 状态迁移（TextDelta append / ToolCall 回填 /
//!    ConfirmRequested / SessionFinished / Error）
//! 3. C 组：AgentSession 端到端驱动面板状态（ScriptedStreamProvider 零 LLM，
//!    含 confirm Approved/Denied 双路径）
//! 4. D 组：TestBackend 渲染（三态不 panic + 关键文本）
//! 5. E 组：App 集成（palette 进面板建 session / Ctrl+D teardown / SendQuery
//!    无会话降级）
//!
//! 运行：`cargo test --release --test test_agent_panel`

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use serde_json::{Value, json};

use proc::agent::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
};
use proc::agent::runner::{AgentOptions, StopCause};
use proc::agent::session::{
    AgentSession, ConfirmDecision, ConfirmRequest, SessionEvent, SessionHandle,
};
use proc::agent::tools::catalog;
use proc::agent::types::{Message, ToolCall};
use proc::app::App;
use proc::app_panel::{AppMode, KillRequest, OpRecord, PanelAction, PanelContext};
use proc::collect::{ProcessInfo, SystemSnapshot};
use proc::dns_log::DnsQuery;
use proc::view_models::{AgentAction, AgentPanel, AgentPanelController, AgentPanelMode, ChatEntry};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

// ===========================================================================
// ScriptedStreamProvider（stage 2 同款精简版——逐 turn 弹脚本化 delta）
// ===========================================================================

struct ScriptedStreamProvider {
    turns: Mutex<VecDeque<Vec<Delta>>>,
}

impl ScriptedStreamProvider {
    fn new(turns: Vec<Vec<Delta>>) -> Self {
        Self {
            turns: Mutex::new(turns.into_iter().collect()),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedStreamProvider {
    fn name(&self) -> &'static str {
        "scripted-stream"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        Err(LlmError::Config("streaming tests only".to_string()))
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        let turn = self.turns.lock().unwrap().pop_front();
        match turn {
            Some(deltas) => futures_util::stream::iter(deltas.into_iter().map(Ok)).boxed(),
            None => futures_util::stream::empty::<Result<Delta, LlmError>>().boxed(),
        }
    }
}

fn text_delta(s: &str) -> Delta {
    Delta::Text(s.to_string())
}

fn tool_delta(name: &str, args: Value) -> Delta {
    Delta::ToolCall(ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments: args,
    })
}

fn end_turn() -> Delta {
    Delta::EndTurn {
        stop_reason: StopReason::EndTurn,
    }
}

fn finish_turn(answer: &str) -> Vec<Delta> {
    vec![
        tool_delta("proc_finish", json!({ "answer": answer })),
        end_turn(),
    ]
}

fn spawn_session(provider: ScriptedStreamProvider) -> SessionHandle {
    AgentSession::spawn(
        Arc::new(provider),
        catalog::default_registry(),
        AgentOptions {
            max_steps: 6,
            ..Default::default()
        },
        proc::agent::SessionRecorder::disabled(),
    )
}

/// 带超时 drain 并逐事件喂 panel：每个事件都 apply（不丢弃），返回谓词是否
/// 命中（谓词在 apply 前检查事件形态）。
fn drain_into_until(
    handle: &SessionHandle,
    panel: &mut AgentPanel,
    mut pred: impl FnMut(&SessionEvent) -> bool,
    what: &str,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(ev) = handle.drain_event() {
            let hit = pred(&ev);
            panel.apply_event(ev);
            if hit {
                return true;
            }
        } else if Instant::now() > deadline {
            panic!("drain_into_until 超时未等到: {what}");
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

// ===========================================================================
// PanelContext 构造（controller.handle_key 需要）
// ===========================================================================

struct CtxLocals {
    status_message: Option<String>,
    detail_process: Option<ProcessInfo>,
    pending_kill: Option<KillRequest>,
    data_dirty: bool,
    pending_redraw: bool,
    alert_manager: proc::alert::AlertManager,
    op_history: VecDeque<OpRecord>,
    dns_log_recent: VecDeque<DnsQuery>,
    pending_container_exec: Option<String>,
    security_scores: HashMap<u32, proc::security::SecurityScore>,
    cached_sorted: Vec<(usize, proc::classify::ProcessClass)>,
}

impl CtxLocals {
    fn new() -> Self {
        Self {
            status_message: None,
            detail_process: None,
            pending_kill: None,
            data_dirty: false,
            pending_redraw: false,
            alert_manager: proc::alert::AlertManager::default(),
            op_history: VecDeque::new(),
            dns_log_recent: VecDeque::new(),
            pending_container_exec: None,
            security_scores: HashMap::new(),
            cached_sorted: Vec::new(),
        }
    }
}

fn make_ctx<'a>(snapshot: &'a SystemSnapshot, l: &'a mut CtxLocals) -> PanelContext<'a> {
    PanelContext {
        snapshot,
        cached_processes: &[],
        cached_sorted: &l.cached_sorted,
        security_scores: &l.security_scores,
        status_message: &mut l.status_message,
        detail_process: &mut l.detail_process,
        pending_kill: &mut l.pending_kill,
        data_dirty: &mut l.data_dirty,
        pending_redraw: &mut l.pending_redraw,
        alert_manager: &mut l.alert_manager,
        op_history: &mut l.op_history,
        dns_log_recent: &mut l.dns_log_recent,
        pending_container_exec: &mut l.pending_container_exec,
        flows: &[],
    }
}

/// controller 测试脚手架：真实 SystemSnapshot（PanelContext 需要）+ locals。
fn with_ctx(f: impl FnOnce(&mut AgentPanelController, &mut CtxLocals, &mut SystemSnapshot)) {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new");
    let mut locals = CtxLocals::new();
    let mut controller = AgentPanelController::new();
    f(&mut controller, &mut locals, &mut snapshot);
}

/// 单键按压：构造 ctx → handle_key → 返回 PanelAction。
fn press(
    c: &mut AgentPanelController,
    l: &mut CtxLocals,
    s: &mut SystemSnapshot,
    key: KeyEvent,
) -> PanelAction {
    let mut ctx = make_ctx(s, l);
    c.handle_key(key, &mut ctx)
}

// ===========================================================================
// A 组：controller 键位状态机
// ===========================================================================

#[test]
fn test_a_idle_input_typing_and_backspace() {
    with_ctx(|c, l, s| {
        press(c, l, s, key(KeyCode::Char('你')));
        press(c, l, s, key(KeyCode::Char('好')));
        press(c, l, s, key(KeyCode::Char(' ')));
        press(c, l, s, key(KeyCode::Char('p')));
        assert_eq!(c.panel().input, "你好 p");
        press(c, l, s, key(KeyCode::Backspace));
        assert_eq!(c.panel().input, "你好 ");
        assert_eq!(c.panel().mode, AgentPanelMode::Idle);
    });
}

#[test]
fn test_a_idle_enter_empty_noop_nonempty_sendquery() {
    with_ctx(|c, l, s| {
        // 空输入 Enter：Noop。
        let a = press(c, l, s, key(KeyCode::Enter));
        assert!(matches!(a, PanelAction::Noop));
        // 非空输入 Enter：SendQuery + input 清空。
        for ch in "列出 top 5".chars() {
            press(c, l, s, key(KeyCode::Char(ch)));
        }
        let a = press(c, l, s, key(KeyCode::Enter));
        match a {
            PanelAction::Agent(AgentAction::SendQuery(q)) => assert_eq!(q, "列出 top 5"),
            other => panic!("expected SendQuery, got {other:?}"),
        }
        assert!(c.panel().input.is_empty(), "发送后输入框应清空");
    });
}

#[test]
fn test_a_idle_esc_exits_streaming_esc_interrupts() {
    with_ctx(|c, l, s| {
        let a = press(c, l, s, key(KeyCode::Esc));
        assert!(matches!(a, PanelAction::Agent(AgentAction::ExitPanel)));

        c.panel_mut().mode = AgentPanelMode::Streaming;
        // Streaming 态 Esc = 中断（优先于退出）；字符被忽略（输入锁定）。
        let a = press(c, l, s, key(KeyCode::Esc));
        assert!(matches!(a, PanelAction::Agent(AgentAction::Interrupt)));
        let a = press(c, l, s, key(KeyCode::Char('x')));
        assert!(matches!(a, PanelAction::Noop));
        assert!(c.panel().input.is_empty(), "生成中输入应锁定");
    });
}

#[test]
fn test_a_awaiting_confirm_y_n_esc() {
    // y → reply 收 Approved + entry 回填 + mode 回 Streaming。
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut panel = AgentPanel::new();
    panel.mode = AgentPanelMode::AwaitingConfirm;
    panel.entries.push(ChatEntry::Confirm {
        tool_name: "proc_kill".to_string(),
        summary: "终止进程 4000000".to_string(),
        decision: None,
    });
    panel.pending_confirm = Some(ConfirmRequest {
        tool_name: "proc_kill".to_string(),
        arguments: json!({"pids": [4000000]}),
        summary: "终止进程 4000000".to_string(),
        reply: tx,
    });
    let mut controller = AgentPanelController::with_panel(panel);

    let mut snapshot = SystemSnapshot::new().expect("snapshot");
    let mut locals = CtxLocals::new();
    let a = press(
        &mut controller,
        &mut locals,
        &mut snapshot,
        key(KeyCode::Char('y')),
    );
    assert!(matches!(a, PanelAction::Noop));
    assert_eq!(rx.blocking_recv(), Ok(ConfirmDecision::Approved));
    assert!(controller.panel().pending_confirm.is_none());
    assert_eq!(controller.panel().mode, AgentPanelMode::Streaming);
    match controller.panel().entries.last() {
        Some(ChatEntry::Confirm { decision, .. }) => {
            assert_eq!(*decision, Some(ConfirmDecision::Approved));
        }
        other => panic!("expected Confirm entry, got {other:?}"),
    }

    // n / Esc → Denied。
    let (tx2, rx2) = tokio::sync::oneshot::channel();
    controller.panel_mut().mode = AgentPanelMode::AwaitingConfirm;
    controller.panel_mut().pending_confirm = Some(ConfirmRequest {
        tool_name: "proc_kill".to_string(),
        arguments: json!({}),
        summary: String::new(),
        reply: tx2,
    });
    press(
        &mut controller,
        &mut locals,
        &mut snapshot,
        key(KeyCode::Char('n')),
    );
    assert_eq!(rx2.blocking_recv(), Ok(ConfirmDecision::Denied));

    let (tx3, rx3) = tokio::sync::oneshot::channel();
    controller.panel_mut().mode = AgentPanelMode::AwaitingConfirm;
    controller.panel_mut().pending_confirm = Some(ConfirmRequest {
        tool_name: "proc_kill".to_string(),
        arguments: json!({}),
        summary: String::new(),
        reply: tx3,
    });
    press(
        &mut controller,
        &mut locals,
        &mut snapshot,
        key(KeyCode::Esc),
    );
    assert_eq!(
        rx3.blocking_recv(),
        Ok(ConfirmDecision::Denied),
        "Esc 应视同拒绝"
    );
}

#[test]
fn test_a_ctrl_d_exits_in_all_modes() {
    with_ctx(|c, l, s| {
        for mode in [
            AgentPanelMode::Idle,
            AgentPanelMode::Streaming,
            AgentPanelMode::AwaitingConfirm,
        ] {
            c.panel_mut().mode = mode;
            let a = press(c, l, s, ctrl(KeyCode::Char('d')));
            assert!(
                matches!(a, PanelAction::Agent(AgentAction::ExitPanel)),
                "mode {mode:?} 下 Ctrl+D 应退出"
            );
        }
    });
}

#[test]
fn test_a_pgup_pgdn_scroll_offset() {
    with_ctx(|c, l, s| {
        assert_eq!(c.panel().scroll_from_bottom, 0);
        press(c, l, s, key(KeyCode::PageUp));
        assert_eq!(c.panel().scroll_from_bottom, 10);
        press(c, l, s, key(KeyCode::PageUp));
        assert_eq!(c.panel().scroll_from_bottom, 20);
        press(c, l, s, key(KeyCode::PageDown));
        press(c, l, s, key(KeyCode::PageDown));
        press(c, l, s, key(KeyCode::PageDown));
        assert_eq!(c.panel().scroll_from_bottom, 0, "减到 0 钉底不下溢");
    });
}

#[test]
fn test_a_short_provider_label() {
    use proc::view_models::short_provider_label;
    let l = short_provider_label(
        "llama-cpp",
        "llama-server: D:\\x\\llama-server.exe | model: D:\\m\\gemma-4-E2B-it-Q4_K_M.gguf",
    );
    assert_eq!(l, "llama-cpp: gemma-4-E2B-it-Q4_K_M");
    assert_eq!(
        short_provider_label("mock", "mock fixtures 回放"),
        "mock: mock fixtures 回放"
    );
    assert_eq!(short_provider_label("llama-cpp", ""), "llama-cpp");
}

// ===========================================================================
// B 组：apply_event 状态迁移
// ===========================================================================

#[test]
fn test_b_text_delta_appends_and_segments() {
    let mut p = AgentPanel::new();
    p.apply_event(SessionEvent::QueryStarted("q1".to_string()));
    p.apply_event(SessionEvent::TextDelta("你".to_string()));
    p.apply_event(SessionEvent::TextDelta("好".to_string()));
    assert!(matches!(
        p.entries.last(),
        Some(ChatEntry::AssistantStreaming(s)) if s == "你好"
    ));
    // tool 步骤后的下一轮文本应开新段。
    p.apply_event(SessionEvent::ToolStart {
        name: "proc_ls".to_string(),
        arguments: json!({}),
    });
    p.apply_event(SessionEvent::TextDelta("分析".to_string()));
    assert_eq!(
        p.entries.len(),
        4,
        "User + AssistantStreaming + ToolCall + 新 AssistantStreaming"
    );
}

#[test]
fn test_b_tool_start_finished_backfill() {
    let mut p = AgentPanel::new();
    p.apply_event(SessionEvent::ToolStart {
        name: "proc_ls".to_string(),
        arguments: json!({"limit": 5}),
    });
    p.apply_event(SessionEvent::ToolStart {
        name: "proc_help".to_string(),
        arguments: json!({"category": "docker"}),
    });
    p.apply_event(SessionEvent::ToolFinished {
        name: "proc_ls".to_string(),
        is_error: false,
        result_chars: 1234,
    });
    let mut ls_done = false;
    for e in &p.entries {
        if let ChatEntry::ToolCall {
            name,
            is_error,
            result_chars,
            ..
        } = e
        {
            if name == "proc_ls" {
                assert_eq!(*is_error, Some(false));
                assert_eq!(*result_chars, 1234);
                ls_done = true;
            }
            if name == "proc_help" {
                assert!(is_error.is_none(), "未完成的 tool 不应被误回填");
            }
        }
    }
    assert!(ls_done);
    assert_eq!(p.tool_steps, 2);
}

#[test]
fn test_b_confirm_requested_sets_pending_and_mode() {
    let mut p = AgentPanel::new();
    p.mode = AgentPanelMode::Streaming;
    let (tx, _rx) = tokio::sync::oneshot::channel();
    p.apply_event(SessionEvent::ConfirmRequested(ConfirmRequest {
        tool_name: "proc_kill".to_string(),
        arguments: json!({"pids": [123]}),
        summary: "终止进程 123".to_string(),
        reply: tx,
    }));
    assert_eq!(p.mode, AgentPanelMode::AwaitingConfirm);
    assert!(p.pending_confirm.is_some());
    assert!(matches!(p.entries.last(), Some(ChatEntry::Confirm { .. })));
}

#[test]
fn test_b_session_finished_endturn_and_interrupted() {
    let mut p = AgentPanel::new();
    p.apply_event(SessionEvent::QueryStarted("q".to_string()));
    p.apply_event(SessionEvent::SessionFinished {
        final_text: "最终回答".to_string(),
        stop: StopCause::EndTurn,
    });
    assert_eq!(p.mode, AgentPanelMode::Idle);
    assert!(p.finished_after.is_some(), "结束后应冻结用时");
    assert!(matches!(
        p.entries.last(),
        Some(ChatEntry::AssistantFinal(t)) if t == "最终回答"
    ));

    // Interrupted：落 Notice 不落占位 final_text。
    p.apply_event(SessionEvent::QueryStarted("q2".to_string()));
    p.apply_event(SessionEvent::SessionFinished {
        final_text: "已中断".to_string(),
        stop: StopCause::Interrupted,
    });
    assert!(matches!(p.entries.last(), Some(ChatEntry::Notice(_))));
}

#[test]
fn test_b_error_resets_to_idle() {
    let mut p = AgentPanel::new();
    p.apply_event(SessionEvent::QueryStarted("q".to_string()));
    p.mode = AgentPanelMode::Streaming;
    assert!(p.apply_event(SessionEvent::Error("ctx 溢出".to_string())));
    assert_eq!(p.mode, AgentPanelMode::Idle);
    assert!(matches!(p.entries.last(), Some(ChatEntry::Error(_))));
}

// ===========================================================================
// C 组：AgentSession 端到端驱动面板（真实 session 线程，零 LLM）
// ===========================================================================

/// drain 全部当前可读事件喂 panel，返回事件计数（App tick_agent 的测试等价物）。
#[allow(dead_code)]
fn drain_into_panel(handle: &SessionHandle, panel: &mut AgentPanel) -> usize {
    let mut n = 0;
    while let Some(ev) = handle.drain_event() {
        panel.apply_event(ev);
        n += 1;
    }
    n
}

#[test]
fn test_c_session_streaming_drives_panel() {
    // 单轮：流式文本 + proc_finish 提交最终答案（纯文本轮会直接 EndTurn，
    // proc_finish 才是多轮 ReAct 的显式结束路径——v0.20 决策 I）。
    let handle = spawn_session(ScriptedStreamProvider::new(vec![vec![
        text_delta("第一段"),
        tool_delta("proc_finish", json!({ "answer": "最终答案 42" })),
        end_turn(),
    ]]));
    let mut panel = AgentPanel::new();
    assert!(handle.send_query("测试 query"), "send_query 应成功");

    drain_into_until(
        &handle,
        &mut panel,
        |ev| matches!(ev, SessionEvent::SessionFinished { .. }),
        "SessionFinished",
    );

    assert_eq!(panel.mode, AgentPanelMode::Idle);
    let has_user = panel
        .entries
        .iter()
        .any(|e| matches!(e, ChatEntry::User(q) if q == "测试 query"));
    let has_stream = panel
        .entries
        .iter()
        .any(|e| matches!(e, ChatEntry::AssistantStreaming(t) if t == "第一段"));
    let has_final = panel
        .entries
        .iter()
        .any(|e| matches!(e, ChatEntry::AssistantFinal(t) if t == "最终答案 42"));
    assert!(
        has_user && has_stream && has_final,
        "entries: {:?}",
        panel.entry_summaries()
    );
    handle.shutdown();
}

#[test]
fn test_c_confirm_approved_roundtrip() {
    // kill 不存在 PID（安全断言——验证放行语义而非真杀）。
    let handle = spawn_session(ScriptedStreamProvider::new(vec![
        vec![
            tool_delta("proc_kill", json!({"pids": [4000000]})),
            end_turn(),
        ],
        finish_turn("kill 已执行"),
    ]));
    let mut panel = AgentPanel::new();
    assert!(handle.send_query("杀掉 4000000"));

    drain_into_until(
        &handle,
        &mut panel,
        |ev| matches!(ev, SessionEvent::ConfirmRequested(_)),
        "ConfirmRequested",
    );
    assert_eq!(panel.mode, AgentPanelMode::AwaitingConfirm);
    assert!(panel.pending_confirm.is_some());

    panel.resolve_confirm(ConfirmDecision::Approved);
    assert_eq!(panel.mode, AgentPanelMode::Streaming);

    drain_into_until(
        &handle,
        &mut panel,
        |ev| matches!(ev, SessionEvent::SessionFinished { .. }),
        "SessionFinished",
    );
    // Approved 真执行：proc_kill 走业务错误路径（PID 不存在）而非 blocked 拦截
    // ——ToolFinished 存在 + 最终回答到达。
    let kill_seen = panel.entries.iter().any(|e| {
        matches!(
            e,
            ChatEntry::ToolCall { name, is_error: Some(_), .. } if name == "proc_kill"
        )
    });
    assert!(
        kill_seen,
        "Approved 后 proc_kill 应真执行（业务结果），entries: {:?}",
        panel.entry_summaries()
    );
    assert!(matches!(
        panel.entries.last(),
        Some(ChatEntry::AssistantFinal(_))
    ));
    handle.shutdown();
}

#[test]
fn test_c_confirm_denied_roundtrip() {
    let handle = spawn_session(ScriptedStreamProvider::new(vec![
        vec![
            tool_delta("proc_kill", json!({"pids": [4000001]})),
            end_turn(),
        ],
        finish_turn("已拒绝并解释"),
    ]));
    let mut panel = AgentPanel::new();
    assert!(handle.send_query("杀掉 4000001"));

    drain_into_until(
        &handle,
        &mut panel,
        |ev| matches!(ev, SessionEvent::ConfirmRequested(_)),
        "ConfirmRequested",
    );
    panel.resolve_confirm(ConfirmDecision::Denied);

    drain_into_until(
        &handle,
        &mut panel,
        |ev| matches!(ev, SessionEvent::SessionFinished { .. }),
        "SessionFinished",
    );
    match panel.entries.last() {
        Some(ChatEntry::AssistantFinal(t)) => assert_eq!(t, "已拒绝并解释"),
        other => panic!("expected final, got {other:?}"),
    }
    handle.shutdown();
}

#[test]
fn test_c_interrupt_leaves_notice() {
    let handle = spawn_session(ScriptedStreamProvider::new(vec![
        vec![text_delta("长文本输出中"), end_turn()],
        finish_turn("done"),
    ]));
    let mut panel = AgentPanel::new();
    assert!(handle.send_query("q"));
    drain_into_until(
        &handle,
        &mut panel,
        |ev| matches!(ev, SessionEvent::SessionFinished { .. }),
        "SessionFinished",
    );
    // 先走完一轮拿回 Idle，再中断第二轮。
    assert!(handle.send_query("q2"));
    drain_into_until(
        &handle,
        &mut panel,
        |ev| matches!(ev, SessionEvent::QueryStarted(_)),
        "QueryStarted2",
    );
    handle.interrupt();
    drain_into_until(
        &handle,
        &mut panel,
        |ev| {
            matches!(
                ev,
                SessionEvent::SessionFinished {
                    stop: StopCause::Interrupted,
                    ..
                }
            )
        },
        "Interrupted SessionFinished",
    );
    assert_eq!(panel.mode, AgentPanelMode::Idle);
    handle.shutdown();
}

// ===========================================================================
// D 组：TestBackend 渲染
// ===========================================================================

fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            let ch = buf[(x, y)].symbol().chars().next().unwrap_or(' ');
            s.push(ch);
        }
        s.push('\n');
    }
    s
}

/// CJK 宽字符在 buffer 提取时第二格是空格——断言用去空格形态匹配中文子串。
fn compact(s: &str) -> String {
    s.replace(' ', "")
}

fn draw_app(app: &App) -> String {
    use ratatui::Terminal;
    let backend = ratatui::backend::TestBackend::new(90, 26);
    let mut terminal = Terminal::new(backend).expect("Terminal::new");
    terminal
        .draw(|f| proc::tui::layout::draw(f, app))
        .expect("draw");
    buffer_text(&terminal)
}

#[test]
fn test_d_render_idle_empty_hint() {
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    app.agent_panel.panel_mut().provider_detail = "llama-cpp: test-model".to_string();
    let text = draw_app(&app);
    let c = compact(&text);
    assert!(text.contains("AI Agent"));
    assert!(c.contains("输入query开始对话"), "空态应有输入提示:\n{text}");
    assert!(text.contains("test-model"), "状态行应显示 provider 短标签");
}

#[test]
fn test_d_render_streaming() {
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    let p = app.agent_panel.panel_mut();
    p.provider_detail = "mock: fixtures".to_string();
    p.mode = AgentPanelMode::Streaming;
    p.entries
        .push(ChatEntry::User("哪个进程占 CPU".to_string()));
    p.entries.push(ChatEntry::AssistantStreaming(
        "根据 proc_ls 结果".to_string(),
    ));
    let text = draw_app(&app);
    let c = compact(&text);
    assert!(c.contains("生成中"), "Streaming 态应有生成中提示:\n{text}");
    assert!(c.contains("哪个进程占CPU"));
    assert!(c.contains("根据proc_ls结果"));
}

#[test]
fn test_d_render_awaiting_confirm() {
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let p = app.agent_panel.panel_mut();
    p.provider_detail = "mock: fixtures".to_string();
    p.mode = AgentPanelMode::AwaitingConfirm;
    p.entries.push(ChatEntry::Confirm {
        tool_name: "proc_kill".to_string(),
        summary: "终止进程 4000000".to_string(),
        decision: None,
    });
    p.pending_confirm = Some(ConfirmRequest {
        tool_name: "proc_kill".to_string(),
        arguments: json!({"pids": [4000000]}),
        summary: "终止进程 4000000".to_string(),
        reply: tx,
    });
    let text = draw_app(&app);
    let c = compact(&text);
    assert!(c.contains("写操作确认"), "确认框标题:\n{text}");
    assert!(c.contains("[y]执行"), "确认框键位:\n{text}");
    assert!(text.contains("4000000"), "确认框应展示 summary:\n{text}");
}

// ===========================================================================
// E 组：App 集成（session 生命周期 / dispatch / None 降级）
// ===========================================================================

#[test]
fn test_e_palette_enter_builds_session_or_error() {
    let mut app = App::new().expect("App::new");
    assert_eq!(app.mode, AppMode::ProcessList);
    assert!(app.agent_session.is_none(), "未进入面板时无会话");

    app.handle_key(ctrl(KeyCode::Char('p')));
    for c in "agentpanel".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.mode, AppMode::Agent);
    // 本机 llama-cpp 配置生效 → Some；无 llama-server 环境构造失败 → None +
    // Error entry。两分支都不 panic 且状态自洽。
    if app.agent_session.is_none() {
        let has_err = app
            .agent_panel
            .panel()
            .entries
            .iter()
            .any(|e| matches!(e, ChatEntry::Error(_)));
        assert!(has_err, "构造失败应有 Error entry 提示");
    }

    // Ctrl+D 退出：teardown（pending confirm None 安全）+ 回 ProcessList。
    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(app.mode, AppMode::ProcessList);
    assert!(app.agent_session.is_none(), "退出面板后会话应拆掉");
}

#[test]
fn test_e_esc_idle_direct_mode_exits_none_safe() {
    // stage 1 既有路径：直接赋 mode（无 session）+ Esc → teardown(None) 不 panic。
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode, AppMode::ProcessList);
    assert!(app.agent_session.is_none());
}

#[test]
fn test_e_sendquery_without_session_degrades() {
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    for ch in "你好".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Enter));
    let has_err = app
        .agent_panel
        .panel()
        .entries
        .iter()
        .any(|e| matches!(e, ChatEntry::Error(t) if t.contains("会话不可用")));
    assert!(has_err, "无会话发送应降级 Error 提示");
}

#[test]
fn test_e_shutdown_teardown_is_idempotent() {
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    app.shutdown();
    app.shutdown();
    assert!(app.agent_session.is_none());
}

#[test]
fn test_e_agent_mode_captures_global_keys() {
    // E2B 实测踩坑回归锚：Agent 输入框必须能打数字 / R / t / c——这些是
    // 全局 tab-switch / 录屏开关 / 主题切换键，Agent 模式下须豁免（否则
    // 「列出 top 3」会把面板切到 PortMap）。
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    for ch in "top 3 Rtc".chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    assert_eq!(app.mode, AppMode::Agent, "数字/字母不应切走面板");
    assert_eq!(app.agent_panel.panel().input, "top 3 Rtc");
    assert!(!app.recording_wanted(), "R 不应触发录屏开关");
}

// ===========================================================================
// 辅助：ChatEntry 摘要（断言失败信息可读）——测试文件私有 trait 扩展。
// ===========================================================================

trait EntrySummary {
    fn entry_summaries(&self) -> Vec<String>;
}

impl EntrySummary for AgentPanel {
    fn entry_summaries(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|e| match e {
                ChatEntry::User(q) => format!("User({q})"),
                ChatEntry::AssistantStreaming(t) | ChatEntry::AssistantFinal(t) => {
                    format!("Assistant({t})")
                }
                ChatEntry::ToolCall { name, is_error, .. } => {
                    format!("Tool({name}, {:?})", is_error)
                }
                ChatEntry::Confirm {
                    tool_name,
                    decision,
                    ..
                } => {
                    format!("Confirm({tool_name}, {:?})", decision)
                }
                ChatEntry::Error(t) => format!("Error({t})"),
                ChatEntry::Notice(t) => format!("Notice({t})"),
            })
            .collect()
    }
}

// ===========================================================================
// F 组：E2B 端到端 #[ignore]（真实 llama-server，App 层全链路）
// ===========================================================================

/// 面板驱动一个 query：type → Enter → 两阶段等待（先离开 Idle——query 被
/// session 接受；再等回到 target 态）。每 tick 即 App::tick——生产主循环
/// 同款 drain 路径。
fn e2b_run_until(app: &mut App, query: &str, target: AgentPanelMode, timeout: Duration) {
    for ch in query.chars() {
        app.handle_key(key(KeyCode::Char(ch)));
    }
    app.handle_key(key(KeyCode::Enter));
    let deadline = Instant::now() + timeout;
    loop {
        app.tick();
        if app.agent_panel.panel().mode != AgentPanelMode::Idle {
            break;
        }
        if Instant::now() > deadline {
            panic!(
                "E2B query 未被接受（一直 Idle）: {:?}",
                app.agent_panel.panel().entry_summaries()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    loop {
        app.tick();
        if app.agent_panel.panel().mode == target {
            return;
        }
        if Instant::now() > deadline {
            panic!(
                "E2B query 超时未到 {target:?}: {:?}",
                app.agent_panel.panel().entry_summaries()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 真实 E2B 端到端（stage 3 手动验收的自动化子集）：
/// ① L0 query 流式 + tool 步骤；② 多轮追问（D4 history 生效）；③ 写操作
/// confirm n 拒绝路径（目标 = 不存在 PID，零风险）；④ Ctrl+D teardown。
/// y 真执行路径 + 视觉/手感留给人工验收。手动跑：
/// `cargo test --release --test test_agent_panel -- --ignored --test-threads=1`
#[test]
#[ignore = "真实 llama-server（E2B 本机），手动单线程跑"]
fn test_f_e2b_app_e2e_smoke() {
    let mut app = App::new().expect("App::new");
    app.handle_key(ctrl(KeyCode::Char('p')));
    for c in "agentpanel".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.mode, AppMode::Agent);
    assert!(
        app.agent_session.is_some(),
        "本机 agent.toml 应已配好 llama-cpp（v0.20 落地）"
    );

    // ① L0：tool 步骤 + 流式 + 最终回答。
    e2b_run_until(
        &mut app,
        "列出 CPU 占用最高的 3 个进程",
        AgentPanelMode::Idle,
        Duration::from_secs(180),
    );
    let p = app.agent_panel.panel();
    assert!(
        p.entries
            .iter()
            .any(|e| matches!(e, ChatEntry::AssistantFinal(t) if !t.is_empty())),
        "应有非空最终回答: {:?}",
        p.entry_summaries()
    );

    // ② 多轮追问（引用第 1 轮上下文）。
    e2b_run_until(
        &mut app,
        "你刚才列出的第一个进程叫什么名字",
        AgentPanelMode::Idle,
        Duration::from_secs(180),
    );

    // ③ confirm 拒绝路径：不存在 PID。措辞引导 E2B 走 proc_help 发现
    // proc_kill（entry 工具集不含写 tool——两层架构，v0.20 决策 J）。
    e2b_run_until(
        &mut app,
        "先调用 proc_help 查看 process 类别有哪些工具，然后用其中的终止工具终止 PID 4000000 的进程",
        AgentPanelMode::AwaitingConfirm,
        Duration::from_secs(180),
    );
    assert!(app.agent_panel.panel().pending_confirm.is_some());
    app.handle_key(key(KeyCode::Char('n')));
    e2b_wait_idle(&mut app, Duration::from_secs(180));
    let p = app.agent_panel.panel();
    assert!(
        p.entries.iter().any(|e| matches!(
            e,
            ChatEntry::Confirm {
                decision: Some(ConfirmDecision::Denied),
                ..
            }
        )),
        "n 后 Confirm entry 应回填 Denied: {:?}",
        p.entry_summaries()
    );

    // ④ Ctrl+D teardown：会话拆掉（llama-server kill 的任务管理器检查留人工）。
    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(app.mode, AppMode::ProcessList);
    assert!(app.agent_session.is_none());
}

/// 等当前 run 收尾回 Idle（confirm 已答复后等剩余事件排空）。
fn e2b_wait_idle(app: &mut App, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        app.tick();
        if app.agent_panel.panel().mode == AgentPanelMode::Idle {
            return;
        }
        if Instant::now() > deadline {
            panic!(
                "E2B 收尾超时: {:?}",
                app.agent_panel.panel().entry_summaries()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
