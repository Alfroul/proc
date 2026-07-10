//! v0.17 主题 B 可观测性 — rmcp 0.11 Resource subscribe 路由。
//!
//! v0.17 阶段 1 Spike 落地：仅含 trait 声明 + 资源 URI 常量（stub）。
//! v0.17 阶段 4 Slice 实装 [`ResourceRoute`] trait impl for
//! [`crate::mcp::handler::ProcMcpHandler`] + client 通过 MCP 协议
//! `resources/list` / `resources/read` 路由到 3 个 URI（详见决策 3）。
//!
//! 资源 URI 设计（ADR-0027）：
//! - `proc://metrics/system` — system metrics 1s tick 推送（client polling via read_resource）
//! - `proc://processes/list` — process list 1s tick 推送（client polling via read_resource）
//! - `proc://docker/events` — docker events 实时推送（client polling via read_resource）
//!
//! 与既有 tool 互补——tool 是 request-response（client 主动调）/ Resource 是
//! subscribe-push（client 订阅后 server 主动推）。但 stage 4 决策 5：subscribe
//! 接受请求但不 push（与 SSE transport partial 落地配套），client 走 polling
//! `resources/read` 拿数据。
//!
//! **v0.18 stage 1 Spike 扩**：trait 加 `subscribe` / `unsubscribe` 方法签名
//! （stub 返 Err "v0.18-stage-2 未实装"，与 brainstorm 项 3 决策对齐）。stage 2
//! 实装真正的 subscribe-push worker lifecycle（[`crate::mcp::subscribe_worker`]）：
//! client subscribe → worker 持 Peer 句柄注册到 `subscribers` HashMap → 1s tick
//! 调 `peer.notify_resource_updated` push 增量 → client 断开自动清理。详见
//! ADR-0027 §关键设计点 5（v0.18 stage 1 Spike 调研结论：rmcp 0.11 `ServerHandler`
//! trait 不暴露 `subscribe_resource` 方法，server 主动 push 走 `Peer::notify_resource_updated`
//! notification 路径，自建 worker lifecycle 必要）。

use serde_json::Value;

use crate::mcp::handler::ProcMcpHandler;

/// 资源 URI 常量数组。
///
/// v0.17 stage 4 实装：3 个 URI 路由到不同 handler：
/// - `proc://metrics/system` → drain `ProcMcpHandler::snapshot` 字段（fallback 现场
///   new）返 system metrics JSON（与 `proc_metrics_system` tool 同款 schema）
/// - `proc://processes/list` → drain `ProcMcpHandler::snapshot` 字段返 process
///   list JSON（top 50 by cpu_usage，与 `proc_ls` tool 同款 schema 但默认 limit=50）
/// - `proc://docker/events` → `cli::make_docker_events_json(Some(50))`（与
///   `proc_docker_events` tool 同款 schema）
pub const PROC_RESOURCE_URIS: &[&str] = &[
    "proc://metrics/system",
    "proc://processes/list",
    "proc://docker/events",
];

/// 资源 URI 的人类可读名字（list_resources 时返）。
pub fn resource_name_for_uri(uri: &str) -> &'static str {
    match uri {
        "proc://metrics/system" => "System Metrics",
        "proc://processes/list" => "Process List (top 50 by cpu)",
        "proc://docker/events" => "Docker Daemon Events",
        _ => "Unknown",
    }
}

/// 资源 URI 的描述（list_resources 时返）。
pub fn resource_description_for_uri(uri: &str) -> &'static str {
    match uri {
        "proc://metrics/system" => {
            "Real-time system metrics snapshot (cpu/memory/swap/disk/network/temperature). \
             Poll via resources/read."
        }
        "proc://processes/list" => {
            "Real-time process list snapshot (top 50 by cpu_usage). Poll via resources/read."
        }
        "proc://docker/events" => {
            "Recent Docker daemon events (one-shot per read; not follow mode). \
             Poll via resources/read."
        }
        _ => "Unknown resource",
    }
}

