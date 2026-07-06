//! MCP `proc_inspect` tool — 类别 2（详情页 Tab 合并）Args + helper。
//!
//! v0.15 cycle stage 1 Spike 落地骨架，stage 2 Slice 填业务逻辑（本文件）。
//! 详见 [`super`] 模块文档与 `docs/stages/v0.15-stage-2.md` 和 ADR-0023。
//!
//! 边界：详情页 6 Tab 数据合并成 1 个 `proc_inspect(pid, tab=..., reveal=...)`
//! tool。`#[tool]` 方法本身在 [`super::mod_rs`] 的 `#[tool_router] impl` 块里
//! （rmcp 0.11 限制：`#[tool_router]` 只收集当前 impl 块内的 `#[tool]` 方法）。

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

/// `proc_inspect` tool 入参。
///
/// `tab` 选 6 个变体之一（默认 `Summary`）；`reveal` 仅 `Env` tab 生效（默认
/// `false` = mask secret 12 关键字，与 v0.6 详情页 env_reveal 同款契约）。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProcInspectArgs {
    /// Process PID to inspect.
    pub pid: u32,
    /// Tab to fetch (default summary).
    #[serde(default)]
    pub tab: InspectTab,
    /// Reveal masked secret values (env tab only).
    ///
    /// Default false = mask. secret pattern 12 关键字（KEY/TOKEN/SECRET/PASSWORD/
    /// PASSWD/PWD/CREDENTIAL/PRIVATE/AUTH/API/DSN/CONNECTION_STRING）+ *_AUTHORIZATION
    /// 后缀 + DATABASE_URL 特例（v0.6 env_mask.rs 同款）。
    #[serde(default)]
    pub reveal: bool,
}

/// `proc_inspect` 的 tab 参数 enum。
///
/// **与 v0.5 TUI `InspectionTab` 同义但独立类型**——MCP 入参用 serde /
/// JsonSchema derive 集，TUI 状态机用 Display / Clone / PartialEq。两者不共享
/// 类型避免 derive 污染（surgical 原则）。详见 CONTEXT.md 术语 InspectTab。
#[derive(Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InspectTab {
    /// Summary + R1-R18 risk factors + signature_status + parent_chain (default).
    #[default]
    Summary,
    /// Environment variables (masked unless reveal=true).
    Env,
    /// Listening + established connections + recent 5 DNS queries for this PID.
    Network,
    /// Loaded modules (Windows DLL / Linux .so, sorted by path).
    Dlls,
    /// Memory map (VirtualQueryEx / /proc/<pid>/maps regions).
    MemoryMap,
    /// Open handles (same schema as the existing proc_handles tool).
    Handles,
}

impl InspectTab {
    /// 用于 helper 输出与日志 debug（stage 1 占位，stage 2 实装路径未用 — 留作
    /// stage 3/4 调试 anchor）。
    #[must_use]
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Env => "env",
            Self::Network => "network",
            Self::Dlls => "dlls",
            Self::MemoryMap => "memory_map",
            Self::Handles => "handles",
        }
    }
}

/// `proc_inspect` — 6 tab 字段裁剪入口（与 ADR-0023 设计对齐）。
///
/// 按 `tab` 分支：
/// - **Summary**：基本信息（pid/name/cpu/mem/cmd/exe/cwd/parent_pid/start_time/
///   run_time/throttled/signature_status）+ parent_chain + risk_factors
/// - **Env**：env_vars[]（secret 默认 mask，`reveal=true` 显示真值）
/// - **Network**：listening[] + established[] + dns_recent[]
/// - **Dlls**：dlls[]（path/base_addr/size）
/// - **MemoryMap**：regions[]（base_addr/size/state/protection/name）
/// - **Handles**：handles[]（与 `proc_handles` 同 schema）
///
/// 详情页视角返完整 cmd / exe / cwd 真值（与 v0.7 `proc_ls` 列表视角字段裁剪互补）。
pub fn make_inspect_json(pid: u32, tab: &InspectTab, reveal: bool) -> Value {
    // 拿 snapshot + 找进程
    let mut snapshot = match crate::collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return super::err(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return super::err(format!("snapshot refresh failed: {e}"));
    }
    let _ = snapshot.refresh_heavy_incremental();
    let processes = snapshot.cached_processes_vec();
    let proc_found = processes.iter().find(|p| p.pid == pid).cloned();

    match tab {
        InspectTab::Summary => make_summary_json(pid, &proc_found, &processes, &snapshot),
        InspectTab::Env => make_env_json(pid, reveal),
        InspectTab::Network => make_network_json(pid),
        InspectTab::Dlls => make_dlls_json(pid),
        InspectTab::MemoryMap => make_memory_json(pid),
        InspectTab::Handles => make_handles_tab_json(pid),
    }
}

