//! MCP `proc_replay_*` / `proc_bookmarks_*` / `proc_eject_status` tool —
//! v0.16 cycle 子 module（类别 5：录屏 v2 replay + bookmarks + USB status）。
//!
//! v0.16 cycle stage 1 Spike 落地骨架（Args struct + stub helper），stage 2 Slice
//! 填 replay + eject_status 业务逻辑，stage 3 Slice 填 bookmarks 业务逻辑。
//! 详见 [`super`] 模块文档与 `docs/stages/v0.16-stage-{1,2,3}.md`。
//!
//! **v0.17 cycle 扩 6 个新 tool stub**（stage 1 Spike 落地，stage 6 Slice 填业务逻辑）：
//! - record 暴露类别（2 tool）：`proc_record_start` / `proc_record_stop`
//!   （spawn `proc record --no-tui` 子进程路径，ADR-0029 决策 4 拍板）
//! - USB release 类别（1 tool）：`proc_usb_release`（kill + flush + eject 三步链路）
//! - docker-rm 类别（3 tool）：`proc_docker_rm` / `proc_docker_image_rm` /
//!   `proc_docker_volume_rm`（bollard API 删容器 / 镜像 / 卷）
//!
//! 上述 5 个 tool 都 `confirm: bool` 必传（ADR-0029 决策 5 拍板）——confirm 与既有
//! `dry_run: bool` 默认 false 契约互补：dry_run 表示「不真正执行」，confirm 表示
//! 「确认风险后再执行」。
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
// Helpers — stage 2/3 业务逻辑全部落地
//
// stage 1 Spike 落地 7 个 stub（schema 占位），stage 2 Slice 替换 replay +
// eject_status 3 个 stub 为真实业务实现（走 crate::record::Player /
// crate::replay::ReplaySearch / crate::eject::scan_device_locks），stage 3 Slice
// 替换 bookmarks 4 个 stub（走 crate::record::BookmarkFile + 私有辅助函数
// validate_frame_idx_and_timestamp / write_sidecar）。
//
// 失败路径（文件不存在 / IO 错误 / deserialization 失败 / frame_idx 越界 / id 不存在）
// 统一走 `super::err(msg)` 返 `{ ok: false, error: <msg> }`。
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

/// `proc_bookmarks_list` — 列出录屏的所有书签（含 sidecar 状态校验）。
///
/// 走 [`crate::record::BookmarkFile::try_load`] 区分 fresh vs stale sidecar，返
/// `sidecar_present` + `source_healthy` 双字段让 agent 区分三态：
/// - **无 sidecar**：sidecar_present=false / source_healthy=true / bookmarks=[]
/// - **fresh sidecar**：sidecar_present=true / source_healthy=true / bookmarks=loaded
/// - **stale sidecar**（size/mtime 不匹配 / 损坏 / magic 错）：sidecar_present=true
///   / source_healthy=false / bookmarks=[]
///
/// 录屏文件不存在 → `{ ok: false, error: "录屏文件不存在" }`（brainstorm §决策 6
/// 路径安全：不返空列表避免 agent 误以为新建了空录屏）。
pub fn make_bookmarks_list_json(file_path: &str) -> Value {
    let path = std::path::Path::new(file_path);

    if !path.exists() {
        return super::err(format!("录屏文件不存在: {file_path}"));
    }

    let sidecar_path = crate::record::BookmarkFile::sidecar_path(path);
    let sidecar_present = sidecar_path.exists();

    // try_load 区分 fresh vs stale（None 时可能是「文件不存在」或「size/mtime/magic 校验失败」）
    let (source_healthy, bookmarks) = match crate::record::BookmarkFile::try_load(path) {
        Some(f) => (true, f.bookmarks),
        None => (!sidecar_present, Vec::new()),
    };

    let bookmarks_json: Vec<Value> = bookmarks
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "frame_idx": b.frame_idx,
                "timestamp_secs": b.timestamp_secs,
                "label": b.label,
                "created_at": b.created_at,
            })
        })
        .collect();

    json!({
        "ok": true,
        "count": bookmarks_json.len(),
        "sidecar_present": sidecar_present,
        "source_healthy": source_healthy,
        "bookmarks": bookmarks_json,
    })
}

