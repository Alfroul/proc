//! MCP `proc_*` tool — 类别 1（CLI 已有命令暴露）Args + helper。
//!
//! v0.15 cycle stage 1 Spike 仅放骨架（Args struct + stub helper），stage 2 Slice
//! 填业务逻辑。详见 [`super`] 模块文档与 `docs/stages/v0.15-stage-1.md`。
//!
//! 边界：所有命令行已有但 v0.7 未暴露的 tool 入参 / 返回 JSON 构造都在这里。
//! `#[tool]` 方法本身在 [`super::mod_rs`] 的 `#[tool_router] impl` 块里（rmcp
//! 0.11 限制：`#[tool_router]` 只收集当前 impl 块内的 `#[tool]` 方法）。

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

// ===========================================================================
// Args structs — 类别 1（9 tool）：每个 tool 一个，rmcp 用 schemars 生成 JSON
// Schema 给 LLM 看。字段文档 = schema description，写清楚 LLM 才能正确调用。
// ===========================================================================

#[derive(Deserialize, schemars::JsonSchema)]
pub struct FlowsArgs {
    /// Max flows to return. None = no limit (default 50 in CLI; MCP also defaults to 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ThrottleArgs {
    /// Target PID.
    pub pid: u32,
    /// set=true enables EcoQoS (throttle); set=false disables; None = query current state.
    #[serde(default)]
    pub set: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ExportArgs {
    /// Output format: "json" (default) | "csv".
    #[serde(default)]
    pub format: Option<String>,
    /// Sort field (same as proc_ls: cpu | mem | name | pid | disk_read | disk_write | net_sent | net_recv).
    #[serde(default)]
    pub sort: Option<String>,
    /// Max processes to export. None = no limit.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DockerEventsArgs {
    /// Max events to return (drain non-follow, default 100). None = no limit.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MonitorAddArgs {
    /// Target kind: "pid" | "port" | "command".
    pub target_kind: String,
    /// Target identifier (PID number / port number / command string).
    pub target: String,
    /// Restart policy: "notify_only" (default) | "auto_restart". Optional.
    #[serde(default)]
    pub restart_policy: Option<String>,
    /// Dry-run preview (default false = real add; true = preview without writing).
    /// v0.15 cycle 默认 dry_run=false，与既有 proc_kill / proc_pkill v0.7 契约一致。
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MonitorRemoveArgs {
    /// Monitor ID to remove.
    pub id: String,
    /// Dry-run preview (default false = real remove; true = preview without writing).
    #[serde(default)]
    pub dry_run: Option<bool>,
}

// ===========================================================================
// Stub helpers — stage 1 占位返回，stage 2 Slice 填业务逻辑。
//
// 占位格式见 v0.15-stage-1.md §决策 4：ok:true + stub:true + stage 字段让
// client（mcp-inspector）验证 schema 但不误用业务数据。
// ===========================================================================

/// `proc_flows` stub — stage 2 实装 ProcessFlow 列表（v0.10 落地跨平台 SNI / DNS）。
pub fn make_flows_json(limit: Option<usize>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_flows tool schema is registered; business logic lands in stage 2",
        "received_limit": limit,
    })
}

/// `proc_throttle` stub — stage 2 实装 EcoQoS 状态查询 + 切换。
pub fn make_throttle_json(pid: u32, set: Option<bool>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_throttle tool schema is registered; business logic lands in stage 2",
        "received_pid": pid,
        "received_set": set,
    })
}

/// `proc_export` stub — stage 2 实装 JSON / CSV 导出。
pub fn make_export_json(format: Option<&str>, sort: Option<&str>, limit: Option<usize>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_export tool schema is registered; business logic lands in stage 2",
        "received_format": format,
        "received_sort": sort,
        "received_limit": limit,
    })
}

/// `proc_docker_inspect` stub — stage 2 实装 bollard::inspect_container。
pub fn make_docker_inspect_json(name: &str) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_docker_inspect tool schema is registered; business logic lands in stage 2",
        "received_name": name,
    })
}

/// `proc_docker_images` stub — stage 2 实装本地镜像列表 + in_use 判定。
pub fn make_docker_images_json() -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_docker_images tool schema is registered; business logic lands in stage 2",
    })
}

/// `proc_docker_volumes` stub — stage 2 实装卷列表 + in_use 反查。
pub fn make_docker_volumes_json() -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_docker_volumes tool schema is registered; business logic lands in stage 2",
    })
}

/// `proc_docker_events` stub — stage 2 实装事件流 drain（一次性，非 follow）。
pub fn make_docker_events_json(limit: Option<usize>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_docker_events tool schema is registered; business logic lands in stage 2 (one-shot drain, not follow)",
        "received_limit": limit,
    })
}

/// `proc_monitor_add` stub — stage 2 实装监控配置 add（dry_run=false 默认）。
pub fn make_monitor_add_json(
    target_kind: &str,
    target: &str,
    restart_policy: Option<&str>,
    dry_run: Option<bool>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_monitor_add tool schema is registered; business logic lands in stage 2 (dry_run defaults to false)",
        "received_target_kind": target_kind,
        "received_target": target,
        "received_restart_policy": restart_policy,
        "received_dry_run": dry_run,
    })
}

/// `proc_monitor_remove` stub — stage 2 实装监控配置 remove（dry_run=false 默认）。
pub fn make_monitor_remove_json(id: &str, dry_run: Option<bool>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_monitor_remove tool schema is registered; business logic lands in stage 2 (dry_run defaults to false)",
        "received_id": id,
        "received_dry_run": dry_run,
    })
}
