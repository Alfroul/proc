//! MCP `proc_replay_*` / `proc_bookmarks_*` / `proc_eject_status` tool —
//! v0.16 cycle 子 module（类别 5：录屏 v2 replay + bookmarks + USB status）。
//!
//! v0.16 cycle stage 1 Spike 落地骨架（Args struct + stub helper），stage 2 Slice
//! 填 replay + eject_status 业务逻辑，stage 3 Slice 填 bookmarks 业务逻辑。
//! 详见 [`super`] 模块文档与 `docs/stages/v0.16-stage-{1,2,3}.md`。
//!
//! 边界：录屏 v2 相关 tool（replay + bookmarks）+ USB status tool 统一放本文件
//! （brainstorm §MCP handler 子 module 扩展 决策——v0.16 cycle 7 tool 量级不需
//! 再拆 usb.rs）。`#[tool]` 方法本身在 [`super::mod_rs`] 的 `#[tool_router] impl`
//! 块里（rmcp 0.11 限制：`#[tool_router]` 只收集当前 impl 块内的 `#[tool]` 方法）。
//!
//! 子 module 命名与 [`crate::record`] 业务模块同名是巧合——前者是 MCP handler
//! 容器（路径 `crate::mcp::handler::record`），后者是录屏 v2 业务模块（路径
//! `crate::record`），Rust 模块系统天然区分。stage 2/3 业务实装时 helper 内调
//! `crate::record::Player::open(...)` / `crate::record::BookmarkFile::load_or_empty(...)`
//! 用全限定路径避免 `use crate::record::*;` 与本子 module 名字冲突（详见 stage-1
//! 已知风险 3）。

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::filter::{
    self, FilterExpr, FrameEvalCtx, FrameField, Value as FilterValue, parse_frame,
};
use crate::record::{Player, VtPlayer, is_vt100_file};