/// `proc_bookmarks_add` — 给录屏加书签（双路径 frame_idx 校验 + sidecar 写盘）。
///
/// 流程：录屏存在校验 → frame_idx 校验（v3 用 [`Player`] / VT100 用 [`VtPlayer`]，
/// VT100 timestamp 走 `time_range_ms` 内插）→ `BookmarkFile::load_or_empty + add +
/// write`。label=None/空 → 默认「书签 #N」。
///
/// `dry_run=true` → 仍调 add（计算真实 id）但不写 sidecar。`dry_run=false` → 写盘，
/// 失败时返 `sidecar_written: false + warning`（决策 3，替代 [`crate::record::BookmarkFile::write`]
/// 静默失败）。
pub fn make_bookmarks_add_json(
    file_path: &str,
    frame_idx: usize,
    label: Option<&str>,
    dry_run: Option<bool>,
) -> Value {
    let path = std::path::Path::new(file_path);
    let dry_run = dry_run.unwrap_or(false);

    if !path.exists() {
        return super::err(format!("录屏文件不存在: {file_path}"));
    }

    let (_total_frames, timestamp_secs) = match validate_frame_idx_and_timestamp(path, frame_idx) {
        Ok(v) => v,
        Err(e) => return super::err(e),
    };

    let mut file = crate::record::BookmarkFile::load_or_empty(path);

    // label 默认值（None / 空字符串 → "书签 #N"，与 bookmarks.len()+1 对齐 stage 2 id 算法）
    let label_str = match label {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => format!("书签 #{}", file.bookmarks.len() + 1),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let bookmark = file.add(frame_idx, timestamp_secs, label_str.clone(), now);
    let id = bookmark.id;

    if dry_run {
        return json!({
            "ok": true,
            "dry_run": true,
            "action": "add",
            "id": id,
            "frame_idx": frame_idx,
            "label": label_str,
            "timestamp_secs": timestamp_secs,
            "sidecar_written": false,
        });
    }

    let (sidecar_written, warning) = write_sidecar(&file, path);
    let mut v = json!({
        "ok": true,
        "dry_run": false,
        "action": "add",
        "id": id,
        "frame_idx": frame_idx,
        "label": label_str,
        "timestamp_secs": timestamp_secs,
        "sidecar_written": sidecar_written,
    });
    if let Some(w) = warning {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("warning".to_string(), Value::String(w));
        }
    }
    v
}

/// `proc_bookmarks_edit` — 编辑书签 label（id 查找 + edit_label + write）。
///
/// 先查 bookmark 拿 `old_label`（让 agent 看到 diff），再调
/// [`crate::record::BookmarkFile::edit_label`] 修改 + write。id 不存在 →
/// `{ ok: false, error: "书签 id=<N> 不存在" }`。
pub fn make_bookmarks_edit_json(
    file_path: &str,
    id: u64,
    label: &str,
    dry_run: Option<bool>,
) -> Value {
    let path = std::path::Path::new(file_path);
    let dry_run = dry_run.unwrap_or(false);

    if !path.exists() {
        return super::err(format!("录屏文件不存在: {file_path}"));
    }

    let mut file = crate::record::BookmarkFile::load_or_empty(path);

    let old_label = match file.bookmarks.iter().find(|b| b.id == id) {
        Some(b) => b.label.clone(),
        None => return super::err(format!("书签 id={id} 不存在")),
    };
    let new_label = label.to_string();

    if dry_run {
        return json!({
            "ok": true,
            "dry_run": true,
            "action": "edit",
            "id": id,
            "old_label": old_label,
            "new_label": new_label,
            "sidecar_written": false,
        });
    }

    let edited = file.edit_label(id, new_label.clone());
    debug_assert!(edited, "edit_label 失败但 step 1 已确认 id 存在");

    let (sidecar_written, warning) = write_sidecar(&file, path);
    let mut v = json!({
        "ok": true,
        "dry_run": false,
        "action": "edit",
        "id": id,
        "old_label": old_label,
        "new_label": new_label,
        "sidecar_written": sidecar_written,
    });
    if let Some(w) = warning {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("warning".to_string(), Value::String(w));
        }
    }
    v
}

