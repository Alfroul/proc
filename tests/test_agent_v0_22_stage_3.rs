//! v0.22 stage 3 测试：session observability（ADR-0032 D5）。
//!
//! - A 组：SessionRecorder（tmpdir 真文件——seq 单调 / ts 非降 / delta 聚合 / 8 变体映射）
//! - B 组：analyze_entries 指标提取（TTFT / 生成时长 / tool / confirm 决策延迟）
//! - C 组：AgentSession 端到端（recorder 接线 + confirm 决策旁路记录 + E2B 冒烟 #[ignore]）
//! - D 组：CLI session-info / format_session_metrics / config [session]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use clap::Parser;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use proc::agent::session::ConfirmRequest;
use proc::agent::session_log::{
    DELTA_MERGE_CHARS, LogEvent, SessionLogEntry, SessionRecorder, analyze_entries,
    analyze_session_log, format_session_metrics,
};
use proc::agent::tools::catalog;
use proc::agent::types::Message;
use proc::agent::{
    AgentOptions, AgentSession, CompleteOptions, CompleteResponse, ConfirmDecision, Delta,
    LlmError, LlmProvider, ProviderStream, SessionEvent, StopCause, StopReason, ToolCall,
};

// ===========================================================================
// ScriptedStreamProvider（与 test_agent_v0_21_stage_2.rs 同款最小副本）
// ===========================================================================

struct ScriptedStreamProvider {
    turns: std::sync::Mutex<std::collections::VecDeque<Vec<Delta>>>,
}

