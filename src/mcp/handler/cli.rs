//! MCP `proc_*` tool — 类别 1（CLI 已有命令暴露）Args + helper。
//!
//! v0.15 cycle stage 1 Spike 落地骨架（Args struct + stub helper），stage 2 Slice
//! 填业务逻辑（本文件）。详见 [`super`] 模块文档与 `docs/stages/v0.15-stage-2.md`。
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
// Helpers — stage 2 业务逻辑实装（替换 stage 1 stub）。
//
// 失败路径返 `{ ok: false, error: <msg> }`（与 mod.rs 既有 helper 同款）。
// ===========================================================================

/// `proc_flows` — 列出当前活跃的 ProcessFlow（Schannel ETW SNI 来源）。
///
/// 走 `App::new() + 2s warm-up` 路径（与 `make_diag_json` 同款），让 Schannel
/// worker 起来收集首批事件。worker 未启动（非管理员 / x86 / session 占用）→
/// `worker: "unavailable"` + 空 flows。
pub fn make_flows_json(limit: Option<usize>) -> Value {
    let mut app = match crate::app::App::new() {
        Ok(a) => a,
        Err(e) => return super::err(format!("App::new failed: {e}")),
    };

    let worker_active = app.workers.schannel_etw_worker.is_some();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        app.tick();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let mut flows: Vec<&crate::flow::ProcessFlow> = app.flows.iter().collect();
    let default_limit = 50usize;
    let n = limit.unwrap_or(default_limit);
    flows.truncate(n);

    let arr: Vec<Value> = flows
        .iter()
        .map(|f| {
            json!({
                "pid": f.pid,
                "start_time": f.start_time,
                "comm": f.comm,
                "local_addr": f.local_addr,
                "remote_addr": f.remote_addr,
                "remote_port": f.remote_port,
                "bytes_out": f.bytes_out,
                "bytes_in": f.bytes_in,
                "dns_name": f.dns_name,
                "sni": f.sni,
                "first_seen_unix": system_time_to_unix(f.first_seen),
                "last_seen_unix": system_time_to_unix(f.last_seen),
                "is_ghost": f.is_ghost(),
            })
        })
        .collect();

    json!({
        "ok": true,
        "count": arr.len(),
        "worker": if worker_active { "schannel_etw" } else { "unavailable" },
        "flows": arr,
    })
}

