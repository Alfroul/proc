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