impl ScriptedStreamProvider {
    fn new(turns: Vec<Vec<Delta>>) -> Self {
        Self {
            turns: std::sync::Mutex::new(turns.into()),
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

fn spawn_session_with_recorder(
    provider: ScriptedStreamProvider,
    recorder: SessionRecorder,
) -> proc::agent::SessionHandle {
    AgentSession::spawn(
        Arc::new(provider),
        catalog::default_registry(),
        AgentOptions {
            max_steps: 6,
            ..Default::default()
        },
        recorder,
    )
}

fn drain_until(
    handle: &proc::agent::SessionHandle,
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

// ===========================================================================
// A 组：SessionRecorder
// ===========================================================================

/// 读 tmpdir 下唯一 JSONL 文件 → 解析后的 entries。
fn read_entries(dir: &Path) -> Vec<SessionLogEntry> {
    let files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .collect();
    assert_eq!(files.len(), 1, "sessions 目录应恰有 1 个 .jsonl");
    let content = std::fs::read_to_string(files[0].path()).unwrap();
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// LogEvent 的 serde tag（kind 字符串）。
fn kind_of(e: &SessionLogEntry) -> String {
    serde_json::to_value(&e.event).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn test_recorder_disabled_noop() {
    let rec = SessionRecorder::disabled();
    assert!(!rec.is_enabled());
    // 全部 log 调用 no-op 不 panic（含 delta 聚合路径）。
    rec.log(&SessionEvent::QueryStarted("q".to_string()));
    for _ in 0..100 {
        rec.log(&SessionEvent::TextDelta("abcd".to_string()));
    }
    rec.log_confirm_decision(false);
    rec.log(&SessionEvent::Error("e".to_string()));
}

#[test]
fn test_recorder_creates_header_entry() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "llama-cpp");
    assert!(rec.is_enabled());
    rec.log(&SessionEvent::QueryStarted("q".to_string()));
    let entries = read_entries(dir.path());
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].seq, 0);
    match &entries[0].event {
        LogEvent::SessionStart {
            provider,
            wall_start,
        } => {
            assert_eq!(provider, "llama-cpp");
            assert!(
                wall_start.ends_with('Z'),
                "墙钟起点是 ISO UTC: {wall_start}"
            );
        }
        other => panic!("seq 0 应是 SessionStart: {other:?}"),
    }
}

#[test]
fn test_recorder_seq_monotonic_ts_nondecreasing() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");
    rec.log(&SessionEvent::QueryStarted("问".to_string()));
    for i in 0..20 {
        rec.log(&SessionEvent::TextDelta(format!("片段{i}")));
        if i % 5 == 0 {
            rec.log(&SessionEvent::ToolStart {
                name: "proc_ls".to_string(),
                arguments: json!({}),
            });
        }
    }
    rec.log(&SessionEvent::SessionFinished {
        final_text: "答".to_string(),
        stop: StopCause::EndTurn,
    });
    let entries = read_entries(dir.path());
    for w in entries.windows(2) {
        assert!(w[0].seq < w[1].seq, "seq 严格递增: {:?} {:?}", w[0], w[1]);
        assert!(
            w[0].ts_rel_ms <= w[1].ts_rel_ms,
            "ts_rel_ms 非降: {:?} {:?}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn test_recorder_delta_coalescing_bound() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");
    rec.log(&SessionEvent::QueryStarted("q".to_string()));
    for _ in 0..200 {
        rec.log(&SessionEvent::TextDelta("四字片段".to_string())); // 4 chars × 200
    }
    rec.log(&SessionEvent::SessionFinished {
        final_text: "答".to_string(),
        stop: StopCause::EndTurn,
    });
    let entries = read_entries(dir.path());
    let delta_entries: Vec<&SessionLogEntry> = entries
        .iter()
        .filter(|e| matches!(e.event, LogEvent::TextDelta { .. }))
        .collect();
    let total_chars: usize = delta_entries
        .iter()
        .map(|e| match e.event {
            LogEvent::TextDelta { chars } => chars,
            _ => 0,
        })
        .sum();
    assert_eq!(total_chars, 800, "chars 全量保留");
    let bound = (800 / DELTA_MERGE_CHARS) + 2; // 聚合段 + 尾部 flush 余量
    assert!(
        delta_entries.len() <= bound && delta_entries.len() < 200,
        "delta 条目 {} 应 ≤ {bound}（不逐 delta 落盘）",
        delta_entries.len()
    );
}

#[test]
fn test_recorder_flush_pending_before_next_event() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");
    rec.log(&SessionEvent::QueryStarted("q".to_string()));
    rec.log(&SessionEvent::TextDelta("一".to_string()));
    rec.log(&SessionEvent::TextDelta("二".to_string()));
    rec.log(&SessionEvent::TextDelta("三".to_string())); // 3 chars < 64 → pending
    rec.log(&SessionEvent::ToolStart {
        name: "proc_ls".to_string(),
        arguments: json!({}),
    });
    let entries = read_entries(dir.path());
    // 顺序：session_start → query_started → text_delta(3 chars) → tool_start
    assert_eq!(entries.len(), 4);
    match &entries[2].event {
        LogEvent::TextDelta { chars } => assert_eq!(*chars, 3, "pending 段在下一事件前 flush"),
        other => panic!("第 3 条应是聚合 TextDelta: {other:?}"),
    }
    assert!(matches!(&entries[3].event, LogEvent::ToolStart { name } if name == "proc_ls"));
}

#[test]
fn test_recorder_summary_mapping_all_variants() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");

    let (reply_tx, _reply_rx) = oneshot::channel();
    rec.log(&SessionEvent::ConfirmRequested(ConfirmRequest {
        tool_name: "proc_kill".to_string(),
        arguments: json!({ "pid": 123 }),
        summary: "结束 PID 123".to_string(),
        reply: reply_tx,
    }));
    rec.log(&SessionEvent::TextDelta("abcdef".to_string()));
    rec.log(&SessionEvent::ToolFinished {
        name: "proc_kill".to_string(),
        is_error: true,
        result_chars: 42,
    });
    rec.log(&SessionEvent::TurnFinished);
    rec.log(&SessionEvent::Error("网络错误".to_string()));
    rec.log_confirm_decision(true);
    rec.log(&SessionEvent::SessionFinished {
        final_text: "最终答案".to_string(),
        stop: StopCause::EndTurn,
    });

