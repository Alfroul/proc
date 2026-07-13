//! v0.18 cycle 集成测试 — 4 项业务实装验证。
//!
//! 与 `test_mcp_v0_15.rs` / `test_mcp_v0_16.rs` / `test_mcp_v0_17.rs` 同款策略：
//! 直接调 helper + handler 字段断言 + 静态断言，验证 stage 2 Slice 改动行为正确。
//! stdio 端到端测试留 manual：`npx mcp-inspector proc mcp serve`。
//!
//! ## stage 2 Slice 落地范围
//!
//! - **项 3 subscribe-push**：worker subscriber_count + ResourceRoute impl 静态断言
//!   + spawn_push_task 在无 tokio runtime 返 Err（真实 lifecycle 用 mcp-inspector
//!     manual 验证）
//! - **项 4 auto-stop**：shutdown::request() flip flag + 静态断言 timer thread 实装
//!   + warning 字段移除 + spawn cmd 加 --duration flag（真实 auto-stop 用 manual 验证）
//! - **项 2 varint 等价性**：v4 varint 写入与 v3 fixint 字节流不同 + round-trip 一致
//!   + v3 fixint 兼容层
//! - **项 1 代码清理**：既有 `test_record_start_stop_round_trip` /
//!   `test_record_stop_no_active` 覆盖（不增测试）

use proc::mcp::PROC_RESOURCE_URIS;
use proc::mcp::subscribe_worker::SubscribePushWorker;

// ===========================================================================
// 项 3：subscribe-push worker lifecycle（stage 2 实装）
// ===========================================================================

#[test]
fn test_subscribe_push_worker_default_empty() {
    // stage 2：新 worker 注册表为空 + push task 未 spawn
    let worker = SubscribePushWorker::new();
    assert_eq!(worker.subscriber_count(), 0);
}

#[test]
fn test_subscribe_push_worker_shutdown_is_noop() {
    // stage 2：shutdown 是 no-op（tokio runtime shutdown 自动 cancel push task）
    let mut worker = SubscribePushWorker::new();
    worker.shutdown();
    assert_eq!(worker.subscriber_count(), 0);
}

#[test]
fn test_subscribe_push_worker_unsubscribe_on_empty_registry_is_idempotent() {
    // stage 2：unsubscribe 不存在的 uri 不报错（idempotent）
    let worker = SubscribePushWorker::new();
    worker.unsubscribe("proc://metrics/system").unwrap();
    worker.unsubscribe("proc://not/subscribed").unwrap();
    assert_eq!(worker.subscriber_count(), 0);
}

#[test]
fn test_subscribe_push_worker_spawn_push_task_returns_err_without_tokio_runtime() {
    // stage 2：spawn_push_task 在无 tokio runtime 上下文返 Err
    // （测试线程默认不在 tokio runtime，TokioHandle::try_current 返 Err）
    let worker = SubscribePushWorker::new();
    let result = worker.spawn_push_task();
    assert!(
        result.is_err(),
        "spawn_push_task 在无 tokio runtime 上下文应返 Err"
    );
}

#[test]
fn test_subscribe_push_worker_source_uses_peer_role_server() {
    // v0.19 stage 1 Spike：注册表 value 类型从 Peer<RoleServer> 升级为
    // Vec<Peer<RoleServer>>（适配 SSE multi-client 场景，与 brainstorm §决策 3 +
    // ADR-0027 §6.3 multi-client 注册表对齐）。stage 2 加 Arc::ptr_eq identity
    // 检测 + JoinSet 并发 push + fail peer 一次精确 retain 清理。
    let source = std::fs::read_to_string("src/mcp/subscribe_worker.rs").expect("source readable");
    assert!(
        source.contains("subscribers: Arc<Mutex<HashMap<String, Vec<Peer<RoleServer>>>>>"),
        "v0.19 stage 1 Spike 注册表 value 应升级为 Vec<Peer<RoleServer>>"
    );
    assert!(
        source.contains("peer.notify_resource_updated"),
        "stage 2 push task 应调 peer.notify_resource_updated"
    );
    assert!(
        source.contains("TokioHandle::try_current"),
        "stage 2 spawn_push_task 应检查 tokio runtime 上下文"
    );
}

#[test]
fn test_resource_route_impl_uses_worker() {
    // stage 2：静态断言 ProcMcpHandler impl ResourceRoute::subscribe/unsubscribe
    // 调 self.subscribe_push_worker（不是 trait 默认实现）
    let source = std::fs::read_to_string("src/mcp/resources.rs").expect("resources.rs readable");
    assert!(
        source.contains("self.subscribe_push_worker.subscribe(uri, peer)"),
        "ProcMcpHandler impl subscribe 应调 worker.subscribe"
    );
    assert!(
        source.contains("self.subscribe_push_worker.unsubscribe(uri)"),
        "ProcMcpHandler impl unsubscribe 应调 worker.unsubscribe"
    );
}

