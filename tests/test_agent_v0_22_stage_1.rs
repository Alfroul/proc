//! v0.22 stage 1 集成测试 — eval harness 骨架（ADR-0032）。
//!
//! 覆盖：queries.toml 加载校验（分布 / 去重 / 链长 / catalog 名单）/ serde
//! schema roundtrip（结果 JSON contract 字段名锁定）/ 判定纯函数（classify_failure
//! 8 变体含 OutputDegraded 优先归类 + tools_subsequence_hit 保序子序列 +
//! is_degraded_output 退化口径）/ CLI 变体 stub（友好拦截不 panic）。
//!
//! runner 执行循环 / 报告生成留 stage 2；session 观测留 stage 3。

use clap::Parser;
use proc::agent::eval::{
    self, EvalReport, FailureMode, LevelSummary, OutcomeSummary, QueryResult, load_eval_queries,
};

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn summary<'a>(
    final_text: &'a str,
    tools: &'a [String],
    stop: &'a str,
    llm_error: bool,
    nudge: bool,
) -> OutcomeSummary<'a> {
    OutcomeSummary {
        final_text,
        actual_tools: tools,
        stop_cause: stop,
        llm_error,
        nudge_fallback: nudge,
    }
}

// ===========================================================================
// A 组：queries.toml 加载校验
// ===========================================================================

#[test]
fn test_eval_queries_load_70() {
    let queries = load_eval_queries().expect("加载 + 全量校验通过");
    assert_eq!(queries.len(), 70);
}

#[test]
fn test_eval_queries_level_distribution() {
    let queries = load_eval_queries().unwrap();
    let l0 = queries.iter().filter(|q| q.level == 0).count();
    let l1 = queries.iter().filter(|q| q.level == 1).count();
    let l2 = queries.iter().filter(|q| q.level == 2).count();
    assert_eq!((l0, l1, l2), (23, 27, 20));
}

#[test]
fn test_eval_queries_no_duplicate_text() {
    let mut seen = std::collections::HashSet::new();
    for q in &load_eval_queries().unwrap() {
        assert!(seen.insert(q.text.clone()), "query 文本重复: {}", q.text);
    }
}

#[test]
fn test_eval_queries_scenarios_match_fixtures() {
    for q in &load_eval_queries().unwrap() {
        assert!(
            eval::FIXTURE_SCENARIOS.contains(&q.scenario.as_str()),
            "scenario 越界: {}",
            q.scenario
        );
    }
}

#[test]
fn test_eval_queries_l2_chains_min_len() {
    for q in &load_eval_queries().unwrap() {
        if q.level == 2 {
            assert!(
                q.expected_tools.len() >= 2,
                "L2 链长度 < 2: {}（{:?}）",
                q.text,
                q.expected_tools
            );
        }
    }
}

#[test]
fn test_eval_queries_expected_tools_in_catalog() {
    let registry = proc::agent::tools::catalog::default_registry();
    for q in &load_eval_queries().unwrap() {
        assert!(
            !q.expected_tools.is_empty(),
            "expected_tools 为空: {}",
            q.text
        );
        for t in &q.expected_tools {
            assert!(registry.get(t).is_some(), "tool 不在 catalog: {t}");
        }
    }
}

// ===========================================================================
// B 组：serde schema roundtrip（结果 JSON contract 字段名锁定）
// ===========================================================================

