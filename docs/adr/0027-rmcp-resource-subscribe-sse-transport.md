# ADR-0027：rmcp 0.11 Resource subscribe + SSE transport 设计

**Status**：Accepted（**v0.17 stage 4 partial 落地 polling-push；v0.18 stage 2 补全 subscribe-push；v0.19 stage 2 补全 SSE transport full 实装 + multi-client 升级（stage 2 已知限制：subscribe dedup + unsubscribe precise removal 推迟 v0.20+ cycle 用 mcp-session-id）**，详见 §关键设计点 5 + §6 SSE transport lifecycle + Migration path）
**Date**：2026-07-07（v0.17.0 阶段 1 落地决策）/ 2026-07-10 扩 §5 subscribe-push lifecycle（v0.18 stage 1 Spike 决策）/ 2026-07-13 扩 §6 SSE transport lifecycle（v0.19 stage 1 Spike 决策）
**Related**：ADR-0009（v0.7 MCP server 设计）、ADR-0024（v0.15 handler 子 module 拆分）、ADR-0026（MCP handler 持久字段策略）、v0.15 TD-52 归档（sparkline 30s 历史不暴露）

## 背景（Context）

v0.15 cycle TD-52 归档「metrics_system sparkline 30s 历史不暴露」：`proc_metrics_system` tool 返一次性 snapshot（cpu_usage / memory_used / swap_used 等当前值），agent 如需查看历史趋势必须多次调用 tool 累积数据——agent 视角不友好，且每次调用都 spawn SystemSnapshot::new + refresh 累积开销大（TD-54 同款问题）。

v0.17 cycle 主题 B 可观测性需 rmcp 0.11 Resource subscribe 能力——client 订阅资源 URI 后 server 1s tick 推送增量，适合 sparkline / 进程列表实时监控场景（与 brainstorm §主题 B + TD-52 同款方向）。

v0.17 cycle 主题 B 还需 SSE transport 替代 stdio transport——stdio 适合单 client 集成（Claude Desktop / Cursor），SSE 适合多 client 监控 / Web dashboard / 远程 agent 集成（长连接场景）。

## 决策（Decision）

**3 件套落地主题 B 可观测性 schema**：

### 1. Resource subscribe（`proc://` 资源 URI 命名空间）

`ProcMcpHandler` impl `ResourceRoute` trait（`src/mcp/resources.rs`），暴露 3 个资源 URI：

| URI | 推送内容 | 推送频率 |
|---|---|---|
| `proc://metrics/system` | system metrics snapshot（cpu / memory / swap / network / temperature）| 1s tick |
| `proc://processes/list` | process list snapshot（top N by cpu_usage，N 默认 50）| 1s tick |
| `proc://docker/events` | docker daemon events（container start / stop / die 等）| 实时推送 |

client 订阅后 server 1s tick 推送增量（与 brainstorm §主题 B + TD-52 sparkline 同款节奏）。Resource subscribe 与既有 tool 互补——tool 是 request-response（client 主动调）/ Resource 是 subscribe-push（client 订阅后 server 主动推）。

### 2. SSE transport（`proc mcp serve --transport sse --port 8080` CLI 入口）

`src/mcp/transport.rs` 实装 `SseTransportConfig` struct + `serve_sse()` 函数：

```rust
pub struct SseTransportConfig {
    pub port: u16,
    pub bind_addr: String,  // "0.0.0.0" 全网卡 / "127.0.0.1" 仅本机
}

pub fn serve_sse(config: &SseTransportConfig) -> Result<Value, String>;
```

CLI 入口 `proc mcp serve --transport sse --port 8080` 让 SSE transport 替代 stdio transport（默认仍是 stdio）。SSE transport 复用 rmcp 0.11 内置 SSE server（如可用），否则自实装 tokio + axum 路由 MCP JSON-RPC over SSE。

### 3. TD-52 sparkline（`proc_metrics_history` tool + `system_history` 字段）

`ProcMcpHandler` 加 `system_history: Arc<Mutex<VecDeque<SystemSnapshot>>>` 字段（ADR-0026 落地），1s tick push 一次，30s cap（VecDeque 长度上限 30）。`proc_metrics_history` tool drain 此字段返 sparkline 数据点：

```json
{
  "ok": true,
  "metric": "cpu",
  "samples": [
    { "ts": 1720345600, "value": 23.5 },
    { "ts": 1720345601, "value": 24.1 },
    // ... 30 个数据点
  ]
}
```