// ===========================================================================
// Args structs — 7 tool（replay 2 + bookmarks 4 + eject_status 1）
//
// 每个 tool 一个 Args struct，rmcp 用 schemars 生成 JSON Schema 给 LLM 看。
// 字段文档 = schema description，写清楚 LLM 才能正确调用。
// ===========================================================================

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReplayInfoArgs {
    /// Recording file path (.prec extension). v3 UiFrame or VT100 format auto-detected.
    pub file_path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReplaySearchArgs {
    /// Recording file path (.prec extension, v3 UiFrame only — VT100 returns ok=false).
    pub file_path: String,
    /// Search query: `chrome` (substring → name regex) or `: cpu > 80 AND name =~ /chrome/`
    /// (FilterExpr with 5 dimensions: timestamp/cpu/mem/name/anomaly.severity).
    pub query: String,
    /// Max matches to return (default 100). truncated=true when match_count > returned.
    /// Agent can re-call with higher limit if needed.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BookmarksListArgs {
    /// Recording file path (.prec extension, v3 UiFrame or VT100).
    pub file_path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BookmarksAddArgs {
    /// Recording file path.
    pub file_path: String,
    /// Frame index (0-based, must be < total_frames).
    pub frame_idx: usize,
    /// Bookmark label. None / empty → default "书签 #N".
    #[serde(default)]
    pub label: Option<String>,
    /// Dry-run preview (default false = real add + write sidecar; true = preview only).
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BookmarksEditArgs {
    /// Recording file path.
    pub file_path: String,
    /// Bookmark id to edit.
    pub id: u64,
    /// New label text.
    pub label: String,
    /// Dry-run preview (default false).
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct BookmarksDeleteArgs {
    /// Recording file path.
    pub file_path: String,
    /// Bookmark id to delete.
    pub id: u64,
    /// Dry-run preview (default false).
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct EjectStatusArgs {
    /// Drive letter ("E" / "E:" / "E:\\" all accepted, same normalize as proc_eject).
    pub drive: String,
}

// ===========================================================================
// Helpers — stage 1 stub（返 placeholder JSON）
//
// stage 2 Slice 替换 replay + eject_status 三个 stub 为真实业务实现
// （走 crate::record::Player / crate::replay::ReplaySearch /
// crate::eject::scan_device_locks）；stage 3 Slice 替换 bookmarks 四个 stub。
//
// stub 返回格式（与 v0.15 stage 1 决策 4 同款）：
// - `ok: true` 让 client（mcp-inspector）调用不报错，能验证 schema 正确生成
// - `stub: true` 让 LLM / client 识别「这是占位返回」避免误用
// - `stage` 字段版本标记让 stage 2/3 完工时容易 grep 验证替换
// - `received_*` 字段保留参数 echo，方便 stage 2/3 调试 schema 反序列化
// ===========================================================================

/// `proc_replay_info` — 读取录屏元数据（双路径：v3 UiFrame footer / VT100 header）。
///
/// 走 [`is_vt100_file`] 分发：VT100 路径用 [`VtPlayer`] 返 `format: "vt100"` +
/// header 字段；v3 路径用 [`Player`] 返 `format: "uiframe"` + 完整 footer 字段。
/// 共通字段：`has_bookmarks_sidecar`、`path`、`size_bytes`。
///
/// 失败路径（文件不存在 / IO 错误 / 反序列化失败）→ `{ ok: false, error: <msg> }`。
pub fn make_replay_info_json(file_path: &str) -> Value {
    let path = std::path::Path::new(file_path);

    let size_bytes = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => {
            return super::err(format!("录屏文件不存在: {file_path} ({e})"));
        }
    };

    let has_bookmarks_sidecar = {
        let mut s = file_path.to_string();
        s.push_str(".bookmarks.json");
        std::path::Path::new(&s).exists()
    };

    if is_vt100_file(path) {
        let player = match VtPlayer::open(path.to_path_buf()) {
            Ok(p) => p,
            Err(e) => {
                return super::err(format!("VT100 录屏打开失败: {e}"));
            }
        };
        let (start_ms, end_ms) = player.time_range_ms();
        let duration_secs = end_ms.saturating_sub(start_ms) / 1000;
        let header = player.header();
        return json!({
            "ok": true,
            "format": "vt100",
            "version": header.version,
            "start_time": header.start_time,
            "frame_count": player.total_frames(),
            "start_ms": start_ms,
            "end_ms": end_ms,
            "duration_secs": duration_secs,
            "width": player.width(),
            "height": player.height(),
            "has_bookmarks_sidecar": has_bookmarks_sidecar,
            "path": file_path,
            "size_bytes": size_bytes,
        });
    }

    let player = match Player::open(path.to_path_buf()) {
        Ok(p) => p,
        Err(e) => {
            return super::err(format!("录屏文件打开失败: {e}"));
        }
    };
    let header = player.header();
    let footer = player.meta();
    let (start_time, end_time) = player.time_range();
    let duration_secs = end_time.saturating_sub(start_time);

    json!({
        "ok": true,
        "format": "uiframe",
        "version": header.version,
        "hostname": header.hostname,
        "start_time": start_time,
        "end_time": end_time,
        "duration_secs": duration_secs,
        "frame_count": player.total_frames(),
        "anomaly_count": footer.anomaly_count,
        "event_count": footer.event_count,
        "max_cpu": footer.max_cpu,
        "max_mem": footer.max_mem,
        "has_bookmarks_sidecar": has_bookmarks_sidecar,
        "path": file_path,
        "size_bytes": size_bytes,
    })
}

/// `proc_replay_search` — FilterExpr / substring 双入口 + 全帧遍历 + limit 截断。
///
/// query `:` 前缀走 [`parse_frame`]（FilterExpr 模式，5 维度 timestamp/cpu/mem/name/
/// anomaly.severity），无 `:` 前缀走 [`filter::build_frame_substring_expr`]（substring
/// → `name =~ /query/i` regex）。VT100 路径不支持 search（无结构化帧）。
///
/// 返回 `{ ok, query, match_count, returned, truncated, limit, matches: [...] }`，
/// matches[] 含 frame_idx / timestamp / cpu_usage / memory_used / matched_processes[]
/// / anomaly_severity。详见 ADR-0025a。
pub fn make_replay_search_json(file_path: &str, query: &str, limit: Option<usize>) -> Value {
    let path = std::path::Path::new(file_path);

    if is_vt100_file(path) {
        return super::err("VT100 录屏不支持 search（仅 v3 UiFrame）");
    }

    let player = match Player::open(path.to_path_buf()) {
        Ok(p) => p,
        Err(e) => {
            return super::err(format!("录屏文件打开失败: {e}"));
        }
    };

    // parse query（与 src/replay/search.rs::reparse 同款双入口）
    let parse_result = if let Some(stripped) = query.strip_prefix(':') {
        parse_frame(stripped)
    } else {
        filter::build_frame_substring_expr(query)
    };
    let expr = match parse_result {
        Ok(e) => e,
        Err(e) => {
            return super::err(format!("搜索表达式解析失败: {}", e.msg));
        }
    };

    let limit = limit.unwrap_or(100);
    let total = player.total_frames();

    // 全帧遍历，apply_frame 命中收集 idx（升序，与 timeline 视觉顺序一致）
    let mut all_matches: Vec<usize> = Vec::new();
    for idx in 0..total {
        if let Some(frame) = player.frame_at(idx) {
            let ctx = FrameEvalCtx { frame: &frame };
            if expr.apply_frame(&ctx) {
                all_matches.push(idx);
            }
        }
    }

    let match_count = all_matches.len();
    let truncated = match_count > limit;
    let taken: Vec<usize> = all_matches.into_iter().take(limit).collect();

    let matches: Vec<Value> = taken
        .iter()
        .filter_map(|&idx| player.frame_at(idx).map(|frame| (idx, frame)))
        .map(|(idx, frame)| {
            let matched_processes = collect_matched_processes(&frame, &expr);
            let anomaly_severity = highest_anomaly_severity(&frame);
            json!({
                "frame_idx": idx,
                "timestamp": frame.timestamp,
                "cpu_usage": frame.cpu_usage,
                "memory_used": frame.memory_used,
                "matched_processes": matched_processes,
                "anomaly_severity": anomaly_severity,
            })
        })
        .collect();

    json!({
        "ok": true,
        "query": query,
        "match_count": match_count,
        "returned": matches.len(),
        "truncated": truncated,
        "limit": limit,
        "matches": matches,
    })
}

/// `proc_bookmarks_list` stub — stage 3 Slice 填 BookmarkFile::load_or_empty +
/// source_healthy 校验业务实现。
pub fn make_bookmarks_list_json(file_path: &str) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.16-stage-3",
        "message": "proc_bookmarks_list tool schema is registered; business logic lands in stage 3",
        "received_file_path": file_path,
    })
}

/// `proc_bookmarks_add` stub — stage 3 Slice 填 BookmarkFile::add + write
/// 业务实现（含 frame_idx 校验 + dry_run 路径）。
pub fn make_bookmarks_add_json(
    file_path: &str,
    frame_idx: usize,
    label: Option<&str>,
    dry_run: Option<bool>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.16-stage-3",
        "message": "proc_bookmarks_add tool schema is registered; business logic lands in stage 3",
        "received_file_path": file_path,
        "received_frame_idx": frame_idx,
        "received_label": label,
        "received_dry_run": dry_run,
    })
}

