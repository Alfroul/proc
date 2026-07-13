//! v0.17 主题 B 可观测性 — SSE transport 容器。
//!
//! v0.17 阶段 1 Spike 落地结构 + 函数 stub / v0.19 stage 2 实装完整 axum +
//! tower StreamableHttpService 路径（`proc mcp serve --transport sse --port 8080`）。
//!
//! 与 rmcp 0.11 stdio transport 并行——stdio 是默认 transport（适合 Claude
//! Desktop / Cursor 等单 client 集成），SSE 是长连接场景替代（适合多 client
//! 监控 / Web dashboard / 远程 agent 集成）。详见 ADR-0027。
//!
//! **v0.19 cycle stage 1 Spike**：加 `TransportKind` enum（Stdio / Sse）+
//! `build_runtime` helper stub（src/mcp/mod.rs）+ `serve_sse` 改 v0.19 stage 1
//! Spike 新格式 + CLI flag stub（src/cli/def.rs）+ 注册表 Vec<Peer> 升级
//! （src/mcp/subscribe_worker.rs）+ 3 项调研（context7 rmcp 0.11 docs + cargo
//! build/tree 实测 + tokio docs 验证 2026-07-13）。
//!
//! **v0.19 cycle stage 2 实装**：runtime 分支 match（[`TransportKind::Stdio`]
//! → current_thread / [`TransportKind::Sse`] → multi_thread worker_threads(4)）+
//! `serve_sse` 真实 axum + tower StreamableHttpService 路径 + multi-client
//! 注册表 identity 检测（详见 ADR-0027 §6.1 / §6.2 / §6.3）。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;

use crate::mcp::handler::ProcMcpHandler;

/// SSE transport 配置（v0.19 stage 2 实装时填字段）。
///
/// v0.17 stage 1 Spike 仅声明 struct / v0.19 stage 2 实装 serve_sse 真正用到字段：
/// - `tokio::net::TcpListener` bind 到 `bind_addr:port`
/// - `axum::Router` `/mcp` POST 路由到 `StreamableHttpService`
/// - service_factory closure 每次连接 new 一个 `ProcMcpHandler`
///
/// **v0.19 stage 1 Spike**：加 `PartialEq` derive 让 `TransportKind::Sse(config)`
/// 可比较（用于 stage 1 单元测试 `transport_kind_clone_partial_eq_works`）。
#[derive(Debug, Clone, PartialEq)]
pub struct SseTransportConfig {
    /// 监听端口（如 8080）。
    pub port: u16,
    /// 绑定地址（CLI flag `--bind-addr` 默认 `"127.0.0.1"` 仅本机；`"0.0.0.0"` 全网卡需显式 opt-in）。
    pub bind_addr: String,
}

impl SseTransportConfig {
    /// 创建新配置。
    ///
    /// 注意：`SseTransportConfig::default()` 保留 `bind_addr = "0.0.0.0"` 用于 backward
    /// compat 与测试场景；生产路径 CLI flag `--bind-addr` 默认 `"127.0.0.1"`（安全默认，
    /// 详见 ADR-0027 §6.4），dispatch 时从 CLI 传值覆盖。
    #[must_use]
    pub fn new(port: u16) -> Self {
        Self {
            port,
            bind_addr: "0.0.0.0".to_string(),
        }
    }

    /// 解析 `(bind_addr, port)` 为 `SocketAddr`（serve_sse + 测试用）。
    ///
    /// # Errors
    ///
    /// `bind_addr` 不是合法 IP 地址（如 `"localhost"`）→ 返 Err 含错误信息。
    fn socket_addr(&self) -> Result<SocketAddr, String> {
        format!("{}:{}", self.bind_addr, self.port)
            .parse()
            .map_err(|e| format!("invalid bind_addr '{}': {e}", self.bind_addr))
    }
}

impl Default for SseTransportConfig {
    fn default() -> Self {
        Self::new(8080)
    }
}