/// `proc_bookmarks_delete` — 删除书签（id 查找 + remove + write）。
///
/// 先查 bookmark 拿 `frame_idx + label`（让 agent 知道删了什么），再调
/// [`crate::record::BookmarkFile::remove`] 删除 + write。id 不存在 →
/// `{ ok: false, error: "书签 id=<N> 不存在" }`。
pub fn make_bookmarks_delete_json(file_path: &str, id: u64, dry_run: Option<bool>) -> Value {
    let path = std::path::Path::new(file_path);
    let dry_run = dry_run.unwrap_or(false);

    if !path.exists() {
        return super::err(format!("录屏文件不存在: {file_path}"));
    }

    let mut file = crate::record::BookmarkFile::load_or_empty(path);

    let (frame_idx, label) = match file.bookmarks.iter().find(|b| b.id == id) {
        Some(b) => (b.frame_idx, b.label.clone()),
        None => return super::err(format!("书签 id={id} 不存在")),
    };

    if dry_run {
        return json!({
            "ok": true,
            "dry_run": true,
            "action": "delete",
            "id": id,
            "frame_idx": frame_idx,
            "label": label,
            "sidecar_written": false,
        });
    }

    let removed = file.remove(id);
    debug_assert!(removed, "remove 失败但 step 1 已确认 id 存在");

    let (sidecar_written, warning) = write_sidecar(&file, path);
    let mut v = json!({
        "ok": true,
        "dry_run": false,
        "action": "delete",
        "id": id,
        "frame_idx": frame_idx,
        "label": label,
        "sidecar_written": sidecar_written,
    });
    if let Some(w) = warning {
        if let Some(obj) = v.as_object_mut() {
            obj.insert("warning".to_string(), Value::String(w));
        }
    }
    v
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

// ===========================================================================
// 私有辅助函数 — stage 3 helpers（bookmarks frame_idx 校验 + sidecar 写盘兜底）
//
// `validate_frame_idx_and_timestamp` 双路径校验 frame_idx 范围并提取该帧的 unix
// timestamp（v3 直接取 `UiFrame.timestamp` / VT100 走 `time_range_ms` 内插），让
// `make_bookmarks_add_json` 不重复写双路径逻辑。
//
// `write_sidecar` 替代 `BookmarkFile::write` 的静默失败路径——序列化失败 / IO 失败
// 时返 `false + Some(warning)`，让 handler 在 JSON 顶层加 `warning` 字段透出错误
// （brainstorm §决策 7）。
// ===========================================================================

/// 校验 frame_idx 在录屏帧范围内，返回 `(total_frames, timestamp_secs)`。
///
/// 双路径（brainstorm §Q2 + §Q4）：
/// - VT100 路径：`is_vt100_file` 真 → `VtPlayer::open + total_frames + time_range_ms 内插`
/// - v3 路径：`Player::open + total_frames + frame_at(frame_idx).timestamp`
///
/// 校验失败 / 打开失败返 `Err(message)`，调用方走 `super::err`。
fn validate_frame_idx_and_timestamp(
    path: &std::path::Path,
    frame_idx: usize,
) -> Result<(usize, u64), String> {
    if is_vt100_file(path) {
        let player =
            VtPlayer::open(path.to_path_buf()).map_err(|e| format!("VT100 录屏打开失败: {e}"))?;
        let total = player.total_frames();
        if frame_idx >= total {
            return Err(format!("frame_idx={frame_idx} 超出范围（总帧数={total}）"));
        }
        // VT100 内插：start_ms + (end_ms - start_ms) * frame_idx / (total - 1)
        // total=1 时退化为 start_ms / 1000（单帧 edge case，避免除零）
        let (start_ms, end_ms) = player.time_range_ms();
        let ts_secs = if total > 1 {
            let span = end_ms.saturating_sub(start_ms);
            let ts_ms = start_ms + span * (frame_idx as u64) / (total as u64 - 1);
            ts_ms / 1000
        } else {
            start_ms / 1000
        };
        Ok((total, ts_secs))
    } else {
        let player =
            Player::open(path.to_path_buf()).map_err(|e| format!("录屏文件打开失败: {e}"))?;
        let total = player.total_frames();
        if frame_idx >= total {
            return Err(format!("frame_idx={frame_idx} 超出范围（总帧数={total}）"));
        }
        let frame = player
            .frame_at(frame_idx)
            .ok_or_else(|| format!("frame_idx={frame_idx} 无法读取"))?;
        Ok((total, frame.timestamp))
    }
}

/// 写 sidecar 文件，返 `(sidecar_written, warning)`。
///
/// 替代 [`crate::record::BookmarkFile::write`] 的静默失败路径（brainstorm §决策 7）。
/// 序列化失败 / IO 失败时返 `(false, Some(warning))`，调用方在 JSON 顶层加 `warning`
/// 字段透出错误，让 agent 决定是否重试或上报。
fn write_sidecar(
    file: &crate::record::BookmarkFile,
    prec_path: &std::path::Path,
) -> (bool, Option<String>) {
    let text = match serde_json::to_string_pretty(file) {
        Ok(s) => s,
        Err(e) => return (false, Some(format!("sidecar 序列化失败: {e}"))),
    };
    let sidecar_path = crate::record::BookmarkFile::sidecar_path(prec_path);
    match std::fs::write(&sidecar_path, &text) {
        Ok(()) => (true, None),
        Err(e) => (
            false,
            Some(format!(
                "sidecar 写盘失败 ({}): {e}",
                sidecar_path.display()
            )),
        ),
    }
}

// ===========================================================================
// v0.17 stage 1 Spike 新增 — 6 个 Args struct + 6 个 stub helper
//
// 范围（与 brainstorm §v0.17 cycle 实际范围表对齐，决策 2 / 决策 3 stub 返回格式）：
// - record 暴露类别（2 tool）：proc_record_start / proc_record_stop
//   spawn `proc record --no-tui --output <path>` 子进程路径（ADR-0029 决策 4 拍板）
// - USB release 类别（1 tool）：proc_usb_release
//   kill_locks → flush_write_cache → eject_device 三步链路
// - docker-rm 类别（3 tool）：proc_docker_rm / proc_docker_image_rm / proc_docker_volume_rm
//   bollard API remove_container / remove_image / remove_volume
//
// 5 个 tool 都 `confirm: bool` 必传（ADR-0029 决策 5 拍板——confirm 与既有
// `dry_run: bool` 默认 false 契约互补：dry_run 是「不真正执行」/ confirm 是
// 「确认风险后再执行」）。stage 1 Spike 仅注册 schema，stage 6 Slice 替换
// stub helper 为真实业务实现。
// ===========================================================================

/// `proc_record_start` tool 入参（stage 6 实装 spawn 子进程）。
///
/// 启动录屏子进程（headless，不 attach TUI）。confirm 必传 true 让 agent
/// 显式确认录屏会捕获屏幕所有内容含 DNS 域名 / 进程 cmd（与 v0.6 落地的
/// `pending_record_confirm` TUI 路径同款契约）。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct RecordStartArgs {
    /// 必须传 true 以确认录屏风险（捕获屏幕所有内容含 DNS 域名 / 进程 cmd）。
    /// confirm=false → ok=false + error "confirm=true 必传以确认录屏风险"。
    pub confirm: bool,
    /// 输出 .prec 文件路径（如 "~/.config/proc/recordings/session_1.prec"）。
    pub file_path: String,
    /// 可选自动停止时长（秒）。None → 手动调 proc_record_stop 停止。
    #[serde(default)]
    pub duration_secs: Option<u64>,
}

