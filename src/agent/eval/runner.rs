//! eval 执行循环（v0.22 stage 2，ADR-0032 D1/D3/D4）：逐 query 走 complete 路径
//! （`runner.run()`，与 stage_3b 验收同款）+ attempts 重试 + 失败分类接线 +
//! 结果 JSON 组装（meta + per-query + 聚合报告）。
//!
//! - 单 query LlmError 用尽 attempts 不中断后续（风险 1 mitigate 2）
//! - `progress` 回调每个 query 完成时触发——CLI 侧用于 PASS/FAIL 进度行 +
//!   结果 JSON 全量重写（每 query 实时落盘，中途崩已跑数据不丢）
//! - 聚合口径（build_report）是 `--compare` 与单 run 报告的单一实现

use super::{
    FIXTURE_SCENARIOS, FailureMode, OutcomeSummary, QueryResult, QuerySpec, classify_failure,
    tools_subsequence_hit,
};
use crate::agent::runner::{AgentRunner, StopCause};

/// `QueryResult::final_text_head` 截断长度（char 级，防 JSON 膨胀）。
pub const FINAL_TEXT_HEAD_CHARS: usize = 200;

/// 结果 JSON 顶层：run meta + per-query 结果 + 聚合报告。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalRunFile {
    pub meta: EvalRunMeta,
    pub results: Vec<QueryResult>,
    pub report: super::EvalReport,
}

/// run 元信息（结果 JSON 的 meta 段）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvalRunMeta {
    /// ISO UTC（不引 chrono——record.rs 同款 epoch → civil 算法）
    pub timestamp: String,
    /// ProviderSpec.name
    pub provider: String,
    /// ProviderSpec.detail（含 model 路径 / 名）
    pub provider_detail: String,
    pub attempts: u8,
    pub max_steps: u32,
    /// `git describe --tags --always --dirty`；git 不可用 fallback CARGO_PKG_VERSION
    pub git_describe: String,
    pub quick: bool,
    pub query_count: usize,
}

/// `--level "0,2"` 解析：逗号分隔 level 集合；非法值报错（列合法值）。
pub fn parse_levels(spec: &str) -> Result<Vec<u8>, String> {
    let mut levels = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match part.parse::<u8>() {
            Ok(l) if l <= 2 => {
                if !levels.contains(&l) {
                    levels.push(l);
                }
            }
            Ok(_) => {
                return Err(format!("level {part:?} 超出范围（合法值：0 / 1 / 2）"));
            }
            Err(_) => {
                return Err(format!("level {part:?} 不是数字（合法值：0 / 1 / 2）"));
            }
        }
    }
    if levels.is_empty() {
        return Err("--level 值为空（合法值：0 / 1 / 2）".to_string());
    }
    Ok(levels)
}

/// query 选择：level / scenario 过滤 + QUICK 每 (scenario, level) 抽第 1 条
/// （stage_3b `selected_queries` 同款模式）。选完为空报错。
pub fn select_queries(
    queries: &[QuerySpec],
    levels: &[u8],
    scenarios: &[String],
    quick: bool,
) -> Result<Vec<QuerySpec>, String> {
    for s in scenarios {
        if !FIXTURE_SCENARIOS.contains(&s.as_str()) {
            return Err(format!(
                "scenario {s:?} 不在基准表 9 场景集合内（合法值：{}）",
                FIXTURE_SCENARIOS.join(" / ")
            ));
        }
    }

    let mut selected: Vec<QuerySpec> = queries
        .iter()
        .filter(|q| levels.is_empty() || levels.contains(&q.level))
        .filter(|q| scenarios.is_empty() || scenarios.iter().any(|s| s == &q.scenario))
        .cloned()
        .collect();

    if quick {
        let mut seen = std::collections::HashSet::new();
        selected.retain(|q| seen.insert((q.scenario.clone(), q.level)));
    }

    if selected.is_empty() {
        return Err("过滤后 query 为空（检查 --level / --scenario 组合）".to_string());
    }
    Ok(selected)
}

/// 逐 query 执行 + attempts 重试 + 失败分类。
///
/// `progress(result, index, total)` 在每个 query 完成时触发（含失败 / LlmError
/// 的半结果）；返回值与回调序列一致。记录**末次 attempt 的状态**（Pass 提前
/// break；失败重试后仍失败 / 最后一次 Err 以末次为准）。
pub async fn run_eval(
    runner: &AgentRunner,
    queries: &[QuerySpec],
    attempts: u8,
    progress: &mut (dyn FnMut(&QueryResult, usize, usize) + Send + Sync),
) -> Vec<QueryResult> {
    let total = queries.len();
    let mut results = Vec::with_capacity(total);
    for (i, q) in queries.iter().enumerate() {
        let r = run_one(runner, q, attempts).await;
        progress(&r, i + 1, total);
        results.push(r);
    }
    results
}

