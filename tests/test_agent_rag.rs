//! v0.24 stage 2：RAG 检索层集成测试（ADR-0034 D1/D3/D4 + D2 模板渲染）。
//! v0.24 stage 3：注入接线测试（E/F/G 组）+ 本地召回对照探针（H 组）。
//!
//! fixture 全构造（tempdir + 真实 struct serde 序列化），不依赖真实
//! `~/.config/proc/sessions/` 与本地 eval run JSON——CI 确定性（H 组
//! `#[ignore]` 探针除外——本机挂机前手动跑）。

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::json;

use proc::agent::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
    Usage,
};
use proc::agent::rag::corpus::{Entry, EntrySource};
use proc::agent::rag::{RagIndex, RagParams, inject_experience};
use proc::agent::runner::{AgentOptions, AgentRunner, StopCause};
use proc::agent::session_log::{LogEvent, SessionLogEntry};
use proc::agent::types::{Message, Role, ToolCall};
use proc::agent::{AgentConfig, EvalReport, EvalRunFile, EvalRunMeta, FailureMode, QueryResult};

// ---------------------------------------------------------------------------
// fixture 构造
// ---------------------------------------------------------------------------

fn entry(query: &str, tools: &[&str], head: &str) -> Entry {
    Entry {
        query: query.to_string(),
        tools: tools.iter().map(|s| s.to_string()).collect(),
        conclusion_head: head.to_string(),
        source: EntrySource::Eval,
    }
}

fn session_line(seq: u64, event: LogEvent) -> String {
    serde_json::to_string(&SessionLogEntry {
        seq,
        ts_rel_ms: seq * 10,
        event,
    })
    .unwrap()
}

/// 一个成功段 + 头部 session_start 的完整 JSONL 内容。
fn good_session_content(query: &str, tool: &str, head: &str) -> String {
    [
        session_line(
            0,
            LogEvent::SessionStart {
                provider: "llama-cpp".into(),
                wall_start: "2026-08-25T00:00:00Z".into(),
            },
        ),
        session_line(1, LogEvent::QueryStarted { text: query.into() }),
        session_line(2, LogEvent::ToolStart { name: tool.into() }),
        session_line(
            3,
            LogEvent::SessionFinished {
                stop: "end_turn".into(),
                final_chars: head.chars().count(),
                final_head: head.into(),
            },
        ),
    ]
    .join("\n")
        + "\n"
}

fn query_result(query: &str, passed: bool, tools: &[&str]) -> QueryResult {
    QueryResult {
        scenario: "usb".into(),
        level: 0,
        query: query.to_string(),
        expected_tools: tools.iter().map(|s| s.to_string()).collect(),
        passed,
        failure_mode: if passed {
            FailureMode::Pass
        } else {
            FailureMode::WrongTool
        },
        chain_steps_hit: if passed { tools.len() } else { 0 },
        actual_tools: tools.iter().map(|s| s.to_string()).collect(),
        stop_cause: "end_turn".into(),
        final_text_head: format!("{query} 的结论"),
        duration_ms: 100,
        attempts_used: 1,
    }
}

fn eval_run_file(results: Vec<QueryResult>) -> EvalRunFile {
    EvalRunFile {
        meta: EvalRunMeta {
            timestamp: "2026-08-25T00:00:00Z".into(),
            provider: "llama-cpp".into(),
            provider_detail: "test".into(),
            attempts: 2,
            max_steps: 10,
            git_describe: "test".into(),
            quick: false,
            query_count: results.len(),
        },
        results,
        report: EvalReport {
            per_level: vec![],
            failure_histogram: vec![],
            total_duration_ms: 0,
        },
    }
}

fn write_file(path: &std::path::Path, content: &str) -> std::path::PathBuf {
    std::fs::write(path, content).unwrap();
    path.to_path_buf()
}

// ---------------------------------------------------------------------------
// A 组：语料装载（RagIndex::build 双语料源 + 降级 + 去重）
// ---------------------------------------------------------------------------