/// `proc_record_stop` tool 入参（stage 6 实装 kill child + 等 flush）。
///
/// 停止录屏子进程并等待 .prec 文件 flush。file_path 必须与
/// `proc_record_start` 的 file_path 匹配（stage 6 实装 handler 持
/// `record_handle: Arc<Mutex<Option<Child>>>` 字段跨 tool call 保活）。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct RecordStopArgs {
    /// 录屏文件路径（必须匹配 proc_record_start 的 file_path）。
    pub file_path: String,
}

/// `proc_usb_release` tool 入参（stage 6 实装 kill + flush + eject 三步链路）。
///
/// 一次完成 kill_locks + flush_write_cache + eject_device。confirm 必传 true
/// 让 agent 显式确认三步破坏性操作。dry_run 默认 false（与 v0.7 proc_kill /
/// v0.15 proc_monitor_add 同款契约），dry_run=true 时仅预演不真正执行。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct UsbReleaseArgs {
    /// 必须传 true 以确认破坏性操作（kill 进程 + flush 缓存 + eject 设备）。
    pub confirm: bool,
    /// 驱动器号（如 "E" / "E:" / "E:\\"，与 proc_eject / proc_eject_status 同款 normalize）。
    pub drive: String,
    /// 要 kill 的进程 PID 列表（agent 通常先调 proc_eject_status 拿 locks[].pid）。
    pub kill_pids: Vec<u32>,
    /// Dry-run 预演（默认 false = 真正执行 kill + flush + eject；true = 仅预演）。
    #[serde(default)]
    pub dry_run: Option<bool>,
}

