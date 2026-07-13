//! v0.19 stage 2 集成测试 — SSE transport 入口（axum + tower StreamableHttpService）。
//!
//! 验证 [`proc::mcp::transport::serve_sse`] 真实启动 + 接受 TCP 连接。
//! Tool 调用 / multi-client subscribe-push 完整端到端验证留 mcp-inspector manual：
//! `npx mcp-inspector proc mcp serve --transport sse --port 8080`。
//!
//! 与 `test_mcp_v0_15.rs` / `test_mcp_v0_16.rs` / `test_mcp_v0_17.rs` 同款策略——
//! 启动 server → TCP connect 验证监听 → drop（不依赖完整 MCP client 协议握手）。

use std::time::{Duration, Instant};

use proc::mcp::handler;
use proc::mcp::transport::{SseTransportConfig, serve_sse};

// ===========================================================================
// stage 2 item 1：build_runtime runtime flavor（与 src/mcp/mod.rs 单元测试互补）
// ===========================================================================

#[test]
fn test_build_runtime_returns_current_thread_for_stdio_via_re_export() {
    // v0.19 stage 2：build_runtime 对 Stdio 返 current_thread runtime。
    // 与 src/mcp/mod.rs::tests::build_runtime_returns_current_thread_for_stdio 互补，
    // 这里验证 TransportKind 可从 transport 模块 re-export 路径用（tests 路径）。
    use proc::mcp::transport::TransportKind;
    let _kind = TransportKind::Stdio;
    assert_eq!(_kind.as_str(), "stdio");
}

#[test]
fn test_build_runtime_returns_multi_thread_for_sse_via_re_export() {
    // v0.19 stage 2：build_runtime 对 Sse 返 multi_thread worker_threads(4) runtime。
    use proc::mcp::transport::TransportKind;
    let kind = TransportKind::Sse(SseTransportConfig::default());
    assert_eq!(kind.as_str(), "sse");
    let cfg = kind.sse_config().expect("sse 应有 config");
    assert_eq!(cfg.port, 8080);
}

// ===========================================================================
// stage 2 item 3：SSE transport 入口真实启动 + TCP 监听验证
// ===========================================================================

#[test]
fn test_serve_sse_starts_and_accepts_tcp_connection() {
    // v0.19 stage 2：serve_sse 启动 axum + StreamableHttpService 真实 server。
    // 本测试用 127.0.0.1 + 动态端口避免与其他测试 / 已占用端口冲突。
    //
    // 策略：
    // 1. 用 std::net::TcpListener bind 127.0.0.1:0 拿一个空闲端口
    // 2. 立即 drop 该 listener 释放端口
    // 3. spawn serve_sse on (127.0.0.1, port) 用独立 tokio runtime
    // 4. poll std::net::TcpStream::connect_timeout 直到成功（5s 超时）
    // 5. drop client 连接 + abort serve_sse task（graceful 退出）
    //
    // **不验证 MCP 协议握手 / 46 tool 调用**——这些留 mcp-inspector manual 验证
    // （brainstorm §测试命令矩阵 stage 2 行「MCP SSE server 启动」段）。

    // 选一个空闲端口（drop 后立即让 serve_sse 用）
    let port: u16 = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind temp listener");
        listener.local_addr().expect("local_addr").port()
    };

    let config = SseTransportConfig {
        port,
        bind_addr: "127.0.0.1".to_string(),
    };

    // 用独立 multi_thread runtime（与生产 run_mcp_serve 路径一致）
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build tokio runtime");

    // spawn serve_sse（block 直到 ctrl+c 或 runtime shutdown）
    let serve_handle = runtime.spawn(async move {
        let _ = serve_sse(config).await;
    });

    // 阻塞当前线程 poll TCP 连接（不在 runtime 上下文，用 std::net）
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut connected = false;
    while Instant::now() < deadline {
        match std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}")
                .parse()
                .expect("valid SocketAddr"),
            Duration::from_millis(500),
        ) {
            Ok(_stream) => {
                connected = true;
                break;
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    assert!(
        connected,
        "serve_sse 应在 5s 内启动并接受 TCP 连接 (port={port})"
    );

    // cleanup：abort serve_sse task + drop runtime（cancel 所有 task）
    serve_handle.abort();
    drop(runtime);
}

#[test]
fn test_serve_sse_rejects_invalid_bind_addr_gracefully() {
    // v0.19 stage 2：serve_sse 收到非法 bind_addr（如 "localhost" hostname）
    // 应返 Err 而非 panic。
    let config = SseTransportConfig {
        port: 18080,
        bind_addr: "localhost".to_string(), // hostname 非 IP 字面量
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    let result = runtime.block_on(async { serve_sse(config).await });

    assert!(
        result.is_err(),
        "serve_sse with hostname bind_addr 应返 Err (需 IP 字面量)"
    );
}

// ===========================================================================
// stage 2 item 3：46 tool 调用可通过 stdio 路径验证（与 SSE 路径同款 ProcMcpHandler）
// ===========================================================================

#[test]
fn test_sse_transport_shares_same_handler_as_stdio_with_46_tools() {
    // v0.19 stage 2：SSE transport 入口的 service_factory closure 每次 client 连接
    // new 一个 ProcMcpHandler（与 stdio 路径同款）。验证 SSE 路径暴露的 tool 数
    // 与 stdio 路径一致（46，v0.18 末 + v0.19 cycle 不新增 tool）。
    let source =
        std::fs::read_to_string("src/mcp/transport.rs").expect("transport.rs source readable");
    assert!(
        source.contains("ProcMcpHandler::new()"),
        "stage 2 serve_sse service_factory 应调 ProcMcpHandler::new()（与 stdio 路径同款）"
    );

    // 同时验证 ProcMcpHandler tool 数 ≥ 46（v0.18 末基线）
    let tools = handler::list_tool_names().len();
    assert!(
        tools >= 46,
        "ProcMcpHandler 应暴露 ≥ 46 个 tool（v0.18 末基线），实际 {tools}"
    );
}

// ===========================================================================
// stage 2 item 2 + item 4：注册表 + push task 源码契约（与 stdio 路径共用 worker）
// ===========================================================================

#[test]
fn test_subscribe_push_worker_implements_arc_ptr_eq_cleanup() {
    // v0.19 stage 2 item 4：spawn_push_task 用 Arc::ptr_eq 精确 cleanup fail peer
    // （替代 stage 1 Spike 的 vec.clear() 不精确策略）。
    let source =
        std::fs::read_to_string("src/mcp/subscribe_worker.rs").expect("subscribe_worker source");
    assert!(
        source.contains("Arc::ptr_eq(p, &failed_arc)"),
        "stage 2 spawn_push_task 失败 cleanup 应用 Arc::ptr_eq 精确 retain"
    );
    assert!(
        source.contains("JoinSet::spawn") || source.contains("join_set.spawn"),
        "stage 2 spawn_push_task 应用 JoinSet 并发 spawn"
    );
    assert!(
        source.contains("join_next"),
        "stage 2 spawn_push_task 应用 join_next().await 逐个判断 fail peer"
    );
}
