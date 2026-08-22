//! v0.21 stage 2 集成测试 — run_streaming 流式 + confirm 协议 + AgentSession。
//!
//! CI 测试零 LLM 调用：ScriptedStreamProvider 逐 turn 弹出脚本化 delta 序列
//! （决策 G 同款思路的流式版——MockProvider 是 hash 单轮回放，不适用于
//! 多轮 loop 测试）。
//!
//! 一个 `#[ignore]` 真实测试（本机显式跑，brainstorm 风险 3）：
//! - `test_agent_v0_21_streaming_e2b_smoke`：真实 llama-server 流式 2 query，
//!   验证 stream + tool_choice=required + proc_finish 组合下的 tool_calls
//!   分片与 EndTurn 行为
//!
//! 运行（真实测试手动）：
//! ```text
//! cargo test --release --test test_agent_v0_21_stage_2 -- --ignored --test-threads=1
//! ```

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};

use proc::agent::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
};
use proc::agent::runner::{AgentOptions, AgentRunner, StopCause, StreamEvent};
use proc::agent::session::{
    AgentSession, ConfirmDecision, ConfirmRequest, MAX_HISTORY_TURNS, SessionCommand, SessionEvent,
    SessionHandle, truncate_history,
};
use proc::agent::tools::{catalog, dispatch};
use proc::agent::types::{Message, Role, ToolCall};

// ===========================================================================
// ScriptedStreamProvider — 逐 turn 弹出脚本化 delta 序列
// ===========================================================================

enum TurnScript {
    Deltas(Vec<Delta>),
    Fail(String),
}

struct ScriptedStreamProvider {
    turns: Mutex<VecDeque<TurnScript>>,
    seen_messages: Mutex<Vec<Vec<Message>>>,
    seen_tools: Mutex<Vec<Vec<String>>>,
}

impl ScriptedStreamProvider {
    fn new(turns: Vec<Vec<Delta>>) -> Self {
        Self {
            turns: Mutex::new(turns.into_iter().map(TurnScript::Deltas).collect()),
            seen_messages: Mutex::new(Vec::new()),
            seen_tools: Mutex::new(Vec::new()),
        }
    }

    fn with_fail(turns: Vec<TurnScript>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
            seen_messages: Mutex::new(Vec::new()),
            seen_tools: Mutex::new(Vec::new()),
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
        tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        self.seen_messages.lock().unwrap().push(messages);
        self.seen_tools
            .lock()
            .unwrap()
            .push(tools.iter().map(|t| t.name.clone()).collect());
        let turn = self.turns.lock().unwrap().pop_front();
        match turn {
            Some(TurnScript::Deltas(deltas)) => {
                futures_util::stream::iter(deltas.into_iter().map(Ok)).boxed()
            }
            Some(TurnScript::Fail(msg)) => {
                futures_util::stream::iter(vec![Err(LlmError::Config(msg))]).boxed()
            }
            None => futures_util::stream::empty::<Result<Delta, LlmError>>().boxed(),
        }
    }
}

// ===========================================================================
// 构造 helper
// ===========================================================================

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

fn runner_with(
    provider: ScriptedStreamProvider,
    max_steps: u32,
) -> (Arc<ScriptedStreamProvider>, AgentRunner) {
    let provider = Arc::new(provider);
    let runner = AgentRunner::new(
        Arc::clone(&provider) as Arc<dyn LlmProvider>,
        catalog::default_registry(),
        AgentOptions {
            max_steps,
            ..Default::default()
        },
    );
    (provider, runner)
}

/// 收集 sink 事件到共享 Vec（测试断言用）。
type EventLog = Arc<Mutex<Vec<StreamEvent>>>;
type EventSink = Arc<dyn Fn(StreamEvent) + Send + Sync>;

fn event_collector() -> (EventLog, EventSink) {
    let events: EventLog = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = Arc::clone(&events);
        Arc::new(move |ev: StreamEvent| {
            events.lock().unwrap().push(ev);
        }) as EventSink
    };
    (events, sink)
}

fn new_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

// ===========================================================================
// A 组：run_streaming 核心循环
// ===========================================================================

