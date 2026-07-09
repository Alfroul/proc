//! MCP `proc_metrics_history` tool — v0.17 cycle 主题 B 可观测性子 module。
//!
//! v0.17 阶段 1 Spike 落地骨架（Args struct + stub helper），阶段 4 Slice 填
//! rmcp 0.11 Resource subscribe（`proc://metrics/system` 等资源 URI 路由）+
//! SSE transport 入口 + TD-52 sparkline worker（30s 历史采样）业务逻辑。
//! 详见 [`super`] 模块文档与 `docs/stages/v0.17-stage-4.md`。
//!
//! 边界：本子 module 是 MCP handler 容器（路径 `crate::mcp::handler::observable`），
//! 与 [`crate::mcp::transport`]（SSE transport 容器）/ [`crate::mcp::resources`]
//! （ResourceRoute trait 容器）三件套共同构成主题 B 可观测性 schema。
//! `#[tool]` 方法本身在 [`super::mod_rs`] 的 `#[tool_router] impl` 块里
//! （rmcp 0.11 限制：`#[tool_router]` 只收集当前 impl 块内的 `#[tool]` 方法）。
//!
//! 与 v0.16 落地的 [`crate::mcp::handler::record`] 子模块对齐——
//! handler 子 module 仅做参数 dispatch + JSON 输出，不调 TUI 路径。

#[cfg(feature = "mcp-persistent-state")]
use std::collections::VecDeque;
#[cfg(feature = "mcp-persistent-state")]
use std::sync::{Arc, Mutex};

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

#[cfg(feature = "mcp-persistent-state")]
use super::MetricsSample;

// ===========================================================================
// Args structs — 1 tool（metrics_history）+ stage 4 加 ResourceRoute trait impl
//
// 每个 tool 一个 Args struct，rmcp 用 schemars 生成 JSON Schema 给 LLM 看。
// 字段文档 = schema description，写清楚 LLM 才能正确调用。
// ===========================================================================

/// `proc_metrics_history` tool 入参（stage 4 TD-52 sparkline 落地）。
///
/// 查询最近 N 秒的系统指标采样序列（默认 30s，上限 30 因 `system_history`
/// 字段 30s cap——见 ADR-0026 / ADR-0027）。返回 `samples: [{ ts, value }]`
/// 让 agent / 用户可视化 sparkline。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsHistoryArgs {
    /// 指标类型：`cpu` / `memory` / `swap`（stage 4 实装时按 metric 分支 drain
    /// `system_history: Arc<Mutex<VecDeque<MetricsSample>>>` 字段）。
    pub metric: String,
    /// 历史时长（秒，默认 30，上限 30 因 system_history 字段 30s cap）。
    /// None → 30；> 30 → 截断到 30。
    #[serde(default)]
    pub seconds: Option<u8>,
}

// ===========================================================================
// Helpers — stage 4 业务逻辑全部落地
//
// stage 1 Spike 落地 1 个 stub（schema 占位），stage 4 Slice 替换为真实业务
// 实现（drain `ProcMcpHandler::system_history` VecDeque + 按 metric 提取
// cpu_usage / memory_used / swap_used 数据点）。
//
// 失败路径（metric 不在 cpu/memory/swap 范围 / system_history mutex 中毒）统一走
// `super::err(msg)` 返 `{ ok: false, error: <msg> }`。空 history 返 count=0
// （worker warm-up 期间 / Default 路径 / cfg-gate 掉字段），让 agent 知道
// 「worker 未起来 / 还在 warm-up」。
// ===========================================================================

/// sparkline 30s cap 常量（与 brainstorm §主题 B + ADR-0027 §4 对齐）。
const MAX_HISTORY_SECONDS: u8 = 30;