#[test]
fn build_from_session_and_eval_corpora() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();

    // 1 个成功段文件 + 1 个空会话文件（附录 B：96% 空会话是常态路径）
    write_file(
        &sessions.join("20260825-100000-llama-cpp.jsonl"),
        &good_session_content("列出 CPU 占用最高的进程", "proc_ls", "java.exe 38%"),
    );
    write_file(
        &sessions.join("20260825-100100-llama-cpp.jsonl"),
        &session_line(
            0,
            LogEvent::SessionStart {
                provider: "llama-cpp".into(),
                wall_start: "2026-08-25T00:01:00Z".into(),
            },
        ),
    );

    let eval_path = write_file(
        &dir.path().join("eval-run.json"),
        &serde_json::to_string(&eval_run_file(vec![
            query_result("列出 USB 盘", true, &["proc_usb_list"]),
            query_result("查看 DNS 解析记录", true, &["proc_dns_log"]),
            query_result("弹盘失败样例", false, &["proc_ls"]),
        ]))
        .unwrap(),
    );

    let index = RagIndex::build(&sessions, &[eval_path]);
    assert_eq!(index.len(), 3); // 1 session + 2 passed eval（failed 不进语料）

    let sources: Vec<&str> = index.entries().iter().map(|e| e.source.label()).collect();
    assert_eq!(sources, vec!["session", "eval", "eval"]); // session 先装载
    assert_eq!(index.entries()[0].tools, vec!["proc_ls"]);
    assert!(!index.entries().iter().any(|e| e.query == "弹盘失败样例"));
}

#[test]
fn build_missing_session_dir_degrades_to_empty() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-subdir");
    let index = RagIndex::build(&missing, &[]);
    assert!(index.is_empty());
}

#[test]
fn eval_dedup_across_runs() {
    let dir = tempfile::tempdir().unwrap();
    let run1 = write_file(
        &dir.path().join("run1.json"),
        &serde_json::to_string(&eval_run_file(vec![query_result(
            "去重查询",
            true,
            &["proc_a"],
        )]))
        .unwrap(),
    );
    let run2 = write_file(
        &dir.path().join("run2.json"),
        &serde_json::to_string(&eval_run_file(vec![
            query_result("去重查询", true, &["proc_a"]),
            query_result("独立查询", true, &["proc_b"]),
        ]))
        .unwrap(),
    );
    let sessions = dir.path().join("sessions"); // 缺失 → 空语料警告降级
    let index = RagIndex::build(&sessions, &[run1, run2]);
    assert_eq!(index.len(), 2); // 跨 run 同 query 去重后 1 条 + 独立 1 条
}

#[test]
fn corrupt_eval_json_skips_source() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    write_file(
        &sessions.join("a.jsonl"),
        &good_session_content("好段", "proc_ls", "ok"),
    );
    let bad = write_file(&dir.path().join("bad.json"), "{ not json");
    let index = RagIndex::build(&sessions, &[bad]);
    assert_eq!(index.len(), 1); // 坏 eval 源跳过，session 语料保留
}

// ---------------------------------------------------------------------------
// B 组：检索命中与排序（min_score 门槛 / 相关性排序 / 并列 tie-break）
// ---------------------------------------------------------------------------

fn usb_dns_corpus() -> RagIndex {
    RagIndex::from_entries(vec![
        entry(
            "列出所有 USB 设备",
            &["proc_usb_list"],
            "共 2 个设备：Kingston 32G / SanDisk 64G",
        ),
        entry(
            "查看 DNS 解析历史",
            &["proc_dns_log"],
            "example.com -> 93.184.216.34",
        ),
        entry(
            "结束占用 CPU 最高的进程",
            &["proc_ls", "proc_kill"],
            "已结束 java.exe",
        ),
    ])
}

#[test]
fn retrieval_ranks_by_relevance_with_min_score_gate() {
    let index = usb_dns_corpus();
    let outcome = index.retrieve("怎么查看 USB 设备列表", &RagParams::default());

    assert_eq!(outcome.excluded, 0); // 覆盖率最高 0.4 < 0.6，零污染排除
    assert_eq!(outcome.hits.len(), 2); // E3 零词元命中 → min_score 门槛拦截
    assert_eq!(outcome.hits[0].0.query, "列出所有 USB 设备"); // usb+设备 双命中
    assert_eq!(outcome.hits[1].0.query, "查看 DNS 解析历史"); // 查看 单命中
    assert!(outcome.hits[0].1 > outcome.hits[1].1);
}

#[test]
fn retrieval_min_score_filters_irrelevant_query() {
    let index = usb_dns_corpus();
    let outcome = index.retrieve("讲个笑话", &RagParams::default());
    assert!(outcome.hits.is_empty());
    assert_eq!(outcome.excluded, 0);
}