#[tokio::test]
async fn test_streaming_single_turn_text() {
    let (provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![vec![text_delta("你"), text_delta("好"), end_turn()]]),
        4,
    );
    let (events, sink) = event_collector();
    let cancel = new_cancel();

    let outcome = runner
        .run_streaming("hi", &[], sink.as_ref(), None, &cancel)
        .await
        .expect("streaming 应成功");
    assert_eq!(outcome.final_text, "你好");
    assert_eq!(outcome.stop, StopCause::EndTurn);
    {
        let events = events.lock().unwrap();
        assert!(matches!(&events[0], StreamEvent::TextDelta(t) if t == "你"));
        assert!(matches!(&events[1], StreamEvent::TextDelta(t) if t == "好"));
        assert!(matches!(events[2], StreamEvent::TurnFinished));
    }
    // 空 query 校验与 complete 路径同款。
    let err = runner
        .run_streaming("   ", &[], sink.as_ref(), None, &cancel)
        .await;
    assert!(err.is_err());
    assert_eq!(provider.seen_messages.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_streaming_tool_then_finish() {
    let (provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_help", json!({ "category": "meta" }))],
            finish_turn("已查询"),
        ]),
        4,
    );
    let (events, sink) = event_collector();
    let cancel = new_cancel();

    let outcome = runner
        .run_streaming("帮我看看", &[], sink.as_ref(), None, &cancel)
        .await
        .expect("应成功");
    assert_eq!(outcome.final_text, "已查询");
    assert_eq!(outcome.stop, StopCause::EndTurn);
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(outcome.steps[0].tool_name, "proc_help");
    assert!(!outcome.steps[0].is_error, "proc_help 业务应正常");

    let events = events.lock().unwrap();
    let has_start = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolStart { name, .. } if name == "proc_help"));
    let has_finished = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolFinished { name, is_error, .. } if name == "proc_help" && !is_error));
    assert!(
        has_start && has_finished,
        "应有 ToolStart/ToolFinished 事件"
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, StreamEvent::TurnFinished))
            .count(),
        2,
        "两轮各一个 TurnFinished"
    );
    // 两轮 provider 调用：第二轮 messages 含 Role::Tool 回填。
    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1].iter().any(|m| m.role == Role::Tool),
        "第二轮应含 tool result 回填"
    );
}

#[tokio::test]
async fn test_streaming_history_passed_to_provider() {
    let history = vec![
        Message::new(Role::User, "第一问"),
        Message::new(Role::Assistant, "第一答"),
    ];
    let (provider, runner) = runner_with(ScriptedStreamProvider::new(vec![finish_turn("ok")]), 4);
    let (_events, sink) = event_collector();
    let cancel = new_cancel();

    runner
        .run_streaming("第二问", &history, sink.as_ref(), None, &cancel)
        .await
        .unwrap();
    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(seen[0].len(), 4, "system + 2 history + user query");
    assert_eq!(seen[0][0].role, Role::System);
    assert_eq!(seen[0][1].content.as_deref(), Some("第一问"));
    assert_eq!(seen[0][2].content.as_deref(), Some("第一答"));
    assert_eq!(seen[0][3].content.as_deref(), Some("第二问"));
}

#[tokio::test]
async fn test_streaming_max_steps() {
    let (provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_help", json!({ "category": "meta" }))],
            vec![tool_delta("proc_help", json!({ "category": "meta" }))],
            vec![tool_delta("proc_help", json!({ "category": "meta" }))],
        ]),
        2,
    );
    let (_events, sink) = event_collector();
    let cancel = new_cancel();

    let outcome = runner
        .run_streaming("q", &[], sink.as_ref(), None, &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.stop, StopCause::MaxSteps);
    assert_eq!(outcome.steps.len(), 2);
    assert_eq!(provider.seen_messages.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_streaming_empty_response_nudge() {
    let (provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![end_turn()], // 空响应（无 text 无 calls）
            finish_turn("缓过来了"),
        ]),
        4,
    );
    let (_events, sink) = event_collector();
    let cancel = new_cancel();

    let outcome = runner
        .run_streaming("q", &[], sink.as_ref(), None, &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.final_text, "缓过来了");
    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(seen.len(), 2, "nudge 后同 turn 二连 stream");
    let last = seen[1].last().unwrap();
    assert!(
        last.role == Role::User && last.content.as_deref().is_some_and(|c| c.contains("空的")),
        "nudge user 消息应在第二轮 messages 末尾: {last:?}"
    );
}

