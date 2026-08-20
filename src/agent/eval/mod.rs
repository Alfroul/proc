//! v0.22 eval harness（ADR-0032）：70 query 基准表 + 结果类型 + 失败模式分类。
//! runner 实装在 stage 2（`proc agent eval`）；session 观测在 stage 3。
//!
//! - 数据文件 `queries.toml` 经 `include_str!` 编译进 binary（与 GBNF 同款模式）
//! - `QueryResult` / `EvalReport` 的 serde schema 即结果 JSON contract（stage 1 锁定）
//! - 判定纯函数（`classify_failure` / `tools_subsequence_hit` / `is_degraded_output`）
//!   本 stage 直接实装——纯函数无副作用；执行循环 / IO / 报告生成留 stage 2

use serde::{Deserialize, Serialize};

/// 70 query 基准表（ADR-0032 D2，编译进 binary）。
pub const EVAL_QUERIES_TOML: &str = include_str!("queries.toml");

/// fixtures 9 场景集合（scenario 名合法域，与 tests/fixtures/agent/ 一致）。
pub const FIXTURE_SCENARIOS: &[&str] = &[
    "performance-diagnose",
    "process-diagnose",
    "docker",
    "usb",
    "security",
    "recording",
    "flow",
    "monitor",
    "dns",
];

/// 基准表规模常量（加载校验断言用）。
pub const EXPECTED_TOTAL: usize = 70;
pub const EXPECTED_L0: usize = 23;
pub const EXPECTED_L1: usize = 27;
pub const EXPECTED_L2: usize = 20;

/// 输出退化的特殊 token 字面量名单（brainstorm 风险 6，2026-08-20 实测）：
/// E2B 异常上下文下把控制 token 当文本吐出（`<eos>` 数百次 / `<tool_call|>` 泄漏）。
pub const DEGRADED_TOKEN_MARKERS: &[&str] = &["<eos>", "<end_of_turn>", "<tool_call", "<turn|>"];

/// 同一片段连续重复的退化阈值（实测 `<eos>` 重复数百次级，远超此线；
/// 正常中文长文本不误伤——单测锁定）。
pub const DEGRADED_REPEAT_LIMIT: usize = 8;

/// 重复检测的片段长度上限（超过此长度的「重复」不视为退化——正常长文本里
/// 8 个长句重复的散文结构罕见，且实测退化都是短 token 级重复）。
const REPEAT_FRAGMENT_MAX_CHARS: usize = 64;

// ---------------------------------------------------------------------------
// 数据类型（serde schema = 结果 JSON contract，字段名 stage 1 锁定）
// ---------------------------------------------------------------------------

/// 单条基准 query（queries.toml 的 `[[query]]` 条目）。
#[derive(Debug, Clone, Deserialize)]
pub struct QuerySpec {
    pub scenario: String,
    pub level: u8,
    pub text: String,
    pub expected_tools: Vec<String>,
}

/// 失败模式分类（确定性，从 RunnerOutcome 判定——ADR-0032 D3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureMode {
    Pass,
    /// 无任何 tool 调用直接文字回答
    NoToolCall,
    /// 有 tool 但 expected 未命中（L0/L1）
    WrongTool,
    /// L2 链部分命中（full-chain 失败但 chain_steps_hit > 0）
    ChainIncomplete,
    /// final_text 空 / nudge 兜底文案
    EmptyAnswer,
    /// stop = MaxSteps 且未通过
    MaxSteps,
    /// LLM 调用失败（attempts 用尽）
    LlmError,
    /// 文本退化：特殊 token 字面量泄漏 / 高重复（2026-08-20 实测，优先归类）
    OutputDegraded,
}

/// per-query 结果（结果 JSON 的原子单元）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub scenario: String,
    pub level: u8,
    pub query: String,
    pub expected_tools: Vec<String>,
    pub passed: bool,
    pub failure_mode: FailureMode,
    /// L2 口径：expected 中命中的 tool 数
    pub chain_steps_hit: usize,
    /// steps trace 的 tool 名序列
    pub actual_tools: Vec<String>,
    pub stop_cause: String,
    /// 截断 ~200 chars（防 JSON 膨胀）
    pub final_text_head: String,
    pub duration_ms: u64,
    pub attempts_used: u8,
}

