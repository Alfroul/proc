//! MCP server handler — 所有 `proc_*` tool 的实现。
//!
//! 每个 tool 都是 thin wrapper：直接调 `crate::collect` / `crate::port_map` 等
//! 数据采集层，把结构化结果序列化成 JSON 返回给 MCP client。
//! 不调 `crate::cli::*::run_*`（那些函数直接 println! 表格，对 LLM 无意义）。
//!
//! 详细设计见 ADR-0009（v0.7 MCP server）与 ADR-0023 / ADR-0024（v0.15 cycle
//! inspect tool 合并 + handler 子 module 拆分决策）。
//!
//! # 模块结构（v0.15 阶段 1 落地）
//!
//! 顶层 [`handler`] 模块拆 4 个文件（rmcp 0.11 限制：`#[tool_router]` 只收集
//! 当前 impl 块内 `#[tool]` 方法，不支持跨 module 收集，所以 32 个 `#[tool]`
//! 都保留在本文件的 `#[tool_router] impl` 块里）：
//!
//! - `mod.rs`（本文件）：`ProcMcpHandler` struct / `Clone` / `Default` / `new`
//!   / `#[tool_router] impl`（32 个 `#[tool]` 方法 = 17 v0.7 既有 + 15 v0.15
//!   新增 stub）/ `ServerHandler` impl / `serve()` / `list_tool_names()` /
//!   既有 17 helper / 公共 helper（`ok_result` / `err` / `parse_sort_field`）
//! - [`cli`]：v0.15 类别 1（CLI 命令暴露，9 tool）Args + stub helper
//! - [`inspect`]：v0.15 类别 2（详情页 6 Tab 合并，1 tool）Args + `InspectTab`
//!   enum + stub helper
//! - [`metrics`]：v0.15 类别 4（系统级 metrics，5 tool）Args + stub helper

pub mod cli;
pub mod inspect;
pub mod metrics;

use std::sync::{Arc, Mutex};

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content, ServerInfo},
    schemars, serve_server, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::collect::{self, SortField};
use crate::dns_log::DnsLogCollector;

// v0.15 阶段 1 子 module re-export：让本文件 `#[tool]` 方法可以直接调
// `cli::make_flows_json(...)` / `inspect::make_inspect_json(...)` /
// `metrics::make_metrics_system_json()`，不需要 `self::cli::` 前缀。
use cli::*;
use inspect::*;
use metrics::*;

/// MCP server 主入口（runtime 已在 [`super::run_mcp_serve`] 起好）。
///
/// `serve_server(handler, (stdin, stdout))` 阻塞直到 client 关闭流。
///
/// **TD-36（v0.12 阶段 5）**：用 [`ProcMcpHandler::new`] 构造（启动持久 DNS
/// collector），不再用 unit struct 字面量。
pub async fn serve() -> anyhow::Result<()> {
    let (stdin, stdout) = rmcp::transport::io::stdio();
    let service = serve_server(ProcMcpHandler::new(), (stdin, stdout))
        .await
        .map_err(|e| anyhow::anyhow!("MCP server init failed: {e:?}"))?;
    service.waiting().await.ok();
    Ok(())
}

