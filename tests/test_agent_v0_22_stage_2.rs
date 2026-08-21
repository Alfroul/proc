//! v0.22 stage 2 测试：eval runner 执行循环（分类接线 / attempts 重试 / 单 query
//! LlmError 不中断）+ query 选择 / 聚合双口径 / markdown 报告 / compare / JSON
//! roundtrip + MockProvider seed 全管线。
//!
//! ScriptedProvider 模式取自 `tests/test_agent_v0_20_stage_3b.rs`（顺序弹出，
//! `None` = 注定 Err——驱动 LlmError 分类与重试语义）。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};

use proc::agent::eval::{
    self, EvalReport, EvalRunFile, EvalRunMeta, FailureMode, LevelSummary, QueryResult, QuerySpec,
    build_report, parse_levels, run_eval, select_queries,
};
use proc::agent::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
    Usage,
};
use proc::agent::runner::{AgentOptions, AgentRunner};
use proc::agent::types::{Message, Role, ToolCall};

// ===========================================================================
// ScriptedProvider（stage_3b 模式 + None = Err 注入）
// ===========================================================================

struct ScriptedProvider {
    /// `None` 元素 = 该次 complete 注定返 Err（LlmError 注入）。
    responses: Mutex<VecDeque<Option<CompleteResponse>>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<Option<CompleteResponse>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(None)
            .ok_or(LlmError::StreamEnded)
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        futures_util::stream::empty::<Result<Delta, LlmError>>().boxed()
    }
}

// ===========================================================================
// 响应构造 helper（stage_3b 同款）
// ===========================================================================

fn text_resp(text: &str) -> CompleteResponse {
    CompleteResponse {
        message: Message::new(Role::Assistant, text),
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
    }
}

fn tool_resp(name: &str, args: Value) -> CompleteResponse {
    CompleteResponse {
        message: Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![call(name, args)],
            tool_results: Vec::new(),
        },
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
    }
}

fn finish_resp(answer: &str) -> CompleteResponse {
    tool_resp("proc_finish", json!({ "answer": answer }))
}

fn empty_resp() -> Option<CompleteResponse> {
    Some(CompleteResponse {
        message: Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
    })
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments: args,
    }
}

fn scripted_runner(script: Vec<Option<CompleteResponse>>, max_steps: u32) -> AgentRunner {
    AgentRunner::new(
        Arc::new(ScriptedProvider::new(script)),
        proc::agent::tools::catalog::default_registry(),
        AgentOptions {
            max_steps,
            ..Default::default()
        },
    )
}

fn spec(scenario: &str, level: u8, text: &str, expected: &[&str]) -> QuerySpec {
    QuerySpec {
        scenario: scenario.to_string(),
        level,
        text: text.to_string(),
        expected_tools: expected.iter().map(|s| s.to_string()).collect(),
    }
}

async fn run_one_eval(runner: &AgentRunner, q: &QuerySpec, attempts: u8) -> QueryResult {
    let mut results = run_eval(runner, std::slice::from_ref(q), attempts, &mut |_, _, _| {}).await;
    results.pop().expect("单 query 应有 1 条结果")
}

// ===========================================================================
// 分类接线（ScriptedProvider 驱动 run_eval，8 失败变体 + 重试语义）
// ===========================================================================

#[tokio::test]
async fn test_eval_pass_single_tool() {
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_ls", json!({ "limit": 5 }))),
            Some(finish_resp("已列出 CPU 占用最高的进程。")),
        ],
        10,
    );
    let q = spec(
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        &["proc_ls"],
    );
    let r = run_one_eval(&runner, &q, 2).await;
    assert!(r.passed);
    assert_eq!(r.failure_mode, FailureMode::Pass);
    assert_eq!(r.actual_tools, vec!["proc_ls".to_string()]);
    assert_eq!(r.attempts_used, 1);
    assert_eq!(r.stop_cause, "end_turn");
    assert!(r.duration_ms <= 60_000);
}

#[tokio::test]
async fn test_eval_no_tool_call() {
    let runner = scripted_runner(vec![Some(text_resp("直接文字回答，不调工具。"))], 10);
    let q = spec("performance-diagnose", 0, "系统正常吗", &["proc_ls"]);
    let r = run_one_eval(&runner, &q, 1).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::NoToolCall);
    assert!(r.actual_tools.is_empty());
}

