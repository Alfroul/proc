//! v0.17 主题 B 可观测性 — SSE transport 容器。
//!
//! v0.17 阶段 1 Spike 落地：仅含结构 + 函数 stub。
//! 阶段 4 Slice 实装 SSE transport 入口（`proc mcp serve --transport sse --port 8080`）。
//!
//! 与 rmcp 0.11 stdio transport 并行——stdio 是默认 transport（适合 Claude
//! Desktop / Cursor 等单 client 集成），SSE 是长连接场景替代（适合多 client
//! 监控 / Web dashboard / 远程 agent 集成）。详见 ADR-0027。
//!
//! **v0.19 cycle stage 1 Spike**：加 `TransportKind` enum（Stdio / Sse）+
//! `build_runtime` helper stub（src/mcp/mod.rs）+ `serve_sse` 改 v0.19 stage 1
//! Spike 新格式 + CLI flag stub（src/cli/def.rs）+ 注册表 Vec<Peer> 升级
//! （src/mcp/subscribe_worker.rs）+ 3 项调研（context7 rmcp 0.11 docs + cargo
//! build/tree 实测 + tokio docs 验证 2026-07-13）。stage 2 实装完整 axum +
//! tower StreamableHttpService 路径 + multi_thread runtime 分支。

use serde_json::Value;

/// SSE transport 配置（stage 4 实装时填字段）。
///
/// stage 1 Spike 仅声明 struct（字段全 stub）。stage 4 实装时加：
/// - `tokio::net::TcpListener` bind 到 `bind_addr:port`
/// - `axum` 或 rmcp 0.11 内置 SSE server 路由 MCP JSON-RPC over SSE
/// - 每个 client 连接 spawn 一个 handler task，共享同一 `ProcMcpHandler`
///
/// **v0.19 stage 1 Spike**：加 `PartialEq` derive 让 `TransportKind::Sse(config)`
/// 可比较（用于 stage 1 单元测试 `transport_kind_clone_partial_eq_works`）。
#[derive(Debug, Clone, PartialEq)]
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

/// MCP transport 类型（v0.19 stage 1 Spike 落地，stage 2 实装 runtime 分支 + 业务）。
///
/// stdio 是默认 transport（v0.7~v0.18 既有路径，单 client 集成适合 Claude Desktop /
/// Cursor）/ Sse 是长连接替代（v0.19 stage 2 实装完整 axum + tower StreamableHttpService
/// 路由，多 client 监控 / Web dashboard / 远程 agent 集成）。
///
/// **stage 1 Spike**：仅声明 enum + Default = Stdio + helper 方法（as_str / sse_config
/// / from_cli_str）。stage 2 实装 `build_runtime(kind) -> Runtime` 按 `TransportKind`
/// 分支选 `current_thread`（Stdio）/ `new_multi_thread().worker_threads(4)`（Sse）
/// runtime，让 stdio 路径零回归（v0.7~v0.18 11 个 cycle 累积路径保持）+ SSE 路径
/// 独立验证。
///
/// 与 ADR-0027 §6.1 runtime 分支选择 + §6.4 bind-addr 安全默认对齐。
///
/// **clap 集成**：stage 2 加 `From<clap::Args>` 实现让 CLI flag `--transport sse`
/// 解析（stage 1 Spike 仅声明 enum，CLI flag 解析逻辑留 stage 2）。
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
    /// stage 2 实装时 main.rs / mcp_cmd.rs dispatch 调本方法把 CLI `transport: String`
    /// 转 `TransportKind`，再传 `run_mcp_serve(kind)`（stage 2 重构签名）。
    ///
    /// **stage 1 Spike 用途**：仅 stage 1 单元测试 + stage 2 dispatch 复用，stage 1
    /// 不接真实 dispatch（dispatch 仍调 `run_mcp_serve()` 无参）。stage 2 重构后
    /// dispatch 改为 `run_mcp_serve(kind)` 才真正使用本方法解析结果。
    ///
    /// **clap 集成备选**：stage 2 评估是否改 clap `ValueEnum` derive（让 clap 自动
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