// ===========================================================================
// Tab 分支实装
// ===========================================================================

fn make_summary_json(
    pid: u32,
    proc_found: &Option<crate::collect::ProcessInfo>,
    all_procs: &[crate::collect::ProcessInfo],
    snapshot: &crate::collect::SystemSnapshot,
) -> Value {
    let Some(proc) = proc_found else {
        return json!({
            "ok": false,
            "pid": pid,
            "tab": "summary",
            "error": format!("process {pid} not found"),
        });
    };

    let (_, total_memory) = snapshot.memory_usage();
    let mem_pct = if total_memory > 0 {
        proc.memory as f64 / total_memory as f64 * 100.0
    } else {
        0.0
    };

    // 风险因子：调 SecurityScorer（local 调用，不依赖 background scorer）。
    // port_entries 现场 scan（与 SecurityScorer 调用方 HeavyWorker 不同——MCP
    // 不在主轮询路径，agent 调用频率低，可接受额外 scan 开销）。
    let port_entries = crate::port_map::scan_ports().unwrap_or_default();
    let flows: Vec<crate::flow::ProcessFlow> = Vec::new();
    let mut scorer = crate::security::SecurityScorer::new();
    let score = scorer.score(proc, all_procs, &port_entries, &flows);

    let risk_factors: Vec<Value> = score
        .factors
        .iter()
        .map(|f| {
            json!({
                "category": format!("{:?}", f.category),
                "name": f.name,
                "weight": f.weight,
                "description": f.description,
            })
        })
        .collect();

    let parent_chain: Vec<Value> = proc
        .parent_chain
        .iter()
        .map(|(ppid, pname)| json!({ "pid": ppid, "name": pname }))
        .collect();

    json!({
        "ok": true,
        "pid": pid,
        "tab": "summary",
        "process": {
            "pid": proc.pid,
            "name": proc.name.as_ref(),
            "cpu_usage": proc.cpu_usage,
            "memory_bytes": proc.memory,
            "memory_pct": (mem_pct * 100.0).round() / 100.0,
            "virtual_memory_bytes": proc.virtual_memory,
            "disk_read_bps": proc.disk_read_speed,
            "disk_write_bps": proc.disk_write_speed,
            "net_sent_bps": proc.net_sent_rate,
            "net_recv_bps": proc.net_recv_rate,
            "status": format!("{:?}", proc.status),
            "exe": proc.exe.as_ref(),
            "cmd": proc.cmd.as_ref(),
            "cwd": proc.cwd.as_ref(),
            "parent_pid": proc.parent_pid,
            "session_id": proc.session_id,
            "user_id": proc.user_id.as_ref(),
            "start_time_unix": proc.start_time,
            "run_time_secs": proc.run_time,
            "throttled": format!("{:?}", proc.throttled),
            "signature_status": format!("{:?}", proc.signature_status),
        },
        "parent_chain": parent_chain,
        "security_score": score.score,
        "signature_status": format!("{:?}", score.signature),
        "risk_factors": risk_factors,
    })
}

