//! agent tool 执行层 — 47 tool 的 dispatch 入口（stage 3b，决策 A）。
//!
//! 复用 MCP handler 的 `make_*_json` helper（ADR-0030「复用同一套 tool 的
//! Rust API」，备选 C 自环已否决）：每个 tool 从 `ToolCall.arguments`
//! 提取类型化参数 → 调对应 helper → `Value::to_string()` 作 ToolResult content。
//!
//! 附加层（MCP 路径没有的 agent 专属语义）：
//! - **写操作拦截**（决策 B）：`WRITE_TOOL_NAMES` 8 个破坏性 tool 在非交互
//!   CLI ask 模式直接拒绝（confirm gate 无交互通道，复用 ADR-0008/0029 契约）
//! - **result 截断**（决策 E）：`truncate_result` 超长内容按 char 截断
//! - **PII 过滤**（决策 E）：`apply_pii_filter` 送 LLM 前按 SECRET_PATTERNS
//!   12 关键字 mask 值（defense-in-depth——MCP 层 env reveal=false 已 mask 一层）
//! - **proc_ls agent 版**：支持 `filter`（FilterExpr）+ 默认 limit 20
//!   （MCP 版无 filter / 无默认 limit，1000 进程全量返回会爆 token）

use std::sync::OnceLock;

use serde_json::{Value, json};

use super::help;
use crate::agent::tool_registry::ToolRegistry;
use crate::agent::types::ToolCall;
use crate::inspect::env_mask::{SECRET_PATTERNS, mask_value};
use crate::mcp::handler::cli::{
    make_docker_events_json, make_docker_images_json, make_docker_inspect_json,
    make_docker_volumes_json, make_export_json, make_flows_json, make_monitor_add_json,
    make_monitor_remove_json, make_throttle_json,
};
use crate::mcp::handler::inspect::{InspectTab, make_inspect_json};
use crate::mcp::handler::metrics::{
    make_metrics_disk_io_json, make_metrics_gpu_json, make_metrics_smart_json,
    make_metrics_system_json, make_metrics_thermal_json,
};
use crate::mcp::handler::observable::make_metrics_history_json_no_state;
use crate::mcp::handler::record::{
    make_bookmarks_add_json, make_bookmarks_delete_json, make_bookmarks_edit_json,
    make_bookmarks_list_json, make_eject_status_json, make_replay_info_json,
    make_replay_search_json,
};
use crate::mcp::handler::{
    make_affinity_json, make_diag_json, make_dns_json, make_docker_logs_json, make_docker_ps_json,
    make_docker_top_json, make_eject_json, make_handles_json, make_monitor_list_json,
    make_port_json, make_priority_json, make_process_tree_json, make_smart_json, make_who_json,
    parse_sort_field,
};

/// 非交互 CLI ask 模式下拦截的破坏性 tool（决策 B）。
///
/// system prompt 教的「先解释 + 等 y/n」流程在单轮 CLI 没有交互通道——
/// 模型若直接 `proc_kill(confirm=true)` 会真杀进程。拦截后模型拿到错误
/// JSON，会转而向用户解释 + 给出等价命令行（few-shot 示例 3 教学意图）。
pub const WRITE_TOOL_NAMES: &[&str] = &[
    "proc_kill",
    "proc_pkill",
    "proc_usb_release",
    "proc_docker_rm",
    "proc_docker_image_rm",
    "proc_docker_volume_rm",
    "proc_record_start",
    "proc_record_stop",
];

/// tool result 送 LLM 前的截断上限（决策 E）。
///
/// 首轮实测 16K chars 会让多轮对话 prompt 溢出 8192 ctx（12K tokens 400 错），
/// 收紧到 8K chars；ctx 同步升到 16384（agent.toml）双保险。
pub const MAX_TOOL_RESULT_CHARS: usize = 8_000;

/// proc_ls 缺省 limit（agent 版；MCP 版 None = 全量）。
const DEFAULT_LS_LIMIT: usize = 20;

fn blocked_json(name: &str) -> Value {
    json!({
        "ok": false,
        "blocked": true,
        "tool": name,
        "error": "写操作已拦截：proc agent ask 是非交互单轮模式，没有确认（y/n）通道。\
                  请向用户解释影响并给出等价的 proc 命令行（如 proc kill <pid> / \
                  proc eject --release E:），让用户自己执行。",
    })
}

