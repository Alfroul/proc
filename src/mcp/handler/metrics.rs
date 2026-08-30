//! MCP `proc_metrics_*` tool — 类别 4（系统级 metrics）Args + helper。
//!
//! v0.15 cycle stage 1 Spike 落地骨架（Args struct + stub helper），stage 3 Slice
//! 填业务逻辑（本文件）。详见 [`super`] 模块文档与 `docs/stages/v0.15-stage-3.md`。
//!
//! 边界：5 个独立 tool（不合并），让 agent 按需调用避免一次返大量不需要的数据
//! （brainstorm FAQ Q4 决策）。`#[tool]` 方法本身在 [`super::mod_rs`] 的
//! `#[tool_router] impl` 块里（rmcp 0.11 限制）。
//!
//! stage 3 决策（详见 stage-3.md）：
//! - 决策 1：metrics 走 SystemSnapshot 直采（不 spawn App，与 make_export_json 同款）
//! - 决策 2：metrics_smart vs proc_smart 选 (b) 聚合 vs 单设备
//! - 决策 3-6：5 tool 字段集
//! - 决策 7：mod.rs impl 块结构稳定（仅 description 字符串更新）

use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

// ===========================================================================
// Args structs — 类别 4（5 tool）
// ===========================================================================

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsSystemArgs {
    // 当前无字段；保留 struct 让未来加 (e.g. include_history: Option<bool>)。
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsGpuArgs {
    // 当前无字段；保留 struct 让未来加 (e.g. device_index: Option<u32>)。
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsDiskIoArgs {
    /// Filter `per_disk` to a specific device (e.g. "PhysicalDrive0", "/dev/sda"). None = all devices.
    /// `total` / `disks` 字段始终返全部，per_disk 按设备过滤。
    /// `per_process` 段不受影响（per-process IO counters 是进程级聚合，无法按盘归属）。
    #[serde(default)]
    pub device: Option<String>,
    /// Max entries in `per_process.processes[]` (sorted by read_bps+write_bps desc).
    /// None = 10.
    #[serde(default)]
    pub top: Option<usize>,
}

/// `proc_metrics_smart` Args（与既有 `SmartArgs` 字段同形但**独立 struct**）。
///
/// stage 3 落地（决策 2）：`device=None` 返系统级聚合摘要（all disks 基本信息）/
/// `device=Some` 返单设备详细 attributes（与 `proc_smart(device=Some)` 同款）。
/// 与 v0.7 `proc_smart` 关系：互补不冲突（聚合 vs 单设备）；stage 4 Review 评估
/// 合并入口 / 废弃 `proc_smart`（暂留 v0.16+ 候选）。
#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsSmartArgs {
    /// Device path. None = aggregated summary across all SMART-readable disks.
    /// Some("PhysicalDrive0") = single-disk detail with full attributes.
    #[serde(default)]
    pub device: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct MetricsThermalArgs {
    // 当前无字段；保留 struct 让未来加 (e.g. core_index: Option<u32>)。
}

// ===========================================================================
// Helpers — stage 3 业务逻辑实装（替换 stage 1 stub）。
//
// 失败路径返 `{ ok: false, error: <msg> }`（与 mod.rs 既有 helper 同款）。
// 不 spawn App（决策 1）：SystemSnapshot 直采 ~500ms 开销，与 make_export_json 同款路径。
// ===========================================================================

/// `proc_metrics_system` — 系统全貌快照（CPU / 内存 / swap / 磁盘 / uptime / 进程数 /
/// 网卡 / TCP / 温度）。
///
/// sparkline 30s 历史由 v0.17 stage 4 TD-52 落地（`ProcMcpHandler::system_history`
/// + `proc_metrics_history` tool——本 helper 不含历史，单次快照语义）。
///
/// v0.17 stage 3 TD-54：保留旧签名作 fallback 路径（测试 + worker warm-up 期间用），
/// 生产路径走 [`metrics_system_json_from_snapshot`] 复用 `ProcMcpHandler::snapshot`
/// 字段（worker 1s tick refresh，跳过 SystemSnapshot::new + refresh ~50ms 开销）。
pub fn make_metrics_system_json() -> Value {
    let mut snapshot = match crate::collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return super::err(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return super::err(format!("snapshot refresh failed: {e}"));
    }
    let _ = snapshot.refresh_heavy_incremental();
    metrics_system_json_from_snapshot(&snapshot)
}

/// v0.17 stage 3 TD-54：从已有 SystemSnapshot 读字段（生产路径，避免现场 new）。
///
/// 公开度 `pub(crate)` 让 `mod.rs::proc_metrics_system` `#[tool]` 方法可调；不暴露
/// 给集成测试以外（既有 `make_metrics_system_json()` 是公开 fallback 入口）。
pub(crate) fn metrics_system_json_from_snapshot(
    snapshot: &crate::collect::SystemSnapshot,
) -> Value {
    let cpu_usage = snapshot.cpu_usage();
    let (mem_used, mem_total) = snapshot.memory_usage();
    let (swap_used, swap_total) = snapshot.swap_usage();
    let (disk_used, disk_total) = snapshot.disk_usage();
    let uptime = crate::collect::SystemSnapshot::uptime_secs();
    let process_count = snapshot.process_count();
    let (cpu_temp, gpu_temp) = snapshot.temperatures();
    let tcp = crate::collect::SystemSnapshot::tcp_stats();
    let net_adapters = snapshot.net_adapters();

    let network_interfaces: Vec<Value> = net_adapters
        .iter()
        .map(|a| {
            json!({
                "name": a.name,
                "ipv4": a.ipv4,
            })
        })
        .collect();

    json!({
        "ok": true,
        "cpu_usage_pct": cpu_usage,
        "memory": usage_obj(mem_used, mem_total),
        "swap": usage_obj(swap_used, swap_total),
        "system_disk": usage_obj(disk_used, disk_total),
        "uptime_secs": uptime,
        "processes_count": process_count,
        "network_interfaces": network_interfaces,
        "tcp_stats": {
            "established": tcp.established,
            "time_wait": tcp.time_wait,
            "close_wait": tcp.close_wait,
            "listen": tcp.listen,
            "retransmitted_segs": tcp.retransmitted_segs,
            "reset_segs": tcp.reset_segs,
            "failed_connections": tcp.failed_connections,
            "out_segs": tcp.out_segs,
        },
        "cpu_temp_c": cpu_temp,
        "gpu_temp_c": gpu_temp,
    })
}

/// `proc_metrics_gpu` — GPU 监控聚合（NVML + DXGI + PDH 三层数据源，见 gpu.rs）。
///
/// 无 NVIDIA GPU / 无 DXGI 支持 / 非 Windows → `gpus: []` + `providers: []` +
/// `note` 字段说明（与 sidebar 同款降级路径）。
pub fn make_metrics_gpu_json() -> Value {
    let mut collector = crate::gpu::GpuCollector::new();
    let gpus = collector.refresh();
    let providers: Vec<&'static str> = collector.provider_names();

    let arr: Vec<Value> = gpus
        .iter()
        .map(|g| {
            let vendor_str = match g.vendor {
                crate::gpu::GpuVendor::Nvidia => "Nvidia",
                crate::gpu::GpuVendor::Amd => "Amd",
                crate::gpu::GpuVendor::Intel => "Intel",
                crate::gpu::GpuVendor::Unknown => "Unknown",
            };
            json!({
                "name": g.name,
                "vendor": vendor_str,
                "utilization_pct": g.utilization_pct,
                "vram": {
                    "used_bytes": g.vram_used,
                    "total_bytes": g.vram_total,
                    "budget_bytes": g.vram_budget,
                },
                "temperature_c": g.temperature,
                "power_watts": g.power_watts,
            })
        })
        .collect();

    let mut out = json!({
        "ok": true,
        "providers": providers,
        "count": arr.len(),
        "gpus": arr,
    });
    if arr.is_empty() {
        out["note"] = json!(
            "no GPU providers available (NVIDIA via NVML + DXGI for all vendors; non-Windows returns empty)"
        );
    }
    out
}

/// `proc_metrics_disk_io` — 全磁盘 IO 速率（total + per_disk）+ 磁盘容量信息 +
/// per-process top-N 段（v0.25 stage 3 TD-53，ADR-0035 D2 改道）。
///
/// v0.17 stage 3 TD-54：保留旧签名作 fallback 路径，生产路径走
/// [`metrics_disk_io_json_from_snapshot`]。
pub fn make_metrics_disk_io_json(device: Option<&str>, top: Option<usize>) -> Value {
    let mut snapshot = match crate::collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return super::err(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return super::err(format!("snapshot refresh failed: {e}"));
    }
    metrics_disk_io_json_from_snapshot(&snapshot, device, top)
}

/// v0.17 stage 3 TD-54：从已有 SystemSnapshot 读字段（生产路径）。
///
/// v0.25 stage 3 TD-53：加 `per_process` 段（read+write 降序 top-N，默认 10）。
/// 速度值由 MCP snapshot worker 的 sysinfo delta 填充（fallback 路径 fresh
/// snapshot 无基线，速度全 0）；`source: "sysinfo-delta"` 声明口径（Windows 下
/// IO counters 含非磁盘 IO 如命名管道，与 TUI 非管理员档同口径）。
pub(crate) fn metrics_disk_io_json_from_snapshot(
    snapshot: &crate::collect::SystemSnapshot,
    device: Option<&str>,
    top: Option<usize>,
) -> Value {
    let (total_read, total_write) = snapshot.disk_io_speed();
    let per_disk: Vec<crate::collect::DiskIoInfo> = snapshot.per_disk_io_speed();
    let disks: Vec<crate::collect::DiskInfo> = snapshot.all_disks();

    let per_disk_filtered: Vec<Value> = per_disk
        .iter()
        .filter(|d| {
            device.is_none_or(|dev| {
                matches_device(&d.name, dev) || matches_device(&d.mount_point, dev)
            })
        })
        .map(|d| {
            json!({
                "name": d.name,
                "mount_point": d.mount_point,
                "read_bps": d.read_speed,
                "write_bps": d.write_speed,
            })
        })
        .collect();

    let disks_arr: Vec<Value> = disks
        .iter()
        .map(|d| {
            json!({
                "name": d.name,
                "mount_point": d.mount_point,
                "used_bytes": d.used,
                "total_bytes": d.total,
                "is_removable": d.is_removable,
            })
        })
        .collect();

    // v0.25 stage 3 TD-53：per-process top-N 段（read+write 降序）。
    // 速度值来自 snapshot worker 的 sysinfo delta（fallback 路径无基线时全 0）。
    let top_n = top.unwrap_or(10);
    let mut ranked: Vec<&crate::collect::ProcessInfo> = snapshot.process_cache().values().collect();
    ranked.sort_by(|a, b| {
        let total_a = a.disk_read_speed.saturating_add(a.disk_write_speed);
        let total_b = b.disk_read_speed.saturating_add(b.disk_write_speed);
        total_b.cmp(&total_a)
    });
    ranked.truncate(top_n);
    let per_process: Vec<Value> = ranked
        .iter()
        .map(|p| {
            json!({
                "pid": p.pid,
                "name": p.name.as_ref(),
                "read_bps": p.disk_read_speed,
                "write_bps": p.disk_write_speed,
            })
        })
        .collect();

    json!({
        "ok": true,
        "device_filter": device,
        "total": {
            "read_bps": total_read,
            "write_bps": total_write,
        },
        "per_disk": per_disk_filtered,
        "disks": disks_arr,
        "per_process": {
            "source": "sysinfo-delta",
            "count": per_process.len(),
            "processes": per_process,
        },
    })
}

/// `proc_metrics_smart` — SMART 磁盘健康（决策 2：聚合 vs 单设备 双路径）。
///
/// - `device=None` → 系统级聚合摘要（list_disks + read_smart 基本信息，与 sidebar 同款视角）
/// - `device=Some` → 单设备详细 attributes（与 `proc_smart(device=Some)` 同款 schema）
///
/// 与 v0.7 既有 `proc_smart` 关系：互补不冲突；stage 4 Review 评估合并入口（暂留 v0.16+）。
pub fn make_metrics_smart_json(device: Option<&str>) -> Value {
    match device {
        None => make_metrics_smart_aggregated(),
        Some(dev) => make_metrics_smart_single(dev),
    }
}

/// `proc_metrics_thermal` — per-core 频率 / 温度 / 节流状态。
///
/// 非 Windows / 无 PROCESSOR_POWER_INFORMATION 访问权限 → `throttle: null` +
/// `reason: "Unavailable"`；per_core_freq/temp 仍能从 sysinfo 拿（Linux cpufreq /
/// Windows 注册表）。
///
/// v0.17 stage 3 TD-54：保留旧签名作 fallback 路径，生产路径走
/// [`metrics_thermal_json_from_snapshot`]。
pub fn make_metrics_thermal_json() -> Value {
    let mut snapshot = match crate::collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => return super::err(format!("SystemSnapshot::new failed: {e}")),
    };
    if let Err(e) = snapshot.refresh() {
        return super::err(format!("snapshot refresh failed: {e}"));
    }
    let _ = snapshot.refresh_heavy_incremental();
    metrics_thermal_json_from_snapshot(&snapshot)
}

/// v0.17 stage 3 TD-54：从已有 SystemSnapshot 读字段（生产路径）。
pub(crate) fn metrics_thermal_json_from_snapshot(
    snapshot: &crate::collect::SystemSnapshot,
) -> Value {
    let per_core_freq: Vec<u64> = snapshot.per_core_freq().to_vec();
    let per_core_temp: Vec<Option<f32>> = snapshot.per_core_temp().to_vec();
    let cpu_usage = snapshot.cpu_usage();
    let (cpu_temp, gpu_temp) = snapshot.temperatures();
    let throttle_info = snapshot.throttle_info().cloned();

    let (throttle_json, reason_str) = match &throttle_info {
        Some(ti) => {
            let reason = crate::throttle::classify_throttle(ti, cpu_usage, cpu_temp);
            let reason_str = match reason {
                crate::throttle::ThrottleReason::None => "None",
                crate::throttle::ThrottleReason::Thermal => "Thermal",
                crate::throttle::ThrottleReason::PowerPolicy => "PowerPolicy",
                crate::throttle::ThrottleReason::Idle => "Idle",
                crate::throttle::ThrottleReason::Unknown => "Unknown",
            };
            (
                json!({
                    "max_mhz": ti.max_mhz,
                    "current_mhz": ti.current_mhz,
                    "mhz_limit": ti.mhz_limit,
                    "is_throttled": ti.is_throttled,
                    "throttle_pct": ti.throttle_pct,
                }),
                reason_str,
            )
        }
        None => (Value::Null, "Unavailable"),
    };

    json!({
        "ok": true,
        "per_core_freq_mhz": per_core_freq,
        "per_core_temp_c": per_core_temp,
        "throttle": throttle_json,
        "reason": reason_str,
        "cpu_temp_c": cpu_temp,
        "gpu_temp_c": gpu_temp,
    })
}

// ===========================================================================
// 内部 helpers
// ===========================================================================

/// 构造 `{ used_bytes, total_bytes, pct }` 对象（避免重复代码）。
fn usage_obj(used: u64, total: u64) -> Value {
    let pct = if total > 0 {
        let ratio = used as f64 / total as f64 * 100.0;
        (ratio * 100.0).round() / 100.0
    } else {
        0.0
    };
    json!({
        "used_bytes": used,
        "total_bytes": total,
        "pct": pct,
    })
}

/// 设备名 / 挂载点匹配（子串包含，大小写不敏感）—— 与 CLI `proc flows --device`
/// 同款语义。PhysicalDrive0 等命名包含 mount_point 也能命中。
fn matches_device(haystack: &str, needle: &str) -> bool {
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    h.contains(&n)
}

/// device=None 路径：list_disks + read_smart 摘要（与 sidebar 同款视角）。
fn make_metrics_smart_aggregated() -> Value {
    let disks = crate::smart::list_disks();
    if disks.is_empty() {
        return json!({
            "ok": true,
            "mode": "aggregated",
            "count": 0,
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
    json!({
        "ok": true,
        "mode": "aggregated",
        "count": arr.len(),
        "disks": arr,
    })
}

/// device=Some 路径：详细 attributes（与 proc_smart 同款 schema）。
fn make_metrics_smart_single(device: &str) -> Value {
    match crate::smart::read_smart(device) {
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
                "mode": "single_device",
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
        Err(e) => super::err(format!("read_smart({device}) failed: {e}")),
    }
}