#[tokio::test]
async fn test_streaming_ignores_tool_result_delta() {
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![vec![
            Delta::ToolResult(proc::agent::types::ToolResult {
                tool_call_id: "x".into(),
                content: "应被忽略".into(),
                is_error: false,
            }),
            text_delta("hi"),
            end_turn(),
        ]]),
        4,
    );
    let (events, sink) = event_collector();
    let cancel = new_cancel();

    let outcome = runner
        .run_streaming("q", &[], sink.as_ref(), None, &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.final_text, "hi");
    assert_eq!(outcome.steps.len(), 0, "ToolResult delta 不触发执行");
    assert_eq!(
        events.lock().unwrap().len(),
        2,
        "仅 TextDelta + TurnFinished"
    );
}

#[tokio::test]
async fn test_streaming_cancel_before_first_turn() {
    let (provider, runner) = runner_with(ScriptedStreamProvider::new(vec![finish_turn("x")]), 4);
    let (_events, sink) = event_collector();
    let cancel = new_cancel();
    cancel.store(true, Ordering::Relaxed);

    let outcome = runner
        .run_streaming("q", &[], sink.as_ref(), None, &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.stop, StopCause::Interrupted);
    assert!(
        provider.seen_messages.lock().unwrap().is_empty(),
        "cancel 后不触达 provider"
    );
}

#[tokio::test]
async fn test_streaming_cancel_mid_stream() {
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![vec![
            text_delta("a"),
            text_delta("b"),
            text_delta("c"),
            end_turn(),
        ]]),
        4,
    );
    let events: Arc<Mutex<Vec<StreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let cancel = new_cancel();
    let c2 = Arc::clone(&cancel);
    let sink = {
        let events = Arc::clone(&events);
        move |ev: StreamEvent| {
            let is_text = matches!(ev, StreamEvent::TextDelta(_));
            events.lock().unwrap().push(ev);
            if is_text {
                c2.store(true, Ordering::Relaxed);
            }
        }
    };

    let outcome = runner
        .run_streaming("q", &[], &sink, None, &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.stop, StopCause::Interrupted);
    assert_eq!(outcome.final_text, "已中断");
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(_))),
        "首个 delta 已透传后才 cancel"
    );
}

#[tokio::test]
async fn test_streaming_expand_tools_via_help() {
    let (provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_help", json!({ "category": "docker" }))],
            finish_turn("done"),
        ]),
        4,
    );
    let (_events, sink) = event_collector();
    let cancel = new_cancel();

    runner
        .run_streaming("q", &[], sink.as_ref(), None, &cancel)
        .await
        .unwrap();
    let seen = provider.seen_tools.lock().unwrap();
    assert!(
        seen[0].iter().all(|n| n != "proc_docker_ps"),
        "首轮 tools 不含 docker 类"
    );
    assert!(
        seen[1].iter().any(|n| n == "proc_docker_ps"),
        "proc_help(docker) 后动态扩入: {:?}",
        seen[1]
    );
}

// ===========================================================================
// B 组：confirm 协议（run_streaming 路径）
// ===========================================================================

/// 不存在的 PID（Windows PID 上限 4194304 内、真实进程表外）——Approved 真执行
/// 走业务错误路径，验证放行语义而不真杀进程（风险 5 测试安全约定）。
const MISSING_PID: u64 = 4_000_000;