    let entries = read_entries(dir.path());
    let kinds: Vec<String> = entries.iter().map(kind_of).collect();
    assert_eq!(
        kinds,
        vec![
            "session_start",
            "confirm_requested",
            "text_delta",
            "tool_finished",
            "turn_finished",
            "error",
            "confirm_decision",
            "session_finished",
        ]
    );
    match &entries[1].event {
        LogEvent::ConfirmRequested { tool_name } => assert_eq!(tool_name, "proc_kill"),
        other => panic!("{other:?}"),
    }
    match &entries[3].event {
        LogEvent::ToolFinished {
            name,
            is_error,
            result_chars,
        } => {
            assert_eq!(name, "proc_kill");
            assert!(*is_error);
            assert_eq!(*result_chars, 42);
        }
        other => panic!("{other:?}"),
    }
    match &entries[6].event {
        LogEvent::ConfirmDecision { approved } => assert!(*approved),
        other => panic!("{other:?}"),
    }
    match &entries[7].event {
        LogEvent::SessionFinished {
            stop,
            final_chars,
            final_head,
        } => {
            assert_eq!(stop, "end_turn");
            assert_eq!(*final_chars, 4);
            assert_eq!(final_head, "最终答案");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_recorder_silent_degrade_on_unwritable_dir() {
    // 目录创建失败 → 静默降级（disabled 语义，不 panic 不影响会话）。
    let rec = SessionRecorder::start_in_dir(Path::new("Z:/definitely/not/writable"), "mock");
    assert!(!rec.is_enabled());
    rec.log(&SessionEvent::QueryStarted("q".to_string()));
}

// ===========================================================================
// B 组：analyze_entries 指标提取
// ===========================================================================

fn entry(seq: u64, ts: u64, event: LogEvent) -> SessionLogEntry {
    SessionLogEntry {
        seq,
        ts_rel_ms: ts,
        event,
    }
}

#[test]
fn test_analyze_ttft_and_duration() {
    let entries = vec![
        entry(
            0,
            0,
            LogEvent::SessionStart {
                provider: "llama-cpp".into(),
                wall_start: "2026-08-21T02:00:00Z".into(),
            },
        ),
        entry(1, 1_000, LogEvent::QueryStarted { text: "问".into() }),
        entry(2, 2_500, LogEvent::TextDelta { chars: 30 }),
        entry(3, 3_000, LogEvent::TextDelta { chars: 50 }),
        entry(4, 4_000, LogEvent::TurnFinished),
        entry(
            5,
            8_000,
            LogEvent::SessionFinished {
                stop: "end_turn".into(),
                final_chars: 80,
                final_head: "答".into(),
            },
        ),
    ];
    let m = analyze_entries(&entries);
    assert_eq!(m.provider, "llama-cpp");
    assert_eq!(m.total_ms, 8_000);
    assert_eq!(m.queries.len(), 1);
    let q = &m.queries[0];
    assert_eq!(
        q.ttft_ms,
        Some(1_500),
        "TTFT = 首段 delta 2500 − query 1000"
    );
    assert_eq!(q.duration_ms, Some(7_000), "生成时长 = 8000 − 1000");
    assert_eq!(q.delta_events, 2);
    assert_eq!(q.delta_chars, 80);
    assert_eq!(q.stop.as_deref(), Some("end_turn"));
    assert_eq!(m.totals.answered, 1);
    assert_eq!(m.totals.ttft_avg_ms, Some(1_500));
    assert_eq!(m.totals.ttft_max_ms, Some(1_500));
    assert_eq!(m.totals.generation_avg_ms, Some(7_000));
}

#[test]
fn test_analyze_tools_and_confirm_behavior() {
    let entries = vec![
        entry(
            0,
            0,
            LogEvent::SessionStart {
                provider: "mock".into(),
                wall_start: "w".into(),
            },
        ),
        entry(
            1,
            100,
            LogEvent::QueryStarted {
                text: "杀进程".into(),
            },
        ),
        entry(
            2,
            200,
            LogEvent::ToolStart {
                name: "proc_kill".into(),
            },
        ),
        entry(
            3,
            300,
            LogEvent::ConfirmRequested {
                tool_name: "proc_kill".into(),
            },
        ),
        entry(4, 2_800, LogEvent::ConfirmDecision { approved: false }),
        entry(
            5,
            3_000,
            LogEvent::ToolFinished {
                name: "proc_kill".into(),
                is_error: false,
                result_chars: 120,
            },
        ),
        entry(
            6,
            3_200,
            LogEvent::ToolStart {
                name: "proc_ls".into(),
            },
        ),
        entry(
            7,
            3_500,
            LogEvent::ToolFinished {
                name: "proc_ls".into(),
                is_error: true,
                result_chars: 10,
            },
        ),
        entry(8, 4_000, LogEvent::TurnFinished),
        entry(
            9,
            5_000,
            LogEvent::SessionFinished {
                stop: "end_turn".into(),
                final_chars: 5,
                final_head: "已拒绝".into(),
            },
        ),
    ];
    let m = analyze_entries(&entries);
    let q = &m.queries[0];
    assert_eq!(q.tool_calls, 2);
    assert_eq!(q.tool_errors, 1);
    assert_eq!(q.turns, 1);
    assert_eq!(q.confirms, 1);
    assert_eq!(
        q.confirm_decision_max_ms,
        Some(2_500),
        "决策延迟 2800 − 300"
    );
    assert_eq!(m.totals.tool_calls, 2);
    assert_eq!(m.totals.tool_errors, 1);
    assert_eq!(m.totals.confirms, 1);
    assert_eq!(m.totals.confirm_denied, 1);
    assert_eq!(m.totals.confirm_approved, 0);
    assert_eq!(m.totals.confirm_decision_avg_ms, Some(2_500));
}

#[test]
fn test_analyze_multi_query_and_decision_distribution() {
    let entries = vec![
        entry(
            0,
            0,
            LogEvent::SessionStart {
                provider: "mock".into(),
                wall_start: "w".into(),
            },
        ),
        // query 1：无流式输出（ttft None），1000ms 完成
        entry(1, 0, LogEvent::QueryStarted { text: "q1".into() }),
        entry(
            2,
            1_000,
            LogEvent::SessionFinished {
                stop: "end_turn".into(),
                final_chars: 1,
                final_head: "a1".into(),
            },
        ),
        // query 2：ttft 500 / 时长 3000 / confirm approved 延迟 1000
        entry(3, 5_000, LogEvent::QueryStarted { text: "q2".into() }),
        entry(4, 5_500, LogEvent::TextDelta { chars: 10 }),
        entry(
            5,
            6_000,
            LogEvent::ConfirmRequested {
                tool_name: "t".into(),
            },
        ),
        entry(6, 7_000, LogEvent::ConfirmDecision { approved: true }),
        entry(
            7,
            8_000,
            LogEvent::SessionFinished {
                stop: "interrupted".into(),
                final_chars: 1,
                final_head: "a2".into(),
            },
        ),
    ];
    let m = analyze_entries(&entries);
    assert_eq!(m.queries.len(), 2);
    assert_eq!(m.queries[0].index, 1);
    assert_eq!(m.queries[1].index, 2);
    assert_eq!(m.queries[0].ttft_ms, None);
    assert_eq!(m.queries[1].ttft_ms, Some(500));
    assert_eq!(m.queries[1].duration_ms, Some(3_000));
    assert_eq!(m.totals.answered, 2);
    // ttft 平均只含有值的 query：500 / 1
    assert_eq!(m.totals.ttft_avg_ms, Some(500));
    // 生成时长平均：(1000 + 3000) / 2
    assert_eq!(m.totals.generation_avg_ms, Some(2_000));
    assert_eq!(m.totals.confirm_approved, 1);
    assert_eq!(m.totals.confirm_decision_avg_ms, Some(1_000));
    assert_eq!(m.total_ms, 8_000);
}

#[test]
fn test_analyze_out_of_scope_error_only_totals() {
    // 空 query 的 Error 在 QueryStarted 之前——只进 totals 不造 query。
    let entries = vec![
        entry(
            0,
            0,
            LogEvent::SessionStart {
                provider: "mock".into(),
                wall_start: "w".into(),
            },
        ),
        entry(
            1,
            5,
            LogEvent::Error {
                message: "query 不能为空".into(),
            },
        ),
    ];
    let m = analyze_entries(&entries);
    assert!(m.queries.is_empty());
    assert_eq!(m.totals.errors, 1);
    assert_eq!(m.totals.queries, 0);
}

#[test]
fn test_analyze_query_error_keeps_open_query() {
    // LLM 错误路径：QueryStarted 后 Error（无 SessionFinished）——query 记录
    // error 但 duration None。
    let entries = vec![
        entry(0, 0, LogEvent::QueryStarted { text: "q".into() }),
        entry(
            1,
            500,
            LogEvent::Error {
                message: "连接失败".into(),
            },
        ),
    ];
    let m = analyze_entries(&entries);
    assert_eq!(m.queries.len(), 1);
    assert_eq!(m.queries[0].duration_ms, None);
    assert_eq!(m.queries[0].error.as_deref(), Some("连接失败"));
    assert_eq!(m.totals.errors, 1);
    assert_eq!(m.totals.answered, 0);
}

#[test]
fn test_analyze_file_errors() {
    let err = analyze_session_log(Path::new("Z:/no/such/file.jsonl"));
    assert!(err.is_err());

    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.jsonl");
    let valid = serde_json::json!({
        "seq": 0,
        "ts_rel_ms": 0,
        "kind": "session_start",
        "provider": "mock",
        "wall_start": "w",
    });
    std::fs::write(&bad, format!("{valid}\nnot json at all\n")).unwrap();
    let err = analyze_session_log(&bad).unwrap_err();
    assert!(err.contains("第 2 行"), "坏行报行号: {err}");
}

#[test]
fn test_analyze_file_roundtrip_from_recorder() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");
    rec.log(&SessionEvent::QueryStarted("问".to_string()));
    rec.log(&SessionEvent::TextDelta("答案内容".to_string()));
    rec.log(&SessionEvent::SessionFinished {
        final_text: "答案内容".to_string(),
        stop: StopCause::EndTurn,
    });
    let file = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .unwrap();
    let m = analyze_session_log(&file).unwrap();
    assert_eq!(m.provider, "mock");
    assert_eq!(m.totals.queries, 1);
    assert_eq!(m.queries[0].delta_chars, 4);
}

// ===========================================================================
// C 组：AgentSession 端到端（recorder 接线）
// ===========================================================================

#[test]
fn test_session_records_full_log_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "scripted");
    let handle = spawn_session_with_recorder(
        ScriptedStreamProvider::new(vec![vec![text_delta("中间"), end_turn()]]),
        rec.clone(),
    );

