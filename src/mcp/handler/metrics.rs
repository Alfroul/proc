//! MCP `proc_metrics_*` tool — 类别 4（系统级 metrics）Args + helper。
//!
//! v0.15 cycle stage 1 Spike 仅放骨架（Args struct + stub helper），stage 3 Slice
//! 填业务逻辑。详见 [`super`] 模块文档与 `docs/stages/v0.15-stage-1.md`。
//!
//! 边界：5 个独立 tool（不合并），让 agent 按需调用避免一次返大量不需要的数据
//! （brainstorm FAQ Q4 决策）。`#[tool]` 方法本身在 [`super::mod_rs`] 的
//! `#[tool_router] impl` 块里（rmcp 0.11 限制）。

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

// ===========================================================================
// Args structs — 类别 4（5 tool）
// ===========================================================================

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsSystemArgs {
    // 当前无字段；保留 struct 让 stage 3 加 (e.g. include_history: Option<bool>)。
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsGpuArgs {
    // 当前无字段；保留 struct 让 stage 3 加 (e.g. device_index: Option<u32>)。
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsDiskIoArgs {
    /// Filter to a specific device (e.g. "PhysicalDrive0", "/dev/sda"). None = all devices.
    #[serde(default)]
    pub device: Option<String>,
}

/// `proc_metrics_smart` Args（与既有 `SmartArgs` 字段同形但**独立 struct**）。
///
/// stage 1 占位：`proc_metrics_smart` 与 v0.7 既有 `proc_smart` 职责重叠。stage 4
/// Review 决策（参考 v0.15-stage-1.md §决策 4c）：(a) 废弃 `proc_smart` /
/// (b) `proc_metrics_smart` 返系统级聚合 vs `proc_smart` 单设备 / (c) 移除重复入口。
/// stage 1 不决策，stub 返占位。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsSmartArgs {
    /// Device path. None = aggregated summary across all SMART-readable disks.
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsThermalArgs {
    // 当前无字段；保留 struct 让 stage 3 加 (e.g. core_index: Option<u32>)。
}

// ===========================================================================
// Stub helpers
// ===========================================================================

/// `proc_metrics_system` stub — stage 3 实装 CPU/内存/swap 使用率 + 火花线图历史。
pub fn make_metrics_system_json() -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-3",
        "message": "proc_metrics_system tool schema is registered; business logic lands in stage 3",
    })
}

/// `proc_metrics_gpu` stub — stage 3 实装 NVML / DXGI / PDH utilization 聚合。
pub fn make_metrics_gpu_json() -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-3",
        "message": "proc_metrics_gpu tool schema is registered; business logic lands in stage 3",
    })
}

/// `proc_metrics_disk_io` stub — stage 3 实装 per-disk + per-process IO 速率。
pub fn make_metrics_disk_io_json(device: Option<&str>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-3",
        "message": "proc_metrics_disk_io tool schema is registered; business logic lands in stage 3",
        "received_device": device,
    })
}

/// `proc_metrics_smart` stub — stage 3 实装（与 proc_smart 关系见 §决策 4c）。
pub fn make_metrics_smart_json(device: Option<&str>) -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-3",
        "message": "proc_metrics_smart tool schema is registered; business logic lands in stage 3 (relationship with proc_smart TBD in stage 4 review)",
        "received_device": device,
    })
}

/// `proc_metrics_thermal` stub — stage 3 实装 per-core 频率 + 温度 + 降频标记。
pub fn make_metrics_thermal_json() -> Value {
    json!({
        "ok": true,
        "stub": true,
        "stage": "v0.15-stage-3",
        "message": "proc_metrics_thermal tool schema is registered; business logic lands in stage 3",
    })
}