/// 列出所有已注册 tool 的名字 — 给集成测试用（验证 tool 数 ≥ 17）。
#[must_use]
pub fn list_tool_names() -> Vec<String> {
    ProcMcpHandler::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// MCP handler。
///
/// **TD-36（v0.12 阶段 5）**：handler 加持久 DNS collector 字段。v0.7 阶段 2
/// 设计为「每个 tool 现场采集」，但 `proc_dns` 的「现场 spawn EtwDnsCollector」
/// 路径让 collector 启动前的 DNS 查询抓不到（ProcessTrace 起来后才能采事件）。
/// 持久 collector 与 server 同生命周期，client 任何时刻调 `proc_dns` 都能 drain
/// 到 server 启动以来累积的所有 DNS 事件（`App::workers.dns_log_worker` 同款
/// 行为）。
///
/// - `Arc<Mutex<...>>` 让 rmcp 内部每次 tool call clone handler 时共享同一
///   collector 实例（不重复 spawn / 不丢事件）。
/// - `Option<...>` 让 [`Default`]（测试路径）不强制 spawn collector——避免单测
///   里跑 ETW session / PowerShell 子进程污染输出。
/// - 生产入口是 [`ProcMcpHandler::new`]（[`serve`] 调用），构造时调
///   `detect_collector()` 一次，结果 move 进 Arc。
pub struct ProcMcpHandler {
    /// TD-36：持久 DNS collector。生产入口 [`ProcMcpHandler::new`] 启动 collector；
    /// [`Default`]（测试路径）保持 `None`。`pub` 让集成测试能访问（验证 Arc 共享
    /// 语义）；生产代码不应直接修改。
    pub dns_collector: Arc<Mutex<Option<Box<dyn DnsLogCollector>>>>,
}

impl Clone for ProcMcpHandler {
    fn clone(&self) -> Self {
        Self {
            dns_collector: Arc::clone(&self.dns_collector),
        }
    }
}

impl Default for ProcMcpHandler {
    fn default() -> Self {
        // 测试 / 未启用 DNS 路径用：不 spawn collector，proc_dns 调用走「无 collector」
        // 错误返回。生产路径必须用 [`Self::new`]。
        Self {
            dns_collector: Arc::new(Mutex::new(None)),
        }
    }
}

impl ProcMcpHandler {
    /// 生产入口：spawn 持久 DNS collector（Windows admin → ETW；Windows 非 admin
    /// → PowerShell fallback；其它平台 → None）。collector 与 handler 同生命周期，
    /// `proc_dns` tool call 通过 `Arc::clone` 共享同一实例。
    #[must_use]
    pub fn new() -> Self {
        let (collector, _kind) = crate::dns_log::detect_collector();
        Self {
            dns_collector: Arc::new(Mutex::new(collector)),
        }
    }
}

// ===========================================================================
// Args structs — 每个 tool 一个，rmcp 用 schemars 生成 JSON Schema 给 LLM 看。
// 字段文档 = schema description，写清楚 LLM 才能正确调用。
// ===========================================================================

#[derive(Deserialize, schemars::JsonSchema)]
struct LsArgs {
    /// Sort field: cpu | mem | name | pid | disk_read | disk_write | net_sent | net_recv
    #[serde(default)]
    sort: Option<String>,
    /// Max processes to return. None = no limit.
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PortArgs {
    /// Filter to a specific local port. None = list all listening/established ports.
    #[serde(default)]
    port: Option<u16>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct KillArgs {
    /// Target PID.
    pid: u32,
    /// Force-kill the whole process tree (children first).
    #[serde(default)]
    force: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PkillArgs {
    /// Process name to match (case-insensitive exact match, e.g. "chrome.exe").
    name: String,
    /// Force-kill matched processes (whole tree).
    #[serde(default)]
    force: bool,
    /// If true, list matches without killing.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct EjectArgs {
    /// Drive letter (e.g. "E"). None = list all removable devices.
    #[serde(default)]
    drive: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct WhoArgs {
    /// Absolute file / directory path to look up.
    target_path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct HandlesArgs {
    /// PID to enumerate handles for.
    #[serde(default)]
    pid: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct PriorityArgs {
    /// Target PID.
    pid: u32,
    /// Set priority: idle | belownormal | normal | abovenormal | high | realtime.
    /// None = query current priority.
    #[serde(default)]
    set: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct AffinityArgs {
    /// Target PID.
    pid: u32,
    /// Set affinity mask (hex, e.g. "0xFF"). None = query current affinity.
    #[serde(default)]
    set: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SmartArgs {
    /// Device path (e.g. "/dev/sda", "PhysicalDrive0"). None = list all disks.
    #[serde(default)]
    device: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct DnsArgs {
    /// Drain recent queries from the in-memory buffer. tail=true is rejected (LLM can't stream).
    #[serde(default)]
    tail: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct DockerNameArgs {
    /// Container name or short ID.
    name: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct DockerLogsArgs {
    /// Container name or short ID.
    name: String,
    /// Number of lines from the end (e.g. "100"). None = all.
    #[serde(default)]
    tail: Option<String>,
}

// ===========================================================================
// Tool implementations — `#[tool_router]` 在编译期收集所有 `#[tool]` 标注的方法
// 并生成 `tool_router()` 关联函数。ServerHandler impl 通过 `#[tool_handler]`
// 复用这个 router 派发 `tools/call` 请求。
// ===========================================================================

#[tool_router]
impl ProcMcpHandler {
    #[tool(
        name = "proc_ls",
        description = "List processes, sorted by cpu/mem/name/pid/disk_read/disk_write/net_sent/net_recv. Returns JSON { ok, sort, count, processes[] }. Fields: pid/name/cpu_usage/memory_bytes/memory_pct/disk_read_bps/disk_write_bps/net_sent_bps/net_recv_bps/status/parent_pid/start_time_unix/run_time_secs/cmd. Verbose fields (exe/cwd/user_id) omitted to avoid leaking sensitive paths."
    )]
    fn proc_ls(&self, Parameters(args): Parameters<LsArgs>) -> Result<CallToolResult, McpError> {
        ok_result(make_processes_json(args.sort.as_deref(), args.limit))
    }

    #[tool(
        name = "proc_tree",
        description = "Build the full process tree (parent → children). Returns JSON { ok, roots[] } where each node has { pid, name, cpu_usage, memory_bytes, status, children[] }. Useful for understanding process ancestry."
    )]
    fn proc_tree(&self) -> Result<CallToolResult, McpError> {
        ok_result(make_process_tree_json())
    }

    #[tool(
        name = "proc_port",
        description = "List TCP/UDP port mappings (PID/process bound to each port). Pass `port` to filter a specific local port. Returns JSON { ok, count, ports[] } with { protocol, local_addr, local_port, remote_addr, remote_port, state, pid, process_name }."
    )]
    fn proc_port(
        &self,
        Parameters(args): Parameters<PortArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_port_json(args.port))
    }

    #[tool(
        name = "proc_kill",
        description = "Kill a process by PID. force=true walks the tree and kills children first. Returns JSON { ok, pid, force, result } where result is one of: Killed | AlreadyGone | AccessDenied | Failed(message). Use proc_pkill for name-based kill."
    )]
    fn proc_kill(
        &self,
        Parameters(args): Parameters<KillArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_kill_json(args.pid, args.force))
    }

    #[tool(
        name = "proc_pkill",
        description = "Find and optionally kill processes by name (case-insensitive exact match). dry_run=true lists matches without killing. Returns JSON { ok, total, killed, failed, results[] } with per-PID outcome. exit code semantics: partial success still returns ok=true but failed>0."
    )]
    fn proc_pkill(
        &self,
        Parameters(args): Parameters<PkillArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_pkill_json(&args.name, args.force, args.dry_run))
    }

    #[tool(
        name = "proc_eject",
        description = "USB / removable device helper. drive=None lists all removable devices with their lock status. drive=\"E\" (drive letter) scans processes locking that volume. Returns JSON { ok, devices[] | locks[] }. Windows-only; other platforms return ok=false with error message."
    )]
    fn proc_eject(
        &self,
        Parameters(args): Parameters<EjectArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_eject_json(args.drive.as_deref()))
    }

    #[tool(
        name = "proc_who",
        description = "Reverse-lookup: which processes hold a lock on this file/directory? Returns JSON { ok, count, lockers[] } with { pid, name, kind, handle_path }. Requires admin privileges to see system process locks."
    )]
    fn proc_who(&self, Parameters(args): Parameters<WhoArgs>) -> Result<CallToolResult, McpError> {
        ok_result(make_who_json(&args.target_path))
    }

    #[tool(
        name = "proc_handles",
        description = "Enumerate all open handles for a process. Returns JSON { ok, count, handles[] } with { kind, name, raw_handle, granted_access }. kind is one of File/RegistryKey/Event/Semaphore/Mutant/Section/Process/Thread/Token/Other. Requires admin privileges for system processes."
    )]
    fn proc_handles(
        &self,
        Parameters(args): Parameters<HandlesArgs>,
    ) -> Result<CallToolResult, McpError> {
        match args.pid {
            Some(pid) => ok_result(make_handles_json(pid)),
            None => ok_result(json!({
                "ok": false,
                "error": "pid is required (use proc_who for file-based reverse lookup)"
            })),
        }
    }

    #[tool(
        name = "proc_priority",
        description = "Get or set process priority class. set=None queries current priority; set=\"high\" changes it. Valid values: idle | belownormal | normal | abovenormal | high | realtime. Returns JSON { ok, pid, action: \"get\"|\"set\", priority? | result? }."
    )]
    fn proc_priority(
        &self,
        Parameters(args): Parameters<PriorityArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_priority_json(args.pid, args.set.as_deref()))
    }

    #[tool(
        name = "proc_affinity",
        description = "Get or set CPU affinity mask. set=None queries current mask; set=\"0xFF\" (hex) changes it. Returns JSON { ok, pid, action, affinity_mask? | result? }. mask is hex string; count_ones gives CPU core count."
    )]
    fn proc_affinity(
        &self,
        Parameters(args): Parameters<AffinityArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_affinity_json(args.pid, args.set.as_deref()))
    }

    #[tool(
        name = "proc_smart",
        description = "SMART disk health. device=None lists all disks with summary; device=\"/dev/sda\" or \"PhysicalDrive0\" returns detailed attributes. Returns JSON { ok, disks[] | disk } with { device, model, serial, temperature, health, attributes[] }. health is one of Ok/Warning/Critical/Unknown."
    )]
    fn proc_smart(
        &self,
        Parameters(args): Parameters<SmartArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_smart_json(args.device.as_deref()))
    }

    #[tool(
        name = "proc_dns",
        description = "Drain recent DNS queries from the in-memory buffer (Windows: PowerShell Get-WinEvent; other platforms return ok=false). tail=true is rejected — use the CLI for streaming. Privacy: queries are kept in memory only, never persisted. Returns JSON { ok, count, queries[] } with { query_name, query_type, pid, process_name, timestamp_unix }."
    )]
    fn proc_dns(&self, Parameters(args): Parameters<DnsArgs>) -> Result<CallToolResult, McpError> {
        // TD-36（v0.12 阶段 5）：从 handler 持久 collector drain，不再现场 spawn。
        ok_result(make_dns_json_from_collector(&self.dns_collector, args.tail))
    }

    #[tool(
        name = "proc_diag",
        description = "Worker diagnostics: avg/max poll latency, poll count, channel-full drops, last error for every background worker (port/usb/net_flow/dns_log/docker). Returns JSON { ok, workers[], dns_collector }. Attach to bug reports — see ADR-0009."
    )]
    fn proc_diag(&self) -> Result<CallToolResult, McpError> {
        ok_result(make_diag_json())
    }

    #[tool(
        name = "proc_monitor_list",
        description = "List configured process/port/command monitors (does NOT include watchdog state from a separate TUI session — this is the static config). Returns JSON { ok, count, monitors[] } with { id, target_kind, target, pid?, status, crash_count, restart_policy }."
    )]
    fn proc_monitor_list(&self) -> Result<CallToolResult, McpError> {
        ok_result(make_monitor_list_json())
    }

    #[tool(
        name = "proc_docker_ps",
        description = "List all Docker containers (running + stopped). Returns JSON { ok, count, containers[] } with { id, name, image, state, status, health, cpu_percent, memory_usage, running_since? }. Returns ok=false if Docker daemon is unreachable."
    )]
    fn proc_docker_ps(&self) -> Result<CallToolResult, McpError> {
        ok_result(make_docker_ps_json())
    }

    #[tool(
        name = "proc_docker_top",
        description = "List processes inside a Docker container (docker top equivalent). Returns JSON { ok, count, processes[] } with { pid, user, command, cpu_time }."
    )]
    fn proc_docker_top(
        &self,
        Parameters(args): Parameters<DockerNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_docker_top_json(&args.name))
    }

    #[tool(
        name = "proc_docker_logs",
        description = "Fetch Docker container logs (one-shot, NOT follow mode — use CLI for tail -f). Returns JSON { ok, count, lines[] } with { timestamp?, message, is_stderr }. tail=\"100\" limits to last N lines; None = all."
    )]
    fn proc_docker_logs(
        &self,
        Parameters(args): Parameters<DockerLogsArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_docker_logs_json(&args.name, args.tail.as_deref()))
    }

    // ====================================================================
    // v0.15 阶段 1 stub 方法 — schema 注册占位，业务逻辑在 stage 2 / stage 3
    // 各 Slice 填（详见 docs/stages/v0.15-stage-1.md）。stub 返
    // `{ ok: true, stub: true, stage: "v0.15-stage-X", ... }` placeholder 让
    // client（mcp-inspector）验证 schema 但不误用业务数据。
    // ====================================================================

    #[tool(
        name = "proc_flows",
        description = "List end-to-end network flows (ProcessFlow: pid + sni + dns_name + remote_addr/port + bytes_out/in + first/last_seen). Spawns a short-lived collector with 2s warm-up (Schannel ETW on Windows). Returns JSON { ok, count, worker, flows[] } — worker='unavailable' when Schannel worker cannot start (non-admin / x86 / session busy)."
    )]
    fn proc_flows(
        &self,
        Parameters(args): Parameters<FlowsArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_flows_json(args.limit))
    }

    #[tool(
        name = "proc_throttle",
        description = "Get or set Windows 11 EcoQoS / Efficiency Mode for a process. set=true throttles (🍃), set=false un-throttles, set=None queries current state. Returns JSON { ok, pid, action: 'get'|'set', state: 'Normal'|'Eco'|'Unknown' }. Windows 11 only (ADR-0022)."
    )]
    fn proc_throttle(
        &self,
        Parameters(args): Parameters<ThrottleArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_throttle_json(args.pid, args.set))
    }

    #[tool(
        name = "proc_export",
        description = "Export process list to JSON or CSV (defaults to JSON; the payload field holds the formatted output — agent decides whether to write a file). format='csv' for CSV. sort/limit follow the same semantics as proc_ls. Returns JSON { ok, format, sort, count, payload }."
    )]
    fn proc_export(
        &self,
        Parameters(args): Parameters<ExportArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_export_json(
            args.format.as_deref(),
            args.sort.as_deref(),
            args.limit,
        ))
    }

    #[tool(
        name = "proc_docker_inspect",
        description = "Docker container inspect: container info (id/name/image/state/health/ports) + health_detail + stats (cpu_percent/memory/network). Returns JSON { ok, container, health_detail, stats } or { ok: false, error } if container not found / Docker daemon unavailable."
    )]
    fn proc_docker_inspect(
        &self,
        Parameters(args): Parameters<DockerNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_docker_inspect_json(&args.name))
    }

    #[tool(
        name = "proc_docker_images",
        description = "List local Docker images with in_use flag (reverse-lookup from containers count). Returns JSON { ok, count, images[] } with { id, short_id, repo_tags, created, size, containers, in_use }."
    )]
    fn proc_docker_images(&self) -> Result<CallToolResult, McpError> {
        ok_result(make_docker_images_json())
    }

    #[tool(
        name = "proc_docker_volumes",
        description = "List Docker volumes with in_use flag (reverse-lookup from container mounts). Returns JSON { ok, count, volumes[] } with { name, driver, mountpoint, created, size, in_use }."
    )]
    fn proc_docker_volumes(&self) -> Result<CallToolResult, McpError> {
        ok_result(make_docker_volumes_json())
    }

    #[tool(
        name = "proc_docker_events",
        description = "Drain recent Docker daemon events (one-shot, NOT follow mode — MCP is request-response). Spawns a 500ms watcher window then drains up to `limit` (default 100) events. Returns JSON { ok, count, events[], note } — empty array + note when daemon is idle."
    )]
    fn proc_docker_events(
        &self,
        Parameters(args): Parameters<DockerEventsArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_docker_events_json(args.limit))
    }

    #[tool(
        name = "proc_monitor_add",
        description = "Add a process/port/command monitor. target_kind='pid'|'port'|'command'. restart_policy='notify_only' (default) | 'auto_restart'. dry_run defaults to FALSE (real add); pass dry_run=true to preview. Returns JSON { ok, dry_run, id?, target_kind, target, restart_policy }."
    )]
    fn proc_monitor_add(
        &self,
        Parameters(args): Parameters<MonitorAddArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_monitor_add_json(
            &args.target_kind,
            &args.target,
            args.restart_policy.as_deref(),
            args.dry_run,
        ))
    }

    #[tool(
        name = "proc_monitor_remove",
        description = "Remove a configured monitor by ID. dry_run defaults to FALSE (real remove); pass dry_run=true to preview. Returns JSON { ok, dry_run, id } or { ok: false, error } if id is not a positive integer / not found."
    )]
    fn proc_monitor_remove(
        &self,
        Parameters(args): Parameters<MonitorRemoveArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_monitor_remove_json(&args.id, args.dry_run))
    }

    #[tool(
        name = "proc_inspect",
        description = "Inspect a process by PID + tab. tab='summary' (default) returns basic info + parent_chain + signature_status + R1-R18 risk_factors + security_score. tab='env' returns env vars (secret masked unless reveal=true). tab='network' returns listening + established + dns_recent (top 5). tab='dlls' returns loaded modules. tab='memory_map' returns memory regions. tab='handles' returns open handles (same schema as proc_handles)."
    )]
    fn proc_inspect(
        &self,
        Parameters(args): Parameters<ProcInspectArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_inspect_json(args.pid, &args.tab, args.reveal))
    }

    #[tool(
        name = "proc_metrics_system",
        description = "System-level metrics: CPU usage %, memory/swap/system_disk usage (used/total/pct), uptime_secs, processes_count, network_interfaces (filtered: excludes 169.254/127.0.0.1), tcp_stats (established/time_wait/close_wait/listen + 4 segment counters), cpu_temp_c, gpu_temp_c. One-shot snapshot (no sparkline history)."
    )]
    fn proc_metrics_system(
        &self,
        Parameters(_args): Parameters<MetricsSystemArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_metrics_system_json())
    }

    #[tool(
        name = "proc_metrics_gpu",
        description = "GPU metrics: per-GPU {name, vendor (Nvidia/Amd/Intel/Unknown), utilization_pct, vram {used/total/budget bytes}, temperature_c, power_watts}. providers[] shows which data sources reported (nvml/dxgi/pdh). Returns gpus: [] + note when no providers available (non-Windows / no GPU)."
    )]
    fn proc_metrics_gpu(
        &self,
        Parameters(_args): Parameters<MetricsGpuArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_metrics_gpu_json())
    }

    #[tool(
        name = "proc_metrics_disk_io",
        description = "Disk I/O: total {read_bps, write_bps}, per_disk[] {name, mount_point, read_bps, write_bps}, disks[] {name, mount_point, used_bytes, total_bytes, is_removable}. Optional device filter narrows per_disk only (total/disks always return all)."
    )]
    fn proc_metrics_disk_io(
        &self,
        Parameters(args): Parameters<MetricsDiskIoArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_metrics_disk_io_json(args.device.as_deref()))
    }

    #[tool(
        name = "proc_metrics_smart",
        description = "SMART disk health. device=None → aggregated mode: disks[] summary {device, model, serial, temperature, health (Ok/Warning/Failing/Unknown), attribute_count}. device=Some(\"PhysicalDrive0\") → single_device mode with full attributes[] (id/name/value/threshold/raw_value/failing). Note: v0.7 proc_smart overlaps single_device mode — relationship TBD in stage 4 review."
    )]
    fn proc_metrics_smart(
        &self,
        Parameters(args): Parameters<MetricsSmartArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_metrics_smart_json(args.device.as_deref()))
    }

    #[tool(
        name = "proc_metrics_thermal",
        description = "Per-core CPU frequency + temperature + throttle state. Returns per_core_freq_mhz[], per_core_temp_c[] (None=that core unavailable, same length), throttle {max_mhz, current_mhz, mhz_limit, is_throttled, throttle_pct} (null on non-Windows), reason string (None/Thermal/PowerPolicy/Idle/Unknown/Unavailable)."
    )]
    fn proc_metrics_thermal(
        &self,
        Parameters(_args): Parameters<MetricsThermalArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(make_metrics_thermal_json())
    }
}

