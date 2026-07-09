//! v0.17 主题 B 可观测性 — SSE transport 容器。
//!
//! v0.17 阶段 1 Spike 落地：仅含结构 + 函数 stub。
//! 阶段 4 Slice 实装 SSE transport 入口（`proc mcp serve --transport sse --port 8080`）。
//!
//! 与 rmcp 0.11 stdio transport 并行——stdio 是默认 transport（适合 Claude
//! Desktop / Cursor 等单 client 集成），SSE 是长连接场景替代（适合多 client
//! 监控 / Web dashboard / 远程 agent 集成）。详见 ADR-0027。

use serde_json::Value;

/// SSE transport 配置（stage 4 实装时填字段）。
///
/// stage 1 Spike 仅声明 struct（字段全 stub）。stage 4 实装时加：
/// - `tokio::net::TcpListener` bind 到 `bind_addr:port`
/// - `axum` 或 rmcp 0.11 内置 SSE server 路由 MCP JSON-RPC over SSE
/// - 每个 client 连接 spawn 一个 handler task，共享同一 `ProcMcpHandler`
#[derive(Debug, Clone)]
pub struct SseTransportConfig {
    /// 监听端口（如 8080）。
    pub port: u16,
    /// 绑定地址（默认 "0.0.0.0" 全网卡；"127.0.0.1" 仅本机）。
    pub bind_addr: String,
}

impl SseTransportConfig {
    /// 创建新配置（stage 4 实装时填默认值）。
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            port,
            bind_addr: "0.0.0.0".to_string(),
        }
    }
}

impl Default for SseTransportConfig {
    fn default() -> Self {
        Self::new(8080)
    }
}

/// 启动 SSE transport MCP server（v0.17 stage 4 结构化 stub；v0.18+ cycle 全实装）。
///
/// **v0.17 stage 4 决策 4**：SSE transport 落地结构化 stub，full 实装推迟
/// v0.18+ cycle。理由：
///
/// 1. **rmcp 0.11 streamable_http_server 模块需 Cargo feature**
///    `transport-streamable-http-server-tower` + 依赖 `axum` / `tower-service` /
///    `http` / `futures`（context7 rmcp 官方文档 2026-07-08 验证）
/// 2. **runtime 重构**：当前 `run_mcp_serve` 用 `tokio::runtime::Builder::new_current_thread()`
///    （避免与 DockerMonitor 内部 block_on 抢线程）；SSE server 多 client 并发需
///    `new_multi_thread()` runtime，与 DockerMonitor 协调需重新评估
/// 3. **subscribe-push 机制**：SSE 真正的价值是 server 主动推送（如 1s tick push
///    system metrics），需 handler 持 `Peer<RoleServer>` 句柄 + worker 调
///    `peer.notify(ResourceUpdated)` —— 与 fire-and-forget snapshot worker 模式不同，
///    需 lifecycle 管理（client 断开后 worker 不能继续 push）
///
/// **Workaround**：用 stdio transport（默认）+ client-side polling via `resources/read`
/// 或 `proc_metrics_history` tool。stage 4 已落地 ResourceRoute 路由（3 URI）+
/// `proc_metrics_history` tool（sparkline 30s 历史），agent 通过 stdio + polling
/// 即可拿到实时数据，无需 SSE。
///
/// **v0.18+ cycle 候选路径**：
/// - 加 Cargo feature `transport-streamable-http-server-tower` + axum 等 deps
/// - 重构 `run_mcp_serve` 让 SSE 路径走 multi_thread runtime
/// - 设计 subscribe-push worker lifecycle（client 连接 → spawn push task →
///   client 断开 → cancel task）
/// - CLI 入口 `proc mcp serve --transport sse --port 8080` clap struct 扩字段
///
/// 与 [`super::handler::serve`]（stdio transport）并行——stdio 是默认 transport，
/// SSE 是 v0.18+ cycle 加 `--transport sse --port 8080` CLI 入口替代。
pub fn serve_sse(config: &SseTransportConfig) -> Result<Value, String> {
    Err(format!(
        "v0.17 stage 4: SSE transport is a structured stub. Full implementation deferred to v0.18+ cycle. \
         Reasons: (1) rmcp 0.11 streamable_http_server module needs Cargo feature \
         'transport-streamable-http-server-tower' + axum/tower/http/futures deps; \
         (2) current run_mcp_serve uses tokio current_thread runtime (avoid conflict with \
         DockerMonitor block_on); SSE multi-client needs multi_thread runtime rework; \
         (3) server-push (notifications/resources/updated) requires Peer<RoleServer> handle \
         + worker lifecycle management (client disconnect → worker stop). \
         Workaround: use stdio transport (default) + client-side polling via resources/read \
         or proc_metrics_history tool. Config received: port={}, bind_addr='{}'.",
        config.port, config.bind_addr
    ))
}
