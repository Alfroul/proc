//! `proc mcp serve` — v0.7.0 阶段 2 入口。
//!
//! 基于 `rmcp` 官方 Rust SDK，stdio transport，把 proc 的 17+ CLI 子命令
//! 暴露为 MCP tools 供 Claude Code / Cursor 等 LLM agent 调用。
//! 详见 ADR-0009 与 `docs/stages/v0.7-stage-2.md`。
//!
//! **v0.17 cycle 主题 B 可观测性**（stage 1 Spike 落地骨架，stage 4 Slice 填业务）：
//! - [`transport`]：SSE transport 容器（`proc mcp serve --transport sse --port 8080`）
//! - [`resources`]：rmcp 0.11 Resource subscribe 路由（`proc://metrics/system` 等 URI）
//!
//! **v0.18 cycle 项 3 subscribe-push**（stage 1 Spike 落地骨架，stage 2 Slice 填业务）：
//! - [`subscribe_worker`]：subscribe-push worker lifecycle 容器
//!   （stage 1 Spike 仅声明 struct + 注册表 stub；stage 2 实装三步 lifecycle）
//!
//! - 入口：[`run_mcp_serve`]（main.rs 调）
//! - 实现：[`handler::ProcMcpHandler`]（`#[tool_router(server_handler)]` 自动生成 ServerHandler impl）

pub mod handler;
pub mod resources;
pub mod subscribe_worker;
pub mod transport;

pub use resources::{PROC_RESOURCE_URIS, ResourceRoute};
pub use subscribe_worker::{SubscribePushWorker, SubscriberId};
pub use transport::{SseTransportConfig, TransportKind, serve_sse};

use std::io::{self, IsTerminal};

/// `proc mcp serve` 真正入口（v0.19 stage 2 加 `kind` 参数）。
///
/// 按 `TransportKind` 分支选 tokio runtime + transport 入口：
/// - [`TransportKind::Stdio`]：current_thread runtime + `handler::serve()` 跑 stdio
///   MCP server（v0.7~v0.18 既有路径零回归）
/// - [`TransportKind::Sse`]：multi_thread worker_threads(4) runtime + `serve_sse()`
///   跑 axum + tower StreamableHttpService（v0.19 stage 2 实装）
///
/// proc 主线程同步 `block_on`，server 跑到 client 关闭流 / Ctrl+C / 进程退出。
pub fn run_mcp_serve(kind: TransportKind) -> anyhow::Result<()> {
    // 安抚读者：MCP server stdio 走 stdin/stdout，对 TTY 没意义。如果用户在交互终端
    // 直接跑 `proc mcp serve`，提示一句让输出可读，但不阻止（脚本里 stdout 重定向时
    // 不见得是 TTY）。SSE 路径走 HTTP 不占 stdin/stdout，TTY 检查跳过。
    if matches!(kind, TransportKind::Stdio)
        && io::stdin().is_terminal()
        && io::stdout().is_terminal()
    {
        eprintln!(
            "提示：MCP server 走 stdio 协议，通常不直接在终端运行。\n\
             接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。\n\
             手动调试用 `npx mcp-inspector proc mcp serve`。\n"
        );
    }

    let runtime = build_runtime(&kind)?;

    match kind {
        TransportKind::Stdio => runtime.block_on(async { handler::serve().await }),
        // config 被 move 进 serve_sse（serve_sse 持 config 跑 axum::serve 到 ctrl+c）
        TransportKind::Sse(config) => {
            runtime.block_on(async { transport::serve_sse(config).await })
        }
    }
}

/// 按 `TransportKind` 分支选 tokio runtime（v0.19 stage 2 实装）。
///
/// 与 ADR-0027 §6.1 runtime 分支选择对齐——stdio 保 `current_thread`（v0.7~v0.18
/// 既有路径零回归）/ SSE 走 `new_multi_thread().worker_threads(4)`（多 client 并发 +
/// axum + tower StreamableHttpService 需要）。
///
/// **stage 2 runtime flavor 测试**（context7 tokio docs 验证 API 2026-07-13）：
///
/// ```rust,ignore
/// use tokio::runtime::RuntimeFlavor;
/// let runtime = build_runtime(&TransportKind::Stdio).unwrap();
/// assert_eq!(runtime.handle().runtime_flavor(), RuntimeFlavor::CurrentThread);
///
/// let runtime = build_runtime(&TransportKind::Sse(SseTransportConfig::default())).unwrap();
/// assert_eq!(runtime.handle().runtime_flavor(), RuntimeFlavor::MultiThread);
/// ```
///
/// # Errors
///
/// tokio runtime 构造失败（如 `enable_all` IO driver 初始化失败）→ 返 Err 含错误信息。
pub fn build_runtime(kind: &TransportKind) -> anyhow::Result<tokio::runtime::Runtime> {
    match kind {
        // stdio 单 client IO-bound，current_thread 足够（v0.7 stage 2 ADR-0009 决策）。
        // 与 v0.7~v0.18 11 个 cycle 既有 stdio 路径保持一致（零回归风险）。
        // DockerMonitor runtime 独立（Runtime::new() 默认 multi_thread），不抢 MCP runtime 线程。
        TransportKind::Stdio => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(Into::into),
        // SSE 多 client 并发，multi_thread worker pool。
        // axum + tower StreamableHttpService 需 multi_thread 才能并发处理 request。
        // worker_threads(4) ≥ 一般 client 数；docker 同步 `block_on` 阻塞 mitigate 详见
        // ADR-0027 §6.1 DockerMonitor block_on 真实风险评估段。
        TransportKind::Sse(_) => tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .map_err(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::RuntimeFlavor;

    #[test]
    fn build_runtime_returns_current_thread_for_stdio() {
        // v0.19 stage 2：build_runtime 对 Stdio 返 current_thread runtime
        // （与 v0.7~v0.18 既有路径保持一致零回归）。
        // 验证用 Handle::runtime_flavor() （context7 tokio docs 验证 API 2026-07-13）
        let runtime = build_runtime(&TransportKind::Stdio).expect("build_runtime 应成功");
        assert_eq!(
            runtime.handle().runtime_flavor(),
            RuntimeFlavor::CurrentThread,
            "stage 2 Stdio 应返 current_thread runtime（与 v0.7~v0.18 既有路径一致）"
        );
    }

    #[test]
    fn build_runtime_returns_multi_thread_for_sse() {
        // v0.19 stage 2：build_runtime 对 Sse 返 multi_thread worker_threads(4) runtime
        // （axum + tower StreamableHttpService 需要 multi_thread 才能并发处理 request）
        let config = SseTransportConfig::default();
        let runtime = build_runtime(&TransportKind::Sse(config)).expect("build_runtime 应成功");
        assert_eq!(
            runtime.handle().runtime_flavor(),
            RuntimeFlavor::MultiThread,
            "stage 2 Sse 应返 multi_thread runtime（多 client 并发 + axum 需要）"
        );
    }

    #[test]
    fn transport_kind_re_exported_from_mod() {
        // v0.19 stage 1 Spike：TransportKind 应从 mod.rs re-export（让 cli 模块可直接用）
        let kind = TransportKind::default();
        assert_eq!(kind.as_str(), "stdio");
    }
}
