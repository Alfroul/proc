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

/// `proc_replay_info` stub — stage 2 Slice 填双路径（v3 UiFrame footer +
/// VT100 header）业务实现。
pub fn make_replay_info_json(file_path: &str) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.16-stage-2",
        "message": "proc_replay_info tool schema is registered; business logic lands in stage 2",
        "received_file_path": file_path,
    })
}

/// `proc_replay_search` stub — stage 2 Slice 填 FilterExpr / substring 双入口 +
/// 帧遍历 + limit 截断业务实现（详见 ADR-0025a）。
pub fn make_replay_search_json(file_path: &str, query: &str, limit: Option<usize>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.16-stage-2",
        "message": "proc_replay_search tool schema is registered; business logic lands in stage 2",
        "received_file_path": file_path,
        "received_query": query,
        "received_limit": limit,
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

/// `proc_eject_status` stub — stage 2 Slice 填 crate::eject::scan_device_locks +
/// 4 档 suggestion 决策业务实现（用户 2026-07-07 追加，详见 brainstorm §决策 9）。
pub fn make_eject_status_json(drive: &str) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.16-stage-2",
        "message": "proc_eject_status tool schema is registered; business logic lands in stage 2",
        "received_drive": drive,
    })
}
