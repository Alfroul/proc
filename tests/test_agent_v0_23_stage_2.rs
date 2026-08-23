//! v0.23 stage 2 集成测试 — proc_record_start/stop agent 侧实装
//! （ADR-0033 D5/D6）。
//!
//! 三组：
//! - **A 组 TUI session 端到端**：AgentSession 真线程 + ScriptedStreamProvider
//!   脚本化 model 行为，confirm（y）后 record_start/stop 真实执行——断言经
//!   provider 二轮 messages 的 Tool result（spawn 的 child 是 test binary
//!   自身（current_exe），带 `record --no-tui` 参数快速退出，断言只锚
//!   ok/pid/file_path/action，不锚 child 存活或真实 .prec 帧）
//! - **B 组 handle 级孤儿清理**：session 线程 start 后经 SessionHandle
//!   stop_orphan_recording 落盘（验证 spawn 内建的 RecordState 跨线程共享）
//! - **C 组 CLI 拦截不变锚**：complete 路径（CLI ask / eval 同款）record 两
//!   tool blocked + 专属文案「仅 TUI AgentPanel 会话支持」；proc_kill 等
//!   其余写 tool 文案不变
//!
//! 孤儿清理的单元级覆盖（fake child kill / 幂等 / 无录制 None）在
//! `src/agent/session.rs` 内联 `#[cfg(test)]`（私有字段注入可达）。

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};

use proc::agent::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
};
use proc::agent::session::{AgentSession, ConfirmDecision, SessionEvent, SessionHandle};
use proc::agent::tools::{catalog, dispatch};
use proc::agent::types::{Message, Role, ToolCall};

// ===========================================================================
// ScriptedStreamProvider — 逐 turn 弹出脚本化 delta 序列（v0.21 stage 2 同款）
// ===========================================================================

struct ScriptedStreamProvider {
    turns: Mutex<VecDeque<Vec<Delta>>>,
    seen_messages: Mutex<Vec<Vec<Message>>>,
}