    assert!(handle.send_query("问"));
    let (_, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit);
    handle.shutdown();

    let entries = read_entries(dir.path());
    let kinds: Vec<String> = entries.iter().map(kind_of).collect();
    assert_eq!(
        kinds,
        vec![
            "session_start",
            "query_started",
            "text_delta",
            "turn_finished",
            "session_finished",
        ],
        "完整生命周期留档"
    );
    match &entries[1].event {
        LogEvent::QueryStarted { text } => assert_eq!(text, "问"),
        other => panic!("{other:?}"),
    }
    match &entries[4].event {
        LogEvent::SessionFinished { stop, .. } => assert_eq!(stop, "end_turn"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_session_confirm_decision_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "scripted");
    let handle = spawn_session_with_recorder(
        ScriptedStreamProvider::new(vec![
            // turn 1：写 tool（confirm hook）→ 拒绝 → turn 2 finish
            vec![
                tool_delta("proc_kill", json!({ "pid": 4000000 })),
                end_turn(),
            ],
            finish_turn("已拒绝并解释"),
        ]),
        rec.clone(),
    );

    assert!(handle.send_query("结束 PID 4000000"));
    // 等确认请求。reply 是 session 层换出后的包装通道——对调用方透明，
    // 决策经包装线程记录 LogEvent::ConfirmDecision 再转发 runner。
    let (events, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::ConfirmRequested(_))
    });
    assert!(hit, "应收到 ConfirmRequested: {events:?}");
    let reply = events
        .into_iter()
        .find_map(|ev| match ev {
            SessionEvent::ConfirmRequested(req) => Some(req.reply),
            _ => None,
        })
        .expect("ConfirmRequest 持 reply");
    reply.send(ConfirmDecision::Denied).expect("send denied");

    let (_, hit) = drain_until(&handle, Duration::from_secs(10), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit, "拒绝后 run 应继续到 SessionFinished");
    handle.shutdown();

    let entries = read_entries(dir.path());
    let decision = entries
        .iter()
        .position(|e| matches!(e.event, LogEvent::ConfirmDecision { .. }))
        .expect("应有 confirm_decision 条目");
    match &entries[decision].event {
        LogEvent::ConfirmDecision { approved } => assert!(!approved),
        other => panic!("{other:?}"),
    }
    // 决策条目在 session_finished 之前（先记录后转发保证日志序）
    let finished = entries
        .iter()
        .position(|e| matches!(e.event, LogEvent::SessionFinished { .. }))
        .expect("应有 session_finished");
    assert!(decision < finished);

    let file = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .unwrap();
    let m = analyze_session_log(&file).unwrap();
    assert_eq!(m.totals.confirms, 1);
    assert_eq!(m.totals.confirm_denied, 1);
    assert!(
        m.totals.confirm_decision_avg_ms.is_some(),
        "决策延迟应可提取: {:?}",
        m.totals
    );
}

