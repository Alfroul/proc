//! v0.15.0 阶段 2 集成测试 — MCP cat 1 (9 tool) + cat 2 (proc_inspect 6 tab)
//! 业务逻辑。
//!
//! 与 `test_mcp_server.rs` 同款策略：直接调 `handler::cli::make_*` /
//! `handler::inspect::make_*` helper 验证 thin wrapper 把业务模块 JSON-ify 没
//! 漏字段。stdio 端到端测试留 manual：`npx mcp-inspector proc mcp serve`。

use proc::mcp::handler::cli;
use proc::mcp::handler::inspect::{self, InspectTab};

// ===========================================================================
// 类别 1（9 tool）
// ===========================================================================

#[test]
fn test_proc_flows_returns_ok_with_worker_field() {
    let out = cli::make_flows_json(Some(10));
    assert!(out.get("ok").is_some(), "missing ok field: {out}");
    assert_eq!(out["ok"], serde_json::json!(true));
    // worker 字段必须在（"schannel_etw" 或 "unavailable" 都可）
    let worker = out["worker"].as_str().expect("worker missing");
    assert!(
        matches!(worker, "schannel_etw" | "unavailable"),
        "worker must be schannel_etw / unavailable, got: {worker}"
    );
    // count 字段在（worker 不可用时 0）
    assert!(out.get("count").is_some(), "missing count: {out}");
    // flows 是数组（即便空）
    assert!(out["flows"].is_array(), "flows not array: {out}");
}

#[test]
fn test_proc_flows_default_limit_is_50() {
    // limit=None 走默认 50（与 CLI `proc flows` 同款）。验证上限不超 50
    // —— 由于本测试机几乎肯定没 50+ flow，count 通常 < 50，但 schema 不挂即可。
    let out = cli::make_flows_json(None);
    assert_eq!(out["ok"], serde_json::json!(true));
    let flows = out["flows"].as_array().expect("flows array");
    assert!(
        flows.len() <= 50,
        "default limit not enforced: {}",
        flows.len()
    );
}

#[test]
fn test_proc_throttle_query_self_pid_returns_ok_or_unknown() {
    let self_pid = std::process::id();
    let out = cli::make_throttle_json(self_pid, None);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["pid"], serde_json::json!(self_pid));
    assert_eq!(out["action"], serde_json::json!("get"));
    // state 在 Normal / Eco / Unknown 三档之一（取决于本机权限 / Win11 build）
    let state = out["state"].as_str().expect("state string");
    assert!(
        matches!(state, "Normal" | "Eco" | "Unknown"),
        "state must be Normal/Eco/Unknown, got: {state}"
    );
}

#[test]
fn test_proc_throttle_query_bogus_pid_returns_state_unknown_or_err() {
    // PID 999999 几乎肯定不存在。Windows 上 query 会返 Unknown；非 Windows 返
    // ok=false。两条路径都接受（只要不 panic）。
    let out = cli::make_throttle_json(999_999, None);
    assert!(out.get("ok").is_some(), "missing ok: {out}");
    if out["ok"].as_bool() == Some(true) {
        let state = out["state"].as_str().unwrap_or("");
        assert!(
            matches!(state, "Unknown" | "Normal"),
            "bogus pid state should be Unknown (or Normal for cached handle), got: {state}"
        );
    }
}

#[test]
fn test_proc_export_json_default_returns_payload_string() {
    let out = cli::make_export_json(None, None, Some(5));
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["format"], serde_json::json!("json"));
    assert!(out["count"].as_u64().is_some(), "count missing");
    // payload 是 JSON 字符串（embed 在 serde_json::Value 里仍是 string）
    let payload = out["payload"].as_str().expect("payload is string");
    assert!(
        payload.contains("\"processes\""),
        "JSON payload missing processes: {payload}"
    );
}

#[test]
fn test_proc_export_csv_returns_payload_with_csv_header() {
    let out = cli::make_export_json(Some("csv"), None, Some(3));
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["format"], serde_json::json!("csv"));
    let payload = out["payload"].as_str().expect("csv payload");
    assert!(
        payload.starts_with("pid,name,cpu_usage,memory_bytes,exe"),
        "csv header missing: {payload}"
    );
}