#[test]
fn test_failure_mode_variants() {
    // Pass + 7 失败变体（含 2026-08-20 实测新增的 OutputDegraded）
    let all = [
        FailureMode::Pass,
        FailureMode::NoToolCall,
        FailureMode::WrongTool,
        FailureMode::ChainIncomplete,
        FailureMode::EmptyAnswer,
        FailureMode::MaxSteps,
        FailureMode::LlmError,
        FailureMode::OutputDegraded,
    ];
    assert_eq!(all.len(), 8);
    for m in all {
        let json = serde_json::to_string(&m).unwrap();
        let back: FailureMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
    assert_eq!(
        serde_json::to_string(&FailureMode::OutputDegraded).unwrap(),
        "\"output_degraded\""
    );
    assert_eq!(
        serde_json::to_string(&FailureMode::ChainIncomplete).unwrap(),
        "\"chain_incomplete\""
    );
}

#[test]
fn test_query_result_schema_roundtrip() {
    let r = QueryResult {
        scenario: "usb".to_string(),
        level: 2,
        query: "多个 USB 设备同时被占用，规划释放顺序".to_string(),
        expected_tools: owned(&["proc_eject_status", "proc_help"]),
        passed: false,
        failure_mode: FailureMode::ChainIncomplete,
        chain_steps_hit: 1,
        actual_tools: owned(&["proc_eject_status"]),
        stop_cause: "end_turn".to_string(),
        final_text_head: "已检测到 2 个设备被占用…".to_string(),
        duration_ms: 12_345,
        attempts_used: 2,
    };
    let json = serde_json::to_string(&r).unwrap();
    for key in [
        "scenario",
        "level",
        "query",
        "expected_tools",
        "passed",
        "failure_mode",
        "chain_steps_hit",
        "actual_tools",
        "stop_cause",
        "final_text_head",
        "duration_ms",
        "attempts_used",
    ] {
        assert!(
            json.contains(&format!("\"{key}\"")),
            "字段 {key} 缺失: {json}"
        );
    }
    let back: QueryResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.scenario, r.scenario);
    assert_eq!(back.level, 2);
    assert_eq!(back.query, r.query);
    assert_eq!(back.expected_tools, r.expected_tools);
    assert!(!back.passed);
    assert_eq!(back.failure_mode, FailureMode::ChainIncomplete);
    assert_eq!(back.chain_steps_hit, 1);
    assert_eq!(back.actual_tools, r.actual_tools);
    assert_eq!(back.stop_cause, "end_turn");
    assert_eq!(back.final_text_head, r.final_text_head);
    assert_eq!(back.duration_ms, 12_345);
    assert_eq!(back.attempts_used, 2);
}

#[test]
fn test_eval_report_schema_roundtrip() {
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
        total_duration_ms: 3_600_000,
    };
    let json = serde_json::to_string(&report).unwrap();
    for key in [
        "per_level",
        "failure_histogram",
        "total_duration_ms",
        "level",
        "total",
        "passed",
        "full_chain",
        "chain_steps_hit",
        "chain_steps_total",
    ] {
        assert!(
            json.contains(&format!("\"{key}\"")),
            "字段 {key} 缺失: {json}"
        );
    }
    let back: EvalReport = serde_json::from_str(&json).unwrap();
    assert_eq!(back.per_level.len(), 1);
    assert_eq!(back.per_level[0].level, 2);
    assert_eq!(back.per_level[0].total, 20);
    assert_eq!(back.per_level[0].passed, 3);
    assert_eq!(back.per_level[0].full_chain, 3);
    assert_eq!(back.per_level[0].chain_steps_hit, 31);
    assert_eq!(back.per_level[0].chain_steps_total, 48);
    assert_eq!(
        back.failure_histogram,
        vec![("chain_incomplete".to_string(), 12)]
    );
    assert_eq!(back.total_duration_ms, 3_600_000);
}

// ===========================================================================
// C 组：classify_failure 判定顺序（8 变体）
// ===========================================================================

#[test]
fn test_classify_no_tool_call() {
    let tools: Vec<String> = vec![];
    let out = summary("系统当前空闲，无需处理。", &tools, "end_turn", false, false);
    assert_eq!(
        eval::classify_failure(&out, &owned(&["proc_metrics_system"])),
        FailureMode::NoToolCall
    );
}

#[test]
fn test_classify_wrong_tool() {
    let tools = owned(&["proc_ls"]);
    let out = summary(
        "磁盘 I/O 主要来自 chrome.exe。",
        &tools,
        "end_turn",
        false,
        false,
    );
    assert_eq!(
        eval::classify_failure(&out, &owned(&["proc_metrics_disk_io"])),
        FailureMode::WrongTool
    );
}

#[test]
fn test_classify_chain_incomplete() {
    // L2 链部分命中：expected 2 命中 1（中间还插了别的 tool）
    let tools = owned(&["proc_eject_status", "proc_ls"]);
    let expected = owned(&["proc_eject_status", "proc_help"]);
    let out = summary(
        "检测到 1 个设备被占用，规划如下…",
        &tools,
        "end_turn",
        false,
        false,
    );
    assert_eq!(
        eval::classify_failure(&out, &expected),
        FailureMode::ChainIncomplete
    );
    // chain_steps_hit 口径正确（保序子序列命中数）
    assert_eq!(eval::tools_subsequence_hit(&tools, &expected), (false, 1));
}

#[test]
fn test_classify_empty_answer() {
    let tools = owned(&["proc_dns"]);
    let out = summary("", &tools, "end_turn", false, false);
    assert_eq!(
        eval::classify_failure(&out, &owned(&["proc_dns"])),
        FailureMode::EmptyAnswer
    );
    // nudge 兜底文案同口径（文本非空但是空响应重试的兜底）
    let nudge = summary(
        "（模型未返回内容，请重试）",
        &tools,
        "empty_after_retry",
        false,
        true,
    );
    assert_eq!(
        eval::classify_failure(&nudge, &owned(&["proc_dns"])),
        FailureMode::EmptyAnswer
    );
}

