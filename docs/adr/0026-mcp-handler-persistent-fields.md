# ADR-0026：MCP handler 持久字段策略（TD-54 落地 + TD-52 + record 暴露前置）

**Status**：Accepted
**Date**：2026-07-07（v0.17.0 阶段 1 落地决策）
**Related**：ADR-0009（v0.7 MCP server 设计）、ADR-0024（v0.15 handler 子 module 拆分）、ADR-0027（rmcp Resource subscribe + SSE transport）、ADR-0029（record 暴露 + 写操作 confirm 机制）、v0.12 TD-36（持久 dns_collector 同款模式）

## 背景（Context）

v0.15 cycle TD-54 归档「MCP handler 多次调用累积 SystemSnapshot / App 开销」：`proc_flows` / `proc_metrics_*` / `proc_export` 等 tool 每次调用都 `SystemSnapshot::new() + refresh() + refresh_heavy_incremental()`，单次 ~50-200ms 累积开销大。v0.15 cycle 量级偏轻（~1700 行业务代码）未做 TD-54，留 v0.17 cycle 主题 A 性能优化实装。

v0.17 cycle 主题 B 可观测性（stage 4 落地）需 worker 持久 `SystemSnapshot` 字段（TD-52 sparkline 30s 历史采样）+ rmcp 0.11 Resource subscribe 1s tick 推送增量。

v0.17 cycle stage 6 record 暴露需持久 `Child` handle 字段（spawn `proc record --no-tui` 子进程跨 tool call 保活，详见 ADR-0029）。

3 个需求都需 `ProcMcpHandler` 加 `Arc<Mutex<T>>` 持久字段，与 v0.12 TD-36 持久 `dns_collector` 同款模式延续。

## 决策（Decision）

**`ProcMcpHandler` 加 3 个 `Arc<Mutex<T>>` 持久字段 + `mcp-persistent-state` feature flag（默认开启）**：

```rust
pub struct ProcMcpHandler {
    pub dns_collector: Arc<Mutex<Option<Box<dyn DnsLogCollector>>>>,

    /// v0.17 stage 3 TD-54 落地：MCP handler 持久 SystemSnapshot，1s tick refresh
    #[cfg(feature = "mcp-persistent-state")]
    pub snapshot: Arc<Mutex<Option<SystemSnapshot>>>,

    /// v0.17 stage 4 TD-52 落地：sparkline 30s 历史，1s tick push
    #[cfg(feature = "mcp-persistent-state")]
    pub system_history: Arc<Mutex<VecDeque<SystemSnapshot>>>,

    /// v0.17 stage 6 record 暴露落地：spawn `proc record` 子进程 handle
    #[cfg(feature = "mcp-persistent-state")]
    pub record_handle: Arc<Mutex<Option<std::process::Child>>>,
}
```

stage 1 Spike 仅声明字段 + `Default` / `new()` 返 `None` / 空 `VecDeque`（与 v0.12 TD-36 `dns_collector` Default 同款规则——测试路径不 spawn worker）。stage 3/4/6 各 Slice 实装 worker spawn + refresh / push / spawn 逻辑后才在 `new()` 生产路径填充。

`mcp-persistent-state` feature flag 默认开启（在 `Cargo.toml` `[features]` 段 `default = ["nvidia", "mcp-persistent-state"]`），让生产路径有这 3 个字段；`--no-default-features` 时 cfg-gate 掉，struct 不含这 3 个字段（最小化 build / 测试路径不强制 spawn worker）。

## 关键设计点

### 1. `Arc<Mutex<T>>` 字段 Clone derive 共享

rmcp 内部每次 tool call clone handler 时，`Arc::clone` 共享同一 `snapshot` / `system_history` / `record_handle` 实例（与 `dns_collector` 同款规则）。`Clone` impl 显式调 `Arc::clone`：

```rust
impl Clone for ProcMcpHandler {
    fn clone(&self) -> Self {
        Self {
            dns_collector: Arc::clone(&self.dns_collector),
            #[cfg(feature = "mcp-persistent-state")]
            snapshot: Arc::clone(&self.snapshot),
            // ...
        }
    }
}
```

### 2. `Default` 返 `None` / 空 `VecDeque`（测试路径不 spawn worker）

与 v0.12 TD-36 `dns_collector` Default 同款规则：测试路径不 spawn worker（避免单测里跑 ETW session / PowerShell 子进程 / SystemSnapshot refresh 污染输出）。生产路径必须用 `ProcMcpHandler::new`。

### 3. `new()` 生产路径 stage 3/4/6 各 Slice 实装 worker spawn

stage 1 Spike 的 `new()` 与 `Default` 同款返 `None` / 空 `VecDeque`（不 spawn worker）。stage 3 TD-54 实装 `snapshot` worker spawn（1s tick refresh SystemSnapshot）；stage 4 TD-52 实装 `system_history` worker push（1s tick push 到 VecDeque，30s cap）；stage 6 record 暴露实装 `record_handle` spawn（`proc_record_start` 调用时 spawn child，`proc_record_stop` 调用时 kill child）。