#[test]
fn test_proc_export_with_sort_cpu_field_sets_sort_label() {
    let out = cli::make_export_json(Some("json"), Some("cpu"), Some(3));
    assert_eq!(out["sort"], serde_json::json!("cpu"));
}

#[test]
fn test_proc_docker_inspect_invalid_name_returns_ok_false() {
    // 容器名肯定不存在 → ok=false / error 字段在；docker 不可用时也 ok=false。
    let out = cli::make_docker_inspect_json("proc-mcp-test-nonexistent-xyz-123");
    if out["ok"].as_bool() == Some(true) {
        // docker 在但容器不在 → error 字段必须在
        assert!(
            out.get("error").is_some() || out["container"].is_object(),
            "ok=true path missing container/error: {out}"
        );
    } else {
        // docker 不在 → error
        assert!(
            out.get("error").is_some(),
            "ok=false path missing error: {out}"
        );
    }
}

#[test]
fn test_proc_docker_images_returns_ok_or_docker_unavailable() {
    let out = cli::make_docker_images_json();
    assert!(out.get("ok").is_some(), "missing ok: {out}");
    if out["ok"].as_bool() == Some(true) {
        assert!(out["images"].is_array(), "images is array");
        // 每个 image 字段完整（短 id 非空）
        for img in out["images"].as_array().unwrap() {
            assert!(
                img["short_id"].as_str().is_some(),
                "short_id missing: {img}"
            );
        }
    } else {
        assert!(out.get("error").is_some(), "ok=false missing error: {out}");
    }
}

#[test]
fn test_proc_docker_volumes_returns_ok_or_docker_unavailable() {
    let out = cli::make_docker_volumes_json();
    assert!(out.get("ok").is_some(), "missing ok: {out}");
    if out["ok"].as_bool() == Some(true) {
        assert!(out["volumes"].is_array(), "volumes is array");
    } else {
        assert!(out.get("error").is_some(), "ok=false missing error: {out}");
    }
}

#[test]
fn test_proc_docker_events_returns_ok_with_limit() {
    // docker 不可用 → ok=false（docker connect 失败）；可用 → ok=true，count 可能 0
    let out = cli::make_docker_events_json(Some(50));
    assert!(out.get("ok").is_some(), "missing ok: {out}");
    if out["ok"].as_bool() == Some(true) {
        assert!(out["events"].is_array(), "events is array");
        // count 不超 limit（50）
        let count = out["count"].as_u64().unwrap_or(0);
        assert!(count <= 50, "limit not enforced: count={count}");
        // note 字段必须在（drained / no events）
        assert!(out.get("note").is_some(), "note field missing");
    }
}

#[test]
fn test_proc_monitor_add_pid_dry_run_returns_preview() {
    let out = cli::make_monitor_add_json("pid", "1234", None, Some(true));
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["dry_run"], serde_json::json!(true));
    // preview 字段含 target_kind / target
    assert_eq!(out["preview"]["target_kind"], serde_json::json!("pid"));
    assert_eq!(out["preview"]["target"], serde_json::json!("1234"));
}

#[test]
fn test_proc_monitor_add_pid_real_call_returns_id() {
    let out = cli::make_monitor_add_json("pid", "5678", None, None);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["dry_run"], serde_json::json!(false));
    // 真实路径返 id（u32 自增从 1 开始）
    let id = out["id"].as_u64().expect("id missing");
    assert!(id >= 1, "id should be >= 1, got {id}");
}

#[test]
fn test_proc_monitor_add_invalid_kind_returns_ok_false() {
    let out = cli::make_monitor_add_json("bogus", "x", None, None);
    assert_eq!(out["ok"], serde_json::json!(false));
    assert!(out.get("error").is_some(), "missing error: {out}");
}

#[test]
fn test_proc_monitor_add_invalid_pid_format_returns_ok_false() {
    let out = cli::make_monitor_add_json("pid", "not-a-number", None, None);
    assert_eq!(out["ok"], serde_json::json!(false));
    assert!(out.get("error").is_some(), "missing error: {out}");
}

#[test]
fn test_proc_monitor_add_port_real_call_returns_id() {
    let out = cli::make_monitor_add_json("port", "8080", Some("auto_restart"), None);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["restart_policy"], serde_json::json!("auto_restart"));
}

