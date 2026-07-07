# ADR-0027：rmcp 0.11 Resource subscribe + SSE transport 设计

**Status**：Accepted
**Date**：2026-07-07（v0.17.0 阶段 1 落地决策）
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
- **v0.17 stage 4 Slice**：ResourceRoute trait impl + SSE transport 入口 + TD-52 sparkline worker + `proc_metrics_history` tool 业务逻辑填充
- **v0.18+ cycle**：评估 WebSocket transport（如 rmcp 0.12+ 支持）/ 评估 Resource subscribe 推送频率可配置（如 5s / 10s tick 替代 1s tick）

## 相关 ADR / 文档

- [ADR-0009](0009-mcp-server.md)：v0.7 MCP server 设计（stdio transport 默认，SSE 是替代）
- [ADR-0024](0024-mcp-handler-module-split.md)：v0.15 handler 子 module 拆分（v0.17 stage 1 加 `observable.rs` 第 6 个子 module 延续）
- [ADR-0026](0026-mcp-handler-persistent-fields.md)：MCP handler 持久字段策略（`system_history` 字段落地基础）
- [ADR-0029](0029-record-exposure-and-confirm-mechanism.md)：record 暴露 + 写操作 confirm 机制（stage 6 实装时复用 `record_handle` 字段）
- v0.15 TD-52 归档：metrics_system sparkline 30s 历史不暴露
- v0.15 TD-54 归档：MCP handler 多次调用累积 SystemSnapshot / App 开销
- [`docs/stages/v0.17-stage-1.md`](../stages/v0.17-stage-1.md) §决策 7（observable.rs 子 module 骨架）
