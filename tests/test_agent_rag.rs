//! v0.24 stage 2：RAG 检索层集成测试（ADR-0034 D1/D3/D4 + D2 模板渲染）。
//!
//! fixture 全构造（tempdir + 真实 struct serde 序列化），不依赖真实
//! `~/.config/proc/sessions/` 与本地 eval run JSON——CI 确定性。

use proc::agent::rag::corpus::{Entry, EntrySource};
use proc::agent::rag::{RagIndex, RagParams, inject_experience};
use proc::agent::session_log::{LogEvent, SessionLogEntry};
use proc::agent::{EvalReport, EvalRunFile, EvalRunMeta, FailureMode, QueryResult};

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