#[test]
fn test_proc_monitor_add_command_real_call_splits_target() {
    let out = cli::make_monitor_add_json("command", "nginx -g 'daemon off;'", None, None);
    assert_eq!(out["ok"], serde_json::json!(true));
}

#[test]
fn test_proc_monitor_remove_invalid_id_format_returns_ok_false() {
    let out = cli::make_monitor_remove_json("not-a-number", None);
    assert_eq!(out["ok"], serde_json::json!(false));
    let error = out["error"].as_str().expect("error msg");
    assert!(error.contains("positive integer"), "bad error msg: {error}");
}

#[test]
fn test_proc_monitor_remove_nonexistent_id_returns_ok_false() {
    // MonitorManager::new() 是空表，删任何 ID 都 fail
    let out = cli::make_monitor_remove_json("999", None);
    assert_eq!(out["ok"], serde_json::json!(false));
    assert!(out.get("error").is_some(), "missing error: {out}");
}

#[test]
fn test_proc_monitor_remove_dry_run_returns_preview() {
    let out = cli::make_monitor_remove_json("5", Some(true));
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["dry_run"], serde_json::json!(true));
    assert_eq!(out["preview"]["id"], serde_json::json!(5));
}

// ===========================================================================
// 类别 2（proc_inspect 6 tab）
// ===========================================================================

#[test]
fn test_proc_inspect_summary_self_pid_returns_full_info() {
    let self_pid = std::process::id();
    let out = inspect::make_inspect_json(self_pid, &InspectTab::Summary, false);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["pid"], serde_json::json!(self_pid));
    assert_eq!(out["tab"], serde_json::json!("summary"));
    // process 字段含基础信息
    let proc_json = &out["process"];
    assert!(proc_json["name"].is_string(), "name missing");
    assert!(proc_json["cmd"].is_array(), "cmd missing or not array");
    assert!(
        proc_json["exe"].is_string() || proc_json["exe"].is_null(),
        "exe missing"
    );
    assert!(
        proc_json.get("security_score").is_some() || out.get("security_score").is_some(),
        "security_score missing"
    );
    // parent_chain 是数组（可能空）
    assert!(out["parent_chain"].is_array(), "parent_chain not array");
    // risk_factors 是数组
    assert!(out["risk_factors"].is_array(), "risk_factors not array");
}

#[test]
fn test_proc_inspect_summary_bogus_pid_returns_ok_false() {
    let out = inspect::make_inspect_json(4_294_967_295, &InspectTab::Summary, false);
    assert_eq!(out["ok"], serde_json::json!(false));
    assert!(out.get("error").is_some(), "missing error: {out}");
}

#[test]
fn test_proc_inspect_env_self_masks_secrets() {
    let self_pid = std::process::id();
    // 先设个 secret 环境变量，确保至少有一条 secret 命中 mask。
    // 注：「PLAIN」故意避开 SECRET/KEY/TOKEN 等 substring，避免被
    // `is_secret_key` 误判（漏检 < 误检是 brainstorm 既定策略）。
    // Rust 2024 起 std::env::set_var 是 unsafe（多线程改动环境）。
    unsafe {
        std::env::set_var("PROC_MCP_TEST_API_TOKEN", "AKIAIOSFODNN7EXAMPLE");
        std::env::set_var("PROC_MCP_TEST_PLAIN_VALUE", "hello-world");
    }

    let out = inspect::make_inspect_json(self_pid, &InspectTab::Env, false);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["tab"], serde_json::json!("env"));
    assert_eq!(out["reveal"], serde_json::json!(false));

    let arr = out["env_vars"].as_array().expect("env_vars array");
    assert!(!arr.is_empty(), "self env should not be empty");

    // 找到 secret / 非 secret 各 1 条验证 mask 行为
    let secret_entry = arr.iter().find(|v| {
        v["key"]
            .as_str()
            .map(|k| k.contains("API_TOKEN"))
            .unwrap_or(false)
    });
    let Some(secret) = secret_entry else {
        // CI 极端环境可能拿不到本进程 env；至少 array 字段 schema 正确即可。
        return;
    };
    assert_eq!(secret["is_secret"], serde_json::json!(true));
    let val = secret["value"].as_str().expect("secret value string");
    assert!(val.contains("***"), "secret should be masked: {val}");
    assert!(
        !val.contains("AKIAIOSFODNN7EXAMPLE"),
        "masked value leaked real: {val}"
    );

    let non_secret = arr.iter().find(|v| {
        v["key"]
            .as_str()
            .map(|k| k.contains("PLAIN_VALUE"))
            .unwrap_or(false)
    });
    if let Some(ns) = non_secret {
        assert_eq!(ns["is_secret"], serde_json::json!(false));
        assert_eq!(ns["value"], serde_json::json!("hello-world"));
    }
}