#[tokio::test]
async fn test_confirm_approved_executes_with_confirm_true() {
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_kill", json!({ "pid": MISSING_PID }))],
            finish_turn("处理完成"),
        ]),
        4,
    );
    let (events, sink) = event_collector();
    let cancel = new_cancel();

    let summary_seen = Arc::new(Mutex::new(None::<String>));
    let s2 = Arc::clone(&summary_seen);
    let hook = move |req: ConfirmRequest| {
        *s2.lock().unwrap() = Some(req.summary.clone());
        let _ = req.reply.send(ConfirmDecision::Approved);
    };

    let outcome = runner
        .run_streaming("杀掉它", &[], sink.as_ref(), Some(&hook), &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.final_text, "处理完成");

    let summary = summary_seen.lock().unwrap().clone().unwrap();
    assert!(
        summary.contains(&MISSING_PID.to_string()),
        "summary 应含 PID: {summary}"
    );

    // Approved → execute_confirmed_tool（真执行路径）：业务失败（pid 不存在）
    // 但不是 blocked 拦截。
    assert_eq!(outcome.steps.len(), 1);
    assert!(!outcome.steps[0].is_error, "业务层失败不是 dispatch 错误");
    let events = events.lock().unwrap();
    if let Some(StreamEvent::ToolFinished { is_error, .. }) = events
        .iter()
        .find(|e| matches!(e, StreamEvent::ToolFinished { .. }))
    {
        assert!(!is_error);
    }
}

#[tokio::test]
async fn test_confirm_denied_returns_blocked() {
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_kill", json!({ "pid": 123 }))],
            finish_turn("已拒绝"),
        ]),
        4,
    );
    let (events, sink) = event_collector();
    let cancel = new_cancel();
    let hook = move |req: ConfirmRequest| {
        let _ = req.reply.send(ConfirmDecision::Denied);
    };

    let outcome = runner
        .run_streaming("杀", &[], sink.as_ref(), Some(&hook), &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.steps.len(), 1);
    assert!(outcome.steps[0].is_error, "Denied → blocked is_error=true");
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::ToolFinished { is_error: true, .. })),
        "ToolFinished 事件应标 is_error"
    );
}

#[tokio::test]
async fn test_confirm_none_keeps_blocked_interception() {
    // CLI ask 语义锚：无 confirm 通道时写 tool 直接 blocked（v0.20 行为不变）。
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_kill", json!({ "pid": 123 }))],
            finish_turn("ok"),
        ]),
        4,
    );
    let (_events, sink) = event_collector();
    let cancel = new_cancel();

    let outcome = runner
        .run_streaming("q", &[], sink.as_ref(), None, &cancel)
        .await
        .unwrap();
    assert!(outcome.steps[0].is_error, "无通道 → blocked 拦截");
}

#[tokio::test]
async fn test_confirm_sender_drop_counts_as_denied() {
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_kill", json!({ "pid": 123 }))],
            finish_turn("ok"),
        ]),
        4,
    );
    let (_events, sink) = event_collector();
    let cancel = new_cancel();
    // hook 拿到 request 但不答复、reply 直接随闭包 drop——rx Err 视同 Denied。
    let hook = move |_req: ConfirmRequest| {};

    let outcome = runner
        .run_streaming("q", &[], sink.as_ref(), Some(&hook), &cancel)
        .await
        .unwrap();
    assert!(outcome.steps[0].is_error, "Sender drop → Denied 语义");
}

#[tokio::test]
async fn test_confirm_cancel_during_await_interrupts() {
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta("proc_kill", json!({ "pid": 123 }))],
            finish_turn("不应到这里"),
        ]),
        4,
    );
    let (_events, sink) = event_collector();
    let cancel = new_cancel();
    let c2 = Arc::clone(&cancel);
    // hook：扣住 reply 不答复 + 立即置 cancel——await_confirm_decision 的
    // select 轮询必须发现 cancel 并放行（否则本测试挂到超时，风险 2）。
    let hook = move |_req: ConfirmRequest| {
        c2.store(true, Ordering::Relaxed);
    };

    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        runner.run_streaming("q", &[], sink.as_ref(), Some(&hook), &cancel),
    )
    .await
    .expect("confirm 挂起遇 cancel 不应悬挂")
    .unwrap();
    assert_eq!(outcome.stop, StopCause::Interrupted);
}

