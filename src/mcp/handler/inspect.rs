//! MCP `proc_inspect` tool — 类别 2（详情页 Tab 合并）Args + helper。
//!
//! v0.15 cycle stage 1 Spike 仅放骨架（Args struct + InspectTab enum + stub
//! helper），stage 2 Slice 填业务逻辑。详见 [`super`] 模块文档与
//! `docs/stages/v0.15-stage-1.md` 和 ADR-0023。
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
    /// 用于 stub helper 输出与 stage 2 日志 debug。
    #[must_use]
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

/// `proc_inspect` stub — stage 2 实装 6 tab 字段裁剪（与 ADR-0023 设计对齐）。
///
/// stage 1 占位返回（ok:true + stub:true + stage 字段）让 client（mcp-inspector）
/// 验证 schema 但不误用业务数据。stage 2 实装时各 tab 返不同字段集（详见
/// v0.15-stage-1.md 任务 4b）。
pub fn make_inspect_json(pid: u32, tab: &InspectTab, reveal: bool) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-2",
        "message": "proc_inspect tool schema is registered; business logic lands in stage 2",
        "received_pid": pid,
        "received_tab": tab.as_str(),
        "received_reveal": reveal,
    })
}