/// `proc_docker_rm` tool 入参（stage 6 实装 bollard remove_container）。
///
/// 删除 Docker 容器。confirm 必传 true 让 agent 显式确认删除操作（不可逆）。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct DockerRmArgs {
    /// 必须传 true 以确认删除容器（不可逆）。
    pub confirm: bool,
    /// 容器 ID 或短 ID / 名称。
    pub container_id: String,
    /// 强制删除（即便容器在运行）。默认 false。
    #[serde(default)]
    pub force: Option<bool>,
    /// 同时删除关联的匿名 volume。默认 false。
    #[serde(default)]
    pub volumes: Option<bool>,
}

/// `proc_docker_image_rm` tool 入参（stage 6 实装 bollard remove_image）。
///
/// 删除 Docker 镜像。confirm 必传 true 让 agent 显式确认删除操作（不可逆）。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct DockerImageRmArgs {
    /// 必须传 true 以确认删除镜像（不可逆）。
    pub confirm: bool,
    /// 镜像 ID 或 tag。
    pub image_id: String,
    /// 强制删除（即便有容器依赖）。默认 false。
    #[serde(default)]
    pub force: Option<bool>,
    /// 同时删除子镜像。默认 false。
    #[serde(default)]
    pub prune_children: Option<bool>,
}

/// `proc_docker_volume_rm` tool 入参（stage 6 实装 bollard remove_volume）。
///
/// 删除 Docker volume。confirm 必传 true 让 agent 显式确认删除操作（不可逆，
/// volume 数据将永久丢失）。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct DockerVolumeRmArgs {
    /// 必须传 true 以确认删除 volume（数据永久丢失，不可逆）。
    pub confirm: bool,
    /// volume 名称。
    pub volume_name: String,
    /// 强制删除（即便有容器挂载）。默认 false。
    #[serde(default)]
    pub force: Option<bool>,
}

// ===========================================================================
// v0.17 stage 1 stub helpers — stage 6 Slice 替换为真实业务实现
//
// stub 返 `{ ok: true, stub: true, stage: "v0.17-stage-6", message, received_* }`
// placeholder JSON（与 v0.16 stage 1 决策 3 同款规则）：
// - ok: true 让 client（mcp-inspector）调用时不报错，能验证 schema 正确生成
// - stub: true 让 LLM / client 能识别「这是占位返回」避免误用
// - stage 字段让 stage 6 完工时容易 grep 验证替换
// - received_* 字段保留参数 echo，方便 stage 6 调试 schema 反序列化
// ===========================================================================