#[tokio::test]
async fn test_eval_wrong_tool() {
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_metrics_system", json!({}))),
            Some(finish_resp("已查看系统指标。")),
        ],
        10,
    );
    let q = spec(
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        &["proc_ls"],
    );
    let r = run_one_eval(&runner, &q, 1).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::WrongTool);
    assert_eq!(r.chain_steps_hit, 0);
}

#[tokio::test]
async fn test_eval_chain_incomplete() {
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_help", json!({ "category": "usb" }))),
            Some(finish_resp("已查询 USB 工具。")),
        ],
        10,
    );
    let q = spec(
        "usb",
        2,
        "安全弹出 E 盘 U 盘",
        &["proc_help", "proc_usb_release"],
    );
    let r = run_one_eval(&runner, &q, 1).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::ChainIncomplete);
    assert_eq!(r.chain_steps_hit, 1);
    assert_eq!(r.expected_tools.len(), 2);
}

#[tokio::test]
async fn test_eval_empty_answer() {
    // 空响应 nudge 后仍空 → EmptyAfterRetry → nudge_fallback → EmptyAnswer
    let runner = scripted_runner(vec![empty_resp(), empty_resp()], 10);
    let q = spec("performance-diagnose", 0, "系统正常吗", &["proc_ls"]);
    let r = run_one_eval(&runner, &q, 1).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::EmptyAnswer);
    assert_eq!(r.stop_cause, "empty_after_retry");
}

#[tokio::test]
async fn test_eval_max_steps() {
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_ls", json!({}))),
            Some(tool_resp("proc_ls", json!({}))),
        ],
        2,
    );
    let q = spec(
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        &["proc_ls"],
    );
    let r = run_one_eval(&runner, &q, 1).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::MaxSteps);
    assert_eq!(r.stop_cause, "max_steps");
}

#[test]
fn test_classify_max_steps_synthetic_summary_not_degraded() {
    // stage 2 实测修正（mock CLI 冒烟发现）：max_steps 的兜底文案是 runner
    // 合成的重复 tool 名列表（触发重复检测），归 MaxSteps 而非 OutputDegraded
    let tools = vec!["proc_ls".to_string(); 10];
    let expected = vec!["proc_ls".to_string()];
    let text = format!(
        "已达到最大步数（10），未能生成最终总结。已执行 tool：{}",
        ["proc_ls"; 10].join(", ")
    );
    // 前提：该文本确实会被退化检测命中（排序才有意义）
    assert!(eval::is_degraded_output(&text));

    let summary = eval::OutcomeSummary {
        final_text: &text,
        actual_tools: &tools,
        stop_cause: "max_steps",
        llm_error: false,
        nudge_fallback: false,
    };
    assert_eq!(
        eval::classify_failure(&summary, &expected),
        FailureMode::MaxSteps
    );

    // 反证：同文本走 end_turn（模型产出）仍是 OutputDegraded（优先级不变）
    let summary = eval::OutcomeSummary {
        final_text: &text,
        actual_tools: &tools,
        stop_cause: "end_turn",
        llm_error: false,
        nudge_fallback: false,
    };
    assert_eq!(
        eval::classify_failure(&summary, &expected),
        FailureMode::OutputDegraded
    );
}

#[tokio::test]
async fn test_eval_llm_error() {
    // 脚本为空 → 每次 complete 都 Err；attempts=2 → 用尽记 LlmError
    let runner = scripted_runner(vec![], 10);
    let q = spec(
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        &["proc_ls"],
    );
    let r = run_one_eval(&runner, &q, 2).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::LlmError);
    assert_eq!(r.attempts_used, 2);
    assert!(r.final_text_head.starts_with("LLM error:"));
}

#[tokio::test]
async fn test_eval_llm_error_does_not_abort_rest() {
    // 第 1 个 query LlmError（None 注入）后第 2 个 query 照常执行（不中断）
    let runner = scripted_runner(
        vec![
            None,
            None,
            Some(tool_resp("proc_ls", json!({ "limit": 5 }))),
            Some(finish_resp("已列出。")),
        ],
        10,
    );
    let queries = vec![
        spec("performance-diagnose", 0, "q-fail", &["proc_ls"]),
        spec("performance-diagnose", 0, "q-pass", &["proc_ls"]),
    ];
    let results = run_eval(&runner, &queries, 2, &mut |_, _, _| {}).await;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].failure_mode, FailureMode::LlmError);
    assert_eq!(results[1].failure_mode, FailureMode::Pass);
}

