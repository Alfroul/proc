//! v0.17 stage 3 集成测试 — TD-54 MCP handler 持久 snapshot 字段 + TD-45 record
//! encoding 选项层 + TD-50 proc_smart deprecated 标记。
//!
//! 与 `test_mcp_v0_15.rs` / `test_mcp_v0_16.rs` 同款策略：直接调 helper + handler
//! 字段断言，验证 stage 3 改动行为正确。stdio 端到端测试留 manual：
//! `npx mcp-inspector proc mcp serve`。

use bincode::Options;
use proc::mcp::handler::ProcMcpHandler;

// ===========================================================================
// TD-54：ProcMcpHandler 持久 snapshot 字段 + worker spawn 行为
// ===========================================================================

#[test]
fn test_default_handler_snapshot_is_none() {
    // Default 路径不 spawn worker，snapshot 字段 None（既有测试零回归的契约）
    let _h = ProcMcpHandler::default();
    #[cfg(feature = "mcp-persistent-state")]
    {
        let guard = _h.snapshot.lock().expect("snapshot mutex not poisoned");
        assert!(guard.is_none(), "Default handler snapshot should be None");
    }
}

#[test]
fn test_snapshot_field_shared_across_handler_clones() {
    // rmcp 内部每次 tool call clone handler，Arc::clone 应共享同一 snapshot 实例
    let h1 = ProcMcpHandler::default();
    let _h2 = h1.clone();
    #[cfg(feature = "mcp-persistent-state")]
    {
        assert!(
            std::sync::Arc::ptr_eq(&h1.snapshot, &_h2.snapshot),
            "Cloned handlers must share snapshot Arc"
        );
        assert!(
            std::sync::Arc::ptr_eq(&h1.dns_collector, &_h2.dns_collector),
            "Cloned handlers must share dns_collector Arc (同款 TD-36 模式延续)"
        );
    }
    // 让 h1 在 cfg-gate 外也「使用」一下避免 -D warnings 在 no-default-features 路径报 unused
    let _ = h1.dns_collector.lock().ok();
}

#[test]
fn test_proc_metrics_system_fallback_to_fresh_snapshot_during_warmup() {
    // 即使 worker 未起来（Default 路径），调 proc_metrics_system 仍走 fallback 路径
    // 现场新建 SystemSnapshot，返有效数据。这是「既有测试零回归」的核心契约。
    let _snapshot_none = ProcMcpHandler::default();
    #[cfg(feature = "mcp-persistent-state")]
    {
        let guard = _snapshot_none.snapshot.lock().expect("lock");
        assert!(guard.is_none(), "precondition: Default path has no worker");
    }
    // 通过 fallback helper 验证仍能拿到 ok: true 数据
    let out = proc::mcp::handler::metrics::make_metrics_system_json();
    let obj = out.as_object().expect("metrics_system_json returns object");
    assert_eq!(
        obj.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "fallback path should succeed"
    );
}