#[test]
fn test_classify_max_steps() {
    let tools = owned(&["proc_ls", "proc_ls", "proc_ls"]);
    let out = summary("仍在排查中…", &tools, "max_steps", false, false);
    assert_eq!(
        eval::classify_failure(&out, &owned(&["proc_metrics_gpu"])),
        FailureMode::MaxSteps
    );
}

#[test]
fn test_classify_llm_error() {
    let tools: Vec<String> = vec![];
    let out = summary("", &tools, "end_turn", true, false);
    assert_eq!(
        eval::classify_failure(&out, &owned(&["proc_dns"])),
        FailureMode::LlmError
    );
}

#[test]
fn test_classify_output_degraded_priority() {
    // tool 命中但 final_text 含 <eos> 字面量 → OutputDegraded 且优先归类
    // （退化即整体 fail，即使 tool 全命中——brainstorm 风险 6）
    let tools = owned(&["proc_ls"]);
    let expected = owned(&["proc_ls"]);
    let out = summary(
        "占用最高的进程是 chrome.exe<eos><eos>患关团",
        &tools,
        "end_turn",
        false,
        false,
    );
    assert_eq!(
        eval::classify_failure(&out, &expected),
        FailureMode::OutputDegraded
    );
}

#[test]
fn test_classify_pass() {
    let tools = owned(&["proc_metrics_system"]);
    let out = summary(
        "CPU 23%，内存 61%，系统状态正常。",
        &tools,
        "end_turn",
        false,
        false,
    );
    assert_eq!(
        eval::classify_failure(&out, &owned(&["proc_metrics_system"])),
        FailureMode::Pass
    );
}

// ===========================================================================
// D 组：is_degraded_output 退化口径
// ===========================================================================

#[test]
fn test_is_degraded_special_token_markers() {
    // 名单 4 个标记各自命中（夹在正常文本中间也算——字面量泄漏即退化）
    for m in eval::DEGRADED_TOKEN_MARKERS {
        assert!(
            eval::is_degraded_output(&format!("分析结果如下 {m} 出现异常")),
            "标记 {m} 应命中"
        );
    }
    // 含 < 的合法内容不误伤（比较式 / 无特殊 token 的中文长文本）
    assert!(!eval::is_degraded_output(
        "如果 a < b 且 5 < 10，说明内存占用低于阈值，属正常状态。"
    ));
    let long_normal = "系统诊断完成。CPU 占用 23%，内存占用 61%，磁盘 I/O 正常，\
                       GPU 温度 62℃。建议保持当前负载，无需干预。";
    assert!(!eval::is_degraded_output(long_normal));
}

#[test]
fn test_is_degraded_repetition() {
    // 实测样本形态：<eos> 字面量重复数百次
    assert!(eval::is_degraded_output(&"<eos>".repeat(300)));
    // 同一短片段连续重复达到阈值（形近字乱码循环）
    assert!(eval::is_degraded_output(
        &"患关团".repeat(eval::DEGRADED_REPEAT_LIMIT)
    ));
    // 正常中文长文本不命中（自然语言无连续同片段重复）
    let normal = "CPU 占用最高的进程是 chrome.exe（23.4%），其次为 code.exe（11.2%）。\
                  内存方面 svchost.exe 占用最多（412MB）。建议关闭不用的浏览器标签页释放资源，\
                  如需进一步分析磁盘 I/O 或网络流量，可以继续追问。";
    assert!(!eval::is_degraded_output(normal));
}

// ===========================================================================
// E 组：tools_subsequence_hit 保序子序列口径
// ===========================================================================

#[test]
fn test_subsequence_hit_ordered() {
    let expected = owned(&["a", "b"]);
    // 中间插其他 tool 不影响（保序子序列，不要求相邻）
    let interleaved = owned(&["x", "a", "y", "b"]);
    assert_eq!(
        eval::tools_subsequence_hit(&interleaved, &expected),
        (true, 2)
    );
    // 顺序打乱不命中：b 在 a 前面，保序只能命中 1 个
    let shuffled = owned(&["b", "x", "a"]);
    assert_eq!(
        eval::tools_subsequence_hit(&shuffled, &expected),
        (false, 1)
    );
    // 计数正确：部分命中
    let partial = owned(&["x", "a"]);
    assert_eq!(eval::tools_subsequence_hit(&partial, &expected), (false, 1));
    // 空 actual / 空 expected
    assert_eq!(eval::tools_subsequence_hit(&[], &expected), (false, 0));
    assert_eq!(eval::tools_subsequence_hit(&interleaved, &[]), (false, 0));
}