agent 调 `proc_metrics_history` 一次拿 30s 历史趋势，无需多次调 `proc_metrics_system` 累积。

## 关键设计点

### 1. 资源 URI 命名空间 `proc://`

与 MCP 协议 `xxxx://` 格式对齐（如 `file://` / `http://`）。`proc://` 命名空间让 client 一眼识别 proc server 资源，避免与其他 MCP server 资源冲突。

### 2. SSE transport 复用 rmcp 0.11 内置 SSE server

stage 4 实装时优先评估 rmcp 0.11 是否内置 SSE server（context7 rmcp 官方文档 2026-07-07 验证）。如可用直接复用；否则自实装 tokio + axum 路由 MCP JSON-RPC over SSE（参考 rmcp 0.11 stdio transport 实现模式）。

### 3. Resource subscribe 与既有 tool 互补

| 维度 | tool（request-response） | Resource（subscribe-push） |
|---|---|---|
| 触发方 | client 主动调 | server 主动推 |
| 频率 | 按需 | 1s tick |
| 数据 | 一次性 snapshot | 增量推送 |
| 适用场景 | 查询当前状态 | 监控趋势 / 实时事件 |

agent 视角：tool 适合「现在 cpu 多少」/ Resource 适合「cpu 飙升时通知我」。

### 4. `system_history` 30s cap 与 sparkline 数据点

`VecDeque<SystemSnapshot>` 长度上限 30（1s tick × 30s = 30 个 snapshot），与 brainstorm §主题 B + TD-52 同款决策。`proc_metrics_history` tool 的 `seconds` 参数上限 30（与 `system_history` 字段 30s cap 对齐）。

### 5. subscribe-push worker lifecycle（v0.18 stage 2 落地，stage 1 Spike 完成 rmcp 0.11 API 调研）

**stage 4 落地的是 polling-push**（client 走 `resources/read` 主动拉）——`ProcMcpHandler` impl `ResourceRoute::route(uri) -> Result<Value, String>` 返一次性 snapshot，client 每次调 `resources/read` 拉一次。这与本 ADR §1 描述「subscribe-push」语义有差距（REVIEW-v0.17 §2 Findings P2-B1）。

**v0.18 stage 1 Spike 调研结论（context7 rmcp 0.11 docs 验证，2026-07-10）**：

| 维度 | rmcp 0.11 原生 API | 结论 |
|---|---|---|
| server 端 capability 声明 | `ServerCapabilitiesBuilder::enable_resources_subscribe()` 让 server 声明支持 resources_subscribe | 复用，server 启动时调一次 |
| client 端发起订阅 | `ServerSink::subscribe(SubscribeRequestParams)` / `unsubscribe(...)` — client 角色的方法 | 不暴露给 server handler |
| server 端 subscribe hook | **`ServerHandler` trait 不暴露 `subscribe_resource` 方法**——client `resources/subscribe` 请求 SDK 内部自动 ACK，不传到 user handler | 自建 worker lifecycle 必要 |
| server 主动 push | `Peer::notify_resource_updated(ResourceUpdatedNotificationParam)` — server 通过 Peer 句柄给 client 发 notification | 复用，worker 1s tick 调此 API |
| client 接收 push | `ClientHandler::on_resource_updated(ResourceUpdatedNotificationParam, ...)` — client 角色的 handler | 自动接收，server 无需关心 |

**关键结论**：rmcp 0.11 没有「server 持注册表 + 1s tick push」的原生 helper，需 proc 自建 worker lifecycle（brainstorm 决策 3 拍板）。

**v0.18 stage 2 实装路径**（stage 1 Spike 设计，stage 2 实装）：

```rust
// src/mcp/subscribe_worker.rs（v0.18 stage 1 Spike 落地骨架，stage 2 实装业务逻辑）
pub struct SubscribePushWorker {
    /// 注册表：SubscriberId → Peer 句柄（client 连接时 add / 断开时 remove）
    /// stage 2 实装时 Peer 句柄通过 RequestContext::extensions 拿到（具体 API stage 2 调研）
    subscribers: Arc<Mutex<HashMap<SubscriberId, Peer<RoleServer>>>>,
    /// 1s tick 通知 worker 遍历注册表 push（spawn 一个 tokio task）
    shutdown_tx: Option<oneshot::Sender<()>>,
}
```