// ===========================================================================
// D 组：CLI / config / 格式化
// ===========================================================================

#[test]
fn test_cli_parse_session_info() {
    let cli = proc::cli::Cli::try_parse_from(["proc", "agent", "session-info", "s.jsonl"]).unwrap();
    match cli.command {
        Some(proc::cli::Command::Agent {
            sub: proc::cli::def::AgentSub::SessionInfo { ref path },
        }) => assert_eq!(path, "s.jsonl"),
        other => panic!("expected Agent(SessionInfo), got {other:?}"),
    }
}

#[test]
fn test_format_session_metrics_output() {
    let entries = vec![
        entry(
            0,
            0,
            LogEvent::SessionStart {
                provider: "llama-cpp".into(),
                wall_start: "2026-08-21T02:00:00Z".into(),
            },
        ),
        entry(
            1,
            1_000,
            LogEvent::QueryStarted {
                text: "列出 top 3".into(),
            },
        ),
        entry(2, 2_500, LogEvent::TextDelta { chars: 30 }),
        entry(
            3,
            4_000,
            LogEvent::ToolStart {
                name: "proc_ls".into(),
            },
        ),
        entry(
            4,
            4_500,
            LogEvent::ToolFinished {
                name: "proc_ls".into(),
                is_error: false,
                result_chars: 800,
            },
        ),
        entry(
            5,
            5_000,
            LogEvent::ConfirmRequested {
                tool_name: "proc_kill".into(),
            },
        ),
        entry(6, 7_000, LogEvent::ConfirmDecision { approved: false }),
        entry(
            7,
            9_000,
            LogEvent::SessionFinished {
                stop: "end_turn".into(),
                final_chars: 100,
                final_head: "前三个是…".into(),
            },
        ),
    ];
    let m = analyze_entries(&entries);
    let out = format_session_metrics(&m);
    assert!(out.contains("session 观测: llama-cpp"));
    assert!(out.contains("TTFT"));
    assert!(out.contains("1.5s"), "TTFT 1.5s 打印: {out}");
    assert!(out.contains("confirm    1（approved 0 / denied 1）"));
    assert!(out.contains("决策延迟 avg 2.0s"));
    assert!(out.contains("per-query"));
    assert!(out.contains("列出 top 3"));
}

