//! `proc mcp serve` — v0.7.0 阶段 2 入口。
//!
//! 把 proc 的 17+ CLI 子命令暴露为 MCP tools（stdio transport），供
//! Claude Desktop / Cursor / Windsurf 等 LLM agent 调用。
//! 详见 `docs/adr/0009-mcp-server.md`。

use colored::Colorize;

use crate::cli::def::McpSub;

/// `proc mcp <sub>` dispatch。
///
/// **v0.19 stage 1 Spike**：`McpSub::Serve` 加 transport / bind_addr / port 三个字段。
/// 当前 dispatch 仍调 `crate::mcp::run_mcp_serve()`（无参，stage 2 重构为
/// `run_mcp_serve(kind: TransportKind)` 后本 dispatch 用 `TransportKind::from_cli_str`
/// 把 String 转 TransportKind）。stage 1 Spike 仅解析 flag 让 mcp-inspector 看到 schema，
/// 不真正实装 SSE transport 路径（serve_sse 返 stage 1 Spike stub 错误）。
pub fn run_mcp(sub: &McpSub) {
    match sub {
        McpSub::Serve {
            transport: _,
            bind_addr: _,
            port: _,
        } => match crate::mcp::run_mcp_serve() {
            Ok(()) => {}
            Err(e) => {
                eprintln!("{} {}", "MCP server 错误:".red(), e);
                std::process::exit(1);
            }
        },
    }
}