/// per-level 聚合（L0/L1 看 passed/total；L2 加 full-chain + chain-step 双口径）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelSummary {
    pub level: u8,
    pub total: usize,
    pub passed: usize,
    /// L2 full-chain：expected 链全部命中的 query 数（L0/L1 为 0）
    pub full_chain: usize,
    /// L2 chain-step：链步命中总数（L0/L1 为 0）
    pub chain_steps_hit: usize,
    /// L2 总链步数（L0/L1 为 0）
    pub chain_steps_total: usize,
}

/// 聚合报告（结果 JSON 的 summary 段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    /// L0/L1: pass/total；L2: full_chain + chain_step 双口径
    pub per_level: Vec<LevelSummary>,
    pub failure_histogram: Vec<(String, usize)>,
    pub total_duration_ms: u64,
}

// ---------------------------------------------------------------------------
// 加载 + 校验
// ---------------------------------------------------------------------------

/// queries.toml 的反序列化目标（`[[query]]` 数组）。
#[derive(Debug, Deserialize)]
struct EvalQueriesFile {
    query: Vec<QuerySpec>,
}

/// queries.toml 加载（include_str! 编译进 binary）+ 全量校验。
///
/// 校验规则（违反返 Err）：总数 70 / level 分布 23-27-20 / query 文本无重复 /
/// scenario ∈ fixtures 9 场景 / expected_tools 非空且 L2 ≥ 2 /
/// tool 名 ∈ agent catalog 47 名单（防拼写错）。
pub fn load_eval_queries() -> Result<Vec<QuerySpec>, String> {
    let file: EvalQueriesFile =
        toml::from_str(EVAL_QUERIES_TOML).map_err(|e| format!("queries.toml 解析失败: {e}"))?;
    let queries = file.query;

    if queries.len() != EXPECTED_TOTAL {
        return Err(format!("基准表总数 {} != {EXPECTED_TOTAL}", queries.len()));
    }
    let l0 = queries.iter().filter(|q| q.level == 0).count();
    let l1 = queries.iter().filter(|q| q.level == 1).count();
    let l2 = queries.iter().filter(|q| q.level == 2).count();
    if (l0, l1, l2) != (EXPECTED_L0, EXPECTED_L1, EXPECTED_L2) {
        return Err(format!(
            "level 分布 L0={l0}/L1={l1}/L2={l2} != {EXPECTED_L0}/{EXPECTED_L1}/{EXPECTED_L2}"
        ));
    }

    let mut seen = std::collections::HashSet::new();
    for q in &queries {
        if !seen.insert(q.text.as_str()) {
            return Err(format!("query 文本重复: {}", q.text));
        }
        if !FIXTURE_SCENARIOS.contains(&q.scenario.as_str()) {
            return Err(format!(
                "scenario {:?} 不在 fixtures 9 场景集合内",
                q.scenario
            ));
        }
        if q.expected_tools.is_empty() {
            return Err(format!("expected_tools 为空: {}", q.text));
        }
        if q.level == 2 && q.expected_tools.len() < 2 {
            return Err(format!("L2 链长度 < 2: {}", q.text));
        }
    }

    let registry = crate::agent::tools::catalog::default_registry();
    for q in &queries {
        for tool in &q.expected_tools {
            if registry.get(tool).is_none() {
                return Err(format!(
                    "expected tool {tool:?} 不在 agent catalog（query: {}）",
                    q.text
                ));
            }
        }
    }

    Ok(queries)
}

// ---------------------------------------------------------------------------
// 判定纯函数（stage 1 实装——v0.20 stage 1「trivial 纯函数直接实装」原则）
// ---------------------------------------------------------------------------

/// `classify_failure` 的输入摘要（RunnerOutcome 的 eval 视图，stage 2 执行循环组装）。
#[derive(Debug, Clone, Copy)]
pub struct OutcomeSummary<'a> {
    /// 最终回答文本（proc_finish answer 或兜底）
    pub final_text: &'a str,
    /// steps trace 的 tool 名序列（按调用顺序）
    pub actual_tools: &'a [String],
    /// StopCause 短标签（"end_turn" / "max_steps" / "empty_after_retry" / "interrupted"）
    pub stop_cause: &'a str,
    /// LLM 调用失败（attempts 用尽）
    pub llm_error: bool,
    /// final_text 是 nudge 兜底文案（EmptyAnswer 口径）
    pub nudge_fallback: bool,
}