/// 启动 SSE transport MCP server（v0.19 stage 1 Spike 结构化 stub，stage 2 实装）。
///
/// **v0.19 stage 1 Spike**：本函数是 stub（返 "v0.19 stage 2 未实装" 错误），
/// stage 2 实装完整 axum + tower + rmcp StreamableHttpService 路径：
///
/// 1. 构造 `StreamableHttpService::new(service_factory, session_manager, config)`
///    （service_factory 是 closure 每次连接 new 一个 `ProcMcpHandler`）
/// 2. axum Router 把 `/mcp` POST 路由到 service
/// 3. `tokio::net::TcpListener` bind `bind_addr:port`
/// 4. `axum::serve(listener, app).await` 跑 server until ctrl+c
///
/// 与 [`super::handler::serve`]（stdio transport）并行——stdio 是默认 transport，
/// SSE 是 v0.19 stage 2 加 `--transport sse --port 8080 --bind-addr 127.0.0.1` CLI 入口替代。
///
/// **stage 1 Spike 调研结论**（context7 rmcp 0.11 docs + cargo build 实测，2026-07-13）：
///
/// - Cargo feature：rmcp 0.11 需 `transport-streamable-http-server` +
///   `transport-streamable-http-server-session` 两个 feature（已加到 Cargo.toml）。
///   brainstorm §决策 4 假设的 `transport-streamable-http-server` 单数正确，需补
///   `-session` 后缀启用 session submodule（含 `StreamableHttpService` struct）。
///   **不存在 `-tower` 后缀变体**——context7 docs 描述的「tower submodule」是 module
///   可见性，需 `transport-streamable-http-server-session` + `transport-streamable-http-server`
///   + `server` 三个 feature 组合启用（cargo build error 实测验证）
/// - `StreamableHttpService::new` API 签名（详见 ADR-0027 §6.2）：
///   `pub fn new<S, M>(service_factory: impl Fn() -> Result<S, Error> + Send + Sync + 'static,
///    session_manager: Arc<M>, config: StreamableHttpServerConfig) -> Self`
/// - axum 0.7.9 + tower 0.5.3 + tower-http 0.6.11 与 rmcp 内部对齐无 duplicate deps
///   （cargo tree -d 实测验证，2026-07-13）
/// - tokio runtime 类型检测 API：`Handle::runtime_flavor() -> RuntimeFlavor`
///   （CurrentThread / MultiThread 两个变体，stage 2 build_runtime 测试用此 API）
///
/// **stage 1 Spike 不动业务代码**——仅更新 doc 注释 + 返 stage 1 Spike 标记错误，
/// 让 stage 2 直接填业务逻辑（与 brainstorm §决策 7 项 3 描述 + ADR-0027 §6.2 对齐）。
///
/// # Errors
///
/// 当前 stage 1 Spike 永远返 Err，stage 2 实装后改成真实 server 启动路径
/// （`axum::serve(TcpListener::bind(...).await?, app).await?`）。
pub fn serve_sse(config: &SseTransportConfig) -> Result<Value, String> {
    Err(format!(
        "v0.19 stage 1 Spike: SSE transport is a structured stub. Full implementation deferred to v0.19 stage 2. \
         stage 1 Spike 已完成 3 项调研（context7 rmcp 0.11 docs + cargo build/tree 实测 2026-07-13）—— \
         (1) Cargo feature 验证: rmcp 0.11 需 'transport-streamable-http-server' + \
         'transport-streamable-http-server-session' 两个 feature（已加 Cargo.toml，brainstorm §决策 4 \
         假设的单数正确，需补 -session 启用 session submodule）; \
         (2) StreamableHttpService::new API 签名验证 (service_factory closure + session_manager Arc<M> + \
         StreamableHttpServerConfig builder)，详见 ADR-0027 §6.2; \
         (3) axum 0.7.9 + tower 0.5.3 + tower-http 0.6.11 与 rmcp 内部对齐无 duplicate deps. \
         Stage 2 实装: axum Router /mcp POST + TcpListener bind + axum::serve. \
         Config received: port={}, bind_addr='{}'. \
         See ADR-0027 §6 SSE transport lifecycle + docs/stages/v0.19-stage-2.md.",
        config.port, config.bind_addr
    ))
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
}