/// Resource 路由 trait（proc 内部 trait，不是 rmcp 0.11 trait）。
///
/// v0.17 stage 4 实装：`ProcMcpHandler` impl 本 trait，按 URI 路由到不同 handler
/// 返 JSON snapshot。`ProcMcpHandler` 同时 impl rmcp 0.11 `ServerHandler::read_resource`
/// 方法，内部调本 trait 的 `route()` 让单一入口路由（ADR-0027 设计）。
///
/// 与既有 tool 互补——tool 是 request-response / Resource 是 subscribe-push
/// （stage 4 实装 polling-push 配套，详见决策 5）。
///
/// **v0.18 stage 1 Spike 扩**：加 `subscribe` / `unsubscribe` 方法签名（stub 返
/// Err "v0.18-stage-2 未实装"）。stage 2 实装真正的 subscribe-push worker
/// lifecycle——subscribe 时把 client Peer 句柄注册到
/// [`crate::mcp::subscribe_worker::SubscribePushWorker`] / 1s tick 调
/// `peer.notify_resource_updated` push 增量 / unsubscribe 或 client 断开自动清理。
pub trait ResourceRoute {
    /// 路由资源 URI 到 JSON 响应（polling-push 路径，client 走 `resources/read`）。
    ///
    /// 按 URI 分支返对应 JSON snapshot：
    /// - `proc://metrics/system` → drain `snapshot` 字段（fallback 现场 new）返
    ///   metrics system JSON
    /// - `proc://processes/list` → drain `snapshot` 字段返 process list JSON（top 50）
    /// - `proc://docker/events` → docker events JSON（recent 50 events）
    /// - 其他 URI → Err 含 valid URI 列表
    fn route(&self, uri: &str) -> Result<Value, String>;

    /// subscribe-push 路径注册（v0.18 stage 1 Spike stub）。
    ///
    /// client 调 `resources/subscribe` → 本方法把 client Peer 句柄注册到
    /// [`crate::mcp::subscribe_worker::SubscribePushWorker`] → worker 1s tick
    /// 调 `peer.notify_resource_updated` push 增量。
    ///
    /// **当前 stage 1 Spike 仅返 Err "v0.18-stage-2 未实装"**——stage 2 实装时
    /// 替换为：worker 注册表 add + 返 Ok 含 SubscriberId 让 client 后续 unsubscribe。
    /// 与 ADR-0027 §关键设计点 5 + brainstorm 决策 3 拍板对齐。
    fn subscribe(&self, uri: &str, subscriber_id: u64) -> Result<(), String> {
        // stage 1 Spike stub：保留签名让 stage 2 直接填充业务逻辑
        let _ = (uri, subscriber_id);
        Err("v0.18-stage-2 未实装：subscribe-push worker lifecycle 留 stage 2 Slice".to_string())
    }

    /// subscribe-push 路径注销（v0.18 stage 1 Spike stub）。
    ///
    /// client 调 `resources/unsubscribe` 或网络断开 → 本方法从 worker 注册表移除。
    /// **当前 stage 1 Spike 仅返 Err "v0.18-stage-2 未实装"**——stage 2 实装时
    /// 替换为：worker 注册表 remove + 返 Ok。
    fn unsubscribe(&self, subscriber_id: u64) -> Result<(), String> {
        // stage 1 Spike stub：保留签名让 stage 2 直接填充业务逻辑
        let _ = subscriber_id;
        Err("v0.18-stage-2 未实装：subscribe-push worker lifecycle 留 stage 2 Slice".to_string())
    }
}

impl ResourceRoute for ProcMcpHandler {
    fn route(&self, uri: &str) -> Result<Value, String> {
        match uri {
            "proc://metrics/system" => {
                // 优先 drain 持久 snapshot 字段（生产路径），fallback 现场 new
                #[cfg(feature = "mcp-persistent-state")]
                {
                    if let Ok(guard) = self.snapshot.lock() {
                        if let Some(s) = guard.as_ref() {
                            return Ok(
                                crate::mcp::handler::metrics::metrics_system_json_from_snapshot(s),
                            );
                        }
                    }
                }
                Ok(crate::mcp::handler::metrics::make_metrics_system_json())
            }
            "proc://processes/list" => {
                // top 50 by cpu_usage（与 brainstorm §主题 B 表对齐）
                #[cfg(feature = "mcp-persistent-state")]
                {
                    if let Ok(guard) = self.snapshot.lock() {
                        if let Some(s) = guard.as_ref() {
                            return Ok(crate::mcp::handler::processes_json_from_snapshot(
                                s,
                                Some("cpu"),
                                Some(50),
                            ));
                        }
                    }
                }
                Ok(crate::mcp::handler::make_processes_json(
                    Some("cpu"),
                    Some(50),
                ))
            }
            "proc://docker/events" => {
                // 与 proc_docker_events tool 同款路径（500ms 窗口采 + limit 50）
                Ok(crate::mcp::handler::cli::make_docker_events_json(Some(50)))
            }
            _ => Err(format!(
                "unknown resource URI: {uri} (valid: {})",
                PROC_RESOURCE_URIS.join(", ")
            )),
        }
    }

    // subscribe / unsubscribe 用 trait 默认实现（stage 1 Spike stub），
    // stage 2 实装时在 impl 块内 override 替换为业务逻辑。
}