/// 失败模式确定性判定（ADR-0032 D3）。优先级：
/// OutputDegraded（final_text 退化即整体 fail，即使 tool 命中）→ LlmError →
/// MaxSteps → EmptyAnswer → NoToolCall → WrongTool/ChainIncomplete → Pass。
pub fn classify_failure(outcome: &OutcomeSummary<'_>, expected: &[String]) -> FailureMode {
    if !outcome.final_text.is_empty() && is_degraded_output(outcome.final_text) {
        return FailureMode::OutputDegraded;
    }
    if outcome.llm_error {
        return FailureMode::LlmError;
    }
    if outcome.stop_cause == "max_steps" {
        return FailureMode::MaxSteps;
    }
    if outcome.final_text.is_empty() || outcome.nudge_fallback {
        return FailureMode::EmptyAnswer;
    }
    if outcome.actual_tools.is_empty() {
        return FailureMode::NoToolCall;
    }
    let (full_hit, steps_hit) = tools_subsequence_hit(outcome.actual_tools, expected);
    if full_hit {
        return FailureMode::Pass;
    }
    if expected.len() >= 2 && steps_hit > 0 {
        return FailureMode::ChainIncomplete;
    }
    FailureMode::WrongTool
}

/// 保序子序列判定（双指针，允许中间插其他 tool）：
/// 返回（expected 是否作为子序列全中，命中的 expected 元素数）。
pub fn tools_subsequence_hit(actual: &[String], expected: &[String]) -> (bool, usize) {
    if expected.is_empty() {
        return (false, 0);
    }
    let mut hit = 0usize;
    let mut ai = 0usize;
    for exp in expected {
        while ai < actual.len() && actual[ai] != *exp {
            ai += 1;
        }
        if ai < actual.len() {
            hit += 1;
            ai += 1;
        }
    }
    (hit == expected.len(), hit)
}

/// 输出退化检测（brainstorm 风险 6）：特殊 token 字面量名单任一子串命中，
/// 或同一片段连续重复 ≥ `DEGRADED_REPEAT_LIMIT`。
pub fn is_degraded_output(text: &str) -> bool {
    if DEGRADED_TOKEN_MARKERS.iter().any(|m| text.contains(m)) {
        return true;
    }
    has_excessive_repetition(text)
}

/// 同一片段连续重复 ≥ DEGRADED_REPEAT_LIMIT 检测（char 级，避免多字节截断）。
fn has_excessive_repetition(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n < DEGRADED_REPEAT_LIMIT {
        return false;
    }
    for frag_len in 1..=REPEAT_FRAGMENT_MAX_CHARS.min(n / DEGRADED_REPEAT_LIMIT) {
        let run = frag_len * DEGRADED_REPEAT_LIMIT;
        let mut i = 0usize;
        while i + run <= n {
            let frag = &chars[i..i + frag_len];
            let repeated = (0..DEGRADED_REPEAT_LIMIT)
                .all(|k| &chars[i + k * frag_len..i + (k + 1) * frag_len] == frag);
            if repeated {
                return true;
            }
            i += 1;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_eval_queries_ok() {
        let queries = load_eval_queries().expect("加载 + 校验应通过");
        assert_eq!(queries.len(), EXPECTED_TOTAL);
    }

    #[test]
    fn test_subsequence_hit_greedy_repeated_expected() {
        // 重复 expected tool（eject_status → kill → eject_status 型链）的贪婪匹配
        let actual: Vec<String> = ["a", "b", "a"].iter().map(|s| s.to_string()).collect();
        let expected: Vec<String> = ["a", "b", "a"].iter().map(|s| s.to_string()).collect();
        assert_eq!(tools_subsequence_hit(&actual, &expected), (true, 3));
    }

    #[test]
    fn test_subsequence_hit_empty_expected() {
        let actual: Vec<String> = vec!["a".to_string()];
        assert_eq!(tools_subsequence_hit(&actual, &[]), (false, 0));
    }

    #[test]
    fn test_is_degraded_normal_text_not_flagged() {
        let normal = "检查了容器列表，发现 postgres 容器处于 unhealthy 状态，\
                      建议查看容器日志确认健康检查失败原因。5 < 10 且 a < b 是合法比较。";
        assert!(!is_degraded_output(normal));
    }

    #[test]
    fn test_is_degraded_repetition_below_limit_not_flagged() {
        // 重复 7 次（< 阈值 8）不命中
        assert!(!is_degraded_output(&"嗯".repeat(7)));
    }

    #[test]
    fn test_is_degraded_repetition_at_limit_flagged() {
        assert!(is_degraded_output(&"嗯".repeat(DEGRADED_REPEAT_LIMIT)));
    }
}