#[test]
fn retrieval_tie_breaks_by_tool_chain_length() {
    let index = RagIndex::from_entries(vec![
        entry("alpha x1", &["proc_a"], "结论一"),
        entry("beta y1 y2", &["proc_a", "proc_b"], "结论二"),
    ]);
    // 两 entry 与 query 各共享 1 个 df=1 词元（alpha / beta）→ score 并列
    // ln3（N=2）→ tie-break：tool 链长的（beta y1 y2，2 tool）在前
    let outcome = index.retrieve("how to alpha and beta now", &RagParams::default());
    assert_eq!(outcome.excluded, 0); // 覆盖率 0.5 / 0.33 均低于阈值
    assert_eq!(outcome.hits.len(), 2);
    assert_eq!(outcome.hits[0].0.query, "beta y1 y2");
    assert!((outcome.hits[0].1 - outcome.hits[1].1).abs() < 1e-9);
}

// ---------------------------------------------------------------------------
// C 组：污染排除三类样例锚定（D4：同款 / 高覆盖改写 / 同场景异意图）
// ---------------------------------------------------------------------------

#[test]
fn pollution_same_query_exact_rewrite_excluded() {
    // 同款：归一化后全等（空白差异）→ exact match 排除
    let index = RagIndex::from_entries(vec![entry("列出 USB 盘", &["proc_usb_list"], "共 2 个")]);
    let outcome = index.retrieve("列出USB盘", &RagParams::default());
    assert_eq!(outcome.excluded, 1);
    assert!(outcome.hits.is_empty());
}

#[test]
fn pollution_high_coverage_rewrite_excluded() {
    // 高覆盖改写（增删一两个词）：coverage = 3/3 = 1.0 ≥ 0.6 → 排除
    let index = RagIndex::from_entries(vec![entry("列出 USB 设备", &["proc_usb_list"], "共 2 个")]);
    let outcome = index.retrieve("列出所有 USB 设备", &RagParams::default());
    assert_eq!(outcome.excluded, 1);
    assert!(outcome.hits.is_empty());
}

#[test]
fn pollution_same_scene_different_intent_still_retrievable() {
    // 同场景异意图（ADR D4 原例）：覆盖率 1/3 ≈ 0.33 < 0.4 → 不排除，
    // 且共享「盘」的稀有权重足以过 min_score——同类经验参考是设计目的
    let index = RagIndex::from_entries(vec![
        entry("列出 USB 盘", &["proc_usb_list"], "共 2 个设备"),
        entry("查看 DNS 解析记录", &["proc_dns_log"], "解析记录"),
    ]);
    let outcome = index.retrieve("弹出 E 盘", &RagParams::default());
    assert_eq!(outcome.excluded, 0);
    assert_eq!(outcome.hits.len(), 1);
    assert_eq!(outcome.hits[0].0.query, "列出 USB 盘");
}

// ---------------------------------------------------------------------------
// D 组：注入模板与预算（D2：格式 / 80 chars 结论 / 整条截断 / 原文透传）
// ---------------------------------------------------------------------------

#[test]
fn inject_renders_d2_template() {
    let index = usb_dns_corpus();
    let query = "怎么查看 USB 设备列表";
    let injected = inject_experience(query, &index, &RagParams::default());

    assert!(injected.injected);
    assert_eq!(injected.injected_entries, 2);
    assert_eq!(injected.excluded_entries, 0);
    assert!(injected.text.starts_with("[历史经验参考] "));
    assert!(
        injected
            .text
            .contains("- \"列出所有 USB 设备\" → proc_usb_list（结论：共 2 个设备")
    );
    assert!(
        injected
            .text
            .contains("- \"查看 DNS 解析历史\" → proc_dns_log（结论：example.com")
    );
    assert!(injected.text.ends_with(&format!("\n[当前问题] {query}")));

    // est_tokens = round(注入段 prefix chars / 1.5)——prefix = text 去掉原 query
    let prefix = &injected.text[..injected.text.len() - query.len()];
    let expected = (prefix.chars().count() as f64 / 1.5).round() as usize;
    assert_eq!(injected.est_tokens, expected);
    assert!(injected.est_tokens > 0);
}

#[test]
fn inject_truncates_conclusion_to_80_chars() {
    let long_head: String = "结".repeat(200);
    // 语料 ≥3 条使共享词元 idf=ln4 ≥ min_score（单条目语料 idf=ln2 过不了门槛）
    let index = RagIndex::from_entries(vec![
        entry("查磁盘", &["proc_disk"], &long_head),
        entry("run alpha", &["proc_a"], "x"),
        entry("run beta", &["proc_b"], "y"),
    ]);
    let injected = inject_experience("怎么看磁盘", &index, &RagParams::default());
    assert!(injected.injected);

    let start = injected.text.find("（结论：").unwrap() + "（结论：".len();
    let end = injected.text[start..].find('）').unwrap() + start;
    let conclusion: String = injected.text[start..end].to_string();
    assert!(conclusion.chars().count() <= 81); // 80 + 省略号
    assert!(conclusion.ends_with('…'));
}