#[tokio::test]
async fn test_eval_output_degraded_priority() {
    // tool 命中 + answer 非空但含 `<eos>` 字面量 → OutputDegraded（优先归类）
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_ls", json!({ "limit": 5 }))),
            Some(finish_resp(&"<eos>".repeat(300))),
        ],
        10,
    );
    let q = spec(
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        &["proc_ls"],
    );
    let r = run_one_eval(&runner, &q, 1).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::OutputDegraded);
    assert_eq!(r.chain_steps_hit, 1); // tool 命中但整体仍失败
}

#[tokio::test]
async fn test_eval_retry_then_pass() {
    // attempt 1 错 tool（WrongTool）→ attempt 2 命中 → Pass + attempts_used=2
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_metrics_system", json!({}))),
            Some(finish_resp("attempt 1 回答")),
            Some(tool_resp("proc_ls", json!({ "limit": 5 }))),
            Some(finish_resp("attempt 2 回答")),
        ],
        10,
    );
    let q = spec(
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        &["proc_ls"],
    );
    let r = run_one_eval(&runner, &q, 2).await;
    assert!(r.passed);
    assert_eq!(r.attempts_used, 2);
}

#[tokio::test]
async fn test_eval_last_attempt_recorded() {
    // attempt 1 Ok-but-Fail → attempt 2 Err → 最终记录末次状态（LlmError）
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_metrics_system", json!({}))),
            Some(finish_resp("attempt 1 回答")),
            None,
        ],
        10,
    );
    let q = spec(
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        &["proc_ls"],
    );
    let r = run_one_eval(&runner, &q, 2).await;
    assert!(!r.passed);
    assert_eq!(r.failure_mode, FailureMode::LlmError);
    assert_eq!(r.attempts_used, 2);
}

#[tokio::test]
async fn test_eval_progress_callback_sequence() {
    let runner = scripted_runner(
        vec![
            Some(tool_resp("proc_ls", json!({}))),
            Some(finish_resp("a")),
            Some(tool_resp("proc_ls", json!({}))),
            Some(finish_resp("b")),
        ],
        10,
    );
    let queries = vec![
        spec("performance-diagnose", 0, "q1", &["proc_ls"]),
        spec("performance-diagnose", 0, "q2", &["proc_ls"]),
    ];
    let mut seen: Vec<(usize, usize, bool)> = Vec::new();
    let results = run_eval(&runner, &queries, 1, &mut |r, idx, total| {
        seen.push((idx, total, r.passed));
    })
    .await;
    assert_eq!(seen, vec![(1, 2, true), (2, 2, true)]);
    assert_eq!(results.len(), 2);
}

// ===========================================================================
// query 选择（parse_levels / select_queries）
// ===========================================================================

#[test]
fn test_parse_levels() {
    assert_eq!(parse_levels("0,2").unwrap(), vec![0, 2]);
    assert_eq!(parse_levels("2,2").unwrap(), vec![2]); // 去重
    assert_eq!(parse_levels(" 1 ").unwrap(), vec![1]);
    assert!(parse_levels("9").unwrap_err().contains("合法值"));
    assert!(parse_levels("x").unwrap_err().contains("合法值"));
    assert!(parse_levels("").is_err());
}

#[test]
fn test_select_queries_quick() {
    let all = eval::load_eval_queries().unwrap();
    let quick = select_queries(&all, &[], &[], true).unwrap();
    // 26 = 9 scenario × (L0 + L1) + 8 scenario × L2（monitor 无 L2 seed——
    // fixtures monitor-l2.jsonl 空文件，brainstorm「≈27」的约数来源）
    assert_eq!(quick.len(), 26);
    let mut groups = std::collections::HashSet::new();
    for q in &quick {
        assert!(
            groups.insert((q.scenario.clone(), q.level)),
            "每 (scenario,level) 恰 1 条"
        );
    }
    assert_eq!(groups.len(), 26);
}

#[test]
fn test_select_queries_filters() {
    let all = eval::load_eval_queries().unwrap();
    let l2 = select_queries(&all, &[2], &[], false).unwrap();
    assert_eq!(l2.len(), 20);
    assert!(l2.iter().all(|q| q.level == 2));

    let usb: Vec<QuerySpec> = select_queries(&all, &[], &["usb".to_string()], false).unwrap();
    assert!(!usb.is_empty());
    assert!(usb.iter().all(|q| q.scenario == "usb"));

    let err = select_queries(&all, &[], &["no-such".to_string()], false).unwrap_err();
    assert!(err.contains("no-such"));
}