/// MCP transport 类型（v0.19 stage 1 Spike 落地，stage 2 实装 runtime 分支 + 业务）。
///
/// stdio 是默认 transport（v0.7~v0.18 既有路径，单 client 集成适合 Claude Desktop /
/// Cursor）/ Sse 是长连接替代（v0.19 stage 2 实装完整 axum + tower StreamableHttpService
/// 路由，多 client 监控 / Web dashboard / 远程 agent 集成）。
///
/// 与 ADR-0027 §6.1 runtime 分支选择 + §6.4 bind-addr 安全默认对齐。
#[derive(Debug, Clone, Default, PartialEq)]
pub enum TransportKind {
    /// stdio transport（默认，单 client，current_thread runtime）。
    ///
    /// 与 v0.7~v0.18 既有路径保持一致——单 client IO-bound 场景 current_thread 足够，
    /// 与 v0.7 stage 2 ADR-0009 决策对齐。stage 2 build_runtime 对此分支返
    /// `Builder::new_current_thread().enable_all().build()`。
    #[default]
    Stdio,
    /// SSE transport（v0.19 stage 2 实装，多 client，multi_thread runtime）。
    ///
    /// 与 [`SseTransportConfig`] 配对——stage 2 build_runtime 对此分支返
    /// `Builder::new_multi_thread().worker_threads(4).enable_all().build()`（多 client
    /// 并发 + axum + tower StreamableHttpService 需要）。worker_threads(4) ≥ 一般
    /// client 数，docker 同步 `block_on` 阻塞风险 mitigate 详见 ADR-0027 §6.1。
    Sse(SseTransportConfig),
}

impl TransportKind {
    /// 返 transport 类型字符串标识（用于 logging / debug）。
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Sse(_) => "sse",
        }
    }

    /// 如是 SSE transport 返 `&SseTransportConfig` 引用，否则 None。
    ///
    /// stage 2 `build_runtime` + `serve_sse` 用此方法拿 config 取 `bind_addr` / `port`。
    #[must_use]
    pub fn sse_config(&self) -> Option<&SseTransportConfig> {
        match self {
            Self::Stdio => None,
            Self::Sse(cfg) => Some(cfg),
        }
    }

    /// 从 CLI flag 字符串解析 `TransportKind`（v0.19 stage 1 Spike 落地）。
    ///
    /// 接受 `"stdio"` / `"sse"`（大小写敏感）。其他值返 `Err` 含合法值清单。
    /// stage 2 `mcp_cmd::run_mcp` dispatch 调本方法把 CLI `transport: String`
    /// 转 `TransportKind`，再传 `run_mcp_serve(kind)`。
    ///
    /// **clap 集成备选**：评估是否改 clap `ValueEnum` derive（让 clap 自动
    /// validate + 生成 help）。当前 enum 含 `Sse(SseTransportConfig)` 数据字段，
    /// `ValueEnum` 只支持 unit-only enum，本方法保留作 fallback。
    ///
    /// # Errors
    ///
    /// 未知 transport 字符串 → 返 `Err` 含合法值清单。
    pub fn from_cli_str(s: &str) -> Result<Self, String> {
        match s {
            "stdio" => Ok(Self::Stdio),
            "sse" => Ok(Self::Sse(SseTransportConfig::default())),
            other => Err(format!(
                "未知 transport '{other}'，合法值: stdio, sse（详见 ADR-0027 §6 SSE transport lifecycle）"
            )),
        }
    }
}

