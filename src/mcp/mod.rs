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
pub use transport::{SseTransportConfig, serve_sse};

use std::io::{self, IsTerminal};

/// `proc mcp serve` 真正入口。
///
/// 启动独立 tokio multi-thread runtime（proc 主线程同步，这里 block_on），
/// 在 stdio 上跑 MCP server，直到 client 关闭流或本地 Ctrl+C。
pub fn run_mcp_serve() -> anyhow::Result<()> {
    // 安抚读者：MCP server 走 stdio，对 TTY 没意义。如果用户在交互终端直接跑
    // `proc mcp serve`，提示一句让输出可读，但不阻止（脚本里 stdout 重定向时不见得是 TTY）。
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        eprintln!(
            "提示：MCP server 走 stdio 协议，通常不直接在终端运行。\n\
             接入 Claude Desktop / Cursor 见 docs/adr/0009-mcp-server.md。\n\
             手动调试用 `npx mcp-inspector proc mcp serve`。\n"
        );
    }

    // 1 个 thread 即够（MCP server 是 IO-bound，worker 跑采集时是同步阻塞调用，
    // 多 thread 收益有限；保持单 thread 让错误诊断更简单）。
    // 但 proc 的 DockerMonitor 内部会自己 block_on 自建 runtime —— MCP runtime
    // 用 current_thread 避免和 docker runtime 抢线程，docker 调用仍走它自己的 rt。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async { handler::serve().await })
}