#[tokio::test]
async fn test_confirm_record_tools_unsupported() {
    // 决策 8：record_start/stop 即使 Approved 也返「不支持」（跨调用子进程
    // 保活需持久状态，v0.22+ 评估）。
    let (_provider, runner) = runner_with(
        ScriptedStreamProvider::new(vec![
            vec![tool_delta(
                "proc_record_start",
                json!({ "file_path": "x.prec" }),
            )],
            finish_turn("ok"),
        ]),
        4,
    );
    let (_events, sink) = event_collector();
    let cancel = new_cancel();
    let hook = move |req: ConfirmRequest| {
        let _ = req.reply.send(ConfirmDecision::Approved);
    };

    let outcome = runner
        .run_streaming("录屏", &[], sink.as_ref(), Some(&hook), &cancel)
        .await
        .unwrap();
    assert!(!outcome.steps[0].is_error);
    // content 不在 outcome 里——经 provider 二轮 messages 验证不可达（scripted）。
    // 断言放 is_error 语义 + 不 panic 即可（content 断言在 C 组 dispatch 直测）。
}

// ===========================================================================
// C 组：dispatch 新函数
// ===========================================================================

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments: args,
    }
}

#[test]
fn test_confirm_summary_per_tool() {
    let s = dispatch::confirm_summary("proc_kill", &json!({ "pid": 1234 }));
    assert!(s.contains("1234"), "{s}");

    let s = dispatch::confirm_summary("proc_pkill", &json!({ "name": "chrome" }));
    assert!(s.contains("chrome"), "{s}");

    let s = dispatch::confirm_summary(
        "proc_usb_release",
        &json!({ "drive": "E", "kill_pids": [111, 222] }),
    );
    assert!(
        s.contains("E") && s.contains("111") && s.contains("222"),
        "{s}"
    );

    let s = dispatch::confirm_summary("proc_usb_release", &json!({ "drive": "F" }));
    assert!(s.contains("F") && s.contains("不杀进程"), "{s}");

    let s = dispatch::confirm_summary("proc_docker_rm", &json!({ "container_id": "nginx" }));
    assert!(s.contains("nginx") && s.contains("容器"), "{s}");

    let s = dispatch::confirm_summary("proc_docker_image_rm", &json!({ "image_id": "abc:1" }));
    assert!(s.contains("abc:1") && s.contains("镜像"), "{s}");

    let s = dispatch::confirm_summary("proc_docker_volume_rm", &json!({ "volume_name": "data" }));
    assert!(s.contains("data") && s.contains("卷"), "{s}");

    let s = dispatch::confirm_summary("proc_record_start", &json!({}));
    assert!(s.contains("录屏"), "{s}");
}

#[test]
fn test_confirm_summary_fallback() {
    let s = dispatch::confirm_summary("proc_kill", &json!({}));
    assert!(s.contains("proc_kill"), "缺参数走 fallback: {s}");
    let s = dispatch::confirm_summary("proc_mystery", &json!({ "a": 1 }));
    assert!(s.contains("proc_mystery"), "{s}");
}

#[test]
fn test_is_write_tool_matches_eight_names() {
    for name in dispatch::WRITE_TOOL_NAMES {
        assert!(dispatch::is_write_tool(name), "{name}");
    }
    assert!(!dispatch::is_write_tool("proc_ls"));
    assert!(!dispatch::is_write_tool("proc_help"));
    assert!(!dispatch::is_write_tool("proc_finish"));
}

#[test]
fn test_blocked_tool_result_shape() {
    let result = dispatch::blocked_tool_result(&call("proc_kill", json!({ "pid": 1 })));
    assert!(result.is_error);
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(false));
    assert_eq!(v["blocked"], json!(true));
    assert_eq!(v["tool"], json!("proc_kill"));
}

#[test]
fn test_execute_confirmed_tool_kill_missing_pid_is_business_error() {
    // Approved 真执行 + 不存在 PID：走 kill_process 业务路径（非 blocked）。
    let result = dispatch::execute_confirmed_tool(&call(
        "proc_kill",
        json!({ "pid": MISSING_PID, "confirm": true }),
    ));
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert!(
        v.get("blocked").is_none(),
        "真执行路径不应是 blocked: {}",
        result.content
    );
    assert!(
        !result.is_error,
        "业务层失败（pid 不存在）不是 dispatch 错误"
    );
}

#[test]
fn test_execute_confirmed_tool_missing_args_is_error() {
    let result = dispatch::execute_confirmed_tool(&call("proc_kill", json!({})));
    assert!(result.is_error, "参数缺失 → is_error=true");
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(false));
}