#[tool_handler(router = Self::tool_router())]
impl ServerHandler for ProcMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: rmcp::model::Implementation {
                name: "proc".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("proc process monitor".into()),
                icons: None,
                website_url: None,
            },
            protocol_version: Default::default(),
            capabilities: rmcp::model::ServerCapabilities {
                tools: Some(rmcp::model::ToolsCapability::default()),
                ..Default::default()
            },
            instructions: Some(
                "proc MCP server — wraps proc CLI subcommands. \
                 All tools return JSON: { ok: true, ...payload } or { ok: false, error: string }. \
                 Tool names follow proc_<subcommand>. \
                 See https://github.com/Alfroul/proc for full docs."
                    .into(),
            ),
        }
    }
}

// ===========================================================================
// Helpers — 每个 tool 一个，纯函数，方便单元测试。
// ===========================================================================

/// 把字符串映射到 SortField，与 `crate::cli::ls::run_ls` 完全一致。
pub(crate) fn parse_sort_field(s: Option<&str>) -> SortField {
    match s {
        Some("mem") | Some("memory") => SortField::Memory,
        Some("name") => SortField::Name,
        Some("pid") => SortField::Pid,
        Some("disk_read") | Some("diskread") => SortField::DiskRead,
        Some("disk_write") | Some("diskwrite") => SortField::DiskWrite,
        Some("net_sent") | Some("netsent") => SortField::NetSent,
        Some("net_recv") | Some("netrecv") => SortField::NetRecv,
        _ => SortField::Cpu,
    }
}