### 4. `app_handle` 不实装（App 不是 Send + Sync）

stage 1 Spike 评估了 `app_handle: Arc<Mutex<Option<AppHandle>>>` 字段（让 `proc_flows` / `proc_diag` 复用 `App::new()` 而非每次新建），但 `App` 不是 `Send + Sync`（含 worker handle + UI 状态 + ratatui Terminal 等非线程安全字段），跨 tool call 共享需重构 App 结构。**v0.17 cycle 不实装**（留 v0.18+ cycle 评估）。

## 备选方案（Alternatives）

### (a) TTL 缓存（snapshot 字段 + 过期时间）

**否决**：freshness 不如 worker 1s tick。TTL 缓存到期前 agent 多次调用都拿到同一 snapshot，stage 4 主题 B Resource subscribe 1s tick 推送增量需要持续 fresh 数据，TTL 缓存无法满足。

### (b) 每次调用新建 SystemSnapshot（v0.16 末态）

**否决**：累积开销大，TD-54 不闭环。v0.15 TD-54 归档时已评估：`proc_flows` 单次 ~200ms（含 2s warm-up），`proc_metrics_system` 单次 ~50ms，agent 多次调用累积 500ms-2s/次。stage 3 TD-54 实装后预期降到 < 100ms/次。

### (c) 持久字段 + worker 1s tick（**本 ADR 选此**）

**接受**：3 个 `Arc<Mutex<T>>` 字段 + worker 1s tick refresh / push / spawn，与 v0.12 TD-36 持久 dns_collector 同款模式延续。stage 3/4/6 各 Slice 实装 worker spawn 逻辑。

## 结果（Consequences）

- **stage 1 Spike 落地**：3 个持久字段声明 + `Default` / `new()` 返 None / 空 VecDeque + `mcp-persistent-state` feature flag 默认开启
- **stage 3/4/6 各 Slice 实装 worker spawn 逻辑**：stage 3 TD-54 实装 `snapshot` worker / stage 4 TD-52 实装 `system_history` worker / stage 6 record 暴露实装 `record_handle` spawn
- **既有 39 tool 零回归**：mod.rs impl 块结构稳定，仅 struct 加字段 + Clone / Default 加初始化，既有 39 tool 逻辑不动

### 负面（Trade-offs）

- **handler 字段增多**：从 1 个字段（`dns_collector`）增到 4 个字段（含 `snapshot` / `system_history` / `record_handle`），struct 略复杂。但每个字段都有明确职责（DNS collector / snapshot cache / sparkline history / record child），无重叠
- **feature flag 复杂度**：`mcp-persistent-state` feature flag 让 `--no-default-features` 路径与默认路径 struct 不同（cfg-gate 掉 3 个字段），贡献者需理解 cfg-gate 规则。但 v0.12 已删 ebpf feature 后 proc feature flags 简单，加 `mcp-persistent-state` 不冲突

## Migration path

- **v0.17 stage 1 Spike**（本 ADR 落地）：3 个持久字段声明 + `Default` / `new()` 返 None / 空 VecDeque + feature flag 默认开启
- **v0.17 stage 3**：TD-54 实装 `snapshot` worker spawn（1s tick refresh SystemSnapshot）+ `proc_metrics_*` / `proc_flows` / `proc_export` 改为 drain `snapshot` 字段
- **v0.17 stage 4**：TD-52 实装 `system_history` worker push（1s tick push 到 VecDeque，30s cap）+ `proc_metrics_history` tool drain 此字段
- **v0.17 stage 6**：record 暴露实装 `record_handle` spawn（`proc_record_start` spawn child + `proc_record_stop` kill child）
- **v0.18+ cycle**：评估 `app_handle` 共享（App 重构为 Send + Sync 后）

## 相关 ADR / 文档

- [ADR-0009](0009-mcp-server.md)：v0.7 MCP server 设计（agent 视角字段裁剪原则延续）
- [ADR-0024](0024-mcp-handler-module-split.md)：v0.15 handler 子 module 拆分（v0.17 stage 1 加 `observable.rs` 第 6 个子 module 延续）
- [ADR-0027](0027-rmcp-resource-subscribe-sse-transport.md)：rmcp Resource subscribe + SSE transport（stage 4 实装时复用 `snapshot` / `system_history` 字段）
- [ADR-0029](0029-record-exposure-and-confirm-mechanism.md)：record 暴露 + 写操作 confirm 机制（stage 6 实装时复用 `record_handle` 字段）
- v0.12 TD-36 持久 dns_collector：同款 `Arc<Mutex<T>>` 模式延续
- v0.15 TD-54 归档：MCP handler 多次调用累积 SystemSnapshot / App 开销
- v0.15 TD-52 归档：metrics_system sparkline 30s 历史不暴露
- [`docs/stages/v0.17-stage-1.md`](../stages/v0.17-stage-1.md) §决策 4（持久字段 stub 设计）