#[test]
fn test_server_handler_subscribe_uses_resource_route() {
    // stage 2：静态断言 ServerHandler::subscribe impl 调 ResourceRoute::subscribe
    // （从 context.peer 拿 Peer 句柄 → 调 ResourceRoute::subscribe(self, &uri, peer)）
    let source = std::fs::read_to_string("src/mcp/handler/mod.rs").expect("mod.rs readable");
    assert!(
        source.contains("let peer = context.peer.clone();"),
        "ServerHandler::subscribe 应从 context.peer 拿 Peer 句柄"
    );
    assert!(
        source.contains("crate::mcp::resources::ResourceRoute::subscribe(self, &uri, peer)"),
        "ServerHandler::subscribe 应调 ResourceRoute::subscribe"
    );
    assert!(
        source.contains("crate::mcp::resources::ResourceRoute::unsubscribe(self, &uri)"),
        "ServerHandler::unsubscribe 应调 ResourceRoute::unsubscribe"
    );
}

#[test]
fn test_proc_handler_has_subscribe_push_worker_field() {
    // stage 2：静态断言 ProcMcpHandler struct 含 subscribe_push_worker 字段
    let source = std::fs::read_to_string("src/mcp/handler/mod.rs").expect("mod.rs readable");
    assert!(
        source.contains("pub subscribe_push_worker: SubscribePushWorker"),
        "ProcMcpHandler 应含 subscribe_push_worker 字段"
    );
}

#[test]
fn test_proc_resource_uris_still_lists_three_uris() {
    // v0.17 stage 4 落地 3 个 URI（stage 2 不动 URI 列表）
    assert_eq!(
        PROC_RESOURCE_URIS.len(),
        3,
        "v0.18 cycle 不新增 URI，仍 3 个"
    );
    assert!(PROC_RESOURCE_URIS.contains(&"proc://metrics/system"));
    assert!(PROC_RESOURCE_URIS.contains(&"proc://processes/list"));
    assert!(PROC_RESOURCE_URIS.contains(&"proc://docker/events"));
}

// ===========================================================================
// 项 3 stage 2 lifecycle 集成测试（manual：mcp-inspector）
// ===========================================================================
//
// stage 2 单元测试无法验证真实 lifecycle（Peer::new 是 pub(crate) 无法在 proc
// 测试中构造）。manual 验证步骤：
// 1. npx mcp-inspector proc mcp serve
// 2. 调 resources/subscribe { uri: "proc://metrics/system" }
// 3. 验证 1s tick 收到 notifications/resources/updated { uri }
// 4. 调 resources/unsubscribe { uri }
// 5. 验证不再收到 push
// 6. 重启 server + subscribe + 关闭 client（断开）→ 验证 push task 自动清理
// TODO v0.18-stage-2：subscribe → 1s tick push → unsubscribe 完整 lifecycle 集成测试
// 1. spawn SubscribePushWorker + tokio runtime
// 2. subscribe 3 个 subscriber_id（不同 URI）
// 3. wait 1.5s 让 push task 至少推一次
// 4. 验证每个 subscriber 至少收到 1 次 notify_resource_updated
// 5. unsubscribe 1 个 → 验证 subscriber_count 减 1
// 6. 模拟 client 断开（drop Peer）→ 验证 push task 自动清理

// ===========================================================================
// 项 4：record auto-stop（stage 1 Spike stub，stage 2 实装真实 auto-stop 测试）
// ===========================================================================

#[test]
fn test_record_start_confirm_false_returns_err() {
    // v0.18 stage 2：confirm=false 路径返 ok:false error（auto-stop 已实装不需 warning）
    use proc::mcp::handler::record::make_record_start_json;
    use std::sync::{Arc, Mutex};

    let record_handle: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));

    let out = make_record_start_json(false, "/tmp/dummy.prec", None, &record_handle);
    let obj = out.as_object().expect("object");
    assert_eq!(obj.get("ok").and_then(|v| v.as_bool()), Some(false));
    // stage 2 移除 warning 字段（auto-stop 已实装）
    assert!(
        obj.get("warning").is_none(),
        "stage 2 应移除 warning 字段（auto-stop 已实装）"
    );
}

#[test]
fn test_record_start_source_no_warning_field() {
    // v0.18 stage 2：静态断言 make_record_start_json 已移除 warning 字段
    // （auto-stop 实装后不再透出 "duration_secs 参数已记录但 auto-stop 当前未实装"）
    let source =
        std::fs::read_to_string("src/mcp/handler/record.rs").expect("record.rs source readable");
    assert!(
        !source.contains("\"warning\": if duration_secs.is_some()"),
        "stage 2 应移除 warning 字段（auto-stop 已实装）"
    );
    assert!(
        source.contains(".arg(\"--duration\")"),
        "stage 2 应在 spawn cmd 加 --duration flag"
    );
}