pub(crate) fn sort_field_label(f: SortField) -> &'static str {
    match f {
        SortField::Cpu => "cpu",
        SortField::Memory => "mem",
        SortField::Name => "name",
        SortField::Pid => "pid",
        SortField::DiskRead => "disk_read",
        SortField::DiskWrite => "disk_write",
        SortField::NetSent => "net_sent",
        SortField::NetRecv => "net_recv",
        SortField::Security => "security",
    }
}

pub fn err(msg: impl Into<String>) -> Value {
    json!({ "ok": false, "error": msg.into() })
}

/// 把 [`Value`] 包成 [`CallToolResult::success`] —— Content::json 会把 v 序列化成
/// MCP 文本 content，LLM 直接拿到字符串后再 JSON.parse。
///
/// 用 `Result<CallToolResult, McpError>` 而不是 `Json<Value>` 是因为后者要求
/// `Value: JsonSchema` 但 Value 没有稳定 schema（schema_for_output 会 panic）。
pub fn ok_result(v: Value) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::json(v)?]))
}

pub fn make_processes_json(sort: Option<&str>, limit: Option<usize>) -> Value {
    let mut snapshot = match collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return err(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return err(format!("snapshot refresh failed: {e}"));
    }
    let _ = snapshot.refresh_heavy_incremental();
    let mut processes = snapshot.cached_processes_vec();
    let sort_field = parse_sort_field(sort);
    collect::sort_processes(&mut processes, sort_field);
    if let Some(n) = limit {
        processes.truncate(n);
    }

    let (_, total_memory) = snapshot.memory_usage();
    let arr: Vec<Value> = processes
        .iter()
        .map(|p| {
            let mem_pct = if total_memory > 0 {
                p.memory as f64 / total_memory as f64 * 100.0
            } else {
                0.0
            };
            json!({
                "pid": p.pid,
                "name": p.name.as_ref(),
                "cpu_usage": p.cpu_usage,
                "memory_bytes": p.memory,
                "memory_pct": (mem_pct * 100.0).round() / 100.0,
                "virtual_memory_bytes": p.virtual_memory,
                "disk_read_bps": p.disk_read_speed,
                "disk_write_bps": p.disk_write_speed,
                "net_sent_bps": p.net_sent_rate,
                "net_recv_bps": p.net_recv_rate,
                "status": format!("{:?}", p.status),
                "parent_pid": p.parent_pid,
                "start_time_unix": p.start_time,
                "run_time_secs": p.run_time,
                "cmd": p.cmd.as_ref(),
            })
        })
        .collect();

    json!({
        "ok": true,
        "sort": sort_field_label(sort_field),
        "count": arr.len(),
        "processes": arr,
    })
}