**lifecycle 三步**：

1. **subscribe**：client 调 `resources/subscribe`（rmcp SDK 自动 ACK）→ proc 通过自定义 hook（stage 2 调研具体路径，候选：`ServerHandler::read_resource` 时检测 `extensions` / `RequestContext` 中携带的订阅标记，或自定义 tool `proc_subscribe`）→ worker 持 `Peer` 句柄 + 记 `SubscriberId` 到注册表
2. **push**：worker 1s tick 遍历注册表 → 对每个 subscriber 调 `peer.notify_resource_updated(ResourceUpdatedNotificationParam { uri })` → client 通过 `ClientHandler::on_resource_updated` 接收
3. **unsubscribe / 断开**：client 主动 `resources/unsubscribe` 或网络断开 → worker 检测 `peer.notify_resource_updated(...)` 返 `Err(ServiceError)` → 从注册表移除（drop Sender 让 push task 退出，与 brainstorm 决策 3 描述对齐）

**与 brainstorm 决策 3「自建 worker lifecycle」对齐**：rmcp 0.11 不提供原生 worker lifecycle helper，proc 自建「注册表 + 1s tick push + client 断开自动清理」三步管理。

### 6. SSE transport lifecycle（v0.19 stage 1 Spike 扩段，stage 2 实装）

**v0.19 cycle 主题**：SSE transport full 实装（v0.17 stage 4 stub 推迟项）+ multi-client subscribe-push 升级（v0.18 P2-S2 finding 补全）。4 子段对应 cycle 4 项业务实装。

#### §6.1 runtime 分支选择（项 1 落地）

stdio transport 保 `current_thread` runtime（v0.7~v0.18 既有路径零回归）/ SSE transport 走 `new_multi_thread().worker_threads(4)` runtime（多 client 并发 + axum + tower StreamableHttpService 需要）。

**stage 1 Spike 调研结论**（context7 tokio docs 验证，2026-07-13）：

| API | 签名 | 用途 |
|---|---|---|
| `tokio::runtime::Handle::runtime_flavor()` | `&self -> RuntimeFlavor` | 检测当前 runtime 类型 |
| `RuntimeFlavor::CurrentThread` | enum 变体 | current_thread runtime 标识 |
| `RuntimeFlavor::MultiThread` | enum 变体 | multi_thread runtime 标识 |
| `RuntimeFlavor` derive | `#[derive(Debug, PartialEq, Eq)] #[non_exhaustive]` | 可直接 == 比较；match 要 `_` arm |

stage 2 测试可直接写：
```rust
let runtime = build_runtime(&TransportKind::Stdio).unwrap();
assert_eq!(runtime.handle().runtime_flavor(), RuntimeFlavor::CurrentThread);
```

**DockerMonitor block_on 真实风险评估**：docker runtime 独立（`Runtime::new()` 默认 multi_thread），不会抢 MCP runtime 的线程。真实风险是 multi_thread worker thread 被 docker 同步 `block_on` 阻塞（多 client 并发调 docker tool 时 worker thread 可能被占满）。mitigate：worker_threads(4) ≥ 一般 client 数；docker `block_on` 通常 < 1s（bollard list_containers 实测）；> 4 个并发 client 同时调 docker tool 概率低。

#### §6.2 axum 路由 + StreamableHttpService（项 3 落地）

**stage 1 Spike 调研关键发现**（cargo build error 实测 + context7 rmcp 0.11 docs 验证，2026-07-13）—— **Cargo feature flag 验证（brainstorm §决策 4 假设基本正确，补 `-session` 后缀）**：

| brainstorm §决策 4 假设 | rmcp 0.11 实际 features 清单（cargo error 输出） | 验证结论 |
|---|---|---|
| `transport-streamable-http-server`（单数） | `transport-streamable-http-server` ✅ + `transport-streamable-http-server-session` ✅ + `transport-streamable-http-client` + `transport-streamable-http-client-reqwest` + `tower`（独立 feature） | brainstorm 假设正确，需补 `-session` 后缀启用 session submodule（含 `StreamableHttpService` struct）。**不存在 `-tower` 后缀变体**——context7 docs 描述的「tower submodule」是 module 可见性，需 `transport-streamable-http-server-session` + `transport-streamable-http-server` + `server` 三个 feature 组合启用（不能有 `local`） |