/// `proc_bookmarks_edit` stub — stage 3 Slice 填 BookmarkFile::edit_label +
/// write 业务实现。
pub fn make_bookmarks_edit_json(
    file_path: &str,
    id: u64,
    label: &str,
    dry_run: Option<bool>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.16-stage-3",
        "message": "proc_bookmarks_edit tool schema is registered; business logic lands in stage 3",
        "received_file_path": file_path,
        "received_id": id,
        "received_label": label,
        "received_dry_run": dry_run,
    })
}

/// `proc_bookmarks_delete` stub — stage 3 Slice 填 BookmarkFile::remove +
/// write 业务实现。
pub fn make_bookmarks_delete_json(file_path: &str, id: u64, dry_run: Option<bool>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.16-stage-3",
        "message": "proc_bookmarks_delete tool schema is registered; business logic lands in stage 3",
        "received_file_path": file_path,
        "received_id": id,
        "received_dry_run": dry_run,
    })
}

/// `proc_eject_status` — USB / removable device eject status（只读，不 kill / 不 flush）。
///
/// 4 档 suggestion 决策树（详见 brainstorm §决策 9）：
/// - `unknown_drive`：drive 字符无效 / 找不到匹配的 removable device
/// - `unavailable`：scan_all_devices / scan_device_locks 调用失败
/// - `kill_locks`：找到设备 + locks 非空（agent 走 proc_kill 杀进程后重查）
/// - `eject_now`：找到设备 + locks 空（可直接弹）
///
/// 返回字段：`{ ok, drive, device, ejectable, lock_count, locks, suggestion, warning? }`。
/// device=null / locks=[] / lock_count=0 / ejectable=false 是 unknown_drive 路径常态。
pub fn make_eject_status_json(drive: &str) -> Value {
    // drive 字符 normalize（与 src/mcp/handler/mod.rs::make_eject_json 同款 inline）
    let cleaned: String = drive.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let Some(letter) = cleaned.chars().next().map(|c| c.to_ascii_uppercase()) else {
        return json!({
            "ok": true,
            "drive": drive,
            "device": Value::Null,
            "ejectable": false,
            "lock_count": 0,
            "locks": Vec::<Value>::new(),
            "suggestion": "unknown_drive",
        });
    };

    let drive_label = format!("{letter}:");

    // 找设备
    let devices = match crate::eject::scan_all_devices() {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "ok": true,
                "drive": drive_label,
                "device": Value::Null,
                "ejectable": false,
                "lock_count": 0,
                "locks": Vec::<Value>::new(),
                "suggestion": "unavailable",
                "warning": format!("scan_all_devices failed: {e}"),
            });
        }
    };

    let Some(dev) = devices.iter().find(|d| d.drive_letter == letter) else {
        return json!({
            "ok": true,
            "drive": drive_label,
            "device": Value::Null,
            "ejectable": false,
            "lock_count": 0,
            "locks": Vec::<Value>::new(),
            "suggestion": "unknown_drive",
        });
    };

    let device_json = json!({
        "drive_letter": dev.drive_letter.to_string(),
        "label": dev.label,
        "total_bytes": dev.total_size,
        "used_bytes": dev.used_size,
        "fs_type": dev.file_system,
        "is_removable": true,
    });

    // 找 locks
    let locks = match crate::eject::scan_device_locks(letter) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "ok": true,
                "drive": drive_label,
                "device": device_json,
                "ejectable": false,
                "lock_count": 0,
                "locks": Vec::<Value>::new(),
                "suggestion": "unavailable",
                "warning": format!("scan_device_locks failed: {e}"),
            });
        }
    };

    let lock_count = locks.len();
    let ejectable = lock_count == 0;
    let suggestion = if ejectable { "eject_now" } else { "kill_locks" };

    let locks_json: Vec<Value> = locks
        .iter()
        .map(|(lock, risk)| {
            json!({
                "pid": lock.pid,
                "name": lock.process_name,
                "exe_path": lock.exe_path,
                "risk": format!("{risk:?}"),
            })
        })
        .collect();

    json!({
        "ok": true,
        "drive": drive_label,
        "device": device_json,
        "ejectable": ejectable,
        "lock_count": lock_count,
        "locks": locks_json,
        "suggestion": suggestion,
    })
}

