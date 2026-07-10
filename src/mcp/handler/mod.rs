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
pub mod observable;
pub mod record;

use std::sync::{Arc, Mutex};

#[cfg(feature = "mcp-persistent-state")]
use std::collections::VecDeque;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, Content, ServerInfo},
    schemars, serve_server, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[cfg(feature = "mcp-persistent-state")]
use crate::collect::SystemSnapshot;
use crate::collect::{self, SortField};
use crate::dns_log::DnsLogCollector;

/// v0.17 stage 4 TD-52：sparkline 单个采样点（轻量 Copy struct）。
///
/// `SystemSnapshot` 含 `JoinHandle` / `Receiver` 等 non-Clone 字段，无法直接
/// 存进 `VecDeque`。worker 每 tick 从 `SystemSnapshot` 提取这 4 个标量字段
/// （cpu/memory/swap + unix timestamp）push 到 `system_history: VecDeque<MetricsSample>`，
/// 让 `proc_metrics_history` tool drain 返 sparkline 数据点（ADR-0026 / ADR-0027）。
///
/// Copy + Clone 让 worker push 用 `*sample` 解引用（零开销），drain 时
/// `iter().rev().take(N).rev()` 直接复制 Copy 数据，无 alloc。
#[cfg(feature = "mcp-persistent-state")]
#[derive(Clone, Copy, Debug)]
pub struct MetricsSample {
    /// CPU 使用率（0-100，f32 与 SystemSnapshot::cpu_usage() 一致）。
    pub cpu_usage: f32,
    /// 已用内存（bytes，u64 与 SystemSnapshot::memory_usage().0 一致）。
    pub memory_used: u64,
    /// 已用 Swap（bytes，u64 与 SystemSnapshot::swap_usage().0 一致）。
    pub swap_used: u64,
    /// 采集时间戳（unix seconds，worker push 时 `SystemSnapshot::uptime_secs()` 或
    /// `chrono::Utc::now().timestamp()`）。
    pub timestamp_unix: u64,
}

#[cfg(feature = "mcp-persistent-state")]
impl MetricsSample {
    /// 从 SystemSnapshot 提取 MetricsSample（worker 每 tick 调一次）。
    fn from_snapshot(s: &SystemSnapshot) -> Self {
        let cpu_usage = s.cpu_usage();
        let (memory_used, _) = s.memory_usage();
        let (swap_used, _) = s.swap_usage();
        let timestamp_unix = SystemSnapshot::uptime_secs();
        Self {
            cpu_usage,
            memory_used,
            swap_used,
            timestamp_unix,
        }
    }
}

// v0.15 阶段 1 子 module re-export：让本文件 `#[tool]` 方法可以直接调
// `cli::make_flows_json(...)` / `inspect::make_inspect_json(...)` /
// `metrics::make_metrics_system_json()`，不需要 `self::cli::` 前缀。
use cli::*;
use inspect::*;
use metrics::*;
use observable::*;
use record::*;

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

    /// v0.17 stage 3 TD-54 落地：MCP handler 持久 SystemSnapshot，1s tick refresh。
    /// stage 1 Spike 仅声明字段 + Default 返 None；stage 3 实装 worker spawn +
    /// refresh 逻辑，让 metrics_* / proc_flows / proc_export 复用同一 snapshot
    /// （ADR-0026）。
    #[cfg(feature = "mcp-persistent-state")]
    pub snapshot: Arc<Mutex<Option<SystemSnapshot>>>,

    /// v0.17 stage 4 TD-52 落地：sparkline 30s 历史，1s tick push。
    /// stage 1 Spike 仅声明字段 + Default 返空 VecDeque；stage 4 实装 worker
    /// push 逻辑，让 `proc_metrics_history` tool drain 此字段返 sparkline
    /// 数据点（ADR-0026 / ADR-0027）。
    ///
    /// **存储 [`MetricsSample`] 而非 `SystemSnapshot`**：SystemSnapshot 含
    /// JoinHandle / Receiver 等 non-Clone 字段，无法直接存进 VecDeque。worker
    /// 每 tick 提取 cpu/memory/swap/ts 4 个标量 push（Copy，零 alloc）。
    #[cfg(feature = "mcp-persistent-state")]
    pub system_history: Arc<Mutex<VecDeque<MetricsSample>>>,

    /// v0.17 stage 6 record 暴露落地：spawn `proc record` 子进程 handle。
    /// stage 1 Spike 仅声明字段 + Default 返 None；stage 6 实装 spawn +
    /// lifecycle 管理，让 `proc_record_start` / `proc_record_stop` 跨 tool
    /// call 保活 child handle（ADR-0026 / ADR-0029）。
    #[cfg(feature = "mcp-persistent-state")]
    pub record_handle: Arc<Mutex<Option<std::process::Child>>>,
}

