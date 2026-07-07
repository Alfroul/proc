//! MCP `proc_metrics_history` tool — v0.17 cycle 主题 B 可观测性子 module。
//!
//! v0.17 阶段 1 Spike 落地骨架（Args struct + stub helper），阶段 4 Slice 填
//! rmcp 0.11 Resource subscribe（`proc://metrics/system` 等资源 URI 路由）+
//! SSE transport 入口 + TD-52 sparkline worker（30s 历史采样）业务逻辑。
//! 详见 [`super`] 模块文档与 `docs/stages/v0.17-stage-1.md`。
//!
//! 边界：本子 module 是 MCP handler 容器（路径 `crate::mcp::handler::observable`），
//! 与 [`crate::mcp::transport`]（SSE transport 容器）/ [`crate::mcp::resources`]
//! （ResourceRoute trait 容器）三件套共同构成主题 B 可观测性 schema。
//! `#[tool]` 方法本身在 [`super::mod_rs`] 的 `#[tool_router] impl` 块里
//! （rmcp 0.11 限制：`#[tool_router]` 只收集当前 impl 块内的 `#[tool]` 方法）。
//!
//! 与 v0.16 落地的 [`crate::mcp::handler::record`] 子模块对齐——
//! handler 子 module 仅做参数 dispatch + JSON 输出，不调 TUI 路径。

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

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
    /// `system_history: Arc<Mutex<VecDeque<SystemSnapshot>>>` 字段）。
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
// 失败路径（metric 不在 cpu/memory/swap 范围 / system_history 为空）统一走
// `super::err(msg)` 返 `{ ok: false, error: <msg> }`。
// ===========================================================================

/// `proc_metrics_history` — stub helper（stage 4 替换为真实业务实现）。
///
/// stage 1 Spike 返 placeholder JSON：`{ ok: true, stub: true, stage:
/// "v0.17-stage-4", message, received_* }` 让 client（mcp-inspector）验证
/// schema 正确生成 + 让 LLM 识别「这是占位返回」避免误用业务数据。
///
/// stage 4 实装时替换为 drain `ProcMcpHandler::system_history`（30s cap 的
/// `VecDeque<SystemSnapshot>`）+ 按 metric 分支提取数据点 + 返
/// `{ ok: true, metric, samples: [{ ts, value }] }`。
pub fn make_metrics_history_json(_metric: &str, _seconds: Option<u8>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.17-stage-4",
        "message": "proc_metrics_history tool schema is registered; business logic lands in stage 4",
        "received_metric": _metric,
        "received_seconds": _seconds,
    })
}
