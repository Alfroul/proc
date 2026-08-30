//! v0.25 stage 3 集成测试 — TD-53（sysinfo delta per-process disk_io）+ TD-50
//! （`proc_smart` x-deprecated schema hint）。
//!
//! TD-53 覆盖两层：
//! - 单元层：`compute_process_disk_speeds` 合成 `HashMap<u32, ProcessInfo>`
//!   （不依赖真实 sysinfo，数值确定性断言——elapsed 用同一 `Instant` 基准构造，
//!   差分除法精确）
//! - 响应层：`make_metrics_disk_io_json` fallback 路径结构断言（fresh snapshot
//!   无 delta 基线速度值不可断言，但段结构 / source 口径 / top 截断 / 降序可确定）
//!
//! TD-50 覆盖：`#[tool]` 宏生成的 pub 静态函数 `{fn}_tool_attr()` 直接拿
//! `rmcp::model::Tool` 断言 `_meta.x-deprecated`——比 v0.17 的源码 grep 静态
//! 断言更强的运行时 schema 断言（`tool_router()` 私有但 `*_tool_attr` 是 pub）。

use std::collections::HashMap;

use proc::collect::ProcessInfo;
#[cfg(feature = "mcp-persistent-state")]
use proc::mcp::handler::compute_process_disk_speeds;
use proc::mcp::handler::{ProcMcpHandler, list_tool_names};
use serde_json::json;

fn mk_proc(pid: u32, start_time: u64, disk_usage: (u64, u64)) -> ProcessInfo {
    let mut p = ProcessInfo::default();
    p.pid = pid;
    p.start_time = start_time;
    p.disk_usage = disk_usage;
    p
}

/// elapsed = 2s 的确定性时间对（`t0 = now - 2s` 与 `now` 同基准，差精确 2s）。
#[cfg(feature = "mcp-persistent-state")]
fn two_sec_pair() -> (std::time::Instant, std::time::Instant) {
    let now = std::time::Instant::now();
    let t0 = now
        .checked_sub(std::time::Duration::from_secs(2))
        .expect("2s-ago Instant representable");
    (t0, now)
}

// ===========================================================================
// TD-53 单元层：compute_process_disk_speeds（合成数据）
// ===========================================================================

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn td53_delta_computes_speeds_from_prev_counters() {
    // proc(pid=10, start=100) disk_usage (1000, 2000)，prev 基线 (800, 1500)，
    // elapsed 精确 2s → read = 200/2 = 100 / write = 500/2 = 250（精确断言）
    let mut procs = HashMap::new();
    procs.insert(10, mk_proc(10, 100, (1000, 2000)));
    let mut prev = HashMap::new();
    prev.insert((10, 100), (800, 1500));
    let (t0, now) = two_sec_pair();
    compute_process_disk_speeds(&mut procs, &mut prev, Some(t0), now);
    assert_eq!(procs[&10].disk_read_speed, 100, "read delta 200 / 2s");
    assert_eq!(procs[&10].disk_write_speed, 250, "write delta 500 / 2s");
}

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn td53_first_observation_builds_baseline_without_speeds() {
    // prev_time = None（首观测）：speeds 保持 0，只建基线；下一 tick 起有差分
    let mut procs = HashMap::new();
    procs.insert(10, mk_proc(10, 100, (1000, 2000)));
    let mut prev = HashMap::new();
    compute_process_disk_speeds(&mut procs, &mut prev, None, std::time::Instant::now());
    assert_eq!(procs[&10].disk_read_speed, 0, "first observation: no speed");
    assert_eq!(
        procs[&10].disk_write_speed, 0,
        "first observation: no speed"
    );
    assert_eq!(prev.get(&(10, 100)), Some(&(1000, 2000)), "baseline built");

    // 第二次调用（1s 精确）：delta 200 → 200 bps
    procs.insert(10, mk_proc(10, 100, (1200, 2000)));
    let now = std::time::Instant::now();
    let t0 = now
        .checked_sub(std::time::Duration::from_secs(1))
        .expect("1s-ago Instant representable");
    compute_process_disk_speeds(&mut procs, &mut prev, Some(t0), now);
    assert_eq!(procs[&10].disk_read_speed, 200);
}

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn td53_counter_regression_saturates_to_zero() {
    // disk_usage 小于 prev（counter 回退 / 进程重启）：saturating_sub 归 0 不 panic
    let mut procs = HashMap::new();
    procs.insert(30, mk_proc(30, 1, (100, 50)));
    let mut prev = HashMap::new();
    prev.insert((30, 1), (5000, 5000));
    let (t0, now) = two_sec_pair();
    compute_process_disk_speeds(&mut procs, &mut prev, Some(t0), now);
    assert_eq!(procs[&30].disk_read_speed, 0);
    assert_eq!(procs[&30].disk_write_speed, 0);
}

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn td53_baseline_rebuild_drops_dead_and_ignores_reused_pid() {
    // prev 含死亡进程 (99,1) 与同 pid 不同 start_time 的旧条目 (10,99)：
    // PID 复用防护——(10,100) ≠ (10,99) 不命中旧条目（speeds 保持 0）；
    // 调用后基线 = 当前存活进程键集（死亡淘汰 / 新进程入库 / 旧条目替换）
    let mut procs = HashMap::new();
    procs.insert(10, mk_proc(10, 100, (1000, 2000)));
    procs.insert(20, mk_proc(20, 5, (300, 400)));
    let mut prev = HashMap::new();
    prev.insert((10, 99), (800, 1500));
    prev.insert((99, 1), (0, 0));
    let (t0, now) = two_sec_pair();
    compute_process_disk_speeds(&mut procs, &mut prev, Some(t0), now);
    assert_eq!(
        procs[&10].disk_read_speed, 0,
        "reused pid must not hit stale entry"
    );
    assert_eq!(procs[&20].disk_read_speed, 0, "new process: no prev entry");
    assert_eq!(prev.len(), 2, "baseline == current process set");
    assert!(prev.contains_key(&(10, 100)));
    assert!(prev.contains_key(&(20, 5)));
    assert!(!prev.contains_key(&(99, 1)), "dead process dropped");
    assert!(
        !prev.contains_key(&(10, 99)),
        "stale pid-reuse entry replaced"
    );
}