impl Clone for ProcMcpHandler {
    fn clone(&self) -> Self {
        Self {
            dns_collector: Arc::clone(&self.dns_collector),
            #[cfg(feature = "mcp-persistent-state")]
            snapshot: Arc::clone(&self.snapshot),
            #[cfg(feature = "mcp-persistent-state")]
            system_history: Arc::clone(&self.system_history),
            #[cfg(feature = "mcp-persistent-state")]
            record_handle: Arc::clone(&self.record_handle),
        }
    }
}

impl Default for ProcMcpHandler {
    fn default() -> Self {
        // 测试 / 未启用 DNS 路径用：不 spawn collector，proc_dns 调用走「无 collector」
        // 错误返回。生产路径必须用 [`Self::new`]。
        // v0.17 持久字段（snapshot / system_history / record_handle）Default 也
        // 返 None / 空 VecDeque——stage 3/4/6 各 Slice 实装 worker spawn 逻辑后
        // 才在 [`ProcMcpHandler::new`] 生产路径填充。
        Self {
            dns_collector: Arc::new(Mutex::new(None)),
            #[cfg(feature = "mcp-persistent-state")]
            snapshot: Arc::new(Mutex::new(None)),
            #[cfg(feature = "mcp-persistent-state")]
            system_history: Arc::new(Mutex::new(VecDeque::new())),
            #[cfg(feature = "mcp-persistent-state")]
            record_handle: Arc::new(Mutex::new(None)),
        }
    }
}

impl ProcMcpHandler {
    /// 生产入口：spawn 持久 DNS collector（Windows admin → ETW；Windows 非 admin
    /// → PowerShell fallback；其它平台 → None）。collector 与 handler 同生命周期，
    /// `proc_dns` tool call 通过 `Arc::clone` 共享同一实例。
    ///
    /// v0.17 stage 3 TD-54：`snapshot` 字段也 spawn 一个 1s tick worker
    /// （`mcp-snapshot-worker`）持续 refresh SystemSnapshot，让 `metrics_*` /
    /// `proc_ls` / `proc_tree` / `proc_export` 等读 SystemSnapshot 的 tool 复用
    /// 字段而非每次现场 `SystemSnapshot::new() + refresh()` 累积 ~50-200ms 开销
    /// （ADR-0026）。fire-and-forget 模式：worker 持 `Arc::clone(&snapshot)`，
    /// handler 不持 `JoinHandle`，进程退出时 worker 自然终止。
    #[must_use]
    pub fn new() -> Self {
        let (collector, _kind) = crate::dns_log::detect_collector();

        #[cfg(feature = "mcp-persistent-state")]
        let snapshot: Arc<Mutex<Option<SystemSnapshot>>> = Arc::new(Mutex::new(None));

        #[cfg(feature = "mcp-persistent-state")]
        let system_history: Arc<Mutex<VecDeque<MetricsSample>>> =
            Arc::new(Mutex::new(VecDeque::new()));

        #[cfg(feature = "mcp-persistent-state")]
        {
            let snapshot_clone = Arc::clone(&snapshot);
            let history_clone = Arc::clone(&system_history);
            // spawn 失败静默降级（与 DNS collector detect 同款规则），handler 仍创建
            // （snapshot 字段为 None，后续 tool 调用走 fallback 现场新建路径）。
            // v0.17 stage 4 TD-52：worker 兼任 system_history VecDeque push（30s cap），
            // 不 spawn 第二个 worker（决策 1）。
            let _ = std::thread::Builder::new()
                .name("mcp-snapshot-worker".to_string())
                .spawn(move || run_snapshot_worker(snapshot_clone, history_clone));
        }

        Self {
            dns_collector: Arc::new(Mutex::new(collector)),
            #[cfg(feature = "mcp-persistent-state")]
            snapshot,
            #[cfg(feature = "mcp-persistent-state")]
            system_history,
            #[cfg(feature = "mcp-persistent-state")]
            record_handle: Arc::new(Mutex::new(None)),
        }
    }
}