#[test]
fn inject_budget_drops_whole_entry_not_half() {
    // E1 双词元命中（cpu+top，score 2·ln3）行短；E2 单词元命中（check，
    // score ln3）结论 200 chars 行长——budget=120 恰容 E1 整条、E2 整条
    // 放不下 → 整条丢弃（text 不含 E2 的 query 半行）
    let long_head = "详".repeat(200);
    let index = RagIndex::from_entries(vec![
        entry("list cpu top extra", &["proc_ls"], "ok"),
        entry("check disk detail", &["proc_cpu_detail"], &long_head),
    ]);
    let params = RagParams {
        budget_chars: 120,
        ..RagParams::default()
    };
    let injected = inject_experience("check cpu top more", &index, &params);

    assert!(injected.injected);
    assert_eq!(injected.injected_entries, 1);
    assert!(injected.text.contains("\"list cpu top extra\""));
    assert!(!injected.text.contains("check disk detail")); // 整条丢弃，无半截残文

    // 预算紧到零条可容 → 原文透传零注入痕迹
    let tight = RagParams {
        budget_chars: 10,
        ..RagParams::default()
    };
    let transparent = inject_experience("check cpu top more", &index, &tight);
    assert!(!transparent.injected);
    assert_eq!(transparent.text, "check cpu top more");
}

#[test]
fn inject_no_hits_transparent_with_exclusion_report() {
    // 空索引：原文透传
    let empty = RagIndex::from_entries(vec![]);
    let injected = inject_experience("任意问题", &empty, &RagParams::default());
    assert!(!injected.injected);
    assert_eq!(injected.text, "任意问题");
    assert_eq!(injected.excluded_entries, 0);

    // 有语料但当前 query 被污染排除全灭：透传 + excluded 计数进报告
    let index = RagIndex::from_entries(vec![entry("列出 USB 盘", &["proc_usb_list"], "共 2 个")]);
    let polluted = inject_experience("列出USB盘", &index, &RagParams::default());
    assert!(!polluted.injected);
    assert_eq!(polluted.text, "列出USB盘");
    assert_eq!(polluted.excluded_entries, 1); // D4 命中次数报告数据源
}

// ---------------------------------------------------------------------------
// stage 3：ScriptedProvider / StreamScriptedProvider（test_agent_v0_20_stage_3b
// 与 v0_21_stage_2 模式——逐轮弹出 + seen_messages 记录，注入断言载体）
// ---------------------------------------------------------------------------

struct ScriptedProvider {
    responses: Mutex<VecDeque<CompleteResponse>>,
    seen_messages: Mutex<Vec<Vec<Message>>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<CompleteResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            seen_messages: Mutex::new(Vec::new()),
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
        messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        self.seen_messages.lock().unwrap().push(messages);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
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

struct StreamScriptedProvider {
    turns: Mutex<VecDeque<Vec<Delta>>>,
    seen_messages: Mutex<Vec<Vec<Message>>>,
}

#[async_trait]
impl LlmProvider for StreamScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted-stream"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        Err(LlmError::Config("streaming only".to_string()))
    }

    fn stream(
        &self,
        messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        self.seen_messages.lock().unwrap().push(messages);
        let deltas = self.turns.lock().unwrap().pop_front().unwrap_or_default();
        futures_util::stream::iter(deltas.into_iter().map(Ok)).boxed()
    }
}

fn finish_resp(answer: &str) -> CompleteResponse {
    CompleteResponse {
        message: Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![ToolCall {
                id: "call-finish".to_string(),
                name: "proc_finish".to_string(),
                arguments: json!({ "answer": answer }),
            }],
            tool_results: Vec::new(),
        },
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
    }
}

fn finish_deltas(answer: &str) -> Vec<Delta> {
    vec![
        Delta::ToolCall(ToolCall {
            id: "call-finish".to_string(),
            name: "proc_finish".to_string(),
            arguments: json!({ "answer": answer }),
        }),
        Delta::EndTurn {
            stop_reason: StopReason::EndTurn,
        },
    ]
}

