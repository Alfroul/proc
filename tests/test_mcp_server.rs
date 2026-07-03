//! v0.7.0 阶段 2 集成测试 — MCP server。
//!
//! 这些测试不真起 stdio MCP server（启动一个 subprocess + 跑 JSON-RPC hand-shake
//! 在 CI 里既慢又脆）。改成两条路验证：
//! 1. `tool_router().list_all()` → 17+ tool 都注册了（验证 `#[tool]` 宏没漏）。
//! 2. 直接调 `handler::make_*_json` helper → 业务逻辑（验证 thin wrapper 把
//!    采集层 JSON-ify 没漏字段）。
//!
//! stdio 端到端测试留 manual：`npx mcp-inspector proc mcp serve`（README 有指引）。

use proc::mcp::handler;

#[test]
fn test_list_tools_has_at_least_17() {
    let names = handler::list_tool_names();
    println!("registered tools ({}): {:?}", names.len(), names);
    assert!(
        names.len() >= 17,
        "expected at least 17 MCP tools, got {}: {names:?}",
        names.len()
    );

    // 关键 tool 名字都得在
    for required in [
        "proc_ls",
        "proc_tree",
        "proc_port",
        "proc_kill",
        "proc_pkill",
        "proc_eject",
        "proc_who",
        "proc_handles",
        "proc_priority",
        "proc_affinity",
        "proc_smart",
        "proc_dns",
        "proc_diag",
        "proc_monitor_list",
        "proc_docker_ps",
        "proc_docker_top",
        "proc_docker_logs",
    ] {
        assert!(
            names.iter().any(|n| n == required),
            "required tool {required} missing from {names:?}"
        );
    }
}

#[test]
fn test_proc_ls_with_limit_returns_n_processes() {
    let out = handler::make_processes_json(None, Some(5));
    assert_eq!(out["ok"], serde_json::json!(true), "ok flag wrong: {out}");
    assert_eq!(out["count"], serde_json::json!(5), "expected 5 entries");
    let procs = out["processes"].as_array().expect("processes is array");
    assert_eq!(procs.len(), 5);
    // 字段完整
    let first = &procs[0];
    assert!(first["pid"].as_u64().is_some(), "pid missing: {first}");
    assert!(first["name"].as_str().is_some(), "name missing");
    assert!(first["cpu_usage"].as_f64().is_some(), "cpu_usage missing");
}

#[test]
fn test_proc_ls_sort_cpu_returns_top_cpu_consumers() {
    let out = handler::make_processes_json(Some("cpu"), Some(3));
    assert_eq!(out["sort"], serde_json::json!("cpu"));
    let procs = out["processes"].as_array().expect("processes is array");
    assert_eq!(procs.len(), 3);
    // 降序：第一项 cpu_usage >= 第二项 >= 第三项
    let cpu_values: Vec<f64> = procs
        .iter()
        .map(|p| p["cpu_usage"].as_f64().unwrap_or(0.0))
        .collect();
    assert!(
        cpu_values[0] >= cpu_values[1] && cpu_values[1] >= cpu_values[2],
        "cpu sort not descending: {cpu_values:?}"
    );
}

#[test]
fn test_proc_kill_nonexistent_pid_does_not_crash() {
    // PID 999999 不太可能存在；要求：返回结构化 JSON（不 panic）。
    // 可能结果：AlreadyGone / Failed / 或底层 error —— 关键是 ok 字段在 + 有 result 描述。
    let out = handler::make_kill_json(999_999, false);
    // 不是 panic 就过；ok 字段必须在（true 或 false 都行）。
    assert!(out.get("ok").is_some(), "missing ok field: {out}");
    assert_eq!(out["pid"], serde_json::json!(999_999));
    println!("proc_kill(999999) → {out}");
}

#[test]
fn test_proc_diag_returns_worker_metrics() {
    // 此测试要起 App（worker 启 2s），慢但必要。
    let out = handler::make_diag_json();
    assert_eq!(out["ok"], serde_json::json!(true), "diag failed: {out}");
    let workers = out["workers"].as_array().expect("workers is array");
    // 至少有 light/port 两个 worker（dns 在非 Windows 跳过）。
    assert!(
        !workers.is_empty(),
        "expected at least one worker in diag: {out}"
    );
    // 字段完整
    let first = &workers[0];
    assert!(first["name"].as_str().is_some(), "name missing: {first}");
    assert!(first["avg_us"].as_u64().is_some(), "avg_us missing");
    assert!(first["max_us"].as_u64().is_some(), "max_us missing");
    assert!(first["polls"].as_u64().is_some(), "polls missing");
    // TD-23（v0.12 阶段 5）：dns_collector 字段必须在 JSON 输出里，三态字符串。
    let dns_kind = out["dns_collector"]
        .as_str()
        .expect("dns_collector missing or not a string");
    assert!(
        matches!(dns_kind, "etw" | "powershell" | "none"),
        "dns_collector must be one of etw/powershell/none, got: {dns_kind}"
    );
}