fn err_json(msg: impl Into<String>) -> Value {
    json!({ "ok": false, "error": msg.into() })
}

// ---------------------------------------------------------------------------
// 参数提取 helper（serde_json::Value → 类型化；缺省 / 类型不符返 None）
// ---------------------------------------------------------------------------

fn str_of<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn owned_str_of(args: &Value, key: &str) -> Option<String> {
    str_of(args, key).map(str::to_string)
}

fn u32_of(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
}

fn u16_of(args: &Value, key: &str) -> Option<u16> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok())
}

fn u8_of(args: &Value, key: &str) -> Option<u8> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u8::try_from(v).ok())
}

fn usize_of(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| usize::try_from(v).ok())
}

fn u64_of(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

fn bool_of(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

// ---------------------------------------------------------------------------
// 执行入口
// ---------------------------------------------------------------------------

/// 执行一个 tool call。返回的 `content` 已过截断 + PII 过滤（决策 E），
/// 可直接作为 `Message{role: Tool}` 的 tool_result 回填 LLM。
pub fn execute_tool(registry: &ToolRegistry, call: &ToolCall) -> crate::agent::types::ToolResult {
    let (value, is_error) = dispatch_value(registry, &call.name, &call.arguments);
    let content = truncate_result(&apply_pii_filter(&value.to_string()));
    crate::agent::types::ToolResult {
        tool_call_id: call.id.clone(),
        content,
        is_error,
    }
}

fn dispatch_value(registry: &ToolRegistry, name: &str, args: &Value) -> (Value, bool) {
    if WRITE_TOOL_NAMES.contains(&name) {
        return (blocked_json(name), true);
    }

    let v = match name {
        // ---- meta ----
        "proc_help" => {
            let category = str_of(args, "category").and_then(parse_category);
            let tools = help::execute(registry, category);
            json!({
                "ok": true,
                "count": tools.len(),
                "tools": tools.iter().map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })).collect::<Vec<_>>(),
            })
        }

        // ---- process ----
        "proc_ls" => agent_proc_ls(args),
        "proc_tree" => make_process_tree_json(),
        "proc_port" => make_port_json(u16_of(args, "port")),
        "proc_handles" => match u32_of(args, "pid") {
            Some(pid) => make_handles_json(pid),
            None => err_json("pid is required (use proc_who for file-based reverse lookup)"),
        },
        "proc_priority" => match u32_of(args, "pid") {
            Some(pid) => make_priority_json(pid, str_of(args, "set")),
            None => err_json("pid is required"),
        },
        "proc_affinity" => match u32_of(args, "pid") {
            Some(pid) => {
                // catalog schema 的 set 是 integer mask；MCP helper 收 hex 字符串。
                let set = args
                    .get("set")
                    .and_then(|v| v.as_u64())
                    .map(|m| format!("{m:#x}"));
                make_affinity_json(pid, set.as_deref())
            }
            None => err_json("pid is required"),
        },
        "proc_throttle" => match u32_of(args, "pid") {
            Some(pid) => {
                let set = match str_of(args, "mode").or_else(|| str_of(args, "set")) {
                    Some("on") | Some("true") => Some(true),
                    Some("off") | Some("false") => Some(false),
                    Some("status") | None => None,
                    Some(other) => {
                        return (
                            err_json(format!("invalid mode '{other}' (on/off/status)")),
                            false,
                        );
                    }
                };
                make_throttle_json(pid, set)
            }
            None => err_json("pid is required"),
        },
        "proc_export" => make_export_json(
            str_of(args, "format"),
            str_of(args, "sort"),
            usize_of(args, "limit"),
        ),
        "proc_who" => match str_of(args, "target_path").or_else(|| str_of(args, "path")) {
            Some(p) => make_who_json(p),
            None => err_json("target_path is required"),
        },

        // ---- performance ----
        "proc_metrics_system" => make_metrics_system_json(),
        "proc_metrics_gpu" => make_metrics_gpu_json(),
        "proc_metrics_disk_io" => make_metrics_disk_io_json(str_of(args, "device")),
        "proc_metrics_smart" => make_metrics_smart_json(str_of(args, "device")),
        "proc_metrics_thermal" => make_metrics_thermal_json(),
        "proc_metrics_history" => match str_of(args, "metric") {
            Some(m) => make_metrics_history_json_no_state(m, u8_of(args, "seconds")),
            None => err_json("metric is required (cpu / memory / swap)"),
        },
        "proc_smart" => make_smart_json(str_of(args, "device")),
        "proc_diag" => make_diag_json(),

        // ---- docker ----
        "proc_docker_ps" => make_docker_ps_json(),
        "proc_docker_top" => match owned_str_of(args, "name") {
            Some(n) => make_docker_top_json(&n),
            None => err_json("name is required"),
        },
        "proc_docker_logs" => match (owned_str_of(args, "name"), args.get("tail")) {
            (Some(n), tail) => {
                // catalog schema 的 tail 是 integer；MCP helper 收字符串（"100"）。
                let tail_str = tail
                    .and_then(|v| v.as_u64())
                    .map(|n| n.to_string())
                    .or_else(|| str_of(args, "tail").map(str::to_string));
                make_docker_logs_json(&n, tail_str.as_deref())
            }
            (None, _) => err_json("name is required"),
        },
        "proc_docker_inspect" => match owned_str_of(args, "name") {
            Some(n) => make_docker_inspect_json(&n),
            None => err_json("name is required"),
        },
        "proc_docker_images" => make_docker_images_json(),
        "proc_docker_volumes" => make_docker_volumes_json(),
        "proc_docker_events" => make_docker_events_json(usize_of(args, "limit")),

        // ---- usb ----
        "proc_eject" => make_eject_json(str_of(args, "drive")),
        "proc_eject_status" => match owned_str_of(args, "drive") {
            Some(d) => make_eject_status_json(&d),
            None => err_json("drive is required (e.g. \"E\")"),
        },

        // ---- security ----
        "proc_inspect" => match u32_of(args, "pid") {
            Some(pid) => {
                let tab = match str_of(args, "tab") {
                    None | Some("summary") => InspectTab::Summary,
                    Some("env") => InspectTab::Env,
                    Some("network") => InspectTab::Network,
                    Some("dlls") => InspectTab::Dlls,
                    Some("memory_map") | Some("memory") => InspectTab::MemoryMap,
                    Some("handles") => InspectTab::Handles,
                    Some(other) => {
                        return (
                            err_json(format!(
                                "invalid tab '{other}' (summary/env/network/dlls/memory_map/handles)"
                            )),
                            false,
                        );
                    }
                };
                make_inspect_json(pid, &tab, bool_of(args, "reveal").unwrap_or(false))
            }
            None => err_json("pid is required"),
        },

        // ---- recording ----
        "proc_replay_info" => {
            match owned_str_of(args, "path").or_else(|| owned_str_of(args, "file_path")) {
                Some(p) => make_replay_info_json(&p),
                None => err_json("path is required"),
            }
        }
        "proc_replay_search" => {
            match (
                owned_str_of(args, "path").or_else(|| owned_str_of(args, "file_path")),
                owned_str_of(args, "query"),
            ) {
                (Some(p), Some(q)) => make_replay_search_json(&p, &q, usize_of(args, "limit")),
                (None, _) => err_json("path is required"),
                (_, None) => err_json("query is required"),
            }
        }
        "proc_bookmarks_list" => {
            match owned_str_of(args, "path").or_else(|| owned_str_of(args, "file_path")) {
                Some(p) => make_bookmarks_list_json(&p),
                None => err_json("path is required"),
            }
        }
        "proc_bookmarks_add" => {
            match (
                owned_str_of(args, "path").or_else(|| owned_str_of(args, "file_path")),
                usize_of(args, "frame_idx"),
            ) {
                (Some(p), Some(idx)) => make_bookmarks_add_json(
                    &p,
                    idx,
                    str_of(args, "label"),
                    bool_of(args, "dry_run"),
                ),
                (None, _) => err_json("path is required"),
                (_, None) => err_json("frame_idx is required"),
            }
        }
        "proc_bookmarks_edit" => {
            match (
                owned_str_of(args, "path").or_else(|| owned_str_of(args, "file_path")),
                u64_of(args, "id"),
                owned_str_of(args, "label"),
            ) {
                (Some(p), Some(id), Some(label)) => {
                    make_bookmarks_edit_json(&p, id, &label, bool_of(args, "dry_run"))
                }
                (None, _, _) => err_json("path is required"),
                (_, None, _) => err_json("id is required"),
                (_, _, None) => err_json("label is required"),
            }
        }
        "proc_bookmarks_delete" => {
            match (
                owned_str_of(args, "path").or_else(|| owned_str_of(args, "file_path")),
                u64_of(args, "id"),
            ) {
                (Some(p), Some(id)) => make_bookmarks_delete_json(&p, id, bool_of(args, "dry_run")),
                (None, _) => err_json("path is required"),
                (_, None) => err_json("id is required"),
            }
        }

        // ---- flow ----
        // filter 参数当前忽略（make_flows_json 无 filter 通道；模型可拿全量后自行
        // 归纳，limit 兜底 token）。
        "proc_flows" => make_flows_json(usize_of(args, "limit").or(Some(50))),

        // ---- monitor ----
        "proc_monitor_list" => make_monitor_list_json(),
        "proc_monitor_add" => match (str_of(args, "target_kind"), str_of(args, "target")) {
            (Some(kind), Some(target)) => make_monitor_add_json(
                kind,
                target,
                str_of(args, "restart_policy"),
                bool_of(args, "dry_run"),
            ),
            (None, _) => err_json("target_kind is required (pid / port / command)"),
            (_, None) => err_json("target is required"),
        },
        "proc_monitor_remove" => match args.get("id").map(|v| match v.as_u64() {
            Some(n) => n.to_string(),
            None => v.as_str().map(str::to_string).unwrap_or_default(),
        }) {
            Some(id) if !id.is_empty() => make_monitor_remove_json(&id, bool_of(args, "dry_run")),
            _ => err_json("id is required"),
        },

        // ---- dns ----
        // 决策 A：agent 单 query 场景走现场采集版（MCP 持久 collector 是 server
        // 生命周期优化，agent CLI 一次性采集语义正确）。limit 截断 queries 数组。
        "proc_dns" => {
            let mut v = make_dns_json(false);
            if let Some(limit) = usize_of(args, "limit") {
                let n = v
                    .get_mut("queries")
                    .and_then(|q| q.as_array_mut())
                    .map(|q| {
                        q.truncate(limit);
                        q.len()
                    });
                if let (Some(n), Some(c)) = (n, v.get_mut("count")) {
                    *c = json!(n);
                }
            }
            v
        }

        _ => {
            return (
                err_json(format!(
                    "unknown tool '{name}'; call proc_help to discover available tools"
                )),
                true,
            );
        }
    };

    // 业务层失败（ok:false 的 JSON）不算 dispatch 异常——LLM 需要读错误语义
    // 自行调整（如 pid 不存在 → 换 proc_ls 找）。is_error=true 仅用于
    // 「调都调不对」（未知 tool / 写拦截 / 参数缺失）。
    (v, false)
}