pub fn make_process_tree_json() -> Value {
    let mut snapshot = match collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return err(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return err(format!("snapshot refresh failed: {e}"));
    }
    let _ = snapshot.refresh_heavy_incremental();
    let processes = snapshot.cached_processes_vec();
    let (_, total_mem) = snapshot.memory_usage();
    let roots = crate::tree::build_process_tree(&processes, total_mem);
    json!({
        "ok": true,
        "roots": roots.iter().map(tree_node_to_json).collect::<Vec<_>>(),
    })
}

pub(crate) fn tree_node_to_json(n: &crate::tree::TreeNode) -> Value {
    json!({
        "pid": n.pid,
        "name": n.name,
        "cpu_usage": n.cpu,
        "memory_bytes": n.memory,
        "memory_pct": n.mem_pct,
        "status": n.status,
        "depth": n.depth,
        "is_orphan": n.is_orphan,
        "is_zombie": n.is_zombie,
        "children": n.children.iter().map(tree_node_to_json).collect::<Vec<_>>(),
    })
}

pub fn make_port_json(filter_port: Option<u16>) -> Value {
    let entries = match filter_port {
        Some(p) => crate::port_map::find_pid_by_port(p),
        None => crate::port_map::scan_ports(),
    };
    let entries = match entries {
        Ok(v) => v,
        Err(e) => return err(format!("port scan failed: {e}")),
    };
    let arr: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "protocol": format!("{:?}", e.protocol),
                "local_addr": e.local_addr.to_string(),
                "local_port": e.local_port,
                "remote_addr": e.remote_addr.map(|a| a.to_string()),
                "remote_port": e.remote_port,
                "state": e.state,
                "pid": e.pid,
                "process_name": e.process_name,
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "ports": arr })
}