#[test]
fn test_config_session_section() {
    use proc::agent::config::AgentConfig;

    // 默认 true
    assert!(AgentConfig::from_toml("").unwrap().session.log);
    // 显式 false
    let cfg = AgentConfig::from_toml("[session]\nlog = false\n").unwrap();
    assert!(!cfg.session.log);
    // 未知键拒绝（deny_unknown_fields）
    assert!(AgentConfig::from_toml("[session]\nlogg = true\n").is_err());
}

#[test]
fn test_run_agent_session_info_paths() {
    // 有效文件 → Ok（打印 stdout）
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");
    rec.log(&SessionEvent::QueryStarted("q".to_string()));
    rec.log(&SessionEvent::SessionFinished {
        final_text: "a".to_string(),
        stop: StopCause::EndTurn,
    });
    let file = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .unwrap();
    let sub = proc::cli::def::AgentSub::SessionInfo {
        path: file.to_string_lossy().into_owned(),
    };
    proc::cli::agent_cmd::run_agent(&sub);
}

#[test]
fn test_run_agent_session_info_missing_file_exits() {
    // run_agent 对 Err 走 exit(1)——不能在测试进程里直接调；用内部函数语义
    // 已由 test_analyze_file_errors 覆盖（analyze Err → CLI 报错退出）。
    // 这里验证 run_agent_eval 等既有路径不受影响：session-info 缺失文件
    // 的行为由 analyze_session_log 的 Err 保证。
    assert!(analyze_session_log(Path::new("Z:/no/such/file.jsonl")).is_err());
}