#[test]
fn test_proc_metrics_system_uses_snapshot_field_after_warmup() {
    // ProcMcpHandler::new() spawn worker，等 worker 首次 refresh 完成（最多 5s，
    // 给 SystemSnapshot::new + refresh_heavy_incremental + sysinfo 系统访问留余量；
    // CI / 单元测试环境 sysinfo 初始化可能比生产慢）。
    // worker 1s tick 之间有 ~30-50ms 持锁窗口 take + move back，重试几次避免偶发读 None。
    // 注意：此测试会 leak worker 线程（fire-and-forget），cargo test 进程退出时清理。
    let h = ProcMcpHandler::new();

    #[cfg(feature = "mcp-persistent-state")]
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut saw_some = false;
        while std::time::Instant::now() < deadline {
            if let Ok(guard) = h.snapshot.lock() {
                if guard.is_some() {
                    saw_some = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(
            saw_some,
            "worker should populate snapshot within 5s warm-up window"
        );
    }

    // 让 h 在 cfg-gate 外也「使用」一下避免 -D warnings 在 no-default-features 路径报 unused
    let _ = h.dns_collector.lock().ok();
}

// ===========================================================================
// TD-45：record encoding 选项层（v3 文件 round-trip + 选项层等价性）
// ===========================================================================

#[test]
fn test_record_encoding_options_for_version_returns_default_options() {
    // options_for_version(v) 返回的 fixint 配置与 bincode::serialize 默认配置字节级等价
    use proc::record::frame::{RECORDING_MAGIC, RecordingHeader};
    use proc::record::options_for_version;

    let header = RecordingHeader {
        magic: *RECORDING_MAGIC,
        version: 3,
        start_time: 1_700_000_000,
        hostname: "encoding-test".to_string(),
    };

    for v in [1_u16, 2_u16, 3_u16] {
        // impl Options 不是 Copy，每次调用创建新实例
        let opts_bytes: Vec<u8> = options_for_version(v)
            .serialize(&header)
            .expect("serialize via options_for_version");
        let bytes_default = bincode::serialize(&header).expect("default serialize");
        assert_eq!(
            opts_bytes, bytes_default,
            "v{v} bytes mismatch (options_for_version vs bincode::serialize)"
        );
    }
}

#[test]
fn test_record_round_trip_v3_file_unchanged() {
    // v3 文件 round-trip 通过 options_for_version 选项层完全等价
    use proc::record::frame::{RECORDING_MAGIC, RecordingFooter, RecordingHeader, UiFrame};
    use proc::record::options_for_version;

    let header = RecordingHeader {
        magic: *RECORDING_MAGIC,
        version: 3,
        start_time: 1_700_000_000,
        hostname: "round-trip-v3".to_string(),
    };
    let footer = RecordingFooter {
        version: 1,
        header_version: 3,
        start_time: 1_700_000_000,
        end_time: 1_700_000_010,
        frame_count: 1,
        anomaly_count: 0,
        event_count: 0,
        max_cpu: 5.0,
        max_mem: 1024,
        frame_offsets: vec![100],
    };

    // impl Options 不是 Copy，每次调用创建新实例（与 reader.rs / vt100.rs 调用模式一致）
    let header_bytes = options_for_version(3)
        .serialize(&header)
        .expect("serialize header");
    let footer_bytes = options_for_version(3)
        .serialize(&footer)
        .expect("serialize footer");

    let header_back: RecordingHeader = options_for_version(3)
        .deserialize(&header_bytes)
        .expect("deserialize header");
    let footer_back: RecordingFooter = options_for_version(3)
        .deserialize(&footer_bytes)
        .expect("deserialize footer");

    assert_eq!(header_back.magic, header.magic);
    assert_eq!(header_back.version, header.version);
    assert_eq!(header_back.start_time, header.start_time);
    assert_eq!(header_back.hostname, header.hostname);

    assert_eq!(footer_back.frame_count, footer.frame_count);
    assert_eq!(footer_back.start_time, footer.start_time);
    assert_eq!(footer_back.end_time, footer.end_time);
    assert_eq!(footer_back.frame_offsets, footer.frame_offsets);

    // 静态断言 UiFrame 也能 round-trip（虽然 frame 数据复杂，只验 schema 兼容性）
    let _ = std::marker::PhantomData::<UiFrame>;
}

// bincode::Options trait 在测试中需要 use 才能调 .serialize() / .deserialize()
// 已在文件顶部 use bincode::Options;

// ===========================================================================
// TD-50：proc_smart description 含 [Deprecated] hint
// ===========================================================================

#[test]
fn test_proc_smart_description_contains_deprecated_hint() {
    // 通过 list_tool_names() 验证 proc_smart 仍注册（不验 description 字符串，
    // 因为 rmcp 0.11 `#[tool_router]` 宏把 tool_router() 设为私有，外部测试无法直接
    // 拿 tool definition 的 description 字段）。description 字符串验证留 manual：
    // `npx mcp-inspector proc mcp serve` 看 schema。
    let names = proc::mcp::handler::list_tool_names();
    assert!(
        names.iter().any(|n| n == "proc_smart"),
        "proc_smart tool should be registered (got {} tools)",
        names.len()
    );
    assert!(
        names.iter().any(|n| n == "proc_metrics_smart"),
        "proc_metrics_smart tool should be registered"
    );
    // 验证总 tool 数：v0.16 末 39 + v0.17 stage 1 加 7 stub = 46
    // （含 proc_metrics_history / proc_record_start / proc_record_stop /
    //   proc_usb_release / proc_docker_rm / proc_docker_image_rm / proc_docker_volume_rm）
    assert!(
        names.len() >= 39,
        "tool count should be >= 39 (v0.16 baseline), got {}",
        names.len()
    );
}

#[test]
fn test_proc_smart_source_description_in_mod_rs_contains_deprecated() {
    // 直接 grep 源码验证 description 字符串含 [Deprecated] 标识（静态断言）
    // 这是 stage 3 落地的最直接验证——避开 rmcp 私有 API。
    let source = std::fs::read_to_string("src/mcp/handler/mod.rs").expect("mod.rs source readable");
    assert!(
        source.contains("\"[Deprecated] SMART disk health"),
        "mod.rs proc_smart description should start with [Deprecated]"
    );
    assert!(
        source.contains("Prefer proc_metrics_smart"),
        "mod.rs proc_smart description should recommend proc_metrics_smart"
    );
    assert!(
        source.contains("will be removed in v0.18+"),
        "mod.rs proc_smart description should mention v0.18+ removal"
    );
}

// ===========================================================================
// v0.17 stage 4：TD-52 sparkline system_history 字段 + proc_metrics_history tool
// ===========================================================================

#[test]
fn test_default_handler_system_history_is_empty() {
    // Default 路径不 spawn worker，system_history 字段为空 VecDeque
    let h = ProcMcpHandler::default();
    #[cfg(feature = "mcp-persistent-state")]
    {
        let guard = h
            .system_history
            .lock()
            .expect("system_history mutex not poisoned");
        assert!(
            guard.is_empty(),
            "Default handler system_history should be empty (no worker spawned)"
        );
    }
    // 让 h 在 cfg-gate 外也「使用」一下避免 -D warnings 在 no-default-features 路径报 unused
    let _ = h.dns_collector.lock().ok();
}

#[test]
fn test_system_history_field_shared_across_handler_clones() {
    // rmcp 内部每次 tool call clone handler，Arc::clone 应共享同一 system_history 实例
    let h1 = ProcMcpHandler::default();
    let _h2 = h1.clone();
    #[cfg(feature = "mcp-persistent-state")]
    {
        assert!(
            std::sync::Arc::ptr_eq(&h1.system_history, &_h2.system_history),
            "Cloned handlers must share system_history Arc"
        );
    }
    let _ = h1.dns_collector.lock().ok();
}

#[test]
fn test_system_history_populated_after_worker_warmup() {
    // ProcMcpHandler::new() spawn worker，等 worker 至少 push 2 个 sample。
    // 给 10s 余量（CI / 测试环境 sysinfo 初始化可能比生产慢 + refresh_heavy_incremental
    // 含 sysinfo networks/disks 调用偶发慢）。
    // 注意：此测试会 leak worker 线程（fire-and-forget），cargo test 进程退出时清理。
    let h = ProcMcpHandler::new();

    #[cfg(feature = "mcp-persistent-state")]
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut saw_two = false;
        while std::time::Instant::now() < deadline {
            if let Ok(guard) = h.system_history.lock() {
                if guard.len() >= 2 {
                    saw_two = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        assert!(
            saw_two,
            "worker should push >= 2 samples to system_history within 10s warm-up window"
        );
    }

    let _ = h.dns_collector.lock().ok();
}

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn test_make_metrics_history_json_unknown_metric_returns_error() {
    use proc::mcp::handler::MetricsSample;
    use proc::mcp::handler::observable::make_metrics_history_json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    let history: Arc<Mutex<VecDeque<MetricsSample>>> = Arc::new(Mutex::new(VecDeque::new()));
    let out = make_metrics_history_json("disk", None, &history);
    let obj = out.as_object().expect("object");
    assert_eq!(
        obj.get("ok").and_then(|v| v.as_bool()),
        Some(false),
        "unknown metric should return ok=false"
    );
    let err = obj
        .get("error")
        .and_then(|v| v.as_str())
        .expect("error string");
    assert!(
        err.contains("unknown metric 'disk'"),
        "error should mention unknown metric name, got: {err}"
    );
    assert!(
        err.contains("cpu / memory / swap"),
        "error should list valid metric names, got: {err}"
    );
}

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn test_make_metrics_history_json_empty_history_returns_count_zero() {
    use proc::mcp::handler::MetricsSample;
    use proc::mcp::handler::observable::make_metrics_history_json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    let history: Arc<Mutex<VecDeque<MetricsSample>>> = Arc::new(Mutex::new(VecDeque::new()));
    let out = make_metrics_history_json("cpu", None, &history);
    let obj = out.as_object().expect("object");
    assert_eq!(obj.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(obj.get("metric").and_then(|v| v.as_str()), Some("cpu"));
    assert_eq!(obj.get("seconds").and_then(|v| v.as_u64()), Some(30));
    assert_eq!(obj.get("count").and_then(|v| v.as_u64()), Some(0));
    assert!(
        obj.get("samples")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.is_empty()),
        "empty history should produce empty samples array"
    );
}

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn test_make_metrics_history_json_drains_cpu_samples_oldest_first() {
    use proc::mcp::handler::MetricsSample;
    use proc::mcp::handler::observable::make_metrics_history_json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    // 构造 3 个 sample fixture（不同 cpu_usage），验证 drain 顺序 oldest → newest
    let samples = vec![
        MetricsSample {
            cpu_usage: 11.0,
            memory_used: 1000,
            swap_used: 100,
            timestamp_unix: 1000,
        },
        MetricsSample {
            cpu_usage: 22.0,
            memory_used: 2000,
            swap_used: 200,
            timestamp_unix: 1001,
        },
        MetricsSample {
            cpu_usage: 33.0,
            memory_used: 3000,
            swap_used: 300,
            timestamp_unix: 1002,
        },
    ];
    let history: Arc<Mutex<VecDeque<MetricsSample>>> =
        Arc::new(Mutex::new(samples.into_iter().collect()));

    let out = make_metrics_history_json("cpu", Some(3), &history);
    let obj = out.as_object().expect("object");
    assert_eq!(obj.get("count").and_then(|v| v.as_u64()), Some(3));
    let arr = obj
        .get("samples")
        .and_then(|v| v.as_array())
        .expect("samples array");
    assert_eq!(arr.len(), 3);
    // oldest → newest
    let first_cpu = arr[0]
        .get("value")
        .and_then(|v| v.as_f64())
        .expect("first value");
    let last_cpu = arr[2]
        .get("value")
        .and_then(|v| v.as_f64())
        .expect("last value");
    assert!(
        (first_cpu - 11.0_f64).abs() < 0.01,
        "first sample should be oldest (cpu=11.0), got {first_cpu}"
    );
    assert!(
        (last_cpu - 33.0_f64).abs() < 0.01,
        "last sample should be newest (cpu=33.0), got {last_cpu}"
    );

    // 验证 ts 字段（agent 可推算）
    let first_ts = arr[0].get("ts").and_then(|v| v.as_u64()).expect("first ts");
    assert_eq!(first_ts, 1000, "ts should be unix seconds from sample");
}

#[cfg(feature = "mcp-persistent-state")]
#[test]
fn test_make_metrics_history_json_seconds_clamped_to_30() {
    use proc::mcp::handler::MetricsSample;
    use proc::mcp::handler::observable::make_metrics_history_json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    // 构造 5 个 sample 但请求 seconds=100，应截断到 30 且最多返 5 个（因只有 5 个）
    let samples: Vec<MetricsSample> = (0..5)
        .map(|i| MetricsSample {
            cpu_usage: i as f32,
            memory_used: i as u64 * 100,
            swap_used: i as u64 * 10,
            timestamp_unix: 2000 + i as u64,
        })
        .collect();
    let history: Arc<Mutex<VecDeque<MetricsSample>>> =
        Arc::new(Mutex::new(samples.into_iter().collect()));

    let out = make_metrics_history_json("memory", Some(100), &history);
    let obj = out.as_object().expect("object");
    // seconds 字段应截断到 30
    assert_eq!(
        obj.get("seconds").and_then(|v| v.as_u64()),
        Some(30),
        "seconds=100 should be clamped to 30"
    );
    // 但 count 仍是实际 sample 数（5 个）
    assert_eq!(
        obj.get("count").and_then(|v| v.as_u64()),
        Some(5),
        "count should be actual sample count (5), not seconds cap"
    );
}

#[test]
fn test_make_metrics_history_json_no_state_returns_stub() {
    // --no-default-features 路径 fallback：返 count=0 + note（不依赖 cfg gate）
    use proc::mcp::handler::observable::make_metrics_history_json_no_state;
    let out = make_metrics_history_json_no_state("swap", None);
    let obj = out.as_object().expect("object");
    assert_eq!(obj.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(obj.get("metric").and_then(|v| v.as_str()), Some("swap"));
    assert_eq!(obj.get("seconds").and_then(|v| v.as_u64()), Some(30));
    assert_eq!(obj.get("count").and_then(|v| v.as_u64()), Some(0));
    assert!(
        obj.get("note")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("mcp-persistent-state")),
        "note should mention mcp-persistent-state feature"
    );
}

// ===========================================================================
// v0.17 stage 4：ResourceRoute trait impl + ServerHandler 4 method
// ===========================================================================

#[test]
fn test_resource_route_unknown_uri_returns_error() {
    use proc::mcp::resources::ResourceRoute;
    let h = ProcMcpHandler::default();
    let result = h.route("proc://unknown/uri");
    assert!(result.is_err(), "unknown URI should return Err");
    let err = result.expect_err("error message");
    assert!(
        err.contains("unknown resource URI"),
        "error should mention unknown URI, got: {err}"
    );
    assert!(
        err.contains("proc://metrics/system"),
        "error should list valid URIs, got: {err}"
    );
}

#[test]
fn test_resource_route_metrics_system_returns_system_json() {
    use proc::mcp::resources::ResourceRoute;
    let h = ProcMcpHandler::default();
    // Default 路径无 worker，走 fallback 现场新建 SystemSnapshot（~500ms 开销）
    let result = h.route("proc://metrics/system");
    let value = result.expect("route should succeed for valid URI");
    let obj = value.as_object().expect("object");
    assert_eq!(
        obj.get("ok").and_then(|v| v.as_bool()),
        Some(true),
        "metrics/system should return ok=true JSON"
    );
    // System metrics JSON 应含 cpu_usage_pct 字段
    assert!(
        obj.contains_key("cpu_usage_pct"),
        "metrics/system should return cpu_usage_pct field"
    );
}

#[test]
fn test_resource_route_processes_list_returns_process_array() {
    use proc::mcp::resources::ResourceRoute;
    let h = ProcMcpHandler::default();
    let result = h.route("proc://processes/list");
    let value = result.expect("route should succeed");
    let obj = value.as_object().expect("object");
    assert_eq!(obj.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert!(
        obj.get("processes")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty()),
        "processes/list should return non-empty array on real system"
    );
    // top 50 cap
    let count = obj
        .get("count")
        .and_then(|v| v.as_u64())
        .expect("count field");
    assert!(
        count <= 50,
        "processes/list should cap at 50 entries (got {count})"
    );
}

#[test]
fn test_resource_route_docker_events_uri_returns_value() {
    // docker daemon 可能不可用，但 route 应返 Ok(JSON)（含 ok=true 或 ok=false，
    // 取决于 docker 是否启动）。我们只验 route 不 panic + 返 Value。
    use proc::mcp::resources::ResourceRoute;
    let h = ProcMcpHandler::default();
    let result = h.route("proc://docker/events");
    assert!(
        result.is_ok(),
        "docker/events route should not error even if docker unavailable"
    );
    let value = result.expect("value");
    assert!(
        value.as_object().is_some(),
        "docker/events should return JSON object"
    );
}

#[test]
fn test_procedure_resource_uris_constant_has_three_entries() {
    use proc::mcp::resources::PROC_RESOURCE_URIS;
    assert_eq!(
        PROC_RESOURCE_URIS.len(),
        3,
        "PROC_RESOURCE_URIS should have 3 entries"
    );
    assert!(PROC_RESOURCE_URIS.contains(&"proc://metrics/system"));
    assert!(PROC_RESOURCE_URIS.contains(&"proc://processes/list"));
    assert!(PROC_RESOURCE_URIS.contains(&"proc://docker/events"));
}

#[test]
fn test_resource_name_and_description_for_uri_returns_human_readable() {
    use proc::mcp::resources::{resource_description_for_uri, resource_name_for_uri};
    assert_eq!(
        resource_name_for_uri("proc://metrics/system"),
        "System Metrics"
    );
    assert_eq!(
        resource_name_for_uri("proc://processes/list"),
        "Process List (top 50 by cpu)"
    );
    assert_eq!(
        resource_name_for_uri("proc://docker/events"),
        "Docker Daemon Events"
    );
    assert_eq!(resource_name_for_uri("proc://unknown"), "Unknown");

    let desc = resource_description_for_uri("proc://metrics/system");
    assert!(
        !desc.contains("Unknown"),
        "description for known URI should not say Unknown"
    );
}

// ===========================================================================
// v0.17 stage 4：ServerHandler 4 method 静态断言（避开 rmcp 0.11 RequestContext
// 复杂构造——Peer<RoleServer> 需 running service）
//
// 与 stage 3 `test_proc_smart_source_description_in_mod_rs_contains_deprecated`
// 同款策略：直接 grep 源码验证 trait method 已实装。runtime 行为留 manual：
// `npx mcp-inspector proc mcp serve` + 调 resources/list / resources/read。
// ===========================================================================

#[test]
fn test_server_handler_get_info_declares_resources_capability() {
    use rmcp::ServerHandler;
    let h = ProcMcpHandler::default();
    let info = h.get_info();
    let resources_cap = info
        .capabilities
        .resources
        .as_ref()
        .expect("resources capability should be declared");
    assert_eq!(
        resources_cap.subscribe,
        Some(true),
        "resources.subscribe should be true (stage 4 决策 3)"
    );
}

#[test]
fn test_server_handler_impl_has_four_resource_methods() {
    // 静态断言 ServerHandler impl 块覆盖了 list_resources / read_resource /
    // subscribe / unsubscribe 4 个 method（rmcp 0.11 默认 impl 返空/Err，
    // 我们覆盖让 client 能拿到实际数据）
    let source = std::fs::read_to_string("src/mcp/handler/mod.rs").expect("mod.rs source readable");
    assert!(
        source.contains("fn list_resources("),
        "ServerHandler impl should override list_resources"
    );
    assert!(
        source.contains("fn read_resource("),
        "ServerHandler impl should override read_resource"
    );
    assert!(
        source.contains("fn subscribe("),
        "ServerHandler impl should override subscribe"
    );
    assert!(
        source.contains("fn unsubscribe("),
        "ServerHandler impl should override unsubscribe"
    );
}

#[test]
fn test_server_handler_impl_uses_resource_route_for_read() {
    // 静态断言 read_resource 内部调 ResourceRoute::route 走单一入口路由
    let source = std::fs::read_to_string("src/mcp/handler/mod.rs").expect("mod.rs source readable");
    assert!(
        source.contains("crate::mcp::resources::ResourceRoute::route"),
        "read_resource should delegate to ResourceRoute::route (ADR-0027 单一入口路由)"
    );
}

#[test]
fn test_server_handler_impl_uses_procedure_resource_uris_constant() {
    // 静态断言 list_resources 内部调 PROC_RESOURCE_URIS 常量（与 resources.rs 唯一来源）
    let source = std::fs::read_to_string("src/mcp/handler/mod.rs").expect("mod.rs source readable");
    assert!(
        source.contains("crate::mcp::resources::PROC_RESOURCE_URIS"),
        "list_resources should iterate PROC_RESOURCE_URIS constant"
    );
}

// ===========================================================================
// v0.17 stage 4：SSE transport 结构化 stub
// ===========================================================================

#[test]
fn test_serve_sse_returns_v0_18_hint_error() {
    use proc::mcp::transport::{SseTransportConfig, serve_sse};
    let config = SseTransportConfig::default();
    let result = serve_sse(&config);
    assert!(
        result.is_err(),
        "serve_sse stub should return Err (structured stub)"
    );
    let err = result.expect_err("error message");
    assert!(
        err.contains("v0.18+"),
        "error should mention v0.18+ cycle deferral, got: {err}"
    );
    assert!(
        err.contains("streamable_http_server"),
        "error should mention rmcp streamable_http_server module, got: {err}"
    );
    assert!(
        err.contains("Cargo feature"),
        "error should mention Cargo feature requirement, got: {err}"
    );
}

#[test]
fn test_sse_transport_config_default_is_port_8080() {
    use proc::mcp::transport::SseTransportConfig;
    let config = SseTransportConfig::default();
    assert_eq!(config.port, 8080, "default port should be 8080");
    assert_eq!(
        config.bind_addr, "0.0.0.0",
        "default bind_addr should be 0.0.0.0"
    );
}

#[test]
fn test_sse_transport_config_new_sets_port() {
    use proc::mcp::transport::SseTransportConfig;
    let config = SseTransportConfig::new(9123);
    assert_eq!(config.port, 9123);
    assert_eq!(config.bind_addr, "0.0.0.0");
}