// ===========================================================================
// TD-53 响应层：make_metrics_disk_io_json per_process 段（fallback 真实快照）
// ===========================================================================

#[test]
fn td53_disk_io_response_has_per_process_segment() {
    let out = proc::mcp::handler::metrics::make_metrics_disk_io_json(None, None);
    assert_eq!(out["ok"], json!(true));
    let seg = &out["per_process"];
    assert_eq!(
        seg["source"],
        json!("sysinfo-delta"),
        "口径声明字段（ADR-0035 D2 精度声明）"
    );
    let arr = seg["processes"].as_array().expect("processes array");
    assert!(arr.len() <= 10, "default top 10, got {}", arr.len());
    assert_eq!(seg["count"].as_u64().unwrap_or(u64::MAX), arr.len() as u64);
    for p in arr {
        assert!(p["pid"].as_u64().is_some(), "pid field: {p}");
        assert!(p["name"].is_string(), "name field: {p}");
        assert!(p["read_bps"].as_u64().is_some(), "read_bps field: {p}");
        assert!(p["write_bps"].as_u64().is_some(), "write_bps field: {p}");
    }
    // 降序锚：read+write 非增
    let totals: Vec<u64> = arr
        .iter()
        .map(|p| {
            p["read_bps"]
                .as_u64()
                .unwrap_or(0)
                .saturating_add(p["write_bps"].as_u64().unwrap_or(0))
        })
        .collect();
    assert!(
        totals.windows(2).all(|w| w[0] >= w[1]),
        "read+write desc order: {totals:?}"
    );
    // 既有字段不缺（tool 语义不变锚——只加不改）
    assert!(out["total"].is_object());
    assert!(out["per_disk"].is_array());
    assert!(out["disks"].is_array());
}

#[test]
fn td53_disk_io_top_param_truncates_per_process() {
    let out = proc::mcp::handler::metrics::make_metrics_disk_io_json(None, Some(3));
    assert_eq!(out["ok"], json!(true));
    let arr = out["per_process"]["processes"]
        .as_array()
        .expect("processes array");
    assert!(arr.len() <= 3, "top=3 override, got {}", arr.len());
}

// ===========================================================================
// TD-50：proc_smart _meta.x-deprecated schema hint（运行时断言）
// ===========================================================================

#[test]
fn td50_proc_smart_tool_meta_has_x_deprecated() {
    let tool = ProcMcpHandler::proc_smart_tool_attr();
    let meta = tool.meta.as_ref().expect("proc_smart meta should be set");
    assert_eq!(
        meta.0.get("x-deprecated"),
        Some(&json!(true)),
        "_meta extension key: {{\"x-deprecated\": true}}"
    );
    // 阴性对照：废弃 hint 只打 proc_smart（其余 45 tool 无 meta）
    let other = ProcMcpHandler::proc_metrics_smart_tool_attr();
    assert!(
        other.meta.is_none(),
        "proc_metrics_smart should not carry deprecated meta"
    );
}

// ===========================================================================
// D4 不变锚：MCP tool 46（运行时断言）
// ===========================================================================

#[test]
fn tool_count_anchor_46() {
    let names = list_tool_names();
    assert_eq!(
        names.len(),
        46,
        "MCP tool count must stay 46 (TD-50 hint 不删 tool / TD-53 响应扩展不加 tool)"
    );
}