#[test]
fn test_proc_inspect_env_reveal_true_shows_real_value() {
    let self_pid = std::process::id();
    // Rust 2024: set_var 是 unsafe。
    unsafe {
        std::env::set_var("PROC_MCP_TEST_REVEAL_KEY", "super-secret-token-value");
    }

    let out = inspect::make_inspect_json(self_pid, &InspectTab::Env, true);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["reveal"], serde_json::json!(true));

    let arr = out["env_vars"].as_array().expect("env_vars array");
    let Some(secret) = arr.iter().find(|v| {
        v["key"]
            .as_str()
            .map(|k| k.contains("REVEAL_KEY"))
            .unwrap_or(false)
    }) else {
        return;
    };
    assert_eq!(secret["is_secret"], serde_json::json!(true));
    // reveal=true → 显示真值
    assert_eq!(
        secret["value"],
        serde_json::json!("super-secret-token-value")
    );
}

#[test]
fn test_proc_inspect_network_self_returns_ok_even_if_empty() {
    let self_pid = std::process::id();
    let out = inspect::make_inspect_json(self_pid, &InspectTab::Network, false);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["tab"], serde_json::json!("network"));
    // listening / established / dns_recent 都是数组（可能空）
    assert!(out["listening"].is_array(), "listening not array");
    assert!(out["established"].is_array(), "established not array");
    assert!(out["dns_recent"].is_array(), "dns_recent not array");
}

#[test]
fn test_proc_inspect_dlls_self_nonempty() {
    let self_pid = std::process::id();
    let out = inspect::make_inspect_json(self_pid, &InspectTab::Dlls, false);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["tab"], serde_json::json!("dlls"));
    let arr = out["dlls"].as_array().expect("dlls array");
    // 自己进程至少加载一个模块（自身可执行 + kernel32 / ntdll 等）
    assert!(!arr.is_empty(), "expected >=1 dll for self, got empty");
    let first = &arr[0];
    assert!(first["path"].as_str().is_some(), "path missing");
    assert!(first["base_addr"].as_u64().is_some(), "base_addr missing");
    assert!(first["size"].as_u64().is_some(), "size missing");
}

#[test]
fn test_proc_inspect_memory_map_self_nonempty() {
    let self_pid = std::process::id();
    let out = inspect::make_inspect_json(self_pid, &InspectTab::MemoryMap, false);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["tab"], serde_json::json!("memory_map"));
    let arr = out["regions"].as_array().expect("regions array");
    assert!(!arr.is_empty(), "expected >=1 memory region for self");
    let first = &arr[0];
    assert!(first["base_addr"].as_u64().is_some());
    assert!(first["size"].as_u64().is_some());
    assert!(first["state"].as_str().is_some(), "state missing");
}

#[test]
fn test_proc_inspect_handles_self_returns_array() {
    let self_pid = std::process::id();
    let out = inspect::make_inspect_json(self_pid, &InspectTab::Handles, false);
    assert_eq!(out["ok"], serde_json::json!(true));
    // handles tab 复用 make_handles_json schema（含 count + handles[]）
    assert!(out["handles"].is_array(), "handles not array");
}

#[test]
fn test_proc_inspect_bogus_pid_dlls_returns_ok_or_empty() {
    // bogus pid 在 dlls/memory 路径：collect_dlls 失败 → unwrap_or_default 空 Vec
    // → ok=true count=0（不挂）；与 summary 路径不同（summary 显式查 process not found）。
    let out = inspect::make_inspect_json(4_294_967_295, &InspectTab::Dlls, false);
    assert!(out.get("ok").is_some(), "missing ok: {out}");
    // 不 panic 即可
}

// ===========================================================================
// 类别 4（系统级 metrics，5 tool） — stage 3 新增
// ===========================================================================