impl ScriptedStreamProvider {
    fn new(turns: Vec<Vec<Delta>>) -> Self {
        Self {
            turns: Mutex::new(turns.into_iter().collect()),
            seen_messages: Mutex::new(Vec::new()),
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
        messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        self.seen_messages.lock().unwrap().push(messages);
        let turn = self.turns.lock().unwrap().pop_front();
        match turn {
            Some(deltas) => futures_util::stream::iter(deltas.into_iter().map(Ok)).boxed(),
            None => futures_util::stream::empty::<Result<Delta, LlmError>>().boxed(),
        }
    }
}

// ===========================================================================
// 构造 helper
// ===========================================================================

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

fn spawn_session(provider: ScriptedStreamProvider) -> (Arc<ScriptedStreamProvider>, SessionHandle) {
    let provider = Arc::new(provider);
    let handle = AgentSession::spawn(
        Arc::clone(&provider) as Arc<dyn LlmProvider>,
        catalog::default_registry(),
        proc::agent::runner::AgentOptions {
            max_steps: 6,
            ..Default::default()
        },
        proc::agent::SessionRecorder::disabled(),
    );
    (provider, handle)
}

/// 轮询 drain 直到谓词命中或超时；返回全部已 drain 事件（含命中项）。
fn drain_until(
    handle: &SessionHandle,
    timeout: Duration,
    mut pred: impl FnMut(&SessionEvent) -> bool,
) -> (Vec<SessionEvent>, bool) {
    let deadline = Instant::now() + timeout;
    let mut collected = Vec::new();
    while Instant::now() < deadline {
        match handle.drain_event() {
            Some(ev) => {
                let hit = pred(&ev);
                collected.push(ev);
                if hit {
                    return (collected, true);
                }
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    (collected, false)
}

/// drain 到下一个 ConfirmRequested 并回传决策（confirm 协议交互的测试侧）。
fn approve_next_confirm(handle: &SessionHandle, events: &mut Vec<SessionEvent>) -> String {
    let (mut got, hit) = drain_until(handle, Duration::from_secs(15), |ev| {
        matches!(ev, SessionEvent::ConfirmRequested(_))
    });
    assert!(hit, "应收到 ConfirmRequested: {got:?}");
    let ev = got.remove(got.len() - 1);
    events.append(&mut got);
    match ev {
        SessionEvent::ConfirmRequested(req) => {
            let name = req.tool_name.clone();
            let _ = req.reply.send(ConfirmDecision::Approved);
            name
        }
        other => panic!("unreachable: {other:?}"),
    }
}

/// 第 idx 次 provider 调用（0-based）收到的第 n 个（0-based）Tool 消息 content。
fn tool_result_content(seen: &[Vec<Message>], call_idx: usize, tool_idx: usize) -> String {
    seen.get(call_idx)
        .and_then(|msgs| {
            msgs.iter()
                .filter(|m| m.role == Role::Tool)
                .nth(tool_idx)
                .and_then(|m| m.tool_results.first().map(|r| r.content.clone()))
        })
        .unwrap_or_else(|| panic!("seen[{call_idx}] 无第 {tool_idx} 个 Tool result"))
}

fn tmp_output_path(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("proc-v023-stage2-{}-{}", std::process::id(), tag));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{tag}.prec"))
}

// ===========================================================================
// A 组：TUI session 端到端（confirm → 真实执行）
// ===========================================================================

#[test]
fn record_start_and_stop_via_confirm_e2e() {
    let output = tmp_output_path("e2e");
    let (provider, handle) = spawn_session(ScriptedStreamProvider::new(vec![
        vec![tool_delta(
            "proc_record_start",
            json!({ "output": output.to_string_lossy(), "confirm": true }),
        )],
        vec![tool_delta("proc_record_stop", json!({}))],
        finish_turn("done"),
    ]));

    assert!(handle.send_query("帮我录屏"));
    let mut events = Vec::new();

    // turn 1：record_start → confirm → Approved → 真实 spawn。
    let name = approve_next_confirm(&handle, &mut events);
    assert_eq!(name, "proc_record_start");
    // confirm summary 已就位（stage 2 不动的锚）：录屏文案。
    if let Some(SessionEvent::ConfirmRequested(req)) = events.iter().rev().find(
        |ev| matches!(ev, SessionEvent::ConfirmRequested(r) if r.tool_name == "proc_record_start"),
    ) {
        assert!(req.summary.contains("录屏"), "{}", req.summary);
    }

    // turn 2：record_stop → confirm → Approved → 真实 kill + flush。
    let name = approve_next_confirm(&handle, &mut events);
    assert_eq!(name, "proc_record_stop");

    let (mut rest, hit) = drain_until(&handle, Duration::from_secs(15), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit, "应收到 SessionFinished: {rest:?}");
    events.append(&mut rest);

    // 两个 record tool 均真实执行（非 blocked → is_error=false）。
    let record_finished: Vec<_> = events
        .iter()
        .filter_map(|ev| match ev {
            SessionEvent::ToolFinished {
                name,
                is_error,
                result_chars,
            } if name.starts_with("proc_record") => Some((*is_error, *result_chars)),
            _ => None,
        })
        .collect();
    assert_eq!(record_finished.len(), 2, "{events:?}");
    assert!(
        record_finished.iter().all(|(e, _)| !e),
        "Approved 真执行不是 dispatch 错误: {record_finished:?}"
    );

    // provider 二轮收到的 Tool result：start ok + 落盘路径 + pid；
    // 三轮收到 stop 结果（child 已快速退出 → metadata 容错，action 仍 stop）。
    let seen = provider.seen_messages.lock().unwrap();
    assert!(seen.len() >= 3, "3 次 provider 调用: {}", seen.len());
    let start_content = tool_result_content(&seen, 1, 0);
    assert!(start_content.contains("\"ok\":true"), "{start_content}");
    assert!(
        start_content.contains("e2e.prec"),
        "落盘路径回显: {start_content}"
    );
    assert!(start_content.contains("\"pid\""), "{start_content}");

    let stop_content = tool_result_content(&seen, 2, 1);
    assert!(
        stop_content.contains("\"action\":\"stop\"") || stop_content.contains("stop"),
        "{stop_content}"
    );
    assert!(stop_content.contains("\"ok\":true"), "{stop_content}");

    handle.shutdown();
}

#[test]
fn record_stop_without_active_recording_is_business_error() {
    let (provider, handle) = spawn_session(ScriptedStreamProvider::new(vec![
        vec![tool_delta("proc_record_stop", json!({}))],
        finish_turn("done"),
    ]));

    assert!(handle.send_query("停录屏"));
    let mut events = Vec::new();
    let name = approve_next_confirm(&handle, &mut events);
    assert_eq!(name, "proc_record_stop");

    let (mut rest, hit) = drain_until(&handle, Duration::from_secs(15), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit, "{rest:?}");
    events.append(&mut rest);

    // 无录制是业务错误（ok:false）而非 dispatch 错误——is_error 不置位。
    let finished = events.iter().find_map(|ev| match ev {
        SessionEvent::ToolFinished { name, is_error, .. } if name == "proc_record_stop" => {
            Some(*is_error)
        }
        _ => None,
    });
    assert_eq!(finished, Some(false), "{events:?}");

    let seen = provider.seen_messages.lock().unwrap();
    let content = tool_result_content(&seen, 1, 0);
    assert!(content.contains("无录屏进行中"), "{content}");
    assert!(content.contains("\"ok\":false"), "{content}");

    handle.shutdown();
}

// ===========================================================================
// B 组：handle 级孤儿清理（RecordState 跨线程共享验证）
// ===========================================================================

#[test]
fn handle_stop_orphan_recording_after_session_start() {
    let output = tmp_output_path("orphan");
    let (provider, handle) = spawn_session(ScriptedStreamProvider::new(vec![
        vec![tool_delta(
            "proc_record_start",
            json!({ "output": output.to_string_lossy(), "confirm": true }),
        )],
        finish_turn("started"),
    ]));

    assert!(handle.send_query("开录"));
    let mut events = Vec::new();
    let name = approve_next_confirm(&handle, &mut events);
    assert_eq!(name, "proc_record_start");
    let (rest, hit) = drain_until(&handle, Duration::from_secs(15), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit, "{rest:?}");
    drop(provider);

    // 会话线程 start 后，Handle 侧（App teardown 路径）能感知并自动落盘。
    let notice = handle
        .stop_orphan_recording()
        .expect("录制中应返 Notice 文案");
    assert!(
        notice.contains("录屏已自动保存至") && notice.contains("orphan.prec"),
        "{notice}"
    );
    // 幂等 + 线程退出兜底（belt）也不再触发。
    assert!(handle.stop_orphan_recording().is_none());
    handle.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_exited() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(handle.is_exited(), "session 线程应退出");
}

// ===========================================================================
// C 组：CLI 拦截不变锚（complete 路径——CLI ask / eval 同款）
// ===========================================================================

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments: args,
    }
}

#[test]
fn cli_intercept_record_tools_message_updated() {
    let registry = catalog::default_registry();
    for name in ["proc_record_start", "proc_record_stop"] {
        let result = dispatch::execute_tool(
            &registry,
            &call(name, json!({ "output": "x.prec", "confirm": true })),
        );
        assert!(result.is_error, "{name} 无 confirm 通道 → blocked");
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["blocked"], json!(true));
        let err = v["error"].as_str().unwrap();
        assert!(
            err.contains("仅 TUI AgentPanel 会话支持"),
            "{name} 专属文案: {err}"
        );
    }
}

#[test]
fn cli_intercept_kill_message_unchanged() {
    // 锚：其余 6 写 tool 的拦截文案逐字不变（v0.20 stage 3b 契约）。
    let registry = catalog::default_registry();
    let result = dispatch::execute_tool(&registry, &call("proc_kill", json!({ "pid": 1 })));
    assert!(result.is_error);
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["blocked"], json!(true));
    let err = v["error"].as_str().unwrap();
    assert!(err.contains("写操作已拦截"), "{err}");
    assert!(err.contains("proc kill <pid>"), "{err}");
}
