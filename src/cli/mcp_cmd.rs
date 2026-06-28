//! `proc mcp serve` — v0.7.0 阶段 2 入口。
//!
//! 把 proc 的 17+ CLI 子命令暴露为 MCP tools（stdio transport），供
//! Claude Desktop / Cursor / Windsurf 等 LLM agent 调用。
//! 详见 `docs/adr/0009-mcp-server.md`。

use colored::Colorize;

use crate::cli::def::McpSub;

/// `proc mcp <sub>` dispatch。
pub fn run_mcp(sub: &McpSub) {
    match sub {
        McpSub::Serve => match crate::mcp::run_mcp_serve() {
            Ok(()) => {}
            Err(e) => {
                eprintln!("{} {}", "MCP server 错误:".red(), e);
                std::process::exit(1);
            }
        },
    }
}