#[test]
fn test_select_queries_empty_selection_errors() {
    // 合成小表构造空选（真实 70 表全 scenario×level 有覆盖，无法天然为空）
    let synthetic = vec![spec("usb", 0, "只有 L0", &["proc_ls"])];
    let err = select_queries(&synthetic, &[2], &[], false).unwrap_err();
    assert!(err.contains("为空"));
}

// ===========================================================================
// 聚合 + 报告 + compare + roundtrip
// ===========================================================================

fn qr(
    level: u8,
    passed: bool,
    mode: FailureMode,
    chain_hit: usize,
    expected_len: usize,
) -> QueryResult {
    QueryResult {
        scenario: "test".to_string(),
        level,
        query: format!("q-L{level}"),
        expected_tools: vec!["proc_ls".to_string(); expected_len],
        passed,
        failure_mode: mode,
        chain_steps_hit: chain_hit,
        actual_tools: vec!["proc_ls".to_string()],
        stop_cause: "end_turn".to_string(),
        final_text_head: "回答".to_string(),
        duration_ms: 100,
        attempts_used: 1,
    }
}

fn test_meta() -> EvalRunMeta {
    EvalRunMeta {
        timestamp: "2026-08-20T12:00:00Z".to_string(),
        provider: "llama-cpp".to_string(),
        provider_detail: "llama-server: x | model: y".to_string(),
        attempts: 2,
        max_steps: 10,
        git_describe: "v0.21.0-test".to_string(),
        quick: false,
        query_count: 7,
    }
}

#[test]
fn test_build_report_dual_caliber() {
    let results = vec![
        qr(0, true, FailureMode::Pass, 1, 1),
        qr(0, true, FailureMode::Pass, 1, 1),
        qr(0, false, FailureMode::WrongTool, 0, 1),
        qr(1, true, FailureMode::Pass, 1, 1),
        // L2：1 full-chain Pass（2/2）+ 1 ChainIncomplete（1/2）+ 1 NoToolCall（0/3）
        qr(2, true, FailureMode::Pass, 2, 2),
        qr(2, false, FailureMode::ChainIncomplete, 1, 2),
        qr(2, false, FailureMode::NoToolCall, 0, 3),
    ];
    let report = build_report(&results);
    assert_eq!(report.per_level.len(), 3);
    let l0 = &report.per_level[0];
    assert_eq!((l0.total, l0.passed), (3, 2));
    let l2 = &report.per_level[2];
    assert_eq!(l2.total, 3);
    assert_eq!(l2.full_chain, 1);
    assert_eq!(l2.chain_steps_hit, 3); // 2 + 1 + 0
    assert_eq!(l2.chain_steps_total, 7); // 2 + 2 + 3
    // 直方图：4 失败（wrong_tool 1 / chain_incomplete 1 / no_tool_call 1），降序 + 名序
    assert_eq!(
        report.failure_histogram,
        vec![
            ("chain_incomplete".to_string(), 1),
            ("no_tool_call".to_string(), 1),
            ("wrong_tool".to_string(), 1),
        ]
    );
    assert_eq!(report.total_duration_ms, 700);
}

#[test]
fn test_render_markdown_sections() {
    let results = vec![
        qr(0, true, FailureMode::Pass, 1, 1),
        qr(2, false, FailureMode::ChainIncomplete, 1, 2),
        qr(2, false, FailureMode::OutputDegraded, 2, 2),
    ];
    let run = EvalRunFile {
        meta: test_meta(),
        report: build_report(&results),
        results,
    };
    let md = eval::report::render_markdown(&run);
    assert!(md.contains("# proc agent eval 报告"));
    assert!(md.contains("- provider: llama-cpp"));
    assert!(md.contains("## 通过率（per level）"));
    assert!(md.contains("| L0 |"));
    assert!(md.contains("L2 full-chain"));
    assert!(md.contains("L2 chain-step"));
    assert!(md.contains("链步命中"));
    assert!(md.contains("## 失败模式直方图"));
    assert!(md.contains("█")); // 条形图存在
    assert!(md.contains("## 失败 query 明细"));
    assert!(md.contains("chain_incomplete"));
    assert!(md.contains("output_degraded"));
}

#[test]
fn test_render_markdown_all_pass_no_failures() {
    let results = vec![qr(0, true, FailureMode::Pass, 1, 1)];
    let run = EvalRunFile {
        meta: test_meta(),
        report: build_report(&results),
        results,
    };
    let md = eval::report::render_markdown(&run);
    assert!(md.contains("（无失败 query）"));
    assert!(md.contains("100%"));
}