**最终 Cargo.toml 配置**（stage 1 Spike 验证 cargo build --release 通过，2026-07-13）：
```toml
rmcp = { version = "0.11", features = ["server", "transport-io", "transport-streamable-http-server", "transport-streamable-http-server-session"] }
```

**理由**：`StreamableHttpService` struct 在 `rmcp::transport::streamable_http_server::tower` 模块内，需 `transport-streamable-http-server`（启用 module）+ `transport-streamable-http-server-session`（启用 session submodule + StreamableHttpService）两个 feature 组合。rmcp 0.11 内部 axum 0.7.9 + tower 0.5.3 + tower-http 0.6.11 被这两个 feature 拉入，proc Cargo.toml 显式加 `axum = "0.7"` / `tower = "0.5"` / `tower-http = "0.6"` 与 rmcp 内部对齐避免 duplicate deps（cargo build 实测无 duplicate，2026-07-13）。

**`StreamableHttpService::new` API 签名**（context7 docs 验证）：
```rust
pub fn new<S, M>(
    service_factory: impl Fn() -> Result<S, Error> + Send + Sync + 'static,
    session_manager: Arc<M>,
    config: StreamableHttpServerConfig,
) -> Self
```

- `service_factory`：closure 每次连接 new 一个 `ProcMcpHandler`（与 brainstorm §决策 7 项 3 描述一致）
- `session_manager`：`Arc<impl SessionManager>`，proc 用 rmcp 内置 `LocalSessionManager` 或自实装（stage 2 调研具体类型）
- `config`：`StreamableHttpServerConfig`（builder 方法 `with_allowed_hosts` / `with_sse_keep_alive` / `with_stateful_mode` / `disable_allowed_hosts` 等）

**axum 版本对齐**（cargo tree -d baseline 验证，2026-07-13）：

- 当前 `Cargo.toml` `rmcp = { version = "0.11", features = ["server", "transport-io"] }` 不引 axum / tower
- 加 `-tower` + `-session` feature 后 rmcp 内部 axum 0.7 / tower 0.5 / http 1.x 被拉入
- proc Cargo.toml 显式加 `axum = "0.7"` / `tower = "0.5"` / `tower-http = "0.6"` 与 rmcp 内部对齐避免 duplicate deps
- stage 1 Spike 落地后跑 `cargo tree -d | grep -E "axum|tower|http"` 验证无 duplicate（baseline 状态当前无 axum/tower deps）

**SessionManager trait**（context7 docs 验证）：8 个方法（create_session / initialize_session / has_session / close_session / create_stream / accept_message / create_standalone_stream / resume / restore_session provided）。proc stage 2 优先复用 rmcp 内置 `LocalSessionManager`（rmcp 内置 impl），如需自定义 session lifecycle（如 session-bound subscribe-push worker）再自实装。

#### §6.3 multi-client 注册表（项 2 落地）

`HashMap<String, Peer<RoleServer>>` 升级为 `HashMap<String, Vec<Peer<RoleServer>>>`（同 URI 多 client 各占 vec 一位）。

**stage 1 Spike 调研结论**（context7 rmcp 0.11 docs 验证，2026-07-13）：

| 维度 | rmcp 0.11 API | 结论 |
|---|---|---|
| `Peer<RoleServer>` derive Eq/Hash | 文档未明确显示 | stage 2 用 `Arc::ptr_eq` 做 vec 去重（Peer 内部 Arc 包装，cheap clone 后 pointer 一致） |
| `RoleServer` impl PartialEq + Eq | `fn eq(&self, other: &RoleServer) -> bool` | 但 RoleServer 是 PhantomData marker，比较无意义（不用于 Peer identity） |
| SSE 路径 session identity | `LocalSessionHandle::id() -> &SessionId` | SSE-only，stdio 路径无 session 概念 |
| HTTP header `mcp-session-id` | `Extension<http::request::Parts>` 在 tool handler 拿 | SSE-only，stdio 无 |
| `Peer<RoleServer>::from_context_part` | 从 RequestContext 拿 Peer | 已用于 v0.18 stage 2 `context.peer.clone()` |

**stage 2 实装策略**（基于调研结论）：