/// `proc_metrics_history` — 实装 drain `ProcMcpHandler::system_history` VecDeque。
///
/// v0.17 stage 4 TD-52 落地。按 metric 分支提取数据点（cpu/memory/swap），
/// drain 顺序 oldest → newest，seconds 参数 None → 30 / > 30 → 截断到 30。
///
/// 返回 schema：
/// ```json
/// {
///   "ok": true,
///   "metric": "cpu",
///   "seconds": 30,
///   "count": <N>,
///   "samples": [{ "ts": <unix_seconds>, "value": <f32|u64> }, ...]  // oldest → newest
/// }
/// ```
///
/// 空 history 返 count=0 + samples=[]（worker warm-up 期间 / Default 路径）。
/// metric 不在 cpu/memory/swap 范围 → `{ ok: false, error: "unknown metric ..." }`。
/// mutex poisoned → `{ ok: false, error: "system_history mutex poisoned" }`。
///
/// **stage 4 决策 1 修正**：`system_history` 字段存 [`MetricsSample`]（Copy
/// struct 含 cpu_usage / memory_used / swap_used / timestamp）而非完整
/// `SystemSnapshot`（含 JoinHandle / Receiver 等 non-Clone 字段）。worker 每
/// tick 从 snapshot 提取 4 个标量 push，零 alloc 开销。
#[cfg(feature = "mcp-persistent-state")]
pub fn make_metrics_history_json(
    metric: &str,
    seconds: Option<u8>,
    history: &Arc<Mutex<VecDeque<MetricsSample>>>,
) -> Value {
    if !["cpu", "memory", "swap"].contains(&metric) {
        return super::err(format!(
            "unknown metric '{metric}' (valid: cpu / memory / swap)"
        ));
    }

    let seconds_clamped = seconds
        .unwrap_or(MAX_HISTORY_SECONDS)
        .min(MAX_HISTORY_SECONDS);

    let samples: Vec<Value> = match history.lock() {
        Ok(g) => {
            let take = g.len().min(seconds_clamped as usize);
            // oldest → newest：iter().rev().take(N) 拿最后 N 个（newest 端），
            // 再 .rev() 翻回 oldest-first 顺序。如 len < seconds，take = len。
            g.iter()
                .rev()
                .take(take)
                .rev()
                .map(|s| sample_to_json(s, metric))
                .collect()
        }
        Err(_) => return super::err("system_history mutex poisoned"),
    };

    let count = samples.len();
    json!({
        "ok": true,
        "metric": metric,
        "seconds": seconds_clamped,
        "count": count,
        "samples": samples,
    })
}

/// `proc_metrics_history` — `--no-default-features` 路径 fallback（无 history 字段）。
///
/// v0.17 stage 4：`mcp-persistent-state` feature gate 让 `--no-default-features`
/// 路径 `ProcMcpHandler` 不含 `system_history` 字段，本 helper 返 placeholder
/// 让 client（mcp-inspector）验证 schema 但不误用业务数据。
///
/// 返回 schema：`{ ok: true, metric, seconds, count: 0, samples: [], note }`。
pub fn make_metrics_history_json_no_state(metric: &str, seconds: Option<u8>) -> Value {
    let seconds_clamped = seconds
        .unwrap_or(MAX_HISTORY_SECONDS)
        .min(MAX_HISTORY_SECONDS);
    json!({
        "ok": true,
        "metric": metric,
        "seconds": seconds_clamped,
        "count": 0,
        "samples": [],
        "note": "mcp-persistent-state feature disabled; build with default features to enable sparkline history",
    })
}

/// 内部 helper：MetricsSample → JSON 对象（按 metric 名选 value 字段）。
///
/// cpu → f32 cpu_usage / memory → u64 memory_used / swap → u64 swap_used。
/// metric 名已在外部 helper 校验，本 helper 不再校验。
#[cfg(feature = "mcp-persistent-state")]
fn sample_to_json(sample: &MetricsSample, metric: &str) -> Value {
    let value = match metric {
        "cpu" => json!(sample.cpu_usage),
        "memory" => json!(sample.memory_used),
        "swap" => json!(sample.swap_used),
        _ => json!(null),
    };
    json!({
        "ts": sample.timestamp_unix,
        "value": value,
    })
}