/// 单 query 的 attempts 循环（stage_3b 验收同款语义：Pass 即停，失败 / Err 重试）。
async fn run_one(runner: &AgentRunner, q: &QuerySpec, attempts: u8) -> QueryResult {
    let start = std::time::Instant::now();
    let attempts = attempts.max(1);
    let mut result: Option<QueryResult> = None;

    for attempt in 1..=attempts {
        match runner.run(&q.text).await {
            Ok(outcome) => {
                let actual_tools: Vec<String> =
                    outcome.steps.iter().map(|s| s.tool_name.clone()).collect();
                let summary = OutcomeSummary {
                    final_text: &outcome.final_text,
                    actual_tools: &actual_tools,
                    stop_cause: outcome.stop.label(),
                    llm_error: false,
                    nudge_fallback: outcome.stop == StopCause::EmptyAfterRetry,
                };
                let mode = classify_failure(&summary, &q.expected_tools);
                let (_, chain_hit) = tools_subsequence_hit(&actual_tools, &q.expected_tools);
                let passed = mode == FailureMode::Pass;
                result = Some(QueryResult {
                    scenario: q.scenario.clone(),
                    level: q.level,
                    query: q.text.clone(),
                    expected_tools: q.expected_tools.clone(),
                    passed,
                    failure_mode: mode,
                    chain_steps_hit: chain_hit,
                    actual_tools,
                    stop_cause: outcome.stop.label().to_string(),
                    final_text_head: truncate_chars(&outcome.final_text, FINAL_TEXT_HEAD_CHARS),
                    duration_ms: 0,
                    attempts_used: attempt,
                });
                if passed {
                    break;
                }
            }
            Err(e) => {
                result = Some(QueryResult {
                    scenario: q.scenario.clone(),
                    level: q.level,
                    query: q.text.clone(),
                    expected_tools: q.expected_tools.clone(),
                    passed: false,
                    failure_mode: FailureMode::LlmError,
                    chain_steps_hit: 0,
                    actual_tools: Vec::new(),
                    stop_cause: "llm_error".to_string(),
                    final_text_head: truncate_chars(
                        &format!("LLM error: {e}"),
                        FINAL_TEXT_HEAD_CHARS,
                    ),
                    duration_ms: 0,
                    attempts_used: attempt,
                });
            }
        }
    }

    // attempts >= 1 保证 result 必有值。
    let mut r = result.expect("run_one 至少执行一次 attempt");
    r.duration_ms = start.elapsed().as_millis() as u64;
    r
}

/// 聚合（`--compare` 与单 run 报告共用的单一实现，brainstorm 风险 5 mitigate 2）：
/// per_level（L2 双口径）/ failure_histogram（仅失败模式，次数降序）/ 总时长。
pub fn build_report(results: &[QueryResult]) -> super::EvalReport {
    let mut levels: Vec<u8> = results.iter().map(|r| r.level).collect();
    levels.sort_unstable();
    levels.dedup();

    let per_level = levels
        .into_iter()
        .map(|level| {
            let rows: Vec<&QueryResult> = results.iter().filter(|r| r.level == level).collect();
            super::LevelSummary {
                level,
                total: rows.len(),
                passed: rows.iter().filter(|r| r.passed).count(),
                full_chain: rows.iter().filter(|r| r.passed).count(),
                chain_steps_hit: rows.iter().map(|r| r.chain_steps_hit).sum(),
                chain_steps_total: rows.iter().map(|r| r.expected_tools.len()).sum(),
            }
        })
        .collect();

    let mut histogram: Vec<(String, usize)> = Vec::new();
    for r in results.iter().filter(|r| !r.passed) {
        let name = r.failure_mode.label().to_string();
        match histogram.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 += 1,
            None => histogram.push((name, 1)),
        }
    }
    histogram.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    super::EvalReport {
        per_level,
        failure_histogram: histogram,
        total_duration_ms: results.iter().map(|r| r.duration_ms).sum(),
    }
}

/// char 级截断（多字节安全），超长加 `…`。
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

// ---------------------------------------------------------------------------
// 时间戳 / git describe（meta 段用）
// ---------------------------------------------------------------------------

/// ISO UTC（`YYYY-MM-DDTHH:MM:SSZ`）。不引 chrono——civil 算法与
/// `src/cli/record.rs::epoch_days_to_ymd` 同款（该函数私有，此处自持一份）。
pub fn utc_timestamp_iso() -> String {
    let (y, mo, d, h, mi, s) = utc_now();
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// 紧凑 UTC（`yyyyMMdd-HHmmss`，输出文件名用）。
pub fn utc_timestamp_compact() -> String {
    let (y, mo, d, h, mi, s) = utc_now();
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

fn utc_now() -> (i64, u32, u32, u64, u64, u64) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let today = secs % 86_400;
    let (y, m, d) = epoch_days_to_ymd(days);
    (y, m, d, today / 3600, (today % 3600) / 60, today % 60)
}

/// Howard Hinnant civil_from_days（1800-2400 准确）。
fn epoch_days_to_ymd(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// `git describe --tags --always --dirty`；失败（无 git / 非 repo）fallback
/// `v{CARGO_PKG_VERSION}`。
pub fn git_describe() -> String {
    std::process::Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")))
}