/// `proc_throttle` — 查询 / 设置 Windows 11 EcoQoS（Efficiency Mode）。
///
/// - `set=None` → 查询当前状态（Normal / Eco / Unknown）
/// - `set=Some(true)` → 启用 EcoQoS（🍃）
/// - `set=Some(false)` → 恢复 Normal
///
/// 非 Windows / Win11 build < 22000 / 权限不足 → `state: "Unknown"` 或 `error`。
pub fn make_throttle_json(pid: u32, set: Option<bool>) -> Value {
    match set {
        None => {
            #[cfg(target_os = "windows")]
            {
                let state = crate::throttle::query_throttle(pid);
                let label = match state {
                    crate::throttle::EcoQoSState::Normal => "Normal",
                    crate::throttle::EcoQoSState::Eco => "Eco",
                    crate::throttle::EcoQoSState::Unknown => "Unknown",
                };
                json!({
                    "ok": true,
                    "pid": pid,
                    "action": "get",
                    "state": label,
                })
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = pid;
                json!({
                    "ok": false,
                    "pid": pid,
                    "action": "get",
                    "error": "EcoQoS is Windows 11 only (ADR-0022)",
                })
            }
        }
        Some(eco) => {
            #[cfg(target_os = "windows")]
            {
                match crate::throttle::set_throttle(pid, eco) {
                    Ok(()) => {
                        let new_state = crate::throttle::query_throttle(pid);
                        let label = match new_state {
                            crate::throttle::EcoQoSState::Normal => "Normal",
                            crate::throttle::EcoQoSState::Eco => "Eco",
                            crate::throttle::EcoQoSState::Unknown => "Unknown",
                        };
                        json!({
                            "ok": true,
                            "pid": pid,
                            "action": "set",
                            "requested": eco,
                            "state": label,
                        })
                    }
                    Err(e) => json!({
                        "ok": false,
                        "pid": pid,
                        "action": "set",
                        "error": e.to_string(),
                    }),
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = eco;
                json!({
                    "ok": false,
                    "pid": pid,
                    "action": "set",
                    "error": "EcoQoS is Windows 11 only (ADR-0022)",
                })
            }
        }
    }
}

/// `proc_export` — 导出进程列表为 JSON 或 CSV。
///
/// 与 CLI `proc export` 同款路径：`SystemSnapshot` + `format::export_processes_as_*`。
/// 默认 `format=None` → JSON；agent 拿到的就是 stdout 字符串（不写文件）。
///
/// v0.17 stage 3 TD-54：保留旧签名作 fallback 路径，生产路径走
/// [`export_json_from_snapshot`]。
pub fn make_export_json(format: Option<&str>, sort: Option<&str>, limit: Option<usize>) -> Value {
    let mut snapshot = match crate::collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return super::err(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return super::err(format!("snapshot refresh failed: {e}"));
    }
    let _ = snapshot.refresh_heavy_incremental();
    export_json_from_snapshot(&snapshot, format, sort, limit)
}

/// v0.17 stage 3 TD-54：从已有 SystemSnapshot 读字段（生产路径）。
pub(crate) fn export_json_from_snapshot(
    snapshot: &crate::collect::SystemSnapshot,
    format: Option<&str>,
    sort: Option<&str>,
    limit: Option<usize>,
) -> Value {
    let mut processes = snapshot.cached_processes_vec();
    let sort_field = super::parse_sort_field(sort);
    crate::collect::sort_processes(&mut processes, sort_field);
    if let Some(n) = limit {
        processes.truncate(n);
    }

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let fmt_str = format.unwrap_or("json").to_lowercase();
    let payload = match fmt_str.as_str() {
        "csv" => crate::format::export_processes_as_csv(&processes),
        _ => crate::format::export_processes_as_json(&processes, epoch_secs),
    };

    json!({
        "ok": true,
        "format": if fmt_str == "csv" { "csv" } else { "json" },
        "sort": super::sort_field_label(sort_field),
        "count": processes.len(),
        "payload": payload,
    })
}

/// `proc_docker_inspect` — 容器详情（health + stats + 基础信息）。
///
/// 与 CLI `proc docker inspect` 同款：list_containers 找到目标 → inspect_health +
/// get_stats。容器不存在 → `ok: false`。
pub fn make_docker_inspect_json(name: &str) -> Value {
    let monitor = match crate::docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => return super::err(format!("Docker connect failed: {e}")),
    };

    let containers = match monitor.list_containers(true) {
        Ok(v) => v,
        Err(e) => return super::err(format!("list_containers failed: {e}")),
    };
    let Some(found) = containers
        .into_iter()
        .find(|c| c.name == name || c.id.starts_with(name))
    else {
        return super::err(format!("container '{name}' not found"));
    };

    let health_json = match monitor.inspect_health(name) {
        Ok(h) => json!({ "ok": true, "data": format!("{h}") }),
        Err(e) => json!({ "ok": false, "error": e.to_string() }),
    };

    let stats_json = match monitor.get_stats(name) {
        Ok(s) => json!({
            "cpu_percent": s.cpu_percent,
            "memory_usage": s.memory_usage,
            "memory_limit": s.memory_limit,
            "network_in": s.network_in,
            "network_out": s.network_out,
        }),
        Err(e) => json!({ "error": e.to_string() }),
    };

    json!({
        "ok": true,
        "container": {
            "id": found.id,
            "name": found.name,
            "image": found.image,
            "state": found.state,
            "status": found.status,
            "health": format!("{}", found.health),
            "ports": found.ports,
            "running_since_unix": found.running_since.map(system_time_to_unix),
        },
        "health_detail": health_json,
        "stats": stats_json,
    })
}