#[test]
fn test_execute_confirmed_tool_record_unsupported() {
    let result = dispatch::execute_confirmed_tool(&call(
        "proc_record_start",
        json!({ "file_path": "x.prec", "confirm": true }),
    ));
    assert!(!result.is_error);
    assert!(result.content.contains("不支持"), "{}", result.content);
}

// ===========================================================================
// D 组：AgentSession 端到端（spawn 真线程）
// ===========================================================================

fn spawn_session(provider: ScriptedStreamProvider) -> (Arc<ScriptedStreamProvider>, SessionHandle) {
    let provider = Arc::new(provider);
    let handle = AgentSession::spawn(
        Arc::clone(&provider) as Arc<dyn LlmProvider>,
        catalog::default_registry(),
        AgentOptions {
            max_steps: 4,
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

#[test]
fn test_session_single_query_event_sequence() {
    let (_p, handle) = spawn_session(ScriptedStreamProvider::new(vec![vec![
        text_delta("答"),
        end_turn(),
    ]]));

    assert!(handle.send_query("问"));
    let (events, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit, "应收到 SessionFinished: {events:?}");

    assert!(matches!(&events[0], SessionEvent::QueryStarted(q) if q == "问"));
    assert!(matches!(&events[1], SessionEvent::TextDelta(t) if t == "答"));
    assert!(matches!(events[2], SessionEvent::TurnFinished));
    match &events[3] {
        SessionEvent::SessionFinished { final_text, stop } => {
            assert_eq!(final_text, "答");
            assert_eq!(*stop, StopCause::EndTurn);
        }
        other => panic!("第 4 个事件应是 SessionFinished: {other:?}"),
    }
    handle.shutdown();
}

#[test]
fn test_session_multi_query_history_grows() {
    let (provider, handle) = spawn_session(ScriptedStreamProvider::new(vec![
        vec![text_delta("第一答"), end_turn()],
        finish_turn("第二答"),
    ]));

    assert!(handle.send_query("第一问"));
    let (_, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit);

    assert!(handle.send_query("第二问"));
    let (_, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit);

    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(seen.len(), 2, "两个 query 各 1 次 provider 调用");
    // seen[1] = system + user1 + assistant1 + user2（4 条）；script 第 2 轮
    // 直接 finish（无中间 tool 回填）。
    assert_eq!(seen[1].len(), 4, "system + 2 history + user query");
    assert_eq!(seen[1][1].content.as_deref(), Some("第一问"));
    assert_eq!(seen[1][2].content.as_deref(), Some("第一答"));
    handle.shutdown();
}

#[test]
fn test_session_drop_during_confirm_does_not_hang() {
    // 风险 2 关键测试：confirm 挂起（reply 不答复）时中断会话，全链路收尾
    // 不挂死——Drop = interrupt + Sender 断开，此处用等效显式链覆盖
    //（interrupt → SessionFinished(Interrupted) → shutdown → 线程退出）。
    let (_p, handle) = spawn_session(ScriptedStreamProvider::new(vec![vec![tool_delta(
        "proc_kill",
        json!({ "pid": 123 }),
    )]]));

    assert!(handle.send_query("杀掉它"));
    let (_, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::ConfirmRequested(_))
    });
    assert!(hit, "应收到 ConfirmRequested（不答复）");

    handle.interrupt();
    let (_, hit) = drain_until(&handle, Duration::from_secs(5), |ev| {
        matches!(
            ev,
            SessionEvent::SessionFinished {
                stop: StopCause::Interrupted,
                ..
            }
        )
    });
    assert!(hit, "cancel 后 run 应 Interrupted 收尾（不悬挂）");

    handle.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_exited() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(handle.is_exited(), "session 线程应已退出");
    drop(handle); // 已退出的线程 drop 立即返回
}

#[test]
fn test_session_interrupt_then_new_query_resets() {
    let (_p, handle) = spawn_session(ScriptedStreamProvider::new(vec![
        vec![text_delta("第一答"), end_turn()],
        vec![text_delta("第二答"), end_turn()],
    ]));

    assert!(handle.send_query("第一问"));
    let (_, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit);

    // idle 时置 cancel——下一个 query 应自动重置（cancel 只作用于当前 run）。
    handle.interrupt();
    assert!(handle.send_query("第二问"));
    let (events, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit, "新 query 应正常完成: {events:?}");
    assert!(events.iter().any(|ev| matches!(
        ev,
        SessionEvent::SessionFinished {
            stop: StopCause::EndTurn,
            ..
        }
    )));
    handle.shutdown();
}

#[test]
fn test_session_provider_error_emits_error_event() {
    let (_p, handle) = spawn_session(ScriptedStreamProvider::with_fail(vec![TurnScript::Fail(
        "provider 炸了".to_string(),
    )]));

    assert!(handle.send_query("q"));
    let (_, hit) = drain_until(
        &handle,
        Duration::from_secs(10),
        |ev| matches!(ev, SessionEvent::Error(msg) if msg.contains("provider 炸了")),
    );
    assert!(hit, "provider 错误应转 Error 事件");
    handle.shutdown();
}

#[test]
fn test_session_shutdown_exits_thread() {
    let (_p, handle) = spawn_session(ScriptedStreamProvider::new(vec![]));
    handle.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_exited() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(handle.is_exited(), "shutdown 后线程应退出");
    // events 通道随线程终结断开（drain 返回 None 即 Disconnected/Empty 均可）。
    assert!(handle.drain_event().is_none());
}

#[test]
fn test_session_empty_query_rejected() {
    let (provider, handle) = spawn_session(ScriptedStreamProvider::new(vec![]));

    assert!(handle.send_query("   "));
    let (_, hit) = drain_until(
        &handle,
        Duration::from_secs(10),
        |ev| matches!(ev, SessionEvent::Error(msg) if msg.contains("query")),
    );
    assert!(hit, "空白 query 应返 Error 事件");
    assert!(
        provider.seen_messages.lock().unwrap().is_empty(),
        "空白 query 不触达 provider"
    );
    handle.shutdown();
}

#[test]
fn test_truncate_history_sliding_window() {
    // 风险 4：15 轮（30 条）drain 到 12 轮（24 条），最旧被截。
    let mut history = Vec::new();
    for i in 0..15 {
        history.push(Message::new(Role::User, format!("问{i}")));
        history.push(Message::new(Role::Assistant, format!("答{i}")));
    }
    truncate_history(&mut history);
    assert_eq!(history.len(), MAX_HISTORY_TURNS * 2);
    assert_eq!(
        history[0].content.as_deref(),
        Some("问3"),
        "最旧 3 轮被截断"
    );
    assert_eq!(history.last().unwrap().content.as_deref(), Some("答14"));
}

#[test]
fn test_session_command_variants_constructible() {
    let _ = SessionCommand::Query("q".into());
    let _ = SessionCommand::Shutdown;
    assert_eq!(ConfirmDecision::Approved, ConfirmDecision::Approved);
    assert_ne!(ConfirmDecision::Approved, ConfirmDecision::Denied);
}

// ===========================================================================
// E 组：builder + 杂项
// ===========================================================================

#[test]
fn test_builder_mock_provider() {
    let (runner, spec) =
        proc::agent::builder::build_runner(Some("mock"), None, 5).expect("mock 档应可构造");
    drop(runner); // 仅验证构造成功（MockProvider 惰性索引不触达文件系统）
    assert_eq!(spec.name, "mock");
}

#[test]
fn test_builder_unknown_provider() {
    let err = match proc::agent::builder::build_runner(Some("bogus"), None, 5) {
        Err(e) => e,
        Ok(_) => panic!("未知 provider 应报错"),
    };
    assert!(err.contains("未知 provider"), "{err}");
}

#[test]
fn test_stop_cause_interrupted_label() {
    assert_eq!(StopCause::Interrupted.label(), "interrupted");
    assert_eq!(StopCause::EndTurn.label(), "end_turn");
}

#[test]
fn test_stage1_session_event_variants_still_constructible() {
    // stage-1 类型面回归锚：实装后无 oneshot 变体依旧可构造。
    let _ = SessionEvent::QueryStarted("q".into());
    let _ = SessionEvent::TextDelta("t".into());
    let _ = SessionEvent::ToolStart {
        name: "n".into(),
        arguments: json!({}),
    };
    let _ = SessionEvent::ToolFinished {
        name: "n".into(),
        is_error: false,
        result_chars: 1,
    };
    let _ = SessionEvent::TurnFinished;
    let _ = SessionEvent::SessionFinished {
        final_text: "f".into(),
        stop: StopCause::EndTurn,
    };
    let _ = SessionEvent::Error("e".into());
}

// ===========================================================================
// E2B 流式冒烟（#[ignore]，本机真实 llama-server，风险 3）
// ===========================================================================

const REAL_SERVER: &str = r"D:\llama.cpp\bin\llama-b8685-bin-win-cuda-12.4-x64\llama-server.exe";
const REAL_MODEL: &str = r"D:\llama.cpp\models\gemma4-e2b\gemma-4-E2B-it-Q4_K_M.gguf";

/// 真实 llama-server 流式冒烟（v0.20 验收全走 complete 非流式；stream +
/// tool_choice=required + proc_finish 组合下的 tool_calls 分片 / EndTurn 时机 /
/// proc_finish 行为本测试首验——brainstorm 风险 3）。
///
/// 断言宽松（E2B 能力边界不设线）：不 Err + 事件面有产出 + 最终文本非空。
#[tokio::test]
#[ignore = "需要本机 llama-server + GGUF 模型（真实推理，~30-90s）"]
async fn test_agent_v0_21_streaming_e2b_smoke() {
    let server = PathBuf::from(REAL_SERVER);
    let model = PathBuf::from(REAL_MODEL);
    if !server.is_file() || !model.is_file() {
        eprintln!("SKIP: 本机 llama-server / 模型不存在，流式冒烟跳过");
        return;
    }
    let provider = proc::agent::llama_cpp_provider::LlamaCppProvider::new(server, model);
    let runner = AgentRunner::new(
        Arc::new(provider),
        catalog::default_registry(),
        AgentOptions {
            max_steps: 6,
            ..Default::default()
        },
    );
    let cancel = Arc::new(AtomicBool::new(false));

    // query 1：tool 轮（proc_ls）→ proc_finish——required + 流式组合主路径。
    let events: Arc<Mutex<Vec<StreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = Arc::clone(&events);
        move |ev: StreamEvent| {
            events.lock().unwrap().push(ev);
        }
    };
    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        runner.run_streaming(
            "列出当前 CPU 占用最高的 3 个进程的名称和占用率",
            &[],
            &sink,
            None,
            &cancel,
        ),
    )
    .await
    .expect("流式 run 不应超时")
    .expect("流式 run 不应 Err");
    eprintln!(
        "e2b smoke q1: stop={:?} steps={} final={:?}",
        outcome.stop,
        outcome.steps.len(),
        outcome.final_text
    );
    {
        let events = events.lock().unwrap();
        let has_text = events
            .iter()
            .any(|e| matches!(e, StreamEvent::TextDelta(_)));
        let has_turn = events
            .iter()
            .any(|e| matches!(e, StreamEvent::TurnFinished));
        eprintln!(
            "e2b smoke q1 events: {} 条（text_delta={has_text} turn_finished={has_turn}）",
            events.len()
        );
    }
    assert!(!outcome.final_text.trim().is_empty(), "最终回答非空");

    // query 2：多轮 history（第二轮能看到第一轮问答）。
    let history = vec![
        Message::new(Role::User, "列出当前 CPU 占用最高的 3 个进程的名称和占用率"),
        Message::new(Role::Assistant, outcome.final_text.clone()),
    ];
    let outcome2 = tokio::time::timeout(
        Duration::from_secs(180),
        runner.run_streaming(
            "你刚才提到的第一个进程是什么？只回答进程名",
            &history,
            &sink,
            None,
            &cancel,
        ),
    )
    .await
    .expect("第二轮流式 run 不应超时")
    .expect("第二轮流式 run 不应 Err");
    eprintln!(
        "e2b smoke q2: stop={:?} final={:?}",
        outcome2.stop, outcome2.final_text
    );
    assert!(!outcome2.final_text.trim().is_empty(), "多轮回答非空");
}