pub fn make_kill_json(pid: u32, force: bool) -> Value {
    match crate::kill::kill_process(pid, force) {
        Ok(result) => {
            let result_str = match &result {
                crate::kill::KillResult::Killed => "Killed",
                crate::kill::KillResult::AlreadyGone => "AlreadyGone",
                crate::kill::KillResult::AccessDenied => "AccessDenied",
                crate::kill::KillResult::Failed(msg) => {
                    return json!({
                        "ok": true,
                        "pid": pid,
                        "force": force,
                        "result": "Failed",
                        "error": msg,
                    });
                }
            };
            json!({
                "ok": true,
                "pid": pid,
                "force": force,
                "result": result_str,
            })
        }
        Err(e) => json!({
            "ok": false,
            "pid": pid,
            "force": force,
            "error": e.to_string(),
        }),
    }
}

pub fn make_pkill_json(name: &str, force: bool, dry_run: bool) -> Value {
    let results = match crate::kill::kill_by_name(name, force, dry_run) {
        Ok(v) => v,
        Err(e) => return err(format!("pkill failed: {e}")),
    };
    let mut killed = 0u32;
    let mut failed = 0u32;
    let arr: Vec<Value> = results
        .iter()
        .map(|r| {
            let (result_str, error) = match &r.outcome {
                None => ("DryRun".to_string(), None),
                Some(crate::kill::KillResult::Killed) => {
                    killed += 1;
                    ("Killed".to_string(), None)
                }
                Some(crate::kill::KillResult::AlreadyGone) => ("AlreadyGone".to_string(), None),
                Some(crate::kill::KillResult::AccessDenied) => {
                    failed += 1;
                    ("AccessDenied".to_string(), None)
                }
                Some(crate::kill::KillResult::Failed(msg)) => {
                    failed += 1;
                    ("Failed".to_string(), Some(msg.clone()))
                }
            };
            json!({
                "pid": r.pid,
                "name": r.name,
                "result": result_str,
                "error": error,
            })
        })
        .collect();
    json!({
        "ok": true,
        "name": name,
        "force": force,
        "dry_run": dry_run,
        "total": arr.len(),
        "killed": killed,
        "failed": failed,
        "results": arr,
    })
}

pub fn make_eject_json(drive: Option<&str>) -> Value {
    // We only expose list / scan-locks (kill_safe is destructive; keep it CLI-only for v0.7).
    match drive {
        None => {
            let devices = match crate::eject::scan_all_devices() {
                Ok(v) => v,
                Err(e) => return err(format!("scan_all_devices failed: {e}")),
            };
            let arr: Vec<Value> = devices
                .iter()
                .map(|d| {
                    json!({
                        "drive_letter": d.drive_letter.to_string(),
                        "label": d.label,
                        "total_bytes": d.total_size,
                        "used_bytes": d.used_size,
                        "fs_type": d.file_system,
                        "is_occupied": d.is_occupied,
                    })
                })
                .collect();
            json!({ "ok": true, "devices": arr })
        }
        Some(drive_str) => {
            // parse_drive_letter is private — repeat the small parse here to avoid widening API.
            let cleaned: String = drive_str
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .collect();
            let letter = match cleaned.chars().next() {
                Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
                _ => return err(format!("invalid drive letter: {drive_str}")),
            };
            let locks = match crate::eject::scan_device_locks(letter) {
                Ok(v) => v,
                Err(e) => return err(format!("scan_device_locks failed: {e}")),
            };
            let arr: Vec<Value> = locks
                .iter()
                .map(|(lock, risk)| {
                    json!({
                        "pid": lock.pid,
                        "name": lock.process_name,
                        "exe_path": lock.exe_path,
                        "risk": format!("{:?}", risk),
                    })
                })
                .collect();
            json!({
                "ok": true,
                "drive": format!("{letter}:"),
                "locks": arr,
            })
        }
    }
}