// ===========================================================================
// 私有辅助函数 — stage 2 helpers（replay_search 字段收集）
//
// `collect_matched_processes` 让 matches[].matched_processes 字段在 expr 含 name
// 约束时返「真正匹配 name 的进程名集合」（agent 一眼看到 chrome / firefox 等关键
// 进程），无 name 约束时退化为「帧内所有进程名」（agent 看到当时跑了什么）。
//
// `highest_anomaly_severity` 让 matches[].anomaly_severity 字段返「帧内最高档」
// （critical > warning > info），让 agent 一眼判断「这一帧有 critical 异常」。
// ===========================================================================

/// 递归扫描 expr，收集 frame.processes 中**真正匹配 name 约束**的进程名集合。
///
/// 设计取舍（决策 6）：FrameRegex/FrameIn/FrameFieldCmp(field=Name) 走精准匹配；
/// 其他变体（cpu/mem/timestamp/anomaly.severity 约束）+ Not 不收集 → 返空 Vec。
/// 调用方根据空 / 非空决定 matched_processes 字段返集合还是全列。
fn collect_name_matches(frame: &crate::record::UiFrame, expr: &FilterExpr) -> Vec<String> {
    match expr {
        FilterExpr::FrameRegex {
            field: FrameField::Name,
            re,
            ..
        } => frame
            .processes
            .iter()
            .filter(|p| re.is_match(&p.name))
            .map(|p| p.name.clone())
            .collect(),
        FilterExpr::FrameIn {
            field: FrameField::Name,
            values,
        } => frame
            .processes
            .iter()
            .filter(|p| values.contains(&FilterValue::Text(p.name.clone())))
            .map(|p| p.name.clone())
            .collect(),
        FilterExpr::FrameFieldCmp {
            field: FrameField::Name,
            op,
            value,
        } => {
            let FilterValue::Text(target) = value else {
                return Vec::new();
            };
            frame
                .processes
                .iter()
                .filter(|p| op.apply_text(&p.name, target))
                .map(|p| p.name.clone())
                .collect()
        }
        FilterExpr::And(l, r) | FilterExpr::Or(l, r) => {
            let mut v = collect_name_matches(frame, l);
            v.extend(collect_name_matches(frame, r));
            v.sort();
            v.dedup();
            v
        }
        // Not 语义复杂（!name=~// 反向匹配难定义），FrameFieldCmp(Cpu/Mem/...) /
        // FrameRegex(AnomalySeverity) / FrameIn(AnomalySeverity) 等数值 / anomaly
        // 字段约束与具体进程不挂钩 → 不收集（调用方走「全列」fallback）。
        _ => Vec::new(),
    }
}

/// 收集 matched_processes[] —— expr 含 name 约束走精准匹配，否则全列帧内进程名。
fn collect_matched_processes(frame: &crate::record::UiFrame, expr: &FilterExpr) -> Vec<String> {
    let matched = collect_name_matches(frame, expr);
    if !matched.is_empty() {
        return matched;
    }
    frame.processes.iter().map(|p| p.name.clone()).collect()
}

/// 返回帧内最高档 anomaly severity（critical > warning > info > 其他 / 空）。
///
/// 帧无 anomaly → None（JSON 序列化为 null）。Agent 看一眼判断「这一帧是否有
/// critical 异常」，不必扫描整个 anomalies[]。
fn highest_anomaly_severity(frame: &crate::record::UiFrame) -> Option<String> {
    let mut max_rank: u8 = 0;
    let mut max_severity: Option<String> = None;
    for a in &frame.anomalies {
        let rank = match a.severity.as_str() {
            "critical" => 3,
            "warning" => 2,
            "info" => 1,
            _ => 0,
        };
        if rank > max_rank {
            max_rank = rank;
            max_severity = Some(a.severity.clone());
        }
    }
    max_severity
}