1. **stdio 路径**（current_thread runtime）：注册表 value `Vec<Peer>` 长度始终 ≤ 1（单 client 假设），identity 检测降级为「vec 是否空」
2. **SSE 路径**（multi_thread runtime）：用 `Arc::ptr_eq(p1, p2)` 做 vec 去重 + fail peer 清理（Peer 是 Arc 包装，cheap clone 后 pointer 一致）
3. **fallback**：如 `Arc::ptr_eq` 不可靠（Peer 内部不是简单 Arc 包装），fallback 到 `Peer::id()` / `mcp-session-id` HTTP header 拿 client 标识（stage 2 调研具体 API）

**push task 改造**（项 4 落地）：1s tick 遍历 `Vec<Peer>` 用 `tokio::task::JoinSet` 并发调 `peer.notify_resource_updated`，`join_next().await` 逐个判断 fail peer 一次性从注册表清理。

`JoinSet` 相对 `join_all` 的优势（brainstorm §FAQ Q5 + 本 ADR §6.3 实装策略对齐）：
1. **fail peer 一次性清理**：JoinSet 的 `join_next().await` 逐个判断 vs `join_all` 等所有 future 完成
2. **abort 能力**：`abort_all()` 在 worker shutdown 时主动取消
3. **backpressure**：可配 `build()` 限并发度，避免 100 个 client 同 URI 一次性 spawn 100 个 task

#### §6.4 bind-addr 安全默认（项 3 落地）

CLI flag `--bind-addr` 默认 `127.0.0.1`（仅本机，与 ADR-0008 self-mitigation policy 对齐）/ `0.0.0.0`（全网卡，用户显式 opt-in）。

**安全考量**：SSE transport 暴露 proc MCP tool（含 `proc_docker_rm` / `proc_docker_image_rm` / `proc_docker_volume_rm` / `proc_usb_release` / `proc_record_start` 等写操作），全网卡监听（`0.0.0.0`）需用户显式 opt-in 避免意外暴露到 LAN。默认 `127.0.0.1` 让本机使用（如 mcp-inspector 调试 / Claude Desktop 本机集成）零配置，LAN 部署需用户明确知晓风险。

**stage 2 落地**：

- `src/cli/def.rs::Command::McpServe` 加 `bind_addr: String` 字段默认 `"127.0.0.1"`（clap derive `#[arg(long, default_value = "127.0.0.1")]`）
- CLI 解析后传入 `SseTransportConfig { bind_addr, port }`
- mcp-inspector manual 验证：`--bind-addr 0.0.0.0` LAN 可访问，默认 `127.0.0.1` 仅本机可访问

## 备选方案（Alternatives）

### (a) 仅 tool，无 Resource subscribe

**否决**：sparkline 30s 历史需多次调用 tool 累积，agent 视角不友好（v0.15 TD-52 归档时已评估）。Resource subscribe 让 client 订阅后 server 主动推，适合实时监控场景。

### (b) Resource subscribe + SSE transport（**本 ADR 选此**）

**接受**：3 件套落地——ResourceRoute trait + SSE transport + TD-52 sparkline。stage 4 Slice 实装完整可观测性 schema。

### (c) WebSocket transport

**否决**：rmcp 0.11 不原生支持 WebSocket（context7 rmcp 官方文档 2026-07-07 验证）。SSE 是 rmcp 0.11 内置支持的长连接 transport（如可用），否则自实装 tokio + axum 路由 SSE。

## 结果（Consequences）

- **stage 1 Spike 落地**：`src/mcp/resources.rs`（ResourceRoute trait stub + PROC_RESOURCE_URIS 常量）+ `src/mcp/transport.rs`（SseTransportConfig struct + serve_sse 函数 stub）+ `src/mcp/handler/observable.rs`（MetricsHistoryArgs + make_metrics_history_json stub helper）
- **stage 4 Slice 实装**：ResourceRoute trait impl + SSE transport 入口 + TD-52 sparkline worker（drain `system_history` 字段）
- **`proc_metrics_history` tool 落地**：agent 调一次拿 30s 历史趋势，无需多次调 `proc_metrics_system` 累积

### 负面（Trade-offs）

- **rmcp 0.11 API 学习曲线**：Resource subscribe 是 rmcp 0.11 新能力，文档稀少（context7 验证），stage 4 实装时可能触发拆分（stage 4a Resource subscribe + SSE / stage 4b TD-52 sparkline worker，brainstorm §决策 8 自适应拆分规则）
- **SSE transport 实装复杂度**：如 rmcp 0.11 不内置 SSE server，需自实装 tokio + axum 路由，工作量 ~200 行（与 brainstorm §主题 B 估算对齐）
- **`system_history` 字段内存占用**：30 个 SystemSnapshot × ~10KB/snapshot = ~300KB 常驻内存，可接受（vs proc 主进程 ~50MB 内存）