pub fn make_who_json(target_path: &str) -> Value {
    let path = std::path::Path::new(target_path);
    let handles = match crate::inspect::handles::find_lockers(path) {
        Ok(v) => v,
        Err(e) => return err(format!("find_lockers failed: {e}")),
    };
    let arr: Vec<Value> = handles
        .iter()
        .map(|h| {
            // find_lockers 反查路径下 raw_handle 字段被借用来存 PID（见 inspect/handles.rs 注释）。
            let pid = h.raw_handle;
            let name = crate::collect::sysinfo_with(|sys| {
                sys.process(sysinfo::Pid::from_u32(pid as u32))
                    .map(|p| p.name().to_string_lossy().to_string())
                    .unwrap_or_else(|| "?".to_string())
            });
            json!({
                "pid": pid,
                "name": name,
                "kind": format!("{:?}", h.kind),
                "handle_path": h.name,
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "lockers": arr })
}

pub fn make_handles_json(pid: u32) -> Value {
    let handles = match crate::inspect::handles::collect_handles(pid) {
        Ok(v) => v,
        Err(e) => return err(format!("collect_handles failed: {e}")),
    };
    let arr: Vec<Value> = handles
        .iter()
        .map(|h| {
            json!({
                "kind": format!("{:?}", h.kind),
                "name": h.name,
                "raw_handle": h.raw_handle,
                "granted_access": h.granted_access,
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "handles": arr })
}

pub fn make_priority_json(pid: u32, set: Option<&str>) -> Value {
    use crate::process_control::{get_priority, set_priority};
    match set {
        None => match get_priority(pid) {
            Ok(class) => json!({
                "ok": true,
                "pid": pid,
                "action": "get",
                "priority": class.label(),
            }),
            Err(e) => json!({
                "ok": false,
                "pid": pid,
                "action": "get",
                "error": e.to_string(),
            }),
        },
        Some(class_str) => {
            let class = match parse_priority_class(class_str) {
                Ok(c) => c,
                Err(msg) => {
                    return json!({
                        "ok": false,
                        "pid": pid,
                        "action": "set",
                        "error": msg,
                    });
                }
            };
            match set_priority(pid, class) {
                Ok(()) => json!({
                    "ok": true,
                    "pid": pid,
                    "action": "set",
                    "priority": class.label(),
                }),
                Err(e) => json!({
                    "ok": false,
                    "pid": pid,
                    "action": "set",
                    "error": e.to_string(),
                }),
            }
        }
    }
}

pub(crate) fn parse_priority_class(
    s: &str,
) -> std::result::Result<crate::process_control::PriorityClass, String> {
    use crate::process_control::PriorityClass;
    match s.to_lowercase().as_str() {
        "idle" => Ok(PriorityClass::Idle),
        "belownormal" | "below_normal" | "below" => Ok(PriorityClass::BelowNormal),
        "normal" => Ok(PriorityClass::Normal),
        "abovenormal" | "above_normal" | "above" => Ok(PriorityClass::AboveNormal),
        "high" => Ok(PriorityClass::High),
        "realtime" => Ok(PriorityClass::Realtime),
        _ => Err(format!(
            "unknown priority '{s}' (valid: idle / belownormal / normal / abovenormal / high / realtime)"
        )),
    }
}

pub fn make_affinity_json(pid: u32, set: Option<&str>) -> Value {
    use crate::process_control::{get_affinity, set_affinity};
    match set {
        None => match get_affinity(pid) {
            Ok(mask) => json!({
                "ok": true,
                "pid": pid,
                "action": "get",
                "affinity_mask": format!("0x{mask:X}"),
                "core_count": u64::count_ones(mask),
            }),
            Err(e) => json!({
                "ok": false,
                "pid": pid,
                "action": "get",
                "error": e.to_string(),
            }),
        },
        Some(hex_str) => {
            let trimmed = hex_str.trim_start_matches("0x").trim_start_matches("0X");
            let mask = match u64::from_str_radix(trimmed, 16) {
                Ok(v) => v,
                Err(_) => {
                    return json!({
                        "ok": false,
                        "pid": pid,
                        "action": "set",
                        "error": format!("--set expects hex (e.g. 0xFF), got '{hex_str}'"),
                    });
                }
            };
            match set_affinity(pid, mask) {
                Ok(()) => json!({
                    "ok": true,
                    "pid": pid,
                    "action": "set",
                    "affinity_mask": format!("0x{mask:X}"),
                    "core_count": u64::count_ones(mask),
                }),
                Err(e) => json!({
                    "ok": false,
                    "pid": pid,
                    "action": "set",
                    "error": e.to_string(),
                }),
            }
        }
    }
}

pub fn make_smart_json(device: Option<&str>) -> Value {
    match device {
        None => {
            let disks = crate::smart::list_disks();
            if disks.is_empty() {
                return json!({
                    "ok": true,
                    "disks": [],
                    "note": "no SMART-readable disks (Linux: check /sys/block; Windows: needs smartmontools for full attributes)"
                });
            }
            let arr: Vec<Value> = disks
                .iter()
                .map(|dev| match crate::smart::read_smart(dev) {
                    Ok(data) => json!({
                        "device": data.device,
                        "model": data.model,
                        "serial": data.serial,
                        "temperature": data.temperature,
                        "health": format!("{:?}", data.health),
                        "attribute_count": data.attributes.len(),
                    }),
                    Err(e) => json!({
                        "device": dev,
                        "error": e.to_string(),
                    }),
                })
                .collect();
            json!({ "ok": true, "disks": arr })
        }
        Some(dev) => match crate::smart::read_smart(dev) {
            Ok(data) => {
                let attrs: Vec<Value> = data
                    .attributes
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id,
                            "name": a.name,
                            "value": a.value,
                            "threshold": a.threshold,
                            "raw_value": a.raw_value,
                            "failing": a.failing,
                        })
                    })
                    .collect();
                json!({
                    "ok": true,
                    "disk": {
                        "device": data.device,
                        "model": data.model,
                        "serial": data.serial,
                        "temperature": data.temperature,
                        "health": format!("{:?}", data.health),
                        "attributes": attrs,
                    }
                })
            }
            Err(e) => err(format!("read_smart({dev}) failed: {e}")),
        },
    }
}

pub fn make_dns_json(tail: bool) -> Value {
    if tail {
        return err(
            "tail mode is streaming-only — use the proc CLI (`proc dns --tail`) instead. MCP tools are one-shot.",
        );
    }
    let (collector, _kind) = crate::dns_log::detect_collector();
    let Some(mut collector) = collector else {
        return err(
            "DNS log collector unavailable on this platform (Windows: ETW primary / PowerShell fallback; see ADR-0020)",
        );
    };
    let queries = collector.drain();
    let arr = dns_queries_to_json(&queries);
    json!({ "ok": true, "count": arr.len(), "queries": arr })
}