fn make_env_json(pid: u32, reveal: bool) -> Value {
    let vars = match crate::inspect::env::collect_env(pid) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "ok": false,
                "pid": pid,
                "tab": "env",
                "error": e.to_string(),
            });
        }
    };
    let arr: Vec<Value> = vars
        .iter()
        .map(|v| {
            let display_value = if v.is_secret && !reveal {
                crate::inspect::env_mask::mask_value(&v.value)
            } else {
                v.value.clone()
            };
            json!({
                "key": v.key,
                "value": display_value,
                "is_secret": v.is_secret,
            })
        })
        .collect();
    json!({
        "ok": true,
        "pid": pid,
        "tab": "env",
        "reveal": reveal,
        "count": arr.len(),
        "env_vars": arr,
    })
}

fn make_network_json(pid: u32) -> Value {
    let ports = crate::port_map::find_ports_by_pid(pid).unwrap_or_default();

    let listening: Vec<Value> = ports
        .iter()
        .filter(|p| {
            p.state
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case("LISTEN"))
                .unwrap_or(false)
        })
        .map(port_entry_to_json)
        .collect();
    let established: Vec<Value> = ports
        .iter()
        .filter(|p| {
            p.state
                .as_deref()
                .map(|s| {
                    s.eq_ignore_ascii_case("ESTABLISHED") || s.eq_ignore_ascii_case("TIME_WAIT")
                })
                .unwrap_or(false)
        })
        .map(port_entry_to_json)
        .collect();

    // DNS 最近 5 条：现场 drain 一次 collector，按 pid 过滤
    let dns_recent = drain_dns_for_pid(pid, 5);

    json!({
        "ok": true,
        "pid": pid,
        "tab": "network",
        "listening": listening,
        "established": established,
        "dns_recent": dns_recent,
    })
}

fn make_dlls_json(pid: u32) -> Value {
    let dlls = crate::inspect::dlls::collect_dlls(pid).unwrap_or_default();
    let arr: Vec<Value> = dlls
        .iter()
        .map(|d| {
            json!({
                "path": d.path,
                "base_addr": d.base_addr,
                "size": d.size,
            })
        })
        .collect();
    json!({
        "ok": true,
        "pid": pid,
        "tab": "dlls",
        "count": arr.len(),
        "dlls": arr,
    })
}

fn make_memory_json(pid: u32) -> Value {
    let regions = crate::inspect::memory::collect_memory(pid).unwrap_or_default();
    let arr: Vec<Value> = regions
        .iter()
        .map(|r| {
            json!({
                "base_addr": r.base_addr,
                "size": r.size,
                "state": format!("{:?}", r.state),
                "protection": r.protection,
                "name": r.name,
            })
        })
        .collect();
    json!({
        "ok": true,
        "pid": pid,
        "tab": "memory_map",
        "count": arr.len(),
        "regions": arr,
    })
}

fn make_handles_tab_json(pid: u32) -> Value {
    // 复用既有 make_handles_json 的字段 schema（与 v0.7 proc_handles 同款）。
    super::make_handles_json(pid)
}

// ===========================================================================
// 内部 helpers
// ===========================================================================

fn port_entry_to_json(p: &crate::port_map::PortEntry) -> Value {
    json!({
        "protocol": format!("{:?}", p.protocol),
        "local_addr": p.local_addr.to_string(),
        "local_port": p.local_port,
        "remote_addr": p.remote_addr.map(|a| a.to_string()),
        "remote_port": p.remote_port,
        "state": p.state,
        "pid": p.pid,
        "process_name": p.process_name,
        "rtt_ms": p.rtt_ms,
    })
}

/// 现场调 detect_collector() 拿一个临时 collector，drain 一次，按 pid 过滤。
/// MCP 不持有持久 collector（与 proc_dns 持久路径不同），调用方需要快速看一眼
/// 该 pid 的最近 DNS 查询。collector 不可用 → 空 Vec（不致命）。
fn drain_dns_for_pid(pid: u32, limit: usize) -> Vec<Value> {
    let (mut collector, _kind) = crate::dns_log::detect_collector();
    let Some(collector) = collector.as_mut() else {
        return Vec::new();
    };
    let queries = collector.drain();
    queries
        .into_iter()
        .filter(|q| q.pid == pid)
        .take(limit)
        .map(|q| {
            json!({
                "query_name": q.query_name,
                "query_type": q.query_type,
                "timestamp_unix": q.start_time,
            })
        })
        .collect()
}
