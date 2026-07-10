//! v0.18 cycle 集成测试 — 4 项残留补全 stub 骨架。
//!
//! 与 `test_mcp_v0_15.rs` / `test_mcp_v0_16.rs` / `test_mcp_v0_17.rs` 同款策略：
//! 直接调 helper + handler 字段断言，验证 stage 1 Spike 改动行为正确。stdio 端到端
//! 测试留 manual：`npx mcp-inspector proc mcp serve`。
//!
//! ## stage 1 Spike 落地范围
//!
//! 仅含 4 项 stub 测试骨架（subscribe / unsubscribe / auto-stop / varint 等价性
//! placeholder）。stage 2 Slice 实装业务逻辑后，本文件扩真实集成测试：
//!
//! - **项 3 subscribe-push**：subscribe → 验证 subscriber_count 增 / unsubscribe →
//!   减 / client 断开自动清理
//! - **项 4 auto-stop**：spawn 子进程 `proc record --no-tui --duration 2` → 2s 后
//!   子进程自动退出 + `.prec` 文件生成
//! - **项 2 varint 等价性**：v4 varint 写入 + 读取 + 与 v3 fixint 输出对比（同
//!   RecordingHeader round-trip 字段一致但字节流不同）
//! - **项 1 代码清理**：既有 `test_record_start_stop_round_trip` / `test_record_stop_no_active`
//!   覆盖（不需新增测试）

use proc::mcp::PROC_RESOURCE_URIS;
use proc::mcp::handler::ProcMcpHandler;
use proc::mcp::resources::ResourceRoute;
use proc::mcp::subscribe_worker::SubscribePushWorker;

// ===========================================================================
// 项 3：subscribe-push worker lifecycle（stage 1 Spike stub）
// ===========================================================================

#[test]
fn test_subscribe_push_worker_default_empty() {
    // stage 1 Spike：新 worker 注册表为空
    let worker = SubscribePushWorker::new();
    assert_eq!(worker.subscriber_count(), 0);
}

#[test]
fn test_subscribe_push_worker_subscribe_stub_returns_err() {
    // stage 1 Spike：subscribe 方法 stub 返 Err "v0.18-stage-2 未实装"
    // stage 2 Slice 替换为真实业务逻辑后，本测试改为验证 subscriber_count 增 1
    let worker = SubscribePushWorker::new();
    let result = worker.subscribe("proc://metrics/system", 1);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("v0.18-stage-2"),
        "stage 1 Spike stub error message"
    );
}

#[test]
fn test_subscribe_push_worker_unsubscribe_stub_returns_err() {
    // stage 1 Spike：unsubscribe 方法 stub 返 Err "v0.18-stage-2 未实装"
    let worker = SubscribePushWorker::new();
    let result = worker.unsubscribe(1);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("v0.18-stage-2"),
        "stage 1 Spike stub error message"
    );
}

#[test]
fn test_subscribe_push_worker_spawn_push_task_stub_returns_err() {
    // stage 1 Spike：spawn_push_task stub 返 Err "v0.18-stage-2 未实装"
    let worker = SubscribePushWorker::new();
    let result = worker.spawn_push_task();
    assert!(result.is_err());
}

#[test]
fn test_subscribe_push_worker_shutdown_is_noop() {
    // stage 1 Spike：shutdown 是 no-op，不应 panic
    let mut worker = SubscribePushWorker::new();
    worker.shutdown();
    assert_eq!(worker.subscriber_count(), 0);
}

#[test]
fn test_resource_route_subscribe_stub_returns_err() {
    // stage 1 Spike：ResourceRoute::subscribe trait 方法 stub 返 Err
    // stage 2 Slice 替换为 ProcMcpHandler impl subscribe 真实业务逻辑后，本测试
    // 改为验证 subscribe 后 subscriber_count 增 1
    let handler = ProcMcpHandler::default();
    let result = handler.subscribe("proc://metrics/system", 1);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("v0.18-stage-2"),
        "ResourceRoute trait stub error message"
    );
}