/// `proc_docker_images` — 本地镜像列表（含 in_use 反查）。
pub fn make_docker_images_json() -> Value {
    let monitor = match crate::docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => return super::err(format!("Docker connect failed: {e}")),
    };
    let images = match monitor.list_images() {
        Ok(v) => v,
        Err(e) => return super::err(format!("list_images failed: {e}")),
    };
    let arr: Vec<Value> = images
        .iter()
        .map(|img| {
            json!({
                "id": img.id,
                "short_id": img.short_id,
                "repo_tags": img.repo_tags,
                "created": img.created,
                "size": img.size,
                "containers": img.containers,
                "in_use": img.in_use(),
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "images": arr })
}

/// `proc_docker_volumes` — Docker volume 列表（含 in_use 反查）。
pub fn make_docker_volumes_json() -> Value {
    let monitor = match crate::docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => return super::err(format!("Docker connect failed: {e}")),
    };
    let volumes = match monitor.list_volumes() {
        Ok(v) => v,
        Err(e) => return super::err(format!("list_volumes failed: {e}")),
    };
    let arr: Vec<Value> = volumes
        .iter()
        .map(|v| {
            json!({
                "name": v.name,
                "driver": v.driver,
                "mountpoint": v.mountpoint,
                "created": v.created,
                "size": v.size,
                "in_use": v.in_use,
            })
        })
        .collect();
    json!({ "ok": true, "count": arr.len(), "volumes": arr })
}

/// `proc_docker_events` — drain 最近一批 Docker daemon 事件（一次性，非 follow）。
///
/// MCP 是 request-response 模型，不能 follow event stream。本 helper 启动
/// [`crate::docker::events::spawn_event_watcher`] → 500ms 短超时窗口 → drain
/// 接收端拿到的事件（按 limit 截断）。低活跃 daemon 可能返 0 条事件。
pub fn make_docker_events_json(limit: Option<usize>) -> Value {
    let monitor = match crate::docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => return super::err(format!("Docker connect failed: {e}")),
    };
    let docker_client = monitor.docker();
    let receiver = crate::docker::events::spawn_event_watcher(docker_client);

    // 500ms 短超时窗口 drain（与 brainstorm §决策 FAQ 同款）。低活跃 daemon 可能
    // 返 0 条事件——返 note 字段让 agent 理解为「暂无事件」而非错误。
    std::thread::sleep(std::time::Duration::from_millis(500));

    let default_limit = 100usize;
    let n = limit.unwrap_or(default_limit);
    let mut events: Vec<crate::docker::events::DockerEvent> = Vec::new();
    while let Some(ev) = receiver.try_recv() {
        if events.len() >= n {
            break;
        }
        events.push(ev);
    }

    let arr: Vec<Value> = events
        .iter()
        .map(|ev| {
            json!({
                "action": ev.action,
                "container_id": ev.container_id,
                "container_name": ev.container_name,
                "timestamp_unix": system_time_to_unix(ev.timestamp),
            })
        })
        .collect();

    json!({
        "ok": true,
        "count": arr.len(),
        "events": arr,
        "note": if arr.is_empty() {
            "no events in 500ms window (low-activity daemon)"
        } else {
            "drained non-follow (MCP request-response model)"
        },
    })
}