/// TD-36（v0.12 阶段 5）：从 handler 持久 collector drain DNS 查询并 JSON-ify。
///
/// 与 [`make_dns_json`] 的差异：不 spawn 新 collector，直接 drain 入参的 Arc
/// collector。collector 为 `None`（Default / 非 Windows / spawn 失败）→ 与旧版
/// 一致的「unavailable」错误信息；Mutex 中毒（panic while holding lock）→ 同样
/// 返 unavailable（panic 不期望出现，但兜底防止 MCP server 整体挂）。
///
/// `pub`（非 `pub(crate)`）让集成测试能直接验证 Arc 共享语义，不在生产代码
/// 文档里暴露（[`ProcMcpHandler::proc_dns`] 是入口）。
pub fn make_dns_json_from_collector(
    dns_collector: &Arc<Mutex<Option<Box<dyn DnsLogCollector>>>>,
    tail: bool,
) -> Value {
    if tail {
        return err(
            "tail mode is streaming-only — use the proc CLI (`proc dns --tail`) instead. MCP tools are one-shot.",
        );
    }
    let Ok(mut guard) = dns_collector.lock() else {
        return err("DNS log collector unavailable (internal: mutex poisoned)");
    };
    let Some(collector) = guard.as_mut() else {
        return err(
            "DNS log collector unavailable on this platform (Windows: ETW primary / PowerShell fallback; see ADR-0020)",
        );
    };
    let queries = collector.drain();
    let arr = dns_queries_to_json(&queries);
    json!({ "ok": true, "count": arr.len(), "queries": arr })
}

/// 把 drain 出的 `Vec<DnsQuery>` JSON-ify——共享给 [`make_dns_json`]（fresh spawn
/// 路径）和 [`make_dns_json_from_collector`]（persistent 路径）。
fn dns_queries_to_json(queries: &[crate::dns_log::DnsQuery]) -> Vec<Value> {
    queries
        .iter()
        .map(|q| {
            json!({
                "query_name": q.query_name,
                "query_type": q.query_type,
                "pid": q.pid,
                "process_name": q.process_name,
                "timestamp_unix": q.start_time,
            })
        })
        .collect()
}

pub fn make_diag_json() -> Value {
    let mut app = match crate::app::App::new() {
        Ok(a) => a,
        Err(e) => return err(format!("App::new failed: {e}")),
    };
    // 等 worker 至少 poll 一次：与 CLI diag 一致，最多 2s（light 1s / port 3s / dns 500ms）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        app.tick();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let metrics = app.worker_metrics();
    let dns_kind = app.workers.dns_collector_kind;
    let arr: Vec<Value> = metrics
        .iter()
        .map(|m| {
            let s = &m.stats;
            json!({
                "name": m.name,
                "health": s.health_badge(),
                "avg_us": s.avg_us,
                "max_us": s.max_us,
                "polls": s.poll_count,
                "drops": s.channel_full,
                "last_error": s.last_error.as_ref().map(|(t, msg)| {
                    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                    json!({ "timestamp_unix": secs, "message": msg })
                }),
            })
        })
        .collect();
    json!({ "ok": true, "workers": arr, "dns_collector": dns_kind })
}

pub fn make_monitor_list_json() -> Value {
    let mgr = crate::monitor::MonitorManager::new();
    let monitors = mgr.list_monitors();
    let arr: Vec<Value> = monitors
        .iter()
        .map(|m| {
            let (kind, target_str, pid): (&'static str, String, Option<u32>) = match &m.target {
                crate::monitor::MonitorTarget::ByPid { pid } => {
                    ("pid", pid.to_string(), Some(*pid))
                }
                crate::monitor::MonitorTarget::ByPort { port } => ("port", port.to_string(), None),
                crate::monitor::MonitorTarget::ByCommand { cmd, args, .. } => {
                    ("command", format!("{} {}", cmd, args.join(" ")), None)
                }
            };
            let policy = match &m.restart_policy {
                crate::monitor::RestartPolicy::NotifyOnly => "notify_only".to_string(),
                crate::monitor::RestartPolicy::AutoRestart {
                    max_retries,
                    base_backoff,
                    max_backoff,
                } => format!(
                    "auto_restart(max_retries={max_retries}, backoff={base_backoff}-{max_backoff}s)"
                ),
            };
            json!({
                "id": m.id,
                "target_kind": kind,
                "target": target_str,
                "pid": pid,
                "status": format!("{:?}", m.status),
                "crash_count": m.crash_count,
                "restart_policy": policy,
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "monitors": arr })
}

pub fn make_docker_ps_json() -> Value {
    let monitor = match crate::docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => return err(format!("Docker connect failed: {e}")),
    };
    let containers = match monitor.list_containers(true) {
        Ok(v) => v,
        Err(e) => return err(format!("list_containers failed: {e}")),
    };
    let arr: Vec<Value> = containers
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "name": c.name,
                "image": c.image,
                "state": c.state,
                "status": c.status,
                "health": format!("{:?}", c.health),
                "cpu_percent": c.cpu_percent,
                "memory_usage": c.memory_usage,
                "running_since_iso": c.running_since.map(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                }),
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "containers": arr })
}

pub fn make_docker_top_json(name: &str) -> Value {
    let monitor = match crate::docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => return err(format!("Docker connect failed: {e}")),
    };
    let procs = match monitor.container_top(name) {
        Ok(v) => v,
        Err(e) => return err(format!("container_top({name}) failed: {e}")),
    };
    let arr: Vec<Value> = procs
        .iter()
        .map(|p| {
            json!({
                "pid": p.pid,
                "user": p.user,
                "command": p.command,
                "cpu_time": p.cpu_time,
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "processes": arr })
}

pub fn make_docker_logs_json(name: &str, tail: Option<&str>) -> Value {
    let monitor = match crate::docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => return err(format!("Docker connect failed: {e}")),
    };
    let logs = match monitor.collect_logs(name, tail) {
        Ok(v) => v,
        Err(e) => return err(format!("collect_logs({name}) failed: {e}")),
    };
    let arr: Vec<Value> = logs
        .iter()
        .map(|l| {
            json!({
                "timestamp": l.timestamp,
                "message": l.message,
                "is_stderr": l.is_stderr,
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "lines": arr })
}

#[allow(unused)]
fn _anchor_unused() {
    let _: Option<McpError> = None;
}