/// category 字符串 → ToolCategory（runner 动态扩 tools 复用，决策 J）。
pub fn parse_category(s: &str) -> Option<crate::agent::types::ToolCategory> {
    use crate::agent::types::ToolCategory;
    Some(match s {
        "process" => ToolCategory::Process,
        "performance" => ToolCategory::Performance,
        "docker" => ToolCategory::Docker,
        "usb" => ToolCategory::Usb,
        "security" => ToolCategory::Security,
        "recording" => ToolCategory::Recording,
        "flow" => ToolCategory::Flow,
        "monitor" => ToolCategory::Monitor,
        "dns" => ToolCategory::Dns,
        _ => return None,
    })
}

/// proc_ls agent 版：filter（FilterExpr）+ 默认 limit 20（决策 E）。
///
/// MCP 版 `make_processes_json` 无 filter 通道且无默认 limit；agent 需要两者
/// （brainstorm 附录 A 多个 query 依赖 filter，1000 进程全量返回爆 token）。
/// 字段映射与 `processes_json_from_snapshot` 保持一致（同一 JSON 契约）。
fn agent_proc_ls(args: &Value) -> Value {
    let sort = str_of(args, "sort");
    let filter = str_of(args, "filter");
    let limit = usize_of(args, "limit").unwrap_or(DEFAULT_LS_LIMIT);

    let mut snapshot = match crate::collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return err_json(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return err_json(format!("snapshot refresh failed: {e}"));
    }
    let _ = snapshot.refresh_heavy_incremental();

    let filter_expr = match filter {
        Some(f) => match crate::filter::parse(f) {
            Ok(expr) => Some(expr),
            Err(e) => return err_json(format!("filter parse failed: {e}")),
        },
        None => None,
    };

    let mut processes = snapshot.cached_processes_vec();
    let sort_field = parse_sort_field(sort);
    crate::collect::sort_processes(&mut processes, sort_field);

    let (_, total_memory) = snapshot.memory_usage();
    if let Some(expr) = &filter_expr {
        // security_score 缺省 100（与 CLI proc ls --filter 同款契约——不现场算分）。
        processes.retain(|p| {
            expr.apply(&crate::filter::EvalCtx {
                process: p,
                security_score: Some(100),
                total_memory,
            })
        });
    }
    processes.truncate(limit);

    // agent 版字段瘦身（vs MCP 版）：cmd / virtual_memory / disk+net 速率 /
    // start_time / run_time 省略——20 行全字段 > 8K chars 会触发截断破坏 JSON，
    // 且这些低频字段按 PID 走 proc_inspect 更合适。
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
                "status": format!("{:?}", p.status),
                "parent_pid": p.parent_pid,
            })
        })
        .collect();

    let note = (processes.len() == limit && filter.is_none()).then(|| {
        format!(
            "truncated to {limit} rows (default limit); pass a larger limit or a filter to see more. \
             Use proc_inspect(pid) for full details of one process"
        )
    });
    json!({
        "ok": true,
        "sort": sort,
        "filter": filter,
        "count": arr.len(),
        "note": note,
        "processes": arr,
    })
}