/// v0.17 stage 3 TD-54 + stage 4 TD-52：1s tick SystemSnapshot refresh + history push worker。
///
/// fire-and-forget 模式（handler 不持 JoinHandle，进程退出时终止）。每 tick：
/// 1. 从 `Arc<Mutex<Option<SystemSnapshot>>>` 字段 take 出 owned snapshot（如有）
/// 2. refresh + refresh_heavy_incremental（复用 sysinfo System 内部增量状态，比
///    每次 `SystemSnapshot::new()` 全新初始化快 ~5x）
/// 3. 从 snapshot 提取 `MetricsSample`（cpu/memory/swap/ts），push 到
///    `system_history: VecDeque<MetricsSample>`，30s cap（TD-52）
/// 4. move owned snapshot 回 snapshot 字段
/// 5. sleep 减去本次耗时（保证 1s tick 节奏）
///
/// 持锁窗口 ~30-50ms（refresh 耗时），1s tick 占空比 < 5%。helper 在持锁窗口
/// 读字段时 take 出来 → 读到 None → fallback 现场新建（与 v0.16 行为一致）。
///
/// refresh 失败时保 `last_snapshot` 不变，下 tick 重试（worker 永不退出）。
///
/// **stage 4 TD-52 兼任 push**（决策 1）：不 spawn 第二个 worker，single worker
/// 写回 snapshot 字段前先提取 `MetricsSample`（Copy struct，零 alloc）push 到
/// system_history VecDeque。system_history 字段不存 SystemSnapshot（含 non-Clone
/// 字段如 JoinHandle / Receiver），仅存 4 个标量。
#[cfg(feature = "mcp-persistent-state")]
fn run_snapshot_worker(
    snapshot: Arc<Mutex<Option<SystemSnapshot>>>,
    system_history: Arc<Mutex<VecDeque<MetricsSample>>>,
) {
    loop {
        let tick_start = std::time::Instant::now();

        // 1. take owned snapshot（如有），没有则 new
        let mut s = match snapshot.lock().ok().and_then(|mut g| g.take()) {
            Some(s) => s,
            None => match SystemSnapshot::new() {
                Ok(s) => s,
                Err(_) => {
                    // new 失败：sleep 1s 重试，字段保持 None（helper 走 fallback）
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
            },
        };

        // 2. refresh（失败保原 snapshot 不变）
        if s.refresh().is_ok() {
            let _ = s.refresh_heavy_incremental();
        }

        // 3. v0.17 stage 4 TD-52：从 snapshot 提取 MetricsSample push 到 system_history
        //    VecDeque（30s cap）。MetricsSample 是 Copy struct，零 alloc；不同 mutex
        //    不冲突，但顺序固定（先 history 再 snapshot）避免同时持两锁。
        let sample = MetricsSample::from_snapshot(&s);
        if let Ok(mut history) = system_history.lock() {
            history.push_back(sample);
            while history.len() > 30 {
                history.pop_front();
            }
        }

        // 4. move owned snapshot 回字段
        if let Ok(mut guard) = snapshot.lock() {
            *guard = Some(s);
        }

        // 5. sleep 减去本次耗时（保证 1s tick 节奏）
        let elapsed = tick_start.elapsed();
        let sleep_dur = std::time::Duration::from_secs(1).saturating_sub(elapsed);
        std::thread::sleep(sleep_dur);
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
        #[cfg(feature = "mcp-persistent-state")]
        {
            if let Ok(guard) = self.snapshot.lock() {
                if let Some(s) = guard.as_ref() {
                    return ok_result(processes_json_from_snapshot(
                        s,
                        args.sort.as_deref(),
                        args.limit,
                    ));
                }
            }
        }
        ok_result(make_processes_json(args.sort.as_deref(), args.limit))
    }

    #[tool(
        name = "proc_tree",
        description = "Build the full process tree (parent → children). Returns JSON { ok, roots[] } where each node has { pid, name, cpu_usage, memory_bytes, status, children[] }. Useful for understanding process ancestry."
    )]
    fn proc_tree(&self) -> Result<CallToolResult, McpError> {
        #[cfg(feature = "mcp-persistent-state")]
        {
            if let Ok(guard) = self.snapshot.lock() {
                if let Some(s) = guard.as_ref() {
                    return ok_result(process_tree_json_from_snapshot(s));
                }
            }
        }
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
        description = "[Deprecated] SMART disk health. Prefer proc_metrics_smart for aggregated or single-device mode (richer schema + same SMART data source). This tool is kept for backward compatibility but will be removed in v0.18+. device=None lists all disks with summary; device=\"/dev/sda\" or \"PhysicalDrive0\" returns detailed attributes. Returns JSON { ok, disks[] | disk } with { device, model, serial, temperature, health, attributes[] }. health is one of Ok/Warning/Critical/Unknown."
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
        #[cfg(feature = "mcp-persistent-state")]
        {
            if let Ok(guard) = self.snapshot.lock() {
                if let Some(s) = guard.as_ref() {
                    return ok_result(cli::export_json_from_snapshot(
                        s,
                        args.format.as_deref(),
                        args.sort.as_deref(),
                        args.limit,
                    ));
                }
            }
        }
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
        #[cfg(feature = "mcp-persistent-state")]
        {
            if let Ok(guard) = self.snapshot.lock() {
                if let Some(s) = guard.as_ref() {
                    return ok_result(metrics::metrics_system_json_from_snapshot(s));
                }
            }
        }
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
        #[cfg(feature = "mcp-persistent-state")]
        {
            if let Ok(guard) = self.snapshot.lock() {
                if let Some(s) = guard.as_ref() {
                    return ok_result(metrics::metrics_disk_io_json_from_snapshot(
                        s,
                        args.device.as_deref(),
                    ));
                }
            }
        }
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
        #[cfg(feature = "mcp-persistent-state")]
        {
            if let Ok(guard) = self.snapshot.lock() {
                if let Some(s) = guard.as_ref() {
                    return ok_result(metrics::metrics_thermal_json_from_snapshot(s));
                }
            }
        }
        ok_result(make_metrics_thermal_json())
    }

    #[tool(
        name = "proc_replay_info",
        description = "Read recording file metadata (v3 UiFrame footer or VT100 header). Returns { ok, format: \"uiframe\"|\"vt100\", version, frame_count, duration_secs, anomaly_count, event_count, max_cpu, max_mem, has_bookmarks_sidecar, path, size_bytes }. v3 UiFrame format includes start_time/end_time/hostname/anomaly_count/event_count. VT100 format includes start_ms/end_ms/width/height (no footer). File not found / not a recording / corrupt → ok=false + error."
    )]
    fn proc_replay_info(
        &self,
        Parameters(args): Parameters<ReplayInfoArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_replay_info_json(&args.file_path))
    }

    #[tool(
        name = "proc_replay_search",
        description = "Search recording frames by FilterExpr or substring (v3 UiFrame only). Query formats: substring `chrome` (matches any frame containing process named \"chrome\"); FilterExpr `: cpu > 80 AND name =~ /chrome/` (5 dimensions: timestamp/cpu/mem/name/anomaly.severity). Returns { ok, match_count, returned, truncated, matches: [{ frame_idx, timestamp, cpu_usage, memory_used, matched_processes[], anomaly_severity? }] }. Default limit: 100. Truncated=true when match_count > returned. VT100 format not supported (returns ok=false). Long recording performance: ~9s per 30min session (one-time scan)."
    )]
    fn proc_replay_search(
        &self,
        Parameters(args): Parameters<ReplaySearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_replay_search_json(
            &args.file_path,
            &args.query,
            args.limit,
        ))
    }

    #[tool(
        name = "proc_bookmarks_list",
        description = "List all bookmarks in a recording's sidecar (.prec.bookmarks.json). Returns { ok, count, sidecar_present, source_healthy, bookmarks: [{ id, frame_idx, timestamp_secs, label, created_at }] }. Sidecar not present → count=0 + sidecar_present=false + bookmarks=[]. Source recording changed (size/mtime mismatch) → source_healthy=false + bookmarks=[]."
    )]
    fn proc_bookmarks_list(
        &self,
        Parameters(args): Parameters<BookmarksListArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_bookmarks_list_json(&args.file_path))
    }

    #[tool(
        name = "proc_bookmarks_add",
        description = "Add a bookmark to a recording's sidecar. Args: file_path, frame_idx (0-based, must be < total_frames), label (optional, empty/None defaults to \"书签 #N\"), dry_run (optional, default false). Returns { ok, dry_run, action: \"add\", id, frame_idx, label, timestamp_secs, sidecar_written }. dry_run=true → sidecar_written=false (preview only)."
    )]
    fn proc_bookmarks_add(
        &self,
        Parameters(args): Parameters<BookmarksAddArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_bookmarks_add_json(
            &args.file_path,
            args.frame_idx,
            args.label.as_deref(),
            args.dry_run,
        ))
    }

    #[tool(
        name = "proc_bookmarks_edit",
        description = "Edit a bookmark's label in a recording's sidecar. Args: file_path, id (bookmark id), label (new label), dry_run (optional, default false). Returns { ok, dry_run, action: \"edit\", id, old_label, new_label, sidecar_written }. Bookmark id not found → ok=false + error."
    )]
    fn proc_bookmarks_edit(
        &self,
        Parameters(args): Parameters<BookmarksEditArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_bookmarks_edit_json(
            &args.file_path,
            args.id,
            &args.label,
            args.dry_run,
        ))
    }

    #[tool(
        name = "proc_bookmarks_delete",
        description = "Delete a bookmark from a recording's sidecar. Args: file_path, id (bookmark id), dry_run (optional, default false). Returns { ok, dry_run, action: \"delete\", id, frame_idx, label, sidecar_written }. Bookmark id not found → ok=false + error."
    )]
    fn proc_bookmarks_delete(
        &self,
        Parameters(args): Parameters<BookmarksDeleteArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_bookmarks_delete_json(
            &args.file_path,
            args.id,
            args.dry_run,
        ))
    }

    #[tool(
        name = "proc_eject_status",
        description = "Query USB / removable device eject status (read-only, no kill / no flush). Use case: agent kills locks then re-checks to confirm kill took effect. Args: drive (drive letter, e.g. \"E\" / \"E:\" / \"E:\\\\\" all accepted). Returns { ok, drive, device: { drive_letter, label, total_bytes, used_bytes, fs_type, is_removable }, ejectable: bool (= lock_count==0), lock_count, locks: [{ pid, name, exe_path, risk }], suggestion: \"eject_now\"|\"kill_locks\"|\"unknown_drive\"|\"unavailable\" }. suggestion=eject_now → can directly eject; suggestion=kill_locks → need to kill processes (use proc_kill); suggestion=unknown_drive → drive letter invalid or not removable; suggestion=unavailable → non-Windows platform or scan failed. Windows-only; other platforms return ok=true + suggestion=\"unavailable\"."
    )]
    fn proc_eject_status(
        &self,
        Parameters(args): Parameters<EjectStatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_eject_status_json(&args.drive))
    }

    // ====================================================================
    // v0.17 阶段 1 stub 方法 — schema 注册占位，业务逻辑在 stage 4 / stage 6
    // 各 Slice 填（详见 docs/stages/v0.17-stage-1.md）。stub 返
    // `{ ok: true, stub: true, stage: "v0.17-stage-{4,6}", ... }` placeholder 让
    // client（mcp-inspector）验证 schema 但不误用业务数据。
    //
    // 7 个新 tool（brainstorm §v0.17 cycle 实际范围表对齐）：
    // - record 暴露类别（2 tool，stage 6 实装）：proc_record_start / proc_record_stop
    // - USB release 类别（1 tool，stage 6 实装）：proc_usb_release
    // - docker-rm 类别（3 tool，stage 6 实装）：proc_docker_rm / image_rm / volume_rm
    // - 可观测性类别（1 tool，stage 4 实装）：proc_metrics_history
    // ====================================================================

    #[tool(
        name = "proc_record_start",
        description = "Start a recording subprocess (headless, no TUI). Args: confirm (must be true to acknowledge recording captures screen content including DNS names / process cmd), file_path (output .prec file path), duration_secs (optional, auto-stop after N seconds). Returns { ok, file_path, started_at, expected_duration_secs, pid }. confirm=false → ok=false + error. Stage 6 business logic: spawn `proc record --no-tui --output <path>` subprocess, handler holds record_handle across tool calls (ADR-0029)."
    )]
    fn proc_record_start(
        &self,
        Parameters(args): Parameters<RecordStartArgs>,
    ) -> Result<CallToolResult, McpError> {
        // v0.17 stage 6：传 &self.record_handle 让 helper 跨 tool call 保活 child。
        // no-default-features build 无此字段，走 stub 路径返「未实装」错误。
        #[cfg(feature = "mcp-persistent-state")]
        {
            ok_result(record::make_record_start_json(
                args.confirm,
                &args.file_path,
                args.duration_secs,
                &self.record_handle,
            ))
        }
        #[cfg(not(feature = "mcp-persistent-state"))]
        {
            let _ = (args.confirm, args.file_path, args.duration_secs);
            ok_result(record::make_record_start_disabled_json())
        }
    }

    #[tool(
        name = "proc_record_stop",
        description = "Stop a recording subprocess and wait for .prec file flush. Args: file_path (must match proc_record_start file_path). Returns { ok, file_path, size_bytes, duration_secs, frame_count, killed, exit_code? }. Stage 6 business logic: kill child + wait flush + read footer metadata (ADR-0029)."
    )]
    fn proc_record_stop(
        &self,
        Parameters(args): Parameters<RecordStopArgs>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(feature = "mcp-persistent-state")]
        {
            ok_result(record::make_record_stop_json(
                &args.file_path,
                &self.record_handle,
            ))
        }
        #[cfg(not(feature = "mcp-persistent-state"))]
        {
            let _ = args.file_path;
            ok_result(record::make_record_stop_disabled_json())
        }
    }

    #[tool(
        name = "proc_usb_release",
        description = "Kill locks + flush cache + eject device in one call (destructive, confirm required). Args: confirm (must be true), drive (e.g. \"E\" / \"E:\" / \"E:\\\\\"), kill_pids (process ids to kill, agent typically gets them from proc_eject_status locks[].pid), dry_run (optional, default false). Returns { ok, dry_run, action: \"release\", drive, killed_pids: [...], flushed: bool, ejected: bool }. confirm=false → ok=false + error. Stage 6 business logic: kill_locks → flush_write_cache (PowerShell blocking 3s+) → eject_device (ADR-0029)."
    )]
    fn proc_usb_release(
        &self,
        Parameters(args): Parameters<UsbReleaseArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_usb_release_json(
            args.confirm,
            &args.drive,
            &args.kill_pids,
            args.dry_run,
        ))
    }

    #[tool(
        name = "proc_docker_rm",
        description = "Remove a Docker container (destructive, confirm required). Args: confirm (must be true), container_id, force (optional, default false), volumes (optional, default false — also remove anonymous volumes). Returns { ok, container_id, removed: bool }. Stage 6 business logic: bollard API remove_container (ADR-0029)."
    )]
    fn proc_docker_rm(
        &self,
        Parameters(args): Parameters<DockerRmArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_docker_rm_json(
            args.confirm,
            &args.container_id,
            args.force,
            args.volumes,
        ))
    }

    #[tool(
        name = "proc_docker_image_rm",
        description = "Remove a Docker image (destructive, confirm required). Args: confirm (must be true), image_id, force (optional, default false), prune_children (optional, default false). Returns { ok, image_id, removed: bool }. Stage 6 business logic: bollard API remove_image (ADR-0029)."
    )]
    fn proc_docker_image_rm(
        &self,
        Parameters(args): Parameters<DockerImageRmArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_docker_image_rm_json(
            args.confirm,
            &args.image_id,
            args.force,
            args.prune_children,
        ))
    }

    #[tool(
        name = "proc_docker_volume_rm",
        description = "Remove a Docker volume (destructive, confirm required — data is permanently lost). Args: confirm (must be true), volume_name, force (optional, default false). Returns { ok, volume_name, removed: bool }. Stage 6 business logic: bollard API remove_volume (ADR-0029)."
    )]
    fn proc_docker_volume_rm(
        &self,
        Parameters(args): Parameters<DockerVolumeRmArgs>,
    ) -> Result<CallToolResult, McpError> {
        ok_result(record::make_docker_volume_rm_json(
            args.confirm,
            &args.volume_name,
            args.force,
        ))
    }

    #[tool(
        name = "proc_metrics_history",
        description = "Query sparkline history (last N seconds, default 30s, max 30s). Args: metric (\"cpu\" / \"memory\" / \"swap\"), seconds (optional, default 30, max 30 because system_history field is 30s cap). Returns { ok, metric, seconds, count, samples: [{ value }] } ordered oldest → newest. Stage 4 TD-52 business logic: drain ProcMcpHandler.system_history VecDeque (1s tick push, 30s cap) + extract metric data points (ADR-0026 / ADR-0027). Empty history (worker warm-up / Default path / no-default-features build) returns count=0 + samples=[]."
    )]
    fn proc_metrics_history(
        &self,
        Parameters(args): Parameters<MetricsHistoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        #[cfg(feature = "mcp-persistent-state")]
        {
            ok_result(observable::make_metrics_history_json(
                &args.metric,
                args.seconds,
                &self.system_history,
            ))
        }
        // --no-default-features 路径无 system_history 字段，返 stub 让 client 知道
        // （observable::make_metrics_history_json_no_state 返 count=0 + note）
        #[cfg(not(feature = "mcp-persistent-state"))]
        {
            ok_result(observable::make_metrics_history_json_no_state(
                &args.metric,
                args.seconds,
            ))
        }
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
                // v0.17 stage 4：暴露 resources capability 让 client 知道此 server
                // 支持 resources/list + resources/read + resources/subscribe。
                // subscribe = true（决策 5：接受请求但不 push，client 走 polling）。
                resources: Some(rmcp::model::ResourcesCapability {
                    subscribe: Some(true),
                    list_changed: None,
                }),
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

    // ========================================================================
    // v0.17 stage 4 Resource subscribe（3 URI 路由 + subscribe no-op）
    //
    // rmcp 0.11 ServerHandler trait 的 list_resources / read_resource / subscribe
    // / unsubscribe 默认 impl 返空 / method_not_found。我们覆盖：
    // - list_resources：返 3 个 proc:// URI 全列表（含 name / description / mime_type）
    // - read_resource：路由到 ResourceRoute::route() 返 JSON snapshot
    // - subscribe / unsubscribe：接受请求返 Ok（决策 5：不做实际 push，client 走 polling）
    // ========================================================================

    fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::ListResourcesResult, McpError>> + Send + '_ {
        let resources: Vec<rmcp::model::Resource> = crate::mcp::resources::PROC_RESOURCE_URIS
            .iter()
            .map(|uri| {
                let raw = rmcp::model::RawResource {
                    uri: (*uri).to_string(),
                    name: crate::mcp::resources::resource_name_for_uri(uri).to_string(),
                    title: None,
                    description: Some(
                        crate::mcp::resources::resource_description_for_uri(uri).to_string(),
                    ),
                    mime_type: Some("application/json".to_string()),
                    size: None,
                    icons: None,
                    meta: None,
                };
                rmcp::model::Annotated::new(raw, None)
            })
            .collect();
        std::future::ready(Ok(rmcp::model::ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        }))
    }

    fn read_resource(
        &self,
        request: rmcp::model::ReadResourceRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<rmcp::model::ReadResourceResult, McpError>> + Send + '_ {
        let uri_str = request.uri.as_str().to_string();
        match crate::mcp::resources::ResourceRoute::route(self, &uri_str) {
            Ok(value) => {
                let text = value.to_string();
                let result = rmcp::model::ReadResourceResult {
                    contents: vec![rmcp::model::ResourceContents::TextResourceContents {
                        uri: uri_str.clone(),
                        mime_type: Some("application/json".to_string()),
                        text,
                        meta: None,
                    }],
                };
                std::future::ready(Ok(result))
            }
            Err(msg) => std::future::ready(Err(rmcp::ErrorData::invalid_request(msg, None))),
        }
    }

    /// v0.17 stage 4 决策 5：subscribe 接受请求但不 push。
    ///
    /// client 订阅后返 Ok(())，但 server 不主动发 `notifications/resources/updated`。
    /// client 应通过 `resources/read` 主动 polling（与既有 `proc_metrics_system` tool
    /// 一次性 query 同款语义）。v0.18+ cycle 评估 server 主动 push（需 Peer<RoleServer>
    /// 句柄 + notification channel lifecycle 管理，与 SSE transport 同款复杂度）。
    fn subscribe(
        &self,
        _request: rmcp::model::SubscribeRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Ok(()))
    }

    fn unsubscribe(
        &self,
        _request: rmcp::model::UnsubscribeRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Ok(()))
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
    processes_json_from_snapshot(&snapshot, sort, limit)
}

/// v0.17 stage 3 TD-54：从已有 SystemSnapshot 读字段（生产路径，避免现场 new）。
pub(crate) fn processes_json_from_snapshot(
    snapshot: &collect::SystemSnapshot,
    sort: Option<&str>,
    limit: Option<usize>,
) -> Value {
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
    process_tree_json_from_snapshot(&snapshot)
}

/// v0.17 stage 3 TD-54：从已有 SystemSnapshot 读字段（生产路径）。
pub(crate) fn process_tree_json_from_snapshot(snapshot: &collect::SystemSnapshot) -> Value {
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
