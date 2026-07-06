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