/// `proc_record_start` — stub helper（stage 6 替换为真实业务实现）。
///
/// stage 6 实装时走 spawn `proc record --no-tui --output <path>` 子进程路径
/// （ADR-0029 决策 4 拍板），handler 持 `record_handle: Arc<Mutex<Option<Child>>>`
/// 字段跨 tool call 保活。
pub fn make_record_start_json(
    _confirm: bool,
    _file_path: &str,
    _duration_secs: Option<u64>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.17-stage-6",
        "message": "proc_record_start tool schema is registered; business logic lands in stage 6",
        "received_confirm": _confirm,
        "received_file_path": _file_path,
        "received_duration_secs": _duration_secs,
    })
}

/// `proc_record_stop` — stub helper（stage 6 替换为真实业务实现）。
///
/// stage 6 实装时走 kill child + 等待 `.prec` 文件 flush + 读 footer metadata
/// 返 `{ ok, file_path, size_bytes, duration_secs, frame_count }`。
pub fn make_record_stop_json(_file_path: &str) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.17-stage-6",
        "message": "proc_record_stop tool schema is registered; business logic lands in stage 6",
        "received_file_path": _file_path,
    })
}

/// `proc_usb_release` — stub helper（stage 6 替换为真实业务实现）。
///
/// stage 6 实装时走 `kill_locks → flush_write_cache (PowerShell 阻塞 3s+) →
/// eject_device` 三步链路，返
/// `{ ok, dry_run, action: "release", drive, killed_pids: [...], flushed: bool, ejected: bool }`。
pub fn make_usb_release_json(
    _confirm: bool,
    _drive: &str,
    _kill_pids: &[u32],
    _dry_run: Option<bool>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.17-stage-6",
        "message": "proc_usb_release tool schema is registered; business logic lands in stage 6",
        "received_confirm": _confirm,
        "received_drive": _drive,
        "received_kill_pids": _kill_pids,
        "received_dry_run": _dry_run,
    })
}

/// `proc_docker_rm` — stub helper（stage 6 替换为真实业务实现）。
///
/// stage 6 实装时走 bollard API `remove_container`，返 `{ ok, container_id, removed: bool }`。
pub fn make_docker_rm_json(
    _confirm: bool,
    _container_id: &str,
    _force: Option<bool>,
    _volumes: Option<bool>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.17-stage-6",
        "message": "proc_docker_rm tool schema is registered; business logic lands in stage 6",
        "received_confirm": _confirm,
        "received_container_id": _container_id,
        "received_force": _force,
        "received_volumes": _volumes,
    })
}

/// `proc_docker_image_rm` — stub helper（stage 6 替换为真实业务实现）。
///
/// stage 6 实装时走 bollard API `remove_image`，返 `{ ok, image_id, removed: bool }`。
pub fn make_docker_image_rm_json(
    _confirm: bool,
    _image_id: &str,
    _force: Option<bool>,
    _prune_children: Option<bool>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.17-stage-6",
        "message": "proc_docker_image_rm tool schema is registered; business logic lands in stage 6",
        "received_confirm": _confirm,
        "received_image_id": _image_id,
        "received_force": _force,
        "received_prune_children": _prune_children,
    })
}

/// `proc_docker_volume_rm` — stub helper（stage 6 替换为真实业务实现）。
///
/// stage 6 实装时走 bollard API `remove_volume`，返 `{ ok, volume_name, removed: bool }`。
pub fn make_docker_volume_rm_json(
    _confirm: bool,
    _volume_name: &str,
    _force: Option<bool>,
) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.17-stage-6",
        "message": "proc_docker_volume_rm tool schema is registered; business logic lands in stage 6",
        "received_confirm": _confirm,
        "received_volume_name": _volume_name,
        "received_force": _force,
    })
}