/// 启动 SSE transport MCP server（v0.19 stage 2 实装）。
///
/// 完整 axum + tower + rmcp `StreamableHttpService` 路径：
///
/// 1. 构造 `StreamableHttpService::new(service_factory, session_manager, config)`：
///    - `service_factory` closure 每次连接 new 一个 [`ProcMcpHandler`]
///    - `session_manager` 用 rmcp 内置 [`LocalSessionManager`]（session-bound lifecycle
///      留 v0.20+ cycle 评估）
///    - `config` 用默认 [`StreamableHttpServerConfig`]（stateful mode + sse_keep_alive）
/// 2. axum `Router` 把 `/mcp` POST 路由到 service（`tower_http::CorsLayer` 留 v0.20+）
/// 3. `tokio::net::TcpListener` bind `(bind_addr, port)`（默认 `127.0.0.1:8080` 仅本机）
/// 4. `axum::serve(listener, app)` 跑 server until ctrl+c / 进程退出
///
/// 与 [`super::handler::serve`]（stdio transport）并行——stdio 是默认 transport，
/// SSE 是 v0.19 stage 2 加 `--transport sse --port 8080 --bind-addr 127.0.0.1` CLI 入口替代。
///
/// **stage 2 调研结论**（context7 rmcp 0.11 docs + cargo build 实测，2026-07-13）：
///
/// - Cargo feature：rmcp 0.11 需 `transport-streamable-http-server` +
///   `transport-streamable-http-server-session` 两个 feature（已加到 Cargo.toml）。
/// - `StreamableHttpService::new` API 签名：`new(service_factory: impl Fn() -> Result<S, Error>
///   + Send + Sync + 'static, session_manager: Arc<M>, config: StreamableHttpServerConfig) -> Self`
/// - axum 0.7.9 + tower 0.5.3 + tower-http 0.6.11 与 rmcp 内部对齐无 duplicate deps。
///
/// **bind-addr 安全默认**（ADR-0027 §6.4）：默认 `127.0.0.1`（仅本机）/ `0.0.0.0`
/// （全网卡，用户显式 opt-in）。SSE 暴露 proc MCP tool（含 `proc_docker_rm` /
/// `proc_docker_image_rm` / `proc_docker_volume_rm` / `proc_usb_release` /
/// `proc_record_start` 等写操作），全网卡监听需用户显式 opt-in 避免意外暴露到 LAN。
///
/// # Errors
///
/// - `bind_addr` 解析 SocketAddr 失败 → 返 Err 含错误信息
/// - `TcpListener::bind` 失败（如端口已占用 / 权限不足）→ 返 Err 含错误信息
/// - `axum::serve` 异常退出 → 返 Err 含错误信息
pub async fn serve_sse(config: SseTransportConfig) -> anyhow::Result<()> {
    let socket_addr = config.socket_addr().map_err(|e| anyhow::anyhow!(e))?;

    // service_factory: 每次 client 连接 new 一个 ProcMcpHandler（与 brainstorm §决策 7
    // 项 3 描述 + ADR-0027 §6.2 对齐）。ProcMcpHandler::new 会 spawn 持久 DNS collector
    // + snapshot worker，每个 session 独立（不共享状态）。rmcp 要求 closure 返
    // `Result<S, std::io::Error>` — `ProcMcpHandler::new` 不会失败，wrap 成 `Ok(...)`。
    let service_factory = || Ok::<ProcMcpHandler, std::io::Error>(ProcMcpHandler::new());

    // LocalSessionManager: rmcp 内置 impl，stateful mode（每个 client 一个 session）。
    // session-bound subscribe-push worker（如需 session-bound 注册表清理）留 v0.20+ cycle。
    let session_manager = Arc::new(LocalSessionManager::default());

    let http_service = StreamableHttpService::new(
        service_factory,
        session_manager,
        StreamableHttpServerConfig::default(),
    );

    // /mcp POST 路由到 StreamableHttpService。tower_http CorsLayer 留 v0.20+ cycle
    // （brainstorm §决策 6 推迟范围，CORS 仅 Web dashboard 集成时才需要）。
    let app = Router::new().route_service("/mcp", http_service);

    eprintln!(
        "MCP SSE server listening on http://{socket_addr}/mcp (bind_addr={}, port={})",
        config.bind_addr, config.port
    );

    let listener = TcpListener::bind(socket_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_kind_default_is_stdio() {
        // v0.19 stage 1 Spike：Default = Stdio（与 v0.7~v0.18 既有路径一致）
        assert_eq!(TransportKind::default(), TransportKind::Stdio);
    }

    #[test]
    fn transport_kind_as_str_returns_correct_identifier() {
        assert_eq!(TransportKind::Stdio.as_str(), "stdio");
        assert_eq!(
            TransportKind::Sse(SseTransportConfig::default()).as_str(),
            "sse"
        );
    }

    #[test]
    fn transport_kind_sse_config_returns_none_for_stdio() {
        // stdio 无 SseTransportConfig
        assert!(TransportKind::Stdio.sse_config().is_none());
    }

    #[test]
    fn transport_kind_sse_config_returns_some_for_sse() {
        let config = SseTransportConfig::new(9090);
        let kind = TransportKind::Sse(config);
        let returned = kind.sse_config().expect("SSE config 应 Some");
        assert_eq!(returned.port, 9090);
        assert_eq!(returned.bind_addr, "0.0.0.0");
    }

    #[test]
    fn transport_kind_from_cli_str_parses_stdio() {
        // v0.19 stage 1 Spike clap parse：from_cli_str("stdio") → Stdio
        let kind = TransportKind::from_cli_str("stdio").expect("stdio 应解析成功");
        assert_eq!(kind, TransportKind::Stdio);
    }

    #[test]
    fn transport_kind_from_cli_str_parses_sse() {
        // v0.19 stage 1 Spike clap parse：from_cli_str("sse") → Sse(default config)
        let kind = TransportKind::from_cli_str("sse").expect("sse 应解析成功");
        assert_eq!(kind.as_str(), "sse");
        let config = kind.sse_config().expect("sse 应有 config");
        assert_eq!(config.port, 8080); // SseTransportConfig::default() port = 8080
    }

    #[test]
    fn transport_kind_from_cli_str_rejects_unknown_value() {
        // v0.19 stage 1 Spike clap parse：未知值返 Err 含合法值清单
        let result = TransportKind::from_cli_str("websocket");
        assert!(result.is_err(), "未知 transport 应返 Err");
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("websocket"),
            "错误信息应含未知值 '{err_msg}'"
        );
        assert!(
            err_msg.contains("stdio") && err_msg.contains("sse"),
            "错误信息应含合法值清单 '{err_msg}'"
        );
    }

    #[test]
    fn transport_kind_clone_partial_eq_works() {
        // v0.19 stage 1 Spike：Debug + Clone + PartialEq derive 让 enum 可比较 + clone
        let kind1 = TransportKind::Stdio;
        let kind2 = kind1.clone();
        assert_eq!(kind1, kind2);
    }

    #[test]
    fn sse_transport_config_default_is_port_8080_bind_0_0_0_0() {
        // v0.19 stage 1 Spike：SseTransportConfig::default() 保留 v0.17 stage 1 默认值
        // （注意：CLI flag bind_addr 默认 127.0.0.1 安全默认，但 SseTransportConfig::default
        // 保留 0.0.0.0 用于 backward compat，stage 2 实装时 dispatch 从 CLI flag 传值）
        let config = SseTransportConfig::default();
        assert_eq!(config.port, 8080);
        assert_eq!(config.bind_addr, "0.0.0.0");
    }

    #[test]
    fn sse_transport_config_socket_addr_parses_localhost_v4() {
        // v0.19 stage 2：socket_addr 解析 (bind_addr, port) → SocketAddr
        let config = SseTransportConfig {
            port: 8080,
            bind_addr: "127.0.0.1".to_string(),
        };
        let addr = config.socket_addr().expect("合法 IPv4 应解析成功");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 8080);
    }

    #[test]
    fn sse_transport_config_socket_addr_parses_wildcard() {
        // v0.19 stage 2：0.0.0.0 全网卡（用户显式 opt-in）
        let config = SseTransportConfig {
            port: 9090,
            bind_addr: "0.0.0.0".to_string(),
        };
        let addr = config.socket_addr().expect("0.0.0.0 应解析成功");
        assert!(addr.is_ipv4());
        assert_eq!(addr.port(), 9090);
    }

    #[test]
    fn sse_transport_config_socket_addr_rejects_hostname() {
        // v0.19 stage 2：socket_addr 不接受 "localhost" 等主机名（需 IP 字面量）
        let config = SseTransportConfig {
            port: 8080,
            bind_addr: "localhost".to_string(),
        };
        let result = config.socket_addr();
        assert!(
            result.is_err(),
            "hostname 应被拒绝（需 IP 字面量，stage 2 安全保守）"
        );
    }
}