// ---------------------------------------------------------------------------
// E 组：RagConfig 解析（完整段 / 默认 / params 映射 / 未知字段拒）
// ---------------------------------------------------------------------------

#[test]
fn rag_config_full_section_parses() {
    let content = "[rag]\nenabled = true\nbudget_tokens = 400\ntop_k = 5\n\
                   exclude_threshold = 0.7\neval_corpora = [\"a.json\", \"b.json\"]\n";
    let config = AgentConfig::from_toml(content).unwrap();
    assert!(config.rag.enabled);
    assert_eq!(config.rag.budget_tokens, 400);
    assert_eq!(config.rag.top_k, 5);
    assert!((config.rag.exclude_threshold - 0.7).abs() < 1e-9);
    assert_eq!(config.rag.eval_corpora, vec!["a.json", "b.json"]);
}

#[test]
fn rag_config_defaults_when_section_empty_or_missing() {
    let empty = AgentConfig::from_toml("[rag]\n").unwrap().rag;
    let missing = AgentConfig::from_toml("").unwrap().rag;
    for cfg in [&empty, &missing] {
        assert!(!cfg.enabled); // 默认 off 保基线（ADR-0034 D2）
        assert_eq!(cfg.budget_tokens, 800);
        assert_eq!(cfg.top_k, 3);
        assert!((cfg.exclude_threshold - 0.6).abs() < 1e-9);
        assert!(cfg.eval_corpora.is_empty());
    }
}

#[test]
fn rag_config_params_maps_to_rag_params() {
    // 默认映射：800 tokens → 1200 chars 与 RagParams::default 同值锚
    let params = AgentConfig::from_toml("").unwrap().rag.params();
    assert_eq!(params.budget_chars, RagParams::default().budget_chars);
    assert_eq!(params.top_k, 3);
    assert_eq!(params.min_score, 1.0);
    assert!((params.exclude_threshold - 0.6).abs() < 1e-9);

    // 奇数 tokens 整数运算 * 3 / 2：801 → 1201
    let odd = AgentConfig::from_toml("[rag]\nbudget_tokens = 801\n")
        .unwrap()
        .rag
        .params();
    assert_eq!(odd.budget_chars, 1201);
}

#[test]
fn rag_config_unknown_field_rejected() {
    assert!(AgentConfig::from_toml("[rag]\nfoo = 1\n").is_err());
}

// ---------------------------------------------------------------------------
// F 组：off 态零开销锚（无 rag 的 runner 行为与 stage 2 前完全一致）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn runner_off_state_passes_query_through_untouched() {
    let provider = Arc::new(ScriptedProvider::new(vec![finish_resp("答案")]));
    let runner = AgentRunner::new(
        provider.clone() as Arc<dyn LlmProvider>,
        proc::agent::tools::catalog::default_registry(),
        AgentOptions::default(),
    );

    let query = "怎么查看 USB 设备列表";
    let outcome = runner.run(query).await.unwrap();
    assert_eq!(outcome.stop, StopCause::EndTurn);

    let seen = provider.seen_messages.lock().unwrap();
    // off 态零注入痕迹：user message 与原文逐字一致
    assert_eq!(seen[0][1].content.as_deref(), Some(query));
}

// ---------------------------------------------------------------------------
// G 组：on 态注入端到端（complete / streaming / 200 chars 同底截断）
// ---------------------------------------------------------------------------

fn rag_runner(provider: Arc<ScriptedProvider>, index: RagIndex) -> AgentRunner {
    AgentRunner::new(
        provider as Arc<dyn LlmProvider>,
        proc::agent::tools::catalog::default_registry(),
        AgentOptions::default(),
    )
    .with_rag(Arc::new(index), RagParams::default())
}

#[tokio::test]
async fn runner_on_state_injects_experience_prefix_complete_path() {
    let provider = Arc::new(ScriptedProvider::new(vec![finish_resp("答案")]));
    let runner = rag_runner(provider.clone(), usb_dns_corpus());

    let query = "怎么查看 USB 设备列表";
    let outcome = runner.run(query).await.unwrap();
    assert_eq!(outcome.stop, StopCause::EndTurn); // 注入不破 loop 语义

    let seen = provider.seen_messages.lock().unwrap();
    let user = seen[0][1].content.as_deref().unwrap();
    assert!(user.starts_with("[历史经验参考] "));
    assert!(user.contains("- \"列出所有 USB 设备\" → proc_usb_list"));
    assert!(user.ends_with(&format!("\n[当前问题] {query}")));
}