// ===========================================================================
// F 组：CLI 变体 stub（友好拦截，不 panic）
// ===========================================================================

#[test]
fn test_cli_eval_stub_friendly() {
    // 无参解析：flags 全默认（stage 2 起 Eval 含 --provider/--model）
    let cli = proc::cli::Cli::try_parse_from(["proc", "agent", "eval"]).unwrap();
    match cli.command {
        Some(proc::cli::Command::Agent {
            sub:
                proc::cli::def::AgentSub::Eval {
                    ref level,
                    ref scenario,
                    quick,
                    attempts,
                    max_steps,
                    ref output,
                    ref compare,
                    ref provider,
                    ref model,
                },
        }) => {
            assert!(level.is_none());
            assert!(scenario.is_empty());
            assert!(!quick);
            assert_eq!(attempts, 2);
            assert_eq!(max_steps, 10);
            assert!(output.is_none());
            assert!(compare.is_empty());
            assert!(provider.is_none());
            assert!(model.is_none());
        }
        other => panic!("expected Agent(Eval), got {other:?}"),
    }
    // dispatch（compare 模式）：读两份临时结果 JSON 产对比报告后正常返回
    // （不实跑、不 panic——stage 2 起无参 eval 会真跑，dispatch 验证改走 compare）
    let mk_run = |passed: usize| {
        let results: Vec<_> = (0..passed)
            .map(|_| proc::agent::eval::QueryResult {
                scenario: "usb".to_string(),
                level: 0,
                query: "q".to_string(),
                expected_tools: vec!["proc_ls".to_string()],
                passed: true,
                failure_mode: proc::agent::eval::FailureMode::Pass,
                chain_steps_hit: 1,
                actual_tools: vec!["proc_ls".to_string()],
                stop_cause: "end_turn".to_string(),
                final_text_head: "ok".to_string(),
                duration_ms: 1,
                attempts_used: 1,
            })
            .collect();
        proc::agent::eval::EvalRunFile {
            meta: proc::agent::eval::EvalRunMeta {
                timestamp: "2026-08-20T00:00:00Z".to_string(),
                provider: "mock".to_string(),
                provider_detail: "test".to_string(),
                attempts: 1,
                max_steps: 10,
                git_describe: "vtest".to_string(),
                quick: false,
                query_count: passed,
            },
            report: proc::agent::eval::build_report(&results),
            results,
        }
    };
    let dir = std::env::temp_dir();
    let a = dir.join(format!("proc-eval-stub-a-{}.json", std::process::id()));
    let b = dir.join(format!("proc-eval-stub-b-{}.json", std::process::id()));
    std::fs::write(&a, serde_json::to_string(&mk_run(1)).unwrap()).unwrap();
    std::fs::write(&b, serde_json::to_string(&mk_run(2)).unwrap()).unwrap();
    let sub = proc::cli::def::AgentSub::Eval {
        level: None,
        scenario: vec![],
        quick: false,
        attempts: 2,
        max_steps: 10,
        output: None,
        compare: vec![
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ],
        provider: None,
        model: None,
    };
    proc::cli::agent_cmd::run_agent(&sub);
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
}

#[test]
fn test_cli_session_info_stub_friendly() {
    let cli = proc::cli::Cli::try_parse_from(["proc", "agent", "session-info", "s.jsonl"]).unwrap();
    match cli.command {
        Some(proc::cli::Command::Agent {
            sub: proc::cli::def::AgentSub::SessionInfo { ref path },
        }) => {
            assert_eq!(path, "s.jsonl");
        }
        other => panic!("expected Agent(SessionInfo), got {other:?}"),
    }
    // dispatch（v0.22 stage 3 起真实现）：缺失文件会 exit(1)，改走真实临时
    // JSONL 文件验证正常返回路径（指标打印细节在 stage 3 测试锁）。
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("s.jsonl");
    let entry = serde_json::json!({
        "seq": 0,
        "ts_rel_ms": 0,
        "kind": "session_start",
        "provider": "mock",
        "wall_start": "2026-08-21T00:00:00Z",
    });
    std::fs::write(&file, format!("{entry}\n")).unwrap();
    let sub = proc::cli::def::AgentSub::SessionInfo {
        path: file.to_string_lossy().into_owned(),
    };
    proc::cli::agent_cmd::run_agent(&sub);
}