// ===========================================================================
// E2B 真实冒烟（#[ignore]——需要本机 llama-server + gemma-4-E2B）
// ===========================================================================

/// 真实链路 session log 冒烟：build_session（默认 agent.toml → llama-cpp E2B）
/// → 1 query → 真实 `~/.config/proc/sessions/` 出 JSONL → analyze TTFT Some。
/// 注意：会向真实 sessions 目录写一个日志文件（功能本体，非测试污染）。
#[test]
#[ignore = "E2B 真实链路：需 llama-server + gemma-4-E2B（~1-2 min）"]
fn test_e2b_session_log_smoke() {
    let before = std::time::SystemTime::now();
    let (handle, _spec) = proc::agent::build_session(None, None, 6).expect("build_session");

    assert!(handle.send_query("列出当前 CPU 占用最高的 3 个进程"));
    let (_, hit) = drain_until(&handle, Duration::from_secs(300), |ev| {
        matches!(ev, SessionEvent::SessionFinished { .. })
    });
    assert!(hit, "E2B 应完成 query");

    handle.shutdown();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !handle.is_exited() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        handle.is_exited(),
        "session 线程应退出（决策转发线程无残留）"
    );

    // 找 start 之后新建的 session 文件
    let dir = proc::dirs_config_dir().join("sessions");
    let found: Vec<_> = std::fs::read_dir(&dir)
        .expect("sessions 目录存在")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "jsonl")
                && std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .map(|t| t > before)
                    .unwrap_or(false)
        })
        .collect();
    assert!(!found.is_empty(), "真实 sessions 目录应有新 JSONL");
    let m = analyze_session_log(&found[0]).unwrap();
    assert_eq!(m.provider, "llama-cpp");
    assert_eq!(m.totals.queries, 1);
    let q = &m.queries[0];
    assert!(q.duration_ms.is_some(), "query 应完成: {q:?}");
    // 实测 E2B 对该 query 可能直接 tool → proc_finish（无流式文本）——
    // TTFT 只在有 TextDelta 时存在（一致性而非存在性断言）。
    assert!(
        q.delta_events == 0 || q.ttft_ms.is_some(),
        "有流式输出则必有 TTFT: {q:?}"
    );
}