/// `proc_monitor_add` — 加监控（dry_run=false 默认，与 proc_kill v0.7 契约一致）。
///
/// `target_kind` = "pid" | "port" | "command"。`dry_run=true` 仅 preview 不真加。
pub fn make_monitor_add_json(
    target_kind: &str,
    target: &str,
    restart_policy: Option<&str>,
    dry_run: Option<bool>,
) -> Value {
    let parsed_target = match parse_monitor_target(target_kind, target) {
        Ok(t) => t,
        Err(msg) => {
            return json!({
                "ok": false,
                "target_kind": target_kind,
                "target": target,
                "dry_run": dry_run.unwrap_or(false),
                "error": msg,
            });
        }
    };
    let policy = parse_restart_policy(restart_policy);

    let dry = dry_run.unwrap_or(false);
    if dry {
        return json!({
            "ok": true,
            "dry_run": true,
            "preview": {
                "target_kind": target_kind,
                "target": target,
                "restart_policy": restart_policy_label(&policy),
            },
        });
    }

    let mut mgr = crate::monitor::MonitorManager::new();
    match mgr.add_monitor(parsed_target, policy.clone()) {
        Ok(id) => json!({
            "ok": true,
            "dry_run": false,
            "id": id,
            "target_kind": target_kind,
            "target": target,
            "restart_policy": restart_policy_label(&policy),
        }),
        Err(e) => json!({
            "ok": false,
            "dry_run": false,
            "target_kind": target_kind,
            "target": target,
            "error": e.to_string(),
        }),
    }
}

/// `proc_monitor_remove` — 按 ID 删监控（dry_run=false 默认）。
pub fn make_monitor_remove_json(id: &str, dry_run: Option<bool>) -> Value {
    let parsed_id: u32 = match id.parse() {
        Ok(v) => v,
        Err(_) => {
            return json!({
                "ok": false,
                "id": id,
                "dry_run": dry_run.unwrap_or(false),
                "error": format!("id must be a positive integer, got '{id}'"),
            });
        }
    };

    let dry = dry_run.unwrap_or(false);
    if dry {
        return json!({
            "ok": true,
            "dry_run": true,
            "preview": { "id": parsed_id },
        });
    }

    let mut mgr = crate::monitor::MonitorManager::new();
    match mgr.remove_monitor(parsed_id) {
        Ok(()) => json!({
            "ok": true,
            "dry_run": false,
            "id": parsed_id,
        }),
        Err(e) => json!({
            "ok": false,
            "dry_run": false,
            "id": parsed_id,
            "error": e.to_string(),
        }),
    }
}

// ===========================================================================
// 内部 helpers
// ===========================================================================

fn parse_monitor_target(
    target_kind: &str,
    target: &str,
) -> std::result::Result<crate::monitor::MonitorTarget, String> {
    use crate::monitor::MonitorTarget;
    match target_kind {
        "pid" => target
            .parse::<u32>()
            .map(|pid| MonitorTarget::ByPid { pid })
            .map_err(|_| format!("invalid pid '{target}'")),
        "port" => target
            .parse::<u16>()
            .map(|port| MonitorTarget::ByPort { port })
            .map_err(|_| format!("invalid port '{target}'")),
        "command" => {
            let mut parts = target.split_whitespace();
            let cmd = parts
                .next()
                .ok_or_else(|| "command target requires at least one word".to_string())?
                .to_string();
            let args: Vec<String> = parts.map(str::to_string).collect();
            Ok(MonitorTarget::ByCommand {
                cmd,
                args,
                cwd: None,
            })
        }
        other => Err(format!(
            "unknown target_kind '{other}' (valid: pid | port | command)"
        )),
    }
}

fn parse_restart_policy(policy: Option<&str>) -> crate::monitor::RestartPolicy {
    use crate::monitor::RestartPolicy;
    match policy {
        Some("auto_restart") => RestartPolicy::AutoRestart {
            max_retries: 5,
            base_backoff: 1,
            max_backoff: 30,
        },
        _ => RestartPolicy::NotifyOnly,
    }
}

fn restart_policy_label(p: &crate::monitor::RestartPolicy) -> &'static str {
    match p {
        crate::monitor::RestartPolicy::NotifyOnly => "notify_only",
        crate::monitor::RestartPolicy::AutoRestart { .. } => "auto_restart",
    }
}

fn system_time_to_unix(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