#[test]
fn test_shutdown_request_flips_requested_flag() {
    // v0.18 stage 2 项 4：shutdown::request() 能 flip flag 让 requested() 返 true
    // （auto-stop timer thread 调本函数触发干净退出）
    //
    // 注意：本测试用独立进程跑（cargo test 单独 process），init() 注册的 ctrlc
    // handler 不影响 test runner。request() 后 requested() 应返 true。
    proc::shutdown::init();
    // 清掉可能的前置状态（如其他测试已 request）
    // 注意：FLAG 是 OnceLock<Arc<AtomicBool>>，无法 reset 到 false。本测试只验证
    // request() 后 requested() 返 true（如已 true 则仍 true，符合预期）。
    proc::shutdown::request();
    assert!(
        proc::shutdown::requested(),
        "shutdown::request() 后 requested() 应返 true"
    );
}

#[test]
fn test_run_record_headless_source_has_timer_thread() {
    // v0.18 stage 2 项 4：静态断言 run_record_headless 实装了 timer thread
    // （spawn std::thread::spawn + sleep + shutdown::request）
    let source = std::fs::read_to_string("src/cli/record.rs").expect("record.rs source readable");
    assert!(
        source.contains("if let Some(secs) = duration"),
        "run_record_headless 应有 duration 参数分支"
    );
    assert!(
        source.contains("std::thread::spawn(move ||"),
        "run_record_headless 应 spawn timer thread"
    );
    assert!(
        source.contains("crate::shutdown::request()"),
        "timer thread 应调 shutdown::request()"
    );
    assert!(
        !source.contains("_duration: Option<u64>"),
        "stage 2 应去掉 _duration 下划线前缀（参数已实装）"
    );
}

// ===========================================================================
// 项 2：varint 等价性（stage 1 Spike stub，stage 2 实装真实 varint 测试）
// ===========================================================================

#[test]
fn test_serialize_with_version_v4_uses_varint() {
    // v0.18 stage 2：version >= 4 走 varint，与 bincode::serialize 默认 fixint
    // 字节流不同（同 RecordingHeader round-trip 字段一致但字节流不同）
    use proc::record::encoding::{deserialize_with_version, serialize_with_version};
    use proc::record::frame::{RECORDING_MAGIC, RECORDING_VERSION, RecordingHeader};

    let header = RecordingHeader {
        magic: *RECORDING_MAGIC,
        version: RECORDING_VERSION, // = 4（v0.18 stage 2 bump）
        start_time: 1_700_000_000,
        hostname: "v4-varint-test".to_string(),
    };
    let bytes_via_helper =
        serialize_with_version(RECORDING_VERSION, &header).expect("serialize via helper");
    let bytes_default_fixint = bincode::serialize(&header).expect("default serialize");
    assert_ne!(
        bytes_via_helper, bytes_default_fixint,
        "v4 varint 字节流应与 fixint 默认不同"
    );

    // round-trip 字段一致
    let back: RecordingHeader =
        deserialize_with_version(RECORDING_VERSION, &bytes_via_helper).expect("deserialize");
    assert_eq!(back.version, header.version);
    assert_eq!(back.hostname, header.hostname);
}

#[test]
fn test_serialize_with_version_v3_fixint_compat() {
    // v0.18 stage 2：v3 旧文件仍走 fixint 兼容层（与 bincode::serialize 默认字节级等价）
    use proc::record::encoding::serialize_with_version;
    use proc::record::frame::{RECORDING_MAGIC, RecordingHeader};

    let header = RecordingHeader {
        magic: *RECORDING_MAGIC,
        version: 3, // v3 旧文件
        start_time: 1_700_000_000,
        hostname: "v3-fixint-compat".to_string(),
    };
    let bytes_via_helper = serialize_with_version(3, &header).expect("serialize via helper");
    let bytes_default = bincode::serialize(&header).expect("default serialize");
    assert_eq!(
        bytes_via_helper, bytes_default,
        "v3 旧文件应走 fixint 兼容层（字节级等价）"
    );
}

// ===========================================================================
// 项 1：P1-R1 + P2-R1 代码清理（stage 2 Slice 实装，本文件不增测试）
// ===========================================================================
//
// v0.17 cycle stage 6 已落地 `test_record_start_stop_round_trip` /
// `test_record_stop_no_active` 等测试覆盖 record 路径。stage 2 代码清理
// （P1-R1 删冗余 / P2-R1 简化 try_clone）只动既有代码不增删行为，既有测试
// 完全覆盖。本文件不增项 1 测试。
