//! 47 tool catalog — 46 个既有 MCP tool + proc_help 映射为 agent ToolSchema
//! （ADR-0030 D2）。Layer 0 entry 4 个 + Layer 1 按 ToolCategory 发现。
//!
//! schema 的 description 在 stage 2 是一句话摘要（stage 3b AgentRunner dispatch
//! 实装时按需充实）；estimated_tokens = (name + description + parameters JSON
//! 字符数) / 4 向上取整到 10。

use super::help;
use crate::agent::tool_registry::ToolRegistry;
use crate::agent::types::{ToolCategory, ToolSchema};

fn tool(
    name: &str,
    description: &str,
    category: ToolCategory,
    parameters: serde_json::Value,
) -> ToolSchema {
    let json_len = name.len() + description.len() + parameters.to_string().len();
    ToolSchema {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
        category,
        estimated_tokens: json_len.div_ceil(4).next_multiple_of(10),
    }
}

fn no_params() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

/// 注册全部 47 tool（46 个 MCP tool + proc_help 元 tool）。
pub fn register_default_tools(registry: &mut ToolRegistry) {
    let all = vec![
        // ---- process（10，proc_ls 是 entry）----
        tool(
            "proc_ls",
            "List processes with cpu/memory/disk/net columns. Supports sort (cpu|mem|pid|name|security_score|disk_read|disk_write|net_sent|net_recv), limit, and filter expression (e.g. \"cpu > 5 AND name =~ /chrome/\").",
            ToolCategory::Process,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "sort": {"type": "string", "description": "Sort field, default cpu"},
                    "limit": {"type": "integer", "description": "Max rows to return"},
                    "filter": {"type": "string", "description": "FilterExpr, e.g. \"cpu > 5\""}
                }
            }),
        ),
        tool(
            "proc_tree",
            "List processes as a parent-child tree view.",
            ToolCategory::Process,
            no_params(),
        ),
        tool(
            "proc_kill",
            "Kill a process by pid. Write operation, dry_run=false default (true = preview only).",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"pid": {"type": "integer"}, "force": {"type": "boolean"}, "dry_run": {"type": "boolean"}}}),
        ),
        tool(
            "proc_pkill",
            "Kill all processes matching a name. Write operation, dry_run=false default.",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}, "force": {"type": "boolean"}, "dry_run": {"type": "boolean"}}}),
        ),
        tool(
            "proc_handles",
            "List handles (files, registry keys, synchronization objects) opened by a process.",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"pid": {"type": "integer"}}}),
        ),
        tool(
            "proc_priority",
            "Get or set process priority (realtime/high/above_normal/normal/below_normal/idle).",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"pid": {"type": "integer"}, "set": {"type": "string"}}}),
        ),
        tool(
            "proc_affinity",
            "Get or set process CPU affinity mask.",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"pid": {"type": "integer"}, "set": {"type": "integer"}}}),
        ),
        tool(
            "proc_throttle",
            "Toggle Windows EcoQoS efficiency mode for a pid (on/off/status).",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"pid": {"type": "integer"}, "mode": {"type": "string", "enum": ["on", "off", "status"]}}}),
        ),
        tool(
            "proc_export",
            "Export the process table snapshot as JSON or CSV string.",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"format": {"type": "string", "enum": ["json", "csv"]}}}),
        ),
        tool(
            "proc_who",
            "Find which process is listening on a TCP port.",
            ToolCategory::Process,
            serde_json::json!({"type": "object", "properties": {"port": {"type": "integer"}}}),
        ),
        // ---- performance（8，proc_metrics_system 是 entry）----
        tool(
            "proc_metrics_system",
            "System-wide metrics snapshot: cpu usage, memory/swap usage, disk io speed, network interfaces, tcp stats, cpu/gpu temperature, uptime.",
            ToolCategory::Performance,
            no_params(),
        ),
        tool(
            "proc_metrics_gpu",
            "Per-GPU metrics: utilization, vram usage, temperature, power draw, data providers.",
            ToolCategory::Performance,
            no_params(),
        ),
        tool(
            "proc_metrics_disk_io",
            "Disk IO metrics: total + per-disk read/write speed. Optional device filter.",
            ToolCategory::Performance,
            serde_json::json!({"type": "object", "properties": {"device": {"type": "string"}}}),
        ),
        tool(
            "proc_metrics_smart",
            "SMART disk health. device omitted = aggregated summary for all disks; device given = detailed attributes.",
            ToolCategory::Performance,
            serde_json::json!({"type": "object", "properties": {"device": {"type": "string"}}}),
        ),
        tool(
            "proc_metrics_thermal",
            "Per-core CPU frequency/temperature and throttle classification.",
            ToolCategory::Performance,
            no_params(),
        ),
        tool(
            "proc_metrics_history",
            "Last 30s sampled system history (sparkline data points for cpu/mem/swap).",
            ToolCategory::Performance,
            serde_json::json!({"type": "object", "properties": {"metric": {"type": "string", "enum": ["cpu", "memory", "swap"]}}}),
        ),
        tool(
            "proc_smart",
            "Single-device SMART attributes in detail (use proc_metrics_smart for all-disk summary).",
            ToolCategory::Performance,
            serde_json::json!({"type": "object", "properties": {"device": {"type": "string"}}}),
        ),
        tool(
            "proc_diag",
            "Worker health diagnostics: poll counts, avg/max latency, drops, crash stats, dns collector kind.",
            ToolCategory::Performance,
            no_params(),
        ),
        // ---- docker（10）----
        tool(
            "proc_docker_ps",
            "List docker containers with state/health/cpu/mem stats.",
            ToolCategory::Docker,
            no_params(),
        ),
        tool(
            "proc_docker_top",
            "Processes running inside a container.",
            ToolCategory::Docker,
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        ),
        tool(
            "proc_docker_logs",
            "Tail docker container logs.",
            ToolCategory::Docker,
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}, "tail": {"type": "integer"}}}),
        ),
        tool(
            "proc_docker_inspect",
            "Container details: config, health detail, resource stats.",
            ToolCategory::Docker,
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        ),
        tool(
            "proc_docker_images",
            "List docker images with in-use flag.",
            ToolCategory::Docker,
            no_params(),
        ),
        tool(
            "proc_docker_volumes",
            "List docker volumes with in-use reverse lookup.",
            ToolCategory::Docker,
            no_params(),
        ),
        tool(
            "proc_docker_events",
            "Drain a short window of recent docker events (non-following).",
            ToolCategory::Docker,
            serde_json::json!({"type": "object", "properties": {"limit": {"type": "integer"}}}),
        ),
        tool(
            "proc_docker_rm",
            "Remove a stopped container. Destructive: confirm=true required.",
            ToolCategory::Docker,
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}, "confirm": {"type": "boolean"}}}),
        ),
        tool(
            "proc_docker_image_rm",
            "Remove a docker image. Destructive: confirm=true required.",
            ToolCategory::Docker,
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}, "confirm": {"type": "boolean"}}}),
        ),
        tool(
            "proc_docker_volume_rm",
            "Remove a docker volume. Destructive: confirm=true required.",
            ToolCategory::Docker,
            serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}, "confirm": {"type": "boolean"}}}),
        ),
        // ---- usb（3）----
        tool(
            "proc_eject",
            "Eject a removable USB drive (kill locks optionally). Write operation.",
            ToolCategory::Usb,
            serde_json::json!({"type": "object", "properties": {"drive": {"type": "string"}, "kill": {"type": "boolean"}}}),
        ),
        tool(
            "proc_eject_status",
            "USB eject status: lock count, holder processes, and next-step suggestion (eject_now/kill_locks/unknown_drive/unavailable).",
            ToolCategory::Usb,
            serde_json::json!({"type": "object", "properties": {"drive": {"type": "string"}}}),
        ),
        tool(
            "proc_usb_release",
            "Safely release a USB drive: flush + kill locks + eject. Destructive: confirm=true required.",
            ToolCategory::Usb,
            serde_json::json!({"type": "object", "properties": {"drive": {"type": "string"}, "confirm": {"type": "boolean"}}}),
        ),
        // ---- security（1，proc_inspect 是 entry）----
        tool(
            "proc_inspect",
            "Deep-dive one process by pid. tab: summary (full fields + parent chain + signature + security score risk factors) | env (secret-masked, reveal=true to unmask) | network | dlls | memory_map | handles.",
            ToolCategory::Security,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "pid": {"type": "integer"},
                    "tab": {"type": "string", "enum": ["summary", "env", "network", "dlls", "memory_map", "handles"]},
                    "reveal": {"type": "boolean"}
                }
            }),
        ),
        // ---- recording（8）----
        tool(
            "proc_replay_info",
            "Recording file metadata: format (vt100/uiframe), frame count, duration, anomaly count.",
            ToolCategory::Recording,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        ),
        tool(
            "proc_replay_search",
            "Search recording timeline by FilterExpr (\":cpu > 80\") or substring. Returns matching frames + processes.",
            ToolCategory::Recording,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}, "query": {"type": "string"}, "limit": {"type": "integer"}}}),
        ),
        tool(
            "proc_bookmarks_list",
            "List bookmarks in a recording's sidecar file.",
            ToolCategory::Recording,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        ),
        tool(
            "proc_bookmarks_add",
            "Add a bookmark at a frame index. dry_run supported.",
            ToolCategory::Recording,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}, "frame_idx": {"type": "integer"}, "label": {"type": "string"}, "dry_run": {"type": "boolean"}}}),
        ),
        tool(
            "proc_bookmarks_edit",
            "Edit a bookmark label by id.",
            ToolCategory::Recording,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}, "id": {"type": "integer"}, "label": {"type": "string"}}}),
        ),
        tool(
            "proc_bookmarks_delete",
            "Delete a bookmark by id.",
            ToolCategory::Recording,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}, "id": {"type": "integer"}}}),
        ),
        tool(
            "proc_record_start",
            "Start a headless screen recording subprocess. confirm=true required (captures screen content).",
            ToolCategory::Recording,
            serde_json::json!({"type": "object", "properties": {"output": {"type": "string"}, "duration": {"type": "integer"}, "confirm": {"type": "boolean"}}}),
        ),
        tool(
            "proc_record_stop",
            "Stop the running headless recording and flush the file.",
            ToolCategory::Recording,
            no_params(),
        ),
        // ---- flow（2）----
        tool(
            "proc_flows",
            "List TLS/网络 flow records: pid, sni, dns_name, remote addr/port, bytes in/out.",
            ToolCategory::Flow,
            serde_json::json!({"type": "object", "properties": {"limit": {"type": "integer"}, "filter": {"type": "string"}}}),
        ),
        tool(
            "proc_port",
            "List listening/established TCP ports with owning pid and retransmission stats.",
            ToolCategory::Flow,
            no_params(),
        ),
        // ---- monitor（3）----
        tool(
            "proc_monitor_list",
            "List configured monitor rules.",
            ToolCategory::Monitor,
            no_params(),
        ),
        tool(
            "proc_monitor_add",
            "Add a monitor rule (process alive / resource threshold alerts). dry_run supported.",
            ToolCategory::Monitor,
            serde_json::json!({"type": "object", "properties": {"target_kind": {"type": "string"}, "target": {"type": "string"}, "dry_run": {"type": "boolean"}}}),
        ),
        tool(
            "proc_monitor_remove",
            "Remove a monitor rule by id.",
            ToolCategory::Monitor,
            serde_json::json!({"type": "object", "properties": {"id": {"type": "integer"}}}),
        ),
        // ---- dns（1）----
        tool(
            "proc_dns",
            "Recent DNS query log: domain, pid, process name, timestamp.",
            ToolCategory::Dns,
            serde_json::json!({"type": "object", "properties": {"limit": {"type": "integer"}}}),
        ),
    ];
    for schema in all {
        registry.register(schema);
    }
    // proc_help 元 tool（meta 类，agent 内部可见，不在 46 个 MCP tool 内）。
    registry.register(help::schema());
}

/// 构造已注册全部 47 tool 的 registry。
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    register_default_tools(&mut registry);
    registry
}