#[test]
fn test_proc_docker_ps_does_not_crash_without_daemon() {
    // 没 docker 跑着时也要返回结构化 JSON（不是 panic）。
    // CI 上 docker 不可用，所以期望 ok=false + error 字段，或 ok=true + count=0。
    let out = handler::make_docker_ps_json();
    assert!(out.get("ok").is_some(), "missing ok field: {out}");
    if out["ok"].as_bool() == Some(true) {
        // docker 在 → 字段齐全
        let containers = out["containers"].as_array().expect("containers is array");
        for c in containers {
            assert!(c["name"].as_str().is_some(), "container name missing: {c}");
            assert!(c["state"].as_str().is_some(), "container state missing");
        }
    } else {
        // docker 不在 → 友好 error
        assert!(
            out.get("error").is_some(),
            "docker unavailable but no error field: {out}"
        );
    }
    println!("proc_docker_ps → {out}");
}

// --- TD-36（v0.12 阶段 5）：MCP DNS tool 持久 collector ---

#[test]
fn td36_default_handler_dns_collector_is_none_no_panic() {
    // Default 构造（不 spawn collector）→ proc_dns 返回 unavailable，不 panic。
    // 这是测试路径 / 非 Windows 平台的行为；生产入口走 new()。
    let h = handler::ProcMcpHandler::default();
    let arc_mutex = std::sync::Arc::clone(&h.dns_collector);
    drop(h); // 持有 collector 的 handler drop；arc 引用还在

    // 多次调用不应 panic（Mutex 互斥正常）。
    for _ in 0..3 {
        let out = handler::make_dns_json_from_collector(&arc_mutex, false);
        // ok 字段必须在（true 或 false）。
        assert!(out.get("ok").is_some(), "ok missing: {out}");
        // Default 路径 collector == None → ok=false。
        assert_eq!(
            out["ok"],
            serde_json::json!(false),
            "Default 路径无 collector 应返 ok=false: {out}"
        );
    }
}

#[test]
fn td36_tail_param_still_rejected_via_persistent_collector() {
    // 持久 collector 路径下 tail=true 仍被拒（streaming-only）。
    let h = handler::ProcMcpHandler::default();
    let arc_mutex = std::sync::Arc::clone(&h.dns_collector);
    let out = handler::make_dns_json_from_collector(&arc_mutex, true);
    assert_eq!(
        out["ok"],
        serde_json::json!(false),
        "tail=true 应被拒: {out}"
    );
    assert!(
        out.get("error").is_some(),
        "tail=true 应有 error 描述: {out}"
    );
}

#[test]
fn td36_new_handler_shares_collector_across_clones() {
    // ProcMcpHandler::new() 在 Windows 上 spawn collector（或非 Windows 返 None）。
    // rmcp serve_server 内部每次 tool call 会 clone handler；本测试验证 Clone
    // 后两个 handler 共享同一 Arc<Mutex<...>> 实例（不会重复 spawn / 丢事件）。
    // 不实际调 detect_collector（避免 spawn ETW session / PowerShell 子进程
    // 污染测试输出），改用 Default + 手动塞 collector 验证 Arc 共享语义。
    use proc::dns_log::detect_collector;

    let h1 = handler::ProcMcpHandler::default();
    let h2 = h1.clone();
    // 两个 handler 的 Arc 指向同一 Mutex（强引用计数 = 2）。
    assert!(
        std::sync::Arc::ptr_eq(&h1.dns_collector, &h2.dns_collector),
        "Clone 后应共享同一 Arc<Mutex<...>> 实例"
    );

    // 模拟 detect_collector 的实际行为：返回 (Option, Kind)。Default 路径 collector
    // 为 None，不影响本测试（只验证 Arc 共享）。
    let (_collector, kind) = detect_collector();
    let _ = kind; // 不强制断言 kind（跨平台 / admin 状态不同）
}