#[test]
fn test_resource_route_unsubscribe_stub_returns_err() {
    // stage 1 Spike：ResourceRoute::unsubscribe trait 方法 stub 返 Err
    let handler = ProcMcpHandler::default();
    let result = handler.unsubscribe(1);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("v0.18-stage-2"),
        "ResourceRoute trait stub error message"
    );
}

#[test]
fn test_proc_resource_uris_still_lists_three_uris() {
    // v0.17 stage 4 落地 3 个 URI（stage 1 Spike 不动 URI 列表，仅加 trait 方法）
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
// 项 3 stage 2 placeholder：lifecycle 集成测试（stage 1 Spike 占位）
// ===========================================================================
//
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
fn test_record_start_duration_secs_param_echoed() {
    // v0.17 stage 6 已落地：duration_secs 参数原样回显到 expected_duration_secs 字段
    // v0.18 stage 1 Spike：保持现状（warning 字段仍透出，stage 2 移除 warning）
    // v0.18 stage 2 实装后：本测试改为验证 spawn 时传 --duration flag + 子进程
    // N secs 后自动退出
    use proc::mcp::handler::record::make_record_start_json;
    use std::sync::{Arc, Mutex};

    let record_handle: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));

    // confirm=false → 返 ok:false error，验证参数 stub 路径正常
    let out = make_record_start_json(false, "/tmp/dummy.prec", None, &record_handle);
    let obj = out.as_object().expect("object");
    assert_eq!(obj.get("ok").and_then(|v| v.as_bool()), Some(false));
}

// TODO v0.18-stage-2：auto-stop 真实测试
// 1. proc_record_start(confirm=true, file_path=tmp, duration_secs=Some(2))
// 2. 验证 spawn 子进程 + 传 --duration 2 flag
// 3. wait 3s（2s auto-stop + 1s buffer）
// 4. 验证 record_handle 已自动 None（child 退出）
// 5. 验证 .prec 文件生成（size > 0）
// 6. 验证 warning 字段已移除（auto-stop 已实装）

// ===========================================================================
// 项 2：varint 等价性（stage 1 Spike stub，stage 2 实装真实 varint 测试）
// ===========================================================================

#[test]
fn test_options_for_version_v4_stub_still_fixint() {
    // v0.18 stage 1 Spike：version >= 4 分支暂仍走 fixint（与 bincode::serialize
    // 默认字节级等价）。stage 2 切换为 varint 后本测试改为验证 v4 varint 字节流
    // 与 v3 fixint 字节流不同（同 RecordingHeader round-trip 一致但字节流不同）
    use bincode::Options;
    use proc::record::encoding::options_for_version;
    use proc::record::frame::{RECORDING_MAGIC, RecordingHeader};

    let header = RecordingHeader {
        magic: *RECORDING_MAGIC,
        version: 4, // v0.18 stage 1 Spike 占位版本号
        start_time: 1_700_000_000,
        hostname: "v4-stub-test".to_string(),
    };
    let bytes_via_opts = options_for_version(4)
        .serialize(&header)
        .expect("serialize via opts");
    let bytes_default = bincode::serialize(&header).expect("default serialize");
    assert_eq!(
        bytes_via_opts, bytes_default,
        "v4 stage 1 Spike stub 应与 fixint 默认字节级等价（stage 2 切 varint 后会不同）"
    );
}

// TODO v0.18-stage-2：varint 等价性真实测试
// 1. 同 RecordingHeader 用 v4 varint 写入 + v3 fixint 写入
// 2. 验证两者 round-trip deserialize 后字段一致
// 3. 验证字节流不同（v4 varint 占 byte 少）
// 4. v3 旧文件用 v3 fixint reader 正常打开（兼容层验证）
// 5. v4 新文件用 v4 varint reader 正常打开

// ===========================================================================
// 项 1：P1-R1 + P2-R1 代码清理（stage 2 Slice 实装，本文件不增测试）
// ===========================================================================
//
// v0.17 cycle stage 6 已落地 `test_record_start_stop_round_trip` /
// `test_record_stop_no_active` 等测试覆盖 record 路径。stage 2 代码清理
// （P1-R1 删冗余 / P2-R1 简化 try_clone）只动既有代码不增删行为，既有测试
// 完全覆盖。本 stage 1 Spike 文件不增项 1 测试。