## Migration path

- **v0.17 stage 1 Spike**（本 ADR 落地）：3 个骨架文件（resources.rs / transport.rs / observable.rs）+ stub helper
- **v0.17 stage 4 Slice**：ResourceRoute trait impl + SSE transport 入口 + TD-52 sparkline worker + `proc_metrics_history` tool 业务逻辑填充（**polling-push partial 落地**——client 走 `resources/read` 主动拉）
- **v0.18 stage 1 Spike**：扩 §5 subscribe-push worker lifecycle + stage 1 调研 rmcp 0.11 API（已确认无原生 `ServerHandler::subscribe_resource`，自建 worker lifecycle 必要）+ `src/mcp/subscribe_worker.rs`（新）骨架 + `ResourceRoute` trait 加 `subscribe` / `unsubscribe` stub
- **v0.18 stage 2 Slice**：subscribe-push worker lifecycle 业务逻辑填充（注册表 + 1s tick `peer.notify_resource_updated` push + client 断开自动清理）+ ADR-0027 Status 备注 「subscribe-push 已补全」
- **v0.19 stage 1 Spike**：扩 §6 SSE transport lifecycle（4 子段：§6.1 runtime 分支 / §6.2 axum 路由 / §6.3 multi-client 注册表 / §6.4 bind-addr 安全默认）+ stage 1 调研 3 子任务（context7 + cargo tree -d 验证）：(a) Cargo feature 修正 `transport-streamable-http-server` → `transport-streamable-http-server-tower` + `transport-streamable-http-server-session` 两个 feature（小修，< 5 处属 stage 1 内修正）；(b) `Peer<RoleServer>` identity 检测策略——文档未明确 Eq/Hash，stage 2 用 `Arc::ptr_eq` + `Arc<Peer>` cheap clone pointer equality；(c) tokio `Handle::runtime_flavor() -> RuntimeFlavor` 完美适配 stage 2 测试 `test_stdio_transport_uses_current_thread_runtime` / `test_sse_transport_uses_multi_thread_runtime`。stage 1 落地 5 项 stub（TransportKind enum / build_runtime / serve_sse 新格式 / CLI flag bind-addr + port / 注册表 Vec<Peer> 类型升级）+ Cargo.toml 加 axum 0.7 / tower 0.5 / tower-http 0.6 + rmcp feature 扩。
- **v0.19 stage 2 Slice**：4 项业务实装（项 1 runtime 分支 build_runtime match / 项 2 multi-client 注册表 Vec<Peer> + Arc::ptr_eq identity / 项 4 push task JoinSet 并发 + fail peer 一次清理 / 项 3 SSE transport 入口 StreamableHttpService::new + axum Router + TcpListener bind）+ ADR-0027 Status 备注 「SSE transport full 实装 + multi-client 升级已补全」
- **v0.20+ cycle**：评估 WebSocket transport（如 rmcp 0.12+ 支持）/ 评估 Resource subscribe 推送频率可配置（如 5s / 10s tick 替代 1s tick）/ SSE 后续能力（graceful shutdown / Auth Bearer token / CORS / TLS）

## 相关 ADR / 文档

- [ADR-0009](0009-mcp-server.md)：v0.7 MCP server 设计（stdio transport 默认，SSE 是替代）
- [ADR-0024](0024-mcp-handler-module-split.md)：v0.15 handler 子 module 拆分（v0.17 stage 1 加 `observable.rs` 第 6 个子 module 延续）
- [ADR-0026](0026-mcp-handler-persistent-fields.md)：MCP handler 持久字段策略（`system_history` 字段落地基础）
- [ADR-0029](0029-record-exposure-and-confirm-mechanism.md)：record 暴露 + 写操作 confirm 机制（stage 6 实装时复用 `record_handle` 字段）
- v0.15 TD-52 归档：metrics_system sparkline 30s 历史不暴露
- v0.15 TD-54 归档：MCP handler 多次调用累积 SystemSnapshot / App 开销
- [`docs/stages/v0.17-stage-1.md`](../stages/v0.17-stage-1.md) §决策 7（observable.rs 子 module 骨架）
