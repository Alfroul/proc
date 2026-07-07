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

/// 启动 SSE transport MCP server（stage 4 实装）。
///
/// stage 1 Spike 返 "v0.17-stage-4 未实装" 错误。stage 4 实装时：
/// - 解析 `config` 拿 port / bind_addr
/// - 启动 tokio + axum（或 rmcp 0.11 内置 SSE server）
/// - 路由 MCP JSON-RPC over SSE，每个 client 连接 spawn handler task
/// - 阻塞直到 Ctrl+C 或所有 client 断开
///
/// 与 [`super::handler::serve`]（stdio transport）并行——stdio 是默认 transport，
/// SSE 是 `proc mcp serve --transport sse --port 8080` CLI 入口替代。
pub fn serve_sse(_config: &SseTransportConfig) -> Result<Value, String> {
    Err("v0.17-stage-4 未实装".to_string())
}