// ---------------------------------------------------------------------------
// 截断 + PII 过滤（决策 E）
// ---------------------------------------------------------------------------

/// 超长 tool result 按 char 截断（UTF-8 安全），加 truncated 标记后缀。
pub fn truncate_result(content: &str) -> String {
    let total = content.chars().count();
    if total <= MAX_TOOL_RESULT_CHARS {
        return content.to_string();
    }
    let head: String = content.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!("{head}\n...[truncated, original {total} chars]")
}

/// PII 过滤：扫 `SECRET_PATTERNS` 12 关键字的赋值形态，值替换为
/// `mask_value` 同款 `{前2字符}***({len} B)`。
///
/// 两种形态（defense-in-depth，tool result 已是序列化 JSON 字符串）：
/// - JSON：`"FOO_API_KEY": "long-secret-value"`（值 ≥ 8 chars 才 mask，
///   `"api_version": "v1"` 这类短值不误伤）
/// - kv / CLI：`--api-key=long-secret-value` / `PASSWORD=long-secret`
pub fn apply_pii_filter(content: &str) -> String {
    static JSON_RE: OnceLock<regex::Regex> = OnceLock::new();
    static KV_RE: OnceLock<regex::Regex> = OnceLock::new();

    let json_re = JSON_RE.get_or_init(|| build_pii_regex(true));
    let out = json_re.replace_all(content, |caps: &regex::Captures| {
        format!("{}{}{}", &caps[1], mask_value(&caps[2]), &caps[3])
    });

    let kv_re = KV_RE.get_or_init(|| build_pii_regex(false));
    kv_re
        .replace_all(&out, |caps: &regex::Captures| {
            format!("{}{}", &caps[1], mask_value(&caps[2]))
        })
        .into_owned()
}

fn build_pii_regex(json_form: bool) -> regex::Regex {
    // 关键字集合复用 env_mask::SECRET_PATTERNS（大小写不敏感 substring）。
    let keywords = SECRET_PATTERNS.join("|");
    if json_form {
        // group1 = "KEY":\s*"  group2 = 值（≥8 chars，不含引号/转义）  group3 = 收尾引号
        regex::RegexBuilder::new(&format!(
            r#"(\"[A-Za-z0-9_\-]*(?:{keywords})[A-Za-z0-9_\-]*\"\s*:\s*")([^"\\]{{8,}})(")"#
        ))
        .case_insensitive(true)
        .build()
        .expect("PII json regex valid")
    } else {
        // group1 = KEY=  或  KEY:  group2 = 值（≥8 chars，遇空白/引号/结构符终止）
        regex::RegexBuilder::new(&format!(
            r#"((?:^|[\s,(\[{{])(?:--)?[A-Za-z0-9_\-]*(?:{keywords})[A-Za-z0-9_\-]*[=:]\s*)([^\s,"'()\[\]{{}}]{{8,}})"#
        ))
        .case_insensitive(true)
        .build()
        .expect("PII kv regex valid")
    }
}
