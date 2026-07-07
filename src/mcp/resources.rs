//! v0.17 主题 B 可观测性 — rmcp 0.11 Resource subscribe 路由。
//!
//! v0.17 阶段 1 Spike 落地：仅含 trait 声明 + 资源 URI 常量（stub）。
//! 阶段 4 Slice 实装 ResourceRoute trait + client 订阅后 worker 1s tick 推送增量。
//!
//! 资源 URI 设计（ADR-0027）：
//! - `proc://metrics/system` — system metrics 1s tick 推送
//! - `proc://processes/list` — process list 1s tick 推送
//! - `proc://docker/events` — docker events 实时推送
//!
//! 与既有 tool 互补——tool 是 request-response（client 主动调）/ Resource 是
//! subscribe-push（client 订阅后 server 主动推），适合 sparkline / 进程列表
//! 实时监控场景。

use serde_json::Value;

/// 资源 URI 常量数组（stage 4 实装时填完整路由表）。
///
/// stage 1 Spike 仅声明常量。stage 4 实装时 [`ResourceRoute`] trait impl 块
/// 按 URI 路由到不同 handler（drain `ProcMcpHandler::snapshot` / `system_history`）。
pub const PROC_RESOURCE_URIS: &[&str] = &[
    "proc://metrics/system",
    "proc://processes/list",
    "proc://docker/events",
];

/// Resource 路由 trait（stage 4 实装）。
///
/// stage 1 Spike 仅声明 trait + `route` 方法返 "v0.17-stage-4 未实装" 错误。
/// stage 4 实装时 `ProcMcpHandler` impl 本 trait，按 URI 路由到不同 handler：
/// - `proc://metrics/system` → drain `snapshot` 字段返 system metrics JSON
/// - `proc://processes/list` → drain `snapshot` 字段返 process list JSON
/// - `proc://docker/events` → drain docker event stream 返 events JSON
///
/// client 订阅后 server 1s tick 推送增量（与 brainstorm §主题 B + TD-52 sparkline
/// 同款节奏）。
pub trait ResourceRoute {
    /// 路由资源 URI 到 JSON 响应（stage 4 实装）。
    ///
    /// stage 1 Spike 返 "v0.17-stage-4 未实装" 错误。stage 4 实装时按 URI
    /// 分支返对应 JSON snapshot。
    fn route(&self, uri: &str) -> Result<Value, String>;
}