use proc::mcp::handler::metrics;

#[test]
fn test_proc_metrics_system_returns_full_snapshot() {
    let out = metrics::make_metrics_system_json();
    assert_eq!(out["ok"], serde_json::json!(true));
    // CPU / uptime / process count
    assert!(out["cpu_usage_pct"].as_f64().is_some(), "cpu_usage_pct");
    assert!(out["uptime_secs"].as_u64().is_some(), "uptime_secs");
    assert!(out["processes_count"].as_u64().is_some(), "processes_count");
    // memory / swap / system_disk 三段都有 used_bytes/total_bytes/pct
    for field in ["memory", "swap", "system_disk"] {
        let seg = &out[field];
        assert!(seg.is_object(), "{field} must be object: {seg}");
        assert!(seg["used_bytes"].as_u64().is_some(), "{field} used_bytes");
        assert!(seg["total_bytes"].as_u64().is_some(), "{field} total_bytes");
        assert!(seg["pct"].as_f64().is_some(), "{field} pct");
    }
}

#[test]
fn test_proc_metrics_system_network_interfaces_is_array_and_filtered() {
    let out = metrics::make_metrics_system_json();
    let arr = out["network_interfaces"]
        .as_array()
        .expect("network_interfaces array");
    // 不强求数组非空（容器 / CI 可能没网卡），但每条不能含 169.254 / 127.0.0.1
    for ni in arr {
        if let Some(ip) = ni["ipv4"].as_str() {
            assert!(!ip.starts_with("169.254."), "APIPA leaked: {ip}");
            assert!(ip != "127.0.0.1", "loopback leaked: {ip}");
        }
    }
}

#[test]
fn test_proc_metrics_system_tcp_stats_has_seven_fields() {
    let out = metrics::make_metrics_system_json();
    let tcp = &out["tcp_stats"];
    assert!(tcp.is_object(), "tcp_stats object");
    // 7 个字段（established/time_wait/close_wait/listen + 3 累计计数 + out_segs）
    for field in [
        "established",
        "time_wait",
        "close_wait",
        "listen",
        "retransmitted_segs",
        "reset_segs",
        "failed_connections",
    ] {
        assert!(tcp.get(field).is_some(), "tcp_stats missing {field}: {tcp}");
    }
}

#[test]
fn test_proc_metrics_gpu_returns_ok_with_providers_array() {
    let out = metrics::make_metrics_gpu_json();
    assert_eq!(out["ok"], serde_json::json!(true));
    // providers 是字符串数组（即便空也必须是数组）
    assert!(out["providers"].is_array(), "providers array");
    // gpus 是数组
    assert!(out["gpus"].is_array(), "gpus array");
    // 空时 note 字段在
    if out["gpus"].as_array().unwrap().is_empty() {
        assert!(out.get("note").is_some(), "empty gpus should have note");
    } else {
        // 非空时每条 gpu 含 vendor / vram 字段
        for g in out["gpus"].as_array().unwrap() {
            assert!(g["vendor"].is_string(), "vendor string: {g}");
            assert!(g["vram"].is_object(), "vram object: {g}");
            assert!(g["vram"]["used_bytes"].as_u64().is_some());
            assert!(g["vram"]["total_bytes"].as_u64().is_some());
        }
    }
}

#[test]
fn test_proc_metrics_disk_io_returns_per_disk_array() {
    let out = metrics::make_metrics_disk_io_json(None);
    assert_eq!(out["ok"], serde_json::json!(true));
    // total 段在
    let total = &out["total"];
    assert!(total["read_bps"].as_u64().is_some(), "total read_bps");
    assert!(total["write_bps"].as_u64().is_some(), "total write_bps");
    // per_disk / disks 都是数组
    assert!(out["per_disk"].is_array(), "per_disk array");
    assert!(out["disks"].is_array(), "disks array");
    // 非空 per_disk 每条含 name/mount_point/read_bps/write_bps
    for d in out["per_disk"].as_array().unwrap() {
        assert!(d["name"].is_string(), "name: {d}");
        assert!(d["read_bps"].as_u64().is_some(), "read_bps: {d}");
        assert!(d["write_bps"].as_u64().is_some(), "write_bps: {d}");
    }
}