#[tokio::test]
async fn runner_on_state_truncates_probe_for_exact_exclusion() {
    // 长 query（250 chars）与条目 query（200 chars + …，即 session 源侧
    // 截断形态）前 200 chars 全等——runner 同款 200 chars 截断后 exact
    // match → 污染排除生效 → 透传（user message 为截断 probe，与 session
    // 源侧 200 chars 同底口径一致；eval query 全部远短于 200 不触发）
    let entry_query = format!("{}…", "查".repeat(200));
    let index = RagIndex::from_entries(vec![
        entry(&entry_query, &["proc_ls"], "结论"),
        entry("run alpha", &["proc_a"], "x"),
    ]);
    let provider = Arc::new(ScriptedProvider::new(vec![finish_resp("答案")]));
    let runner = rag_runner(provider.clone(), index);

    let long_query = "查".repeat(250);
    runner.run(&long_query).await.unwrap();

    let seen = provider.seen_messages.lock().unwrap();
    let user = seen[0][1].content.as_deref().unwrap();
    assert!(!user.contains("[历史经验参考]")); // 排除生效零注入
    assert_eq!(user, entry_query); // 透传 = 截断 probe（200 + …）
}

#[tokio::test]
async fn runner_on_state_injects_experience_prefix_streaming_path() {
    let provider = Arc::new(StreamScriptedProvider {
        turns: Mutex::new(VecDeque::from(vec![finish_deltas("答案")])),
        seen_messages: Mutex::new(Vec::new()),
    });
    let runner = AgentRunner::new(
        provider.clone() as Arc<dyn LlmProvider>,
        proc::agent::tools::catalog::default_registry(),
        AgentOptions::default(),
    )
    .with_rag(Arc::new(usb_dns_corpus()), RagParams::default());

    let cancel = AtomicBool::new(false);
    let outcome = runner
        .run_streaming("怎么查看 USB 设备列表", &[], &|_| {}, None, &cancel)
        .await
        .unwrap();
    assert_eq!(outcome.stop, StopCause::EndTurn);

    let seen = provider.seen_messages.lock().unwrap();
    let user = seen[0][1].content.as_deref().unwrap();
    assert!(user.starts_with("[历史经验参考] "));
    assert!(user.ends_with("\n[当前问题] 怎么查看 USB 设备列表"));
}

// ---------------------------------------------------------------------------
// H 组：本地召回对照探针（#[ignore]——D5 主指标①离线工具，挂机前本机跑）
// ---------------------------------------------------------------------------

/// 抽样 15 条覆盖 9 场景 × 3 level（perf 0/8 · proc 12/15/18 · docker 21/27 ·
/// usb 31/37 · security 42/46 · recording 52 · flow 57 · monitor 62 · dns 67；
/// L0 6 / L1 4 / L2 5）。人工标注每条的相关经验条目集合后对照 top-3 算
/// 命中率（top-3 含 ≥1 标注相关条目的 query 占比）与平均命中条数。
#[test]
#[ignore = "需本地三基线 JSON（从仓库根 cwd 跑），挂机前手动执行"]
fn local_recall_probe_prints_top3_for_sampled_queries() {
    let files = [
        "eval-e2b-70q.json",
        "eval-promptv2-70q.json",
        "eval-best-70q.json",
    ];
    for f in files {
        assert!(
            std::path::Path::new(f).is_file(),
            "缺基线 JSON {f}（从仓库根目录跑）"
        );
    }
    let paths: Vec<std::path::PathBuf> = files.iter().map(std::path::PathBuf::from).collect();
    let empty_sessions = tempfile::tempdir().unwrap();
    let index = RagIndex::build(empty_sessions.path(), &paths);
    assert!(index.len() >= 30, "bootstrap 去重池异常: {}", index.len()); // 附录 B 口径 40

    let queries = proc::agent::eval::load_eval_queries().unwrap();
    let sample = [
        0usize, 8, 12, 15, 18, 21, 27, 31, 37, 42, 46, 52, 57, 62, 67,
    ];
    for idx in sample {
        let q = &queries[idx];
        let outcome = index.retrieve(&q.text, &RagParams::default());
        println!("idx {idx} [{}/L{}] {}", q.scenario, q.level, q.text);
        println!(
            "  excluded={} hits={}",
            outcome.excluded,
            outcome.hits.len()
        );
        for (e, score) in &outcome.hits {
            println!(
                "  {score:.2} [{}] {} → {}",
                e.source.label(),
                e.query,
                e.tools.join(" → ")
            );
        }
    }
}