#[test]
fn test_render_compare_markdown() {
    let mk = |l0_pass: usize, wrong_tool: usize| -> EvalRunFile {
        let results = {
            let mut v: Vec<QueryResult> = (0..l0_pass)
                .map(|_| qr(0, true, FailureMode::Pass, 1, 1))
                .collect();
            v.extend((0..wrong_tool).map(|_| qr(0, false, FailureMode::WrongTool, 0, 1)));
            v
        };
        EvalRunFile {
            meta: test_meta(),
            report: build_report(&results),
            results,
        }
    };
    let a = mk(1, 3);
    let b = mk(4, 0);
    let labels = vec!["a.json".to_string(), "b.json".to_string()];
    let md = eval::report::render_compare_markdown(&[a, b], &labels);
    assert!(md.contains("# proc agent eval 对比报告"));
    assert!(md.contains("| a.json | llama-cpp | 1/4 |"));
    assert!(md.contains("| b.json | llama-cpp | 4/4 |"));
    assert!(md.contains("## 失败模式迁移（a.json → b.json）"));
    assert!(md.contains("| wrong_tool | 3 | 0 | -3 |"));
}

#[test]
fn test_eval_run_file_roundtrip() {
    let results = vec![
        qr(0, true, FailureMode::Pass, 1, 1),
        qr(2, false, FailureMode::ChainIncomplete, 1, 2),
    ];
    let run = EvalRunFile {
        meta: test_meta(),
        report: build_report(&results),
        results,
    };
    let json = serde_json::to_string_pretty(&run).unwrap();
    let back: EvalRunFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back.meta.provider, "llama-cpp");
    assert_eq!(back.meta.attempts, 2);
    assert_eq!(back.results.len(), 2);
    assert_eq!(back.results[1].failure_mode, FailureMode::ChainIncomplete);
    assert_eq!(back.results[1].chain_steps_hit, 1);
    assert_eq!(back.report.per_level.len(), 2);
    assert_eq!(back.report.total_duration_ms, 200);
}

#[test]
fn test_eval_report_serde_shape() {
    // LevelSummary / EvalReport 字段名锁定（compare 与报告的事实 API）
    let report = EvalReport {
        per_level: vec![LevelSummary {
            level: 2,
            total: 20,
            passed: 3,
            full_chain: 3,
            chain_steps_hit: 31,
            chain_steps_total: 48,
        }],
        failure_histogram: vec![("chain_incomplete".to_string(), 12)],
        total_duration_ms: 12345,
    };
    let json = serde_json::to_value(&report).unwrap();
    assert!(json.get("per_level").is_some());
    assert!(json.get("failure_histogram").is_some());
    assert!(json.get("total_duration_ms").is_some());
    let ls = &json["per_level"][0];
    for field in [
        "level",
        "total",
        "passed",
        "full_chain",
        "chain_steps_hit",
        "chain_steps_total",
    ] {
        assert!(ls.get(field).is_some(), "LevelSummary 缺字段 {field}");
    }
}

// ===========================================================================
// MockProvider seed 全管线（录制 fixture 含 proc_finish → 一轮终止）
// ===========================================================================

#[cfg(feature = "mock-provider")]
#[tokio::test]
async fn test_eval_pipeline_mock_seed() {
    use std::path::PathBuf;

    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("agent");
    let provider = proc::agent::mock_provider::MockProvider::new(fixtures);
    let runner = AgentRunner::new(
        Arc::new(provider),
        proc::agent::tools::catalog::default_registry(),
        AgentOptions {
            max_steps: 10,
            ..Default::default()
        },
    );

    let all = eval::load_eval_queries().unwrap();
    let selected =
        select_queries(&all, &[0], &["performance-diagnose".to_string()], false).unwrap();
    assert_eq!(selected.len(), 3);

    let results = run_eval(&runner, &selected, 1, &mut |_, _, _| {}).await;
    assert_eq!(results.len(), 3);

    let run = EvalRunFile {
        meta: test_meta(),
        report: build_report(&results),
        results,
    };
    let json = serde_json::to_string(&run).unwrap();
    let back: EvalRunFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back.results.len(), 3);

    let md = eval::report::render_markdown(&run);
    assert!(md.contains("## 通过率（per level）"));
    assert!(md.contains("| L0 | 3 | 3 |") || md.contains("| L0 |"));
}