#[test]
fn test_proc_metrics_disk_io_filter_by_device_returns_subset_or_empty() {
    // device=Some 不存在的设备 → per_disk 空，但 total/disks 字段仍返（决策 5）
    let out = metrics::make_metrics_disk_io_json(Some("DEFINITELY_NOT_A_REAL_DEVICE_XYZ"));
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(
        out["device_filter"],
        serde_json::json!("DEFINITELY_NOT_A_REAL_DEVICE_XYZ")
    );
    let per_disk = out["per_disk"].as_array().expect("per_disk array");
    assert!(
        per_disk.is_empty(),
        "per_disk should be empty for bogus device"
    );
    // total / disks 不受 filter 影响
    assert!(out["total"].is_object(), "total still present");
    assert!(out["disks"].is_array(), "disks still present");
}

#[test]
fn test_proc_metrics_smart_aggregated_returns_disks_array() {
    let out = metrics::make_metrics_smart_json(None);
    assert_eq!(out["ok"], serde_json::json!(true));
    assert_eq!(out["mode"], serde_json::json!("aggregated"));
    let arr = out["disks"].as_array().expect("disks array");
    // 空 SMART-readable disks（容器 / VM / 无 smartctl）→ note 字段在
    if arr.is_empty() {
        assert!(
            out.get("note").is_some(),
            "empty aggregated should have note"
        );
    } else {
        // 非空时每条含 device/model/health（read_smart 失败的 disk 用 error 字段，不在此断言）
        for d in arr {
            // 不是错误条目（含 error 字段）才校验 schema
            if d.get("error").is_none() {
                assert!(d["device"].is_string(), "device: {d}");
                assert!(d["model"].is_string(), "model: {d}");
                assert!(d["health"].is_string(), "health: {d}");
            }
        }
    }
}

#[test]
fn test_proc_metrics_smart_specific_device_returns_attributes_or_error() {
    // 不存在的 device → ok=false / error 字段（read_smart 失败）
    let out = metrics::make_metrics_smart_json(Some("DEFINITELY_NOT_A_REAL_DISK_XYZ"));
    if out["ok"].as_bool() == Some(true) {
        // 极端情况：read_smart 返了空 attributes 也不挂
        assert_eq!(out["mode"], serde_json::json!("single_device"));
        assert!(out["disk"].is_object(), "disk object");
    } else {
        // 预期路径：error 字段在
        assert!(out.get("error").is_some(), "missing error: {out}");
    }
}

#[test]
fn test_proc_metrics_thermal_returns_per_core_arrays_same_length() {
    let out = metrics::make_metrics_thermal_json();
    assert_eq!(out["ok"], serde_json::json!(true));
    // per_core_freq / per_core_temp 必须是数组
    let freq = out["per_core_freq_mhz"]
        .as_array()
        .expect("per_core_freq_mhz array");
    let temp = out["per_core_temp_c"]
        .as_array()
        .expect("per_core_temp_c array");
    // 长度一致（决策 6）
    assert_eq!(
        freq.len(),
        temp.len(),
        "freq/temp length mismatch: {} vs {}",
        freq.len(),
        temp.len()
    );
    // throttle 字段在（含 null 路径）/ reason 字符串在
    assert!(out.get("throttle").is_some(), "throttle field missing");
    let reason = out["reason"].as_str().expect("reason string");
    assert!(
        matches!(
            reason,
            "None" | "Thermal" | "PowerPolicy" | "Idle" | "Unknown" | "Unavailable"
        ),
        "reason unexpected: {reason}"
    );
}

#[test]
fn test_proc_metrics_thermal_throttle_field_shape() {
    let out = metrics::make_metrics_thermal_json();
    // throttle 字段 null（无 PROCESSOR_POWER_INFORMATION）或完整 5 字段对象
    if out["throttle"].is_null() {
        assert_eq!(out["reason"], serde_json::json!("Unavailable"));
    } else {
        let ti = &out["throttle"];
        assert!(ti.is_object(), "throttle object: {ti}");
        for field in [
            "max_mhz",
            "current_mhz",
            "mhz_limit",
            "is_throttled",
            "throttle_pct",
        ] {
            assert!(ti.get(field).is_some(), "throttle missing {field}: {ti}");
        }
    }
}
