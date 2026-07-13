//! `proc mcp serve` — v0.7.0 阶段 2 入口。
//!
//! 把 proc 的 17+ CLI 子命令暴露为 MCP tools（stdio / SSE transport），供
//! Claude Desktop / Cursor / Windsurf 等 LLM agent 调用。
//! 详见 `docs/adr/0009-mcp-server.md` + `docs/adr/0027-rmcp-resource-subscribe-sse-transport.md`。

use colored::Colorize;

use crate::cli::def::McpSub;
use crate::mcp::transport::{SseTransportConfig, TransportKind};

/// `proc mcp <sub>` dispatch。
///
/// **v0.19 stage 2 实装**：从 CLI flags 构造 `TransportKind` 传给
/// `crate::mcp::run_mcp_serve(kind)`：
/// - `--transport stdio`（默认）→ `TransportKind::Stdio`（current_thread runtime）
/// - `--transport sse --port 8080 --bind-addr 127.0.0.1` → `TransportKind::Sse(config)`
///   （multi_thread worker_threads(4) runtime + axum + tower StreamableHttpService）
///
/// 未知 transport 字符串 → red 错误 + exit 1。
pub fn run_mcp(sub: &McpSub) {
    match sub {
        McpSub::Serve {
            transport,
            bind_addr,
            port,
        } => {
            let kind = build_transport_kind(transport, bind_addr, *port);
            match kind {
                Ok(k) => match crate::mcp::run_mcp_serve(k) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("{} {}", "MCP server 错误:".red(), e);
                        std::process::exit(1);
                    }
                },
                Err(msg) => {
                    eprintln!("{} {}", "MCP transport 解析错误:".red(), msg);
                    std::process::exit(1);
                }
            }
        }
    }
}

/// 把 CLI flags 转成 `TransportKind`。
///
/// 与 ADR-0027 §6.4 bind-addr 安全默认对齐——SSE transport 的 `bind_addr` /
/// `port` 从 CLI flags 直接覆盖 `SseTransportConfig::default()`，让用户显式
/// 控制暴露面（默认 `127.0.0.1` 仅本机 / `0.0.0.0` 全网卡需显式 opt-in）。
///
/// # Errors
///
/// 未知 transport 字符串（非 `stdio` / `sse`）→ 返 Err 含合法值清单。
fn build_transport_kind(
    transport: &str,
    bind_addr: &str,
    port: u16,
) -> Result<TransportKind, String> {
    match transport {
        "stdio" => Ok(TransportKind::Stdio),
        "sse" => Ok(TransportKind::Sse(SseTransportConfig {
            port,
            bind_addr: bind_addr.to_string(),
        })),
        other => Err(format!(
            "未知 transport '{other}'，合法值: stdio, sse（详见 ADR-0027 §6 SSE transport lifecycle）"
        )),
    }
}
