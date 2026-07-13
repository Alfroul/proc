# REVIEW-v0.19 — v0.19 cycle Review（SSE transport full 实装 + multi-client 升级 cycle 完结）

> **cycle 范围**：v0.18 cycle 末段 REVIEW-v0.18 §v0.19+ 候选方向段第 1 项（SSE transport full 实装）+ 第 5 项（SSE multi-client subscribe-push 升级）合并 —— 4 项基础设施补全（runtime 分支 + multi-client 注册表 + push task 并发 + SSE transport 入口）
>
> **Review 范围**：3 stage 全部产出（1 Spike + 1 Slice + 本 Review+收尾合并段）
>
> **基线**：1447 passed / 0 failed / 4 ignored（stage 2 后）/ fmt / clippy / build（含 --no-default-features）/ bench --no-run 全过
>
> **Review 日期**：2026-07-14
>
> **Reviewer**：Claude（stage 3 会话）

---

## 概览

v0.19 cycle 是 proc 历史上较小 cycle（~900 行总改动，与 v0.18 cycle ~820 行同档轻量），3 stage 节奏紧凑（1 Spike + 1 Slice + Review+收尾合并段，与 v0.15/v0.16/v0.17/v0.18 cycle 同款合并模式延续）。MCP tool 总数 46 → 46（不变——4 项都是 transport 层与注册表基础设施补全），0 份新 ADR + 1 份扩段（ADR-0027 §6 SSE transport lifecycle 4 子段），首次引入 axum + tower + tower-http web framework deps（与 rmcp 0.11 内部 axum 0.7.9 / tower 0.5.3 / tower-http 0.6.11 对齐无 duplicate）。

**Findings 汇总**：P0 0 / P1 0 / P2 4（详见末尾表）。预期不触发 brainstorm §决策 1 自适应拆分（阈值 P0 ≥ 1 或 P1 ≥ 5）。

---

## 1. 项 1：runtime 分支切换（stage 2 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| `TransportKind` enum | stage 1 Spike 落地 `enum { Stdio, Sse(SseTransportConfig) }` + Default = Stdio + `as_str` / `sse_config` / `from_cli_str` helper + Debug/Clone/PartialEq derive | `src/mcp/transport.rs` |
| `build_runtime(kind)` 分支 | stage 1 Spike stub（无参返 current_thread 占位）→ stage 2 match 分支：`Stdio` → `Builder::new_current_thread().enable_all()` / `Sse(_)` → `Builder::new_multi_thread().worker_threads(4).enable_all()` | `src/mcp/mod.rs` |
| `run_mcp_serve(kind)` 重构 | 当前 `run_mcp_serve()` 无参 → stage 2 加 `kind: TransportKind` 参数 + TTY 提示分支（stdio 走 stdin/stdout 才需 TTY 检查 / SSE 走 HTTP 跳过）+ 按 kind dispatch 到 `handler::serve()` 或 `transport::serve_sse(config)` | `src/mcp/mod.rs` + `src/cli/mcp_cmd.rs` |

### 4 维度审查

**代码质量** ✅：
- `build_runtime` match 分支简洁清晰，与 ADR-0027 §6.1 + brainstorm §决策 2 拍板对齐
- `Handle::runtime_flavor()` 检测 runtime 类型让 stage 2 测试 `build_runtime_returns_current_thread_for_stdio` / `build_runtime_returns_multi_thread_for_sse` 直接断言 runtime flavor
- TTY 提示仅 stdio 路径触发（避免 SSE server 启动时误报「not a TTY」噪音）

**架构** ✅：
- stdio 路径保 `current_thread` runtime（与 v0.7~v0.18 11 个 cycle 既有路径零回归）
- SSE 路径独立 `multi_thread worker_threads(4)` runtime（多 client 并发 + axum + tower StreamableHttpService 需要）
- `TransportKind::from_cli_str` 把 CLI String 转 enum，dispatch 路径清晰（`mcp_cmd::run_mcp` → `build_transport_kind` helper → `run_mcp_serve(kind)`）

**性能** ✅：
- stdio `current_thread` 单 client IO-bound 场景足够（与 brainstorm §决策 2 备选方案 (b) 对齐）
- SSE `multi_thread worker_threads(4) ≥ 一般 client 数`，docker 同步 `block_on` 阻塞风险 mitigate（ADR-0027 §6.1 DockerMonitor block_on 真实风险评估段）

**完整性** ✅：
- stage 1 Spike 落地 3 个 build_runtime stub 测试（用 `Handle::runtime_flavor()` 检测）/ stage 2 改写为 match 分支测试（同款 API + 真实分支断言）
- `test_build_runtime_returns_*_via_re_export` × 2（tests/test_sse_transport.rs）验证 TransportKind re-export 路径用

---

## 2. 项 2：multi-client 注册表升级（stage 2 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| 注册表 value 类型升级 | stage 1 Spike `Vec<Peer<RoleServer>>` 占位 → stage 2 `Vec<Arc<Peer<RoleServer>>>`（让 push task 失败 cleanup 走 `Arc::ptr_eq` 精确 retain）+ `type SubscribeRegistry = HashMap<String, Vec<Arc<Peer<RoleServer>>>>` 类型别名（clippy `type_complexity` mitigate） | `src/mcp/subscribe_worker.rs` |
| subscribe 业务 | stage 1 Spike `vec.push(peer)` 简单追加 → stage 2 `Arc::new(peer)` 包装后 push（**stage 2 已知限制**——subscribe 跨调用不做 dedup，rmcp 0.11 `Peer` 不暴露 public identity API）| 同上 |
| unsubscribe 业务 | stage 1 Spike `vec.clear()` → stage 2 仍清空整个 vec（**stage 2 已知限制**——SSE multi-client 下 A client unsubscribe 会清掉 B client 同 URI 订阅；stdio 单 client 场景正确）| 同上 |

### 4 维度审查

**代码质量** ✅：
- `Arc<Peer>` wrap 让 push task 失败 cleanup 走 `Arc::ptr_eq` 精确 retain（stored Arc 与 spawned `Arc::clone` 共享同一 allocation）
- `type SubscribeRegistry` 别名拆 clippy `type_complexity` 警告，代码可读性提升
- doc 注释明确标注「stage 2 已知限制」+「v0.20+ 改进方向（mcp-session-id-based identity）」

**架构** ✅：
- v0.18 stage 2 `HashMap<String, Peer>` 单 client 假设补全为 `HashMap<String, Vec<Arc<Peer>>>` 多 client 场景
- push task 失败 cleanup 兜底机制让 client 断开后下一次 push 失败的 peer 被精确移除（不影响其他 client）

**性能** ✅：
- `Arc::new(peer)` 包装是 cheap allocation（仅一次 atomic ref count inc）
- `Arc::ptr_eq` 比较 pointer equality 是 O(1) 操作

**完整性** ⚠️：
- stage 2 已知限制：subscribe 跨调用不做 dedup / SSE multi-client unsubscribe 不精确（rmcp 0.11 `Peer` 不暴露 public identity API）
- 单元测试用源码静态断言（`test_subscribe_worker_source_documents_known_limitations` 检查源码含「stage 2 已知限制」+「mcp-session-id」字样）替代 identity lifecycle 测试

### Findings

**P2-S4**：subscribe 跨调用不做 dedup（同 client 多次 subscribe 同 URI 会在 vec push 多个 Arc<Peer> 项）——这是 stage 2 兜底策略（rmcp 0.11 Peer identity 不可用），依赖 push task 失败 cleanup 兜底（client 断开后下一次 push 失败对应 Arc 被精确移除）。v0.20+ cycle 评估从 `RequestContext::extensions` 拿 `mcp-session-id` HTTP header（SSE 路径）作 client identity。

**P2-S5**：SSE multi-client unsubscribe 清空整个 vec（A client unsubscribe 会清掉 B client 同 URI 订阅）——这是 stage 2 兜底策略（MCP 协议 `UnsubscribeRequestParam` 只有 uri 不携带 client identity）。stdio 单 client 场景正确（vec 长度 ≤ 1）；SSE multi-client 场景下 B client 想继续订阅需 client-side 重 subscribe。v0.20+ cycle 用 mcp-session-id 字符串作 secondary index 走 `vec.retain(|p| !identity_match)` 精确移除。

---

## 3. 项 4：push task 并发改造（stage 2 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| `spawn_push_task` 改造 | stage 1 Spike 双层 for 循环顺序遍历 → stage 2 `tokio::task::JoinSet::spawn` 并发调每个 `Arc<Peer>::clone` 的 `notify_resource_updated` + `join_next().await` 逐个判断 fail peer + `Arc::ptr_eq(p, &failed_arc)` 一次性从 vec retain 精确清理 | `src/mcp/subscribe_worker.rs` |

### 4 维度审查

**代码质量** ✅：
- snapshot 注册表 `Vec<(String, Vec<Arc<Peer>>)>` 持锁窗口短（仅 clone 操作）
- JoinSet 并发 spawn 让 100 个 client 同 URI 订阅时不阻塞（vs 顺序 for 等每个 `notify_resource_updated` 完成）
- `join_next().await` 逐个判断 + `Arc::ptr_eq` 精确 retain，替代 stage 1 Spike `vec.clear()` 不精确策略

**架构** ✅：
- JoinSet 相对 `join_all` 优势：fail peer 一次性清理（`join_next` 逐个判断 vs `join_all` 等所有 future 完成）+ abort 能力（`abort_all()` worker shutdown 时主动取消）+ backpressure（v0.20+ cycle 评估 `JoinSet::build()` 限并发度）
- 与 ADR-0027 §6.3 + brainstorm FAQ Q5 对齐

**性能** ✅：
- 1s tick 通知 worker 遍历所有 subscriber 并发 push（vs stage 1 Spike 顺序遍历，多 client 场景延迟改善）
- fail peer 一次性 retain cleanup 避免 stage 1 Spike `vec.clear()` 误清空所有 client

**完整性** ✅：
- 源码契约测试 `test_subscribe_push_worker_implements_arc_ptr_eq_cleanup`（tests/test_sse_transport.rs）断言源码含 `JoinSet` / `Arc::ptr_eq` / `join_next`

---

## 4. 项 3：SSE transport 入口（stage 2 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| `serve_sse` 真实实装 | stage 1 Spike `Result<Value, String>` 同步 stub → stage 2 `async fn serve_sse(config: SseTransportConfig) -> anyhow::Result<()>`：(1) `StreamableHttpService::new(service_factory, LocalSessionManager, StreamableHttpServerConfig)` + (2) axum `Router::new().route_service("/mcp", http_service)` + (3) `TcpListener::bind(socket_addr).await` + (4) `axum::serve(listener, app).await` | `src/mcp/transport.rs` |
| `SseTransportConfig::socket_addr()` | private helper 解析 `(bind_addr, port)` 为 `SocketAddr`（仅接 IP 字面量，hostname 如 "localhost" 拒绝——stage 2 安全保守）| 同上 |
| 集成测试 | 6 个测试：`test_serve_sse_starts_and_accepts_tcp_connection`（动态端口 + multi_thread runtime + tokio::spawn + 5s 内 TCP connect 验证监听）/ `test_serve_sse_rejects_invalid_bind_addr_gracefully`（hostname 返 Err 不 panic）/ `test_sse_transport_shares_same_handler_as_stdio_with_46_tools`（service_factory 调 `ProcMcpHandler::new` + tool 数 ≥ 46）/ 2 个 `test_build_runtime_returns_*_via_re_export` / `test_subscribe_push_worker_implements_arc_ptr_eq_cleanup`（项 4 源码契约）| `tests/test_sse_transport.rs`（新）|

### 4 维度审查

**代码质量** ✅：
- `service_factory` closure `|| Ok::<ProcMcpHandler, std::io::Error>(ProcMcpHandler::new())` —— rmcp 0.11 要求 closure 返 `Result<S, std::io::Error>` 非 `Infallible`（stage 1 Spike 假设修正，cargo build error 实测）
- `LocalSessionManager` 用 rmcp 内置 impl（stateful mode，每个 client 一个 session），session-bound lifecycle 留 v0.20+ cycle
- `socket_addr()` 仅接 IP 字面量（hostname 拒绝），与 ADR-0027 §6.4 bind-addr 安全默认对齐
- `eprintln!` 启动 banner `MCP SSE server listening on http://{socket_addr}/mcp (bind_addr={}, port={})` 让用户明确知晓监听位置

**架构** ✅：
- 与 v0.7~v0.18 既有 stdio transport 并行——stdio 是默认 transport（单 client 集成 Claude Desktop / Cursor）/ SSE 是长连接场景替代（多 client 监控 / Web dashboard / 远程 agent 集成）
- service_factory closure 每次连接 new 一个 `ProcMcpHandler`（与 brainstorm §决策 7 项 3 + ADR-0027 §6.2 对齐），每个 session 独立不共享状态
- CORS / graceful shutdown / Auth / TLS 都不在 stage 2 范围（推迟 v0.20+ cycle，与 brainstorm §决策 6 拍板对齐）

**性能** ✅：
- axum 0.7.9 + tower 0.5.3 + tower-http 0.6.11 与 rmcp 内部对齐无 duplicate deps（`cargo tree -d` 验证）
- multi_thread worker_threads(4) runtime 让 axum + tower StreamableHttpService 并发处理 request
- `TcpListener::bind(socket_addr).await` 异步监听不阻塞 main thread

**完整性** ✅：
- 集成测试覆盖 server 启动 + TCP 监听 + 46 tool 数 + bind-addr 安全默认 + 源码契约
- stage 3 manual 验证：`./target/release/proc.exe mcp serve --transport sse --port 18080` 启动 banner `MCP SSE server listening on http://127.0.0.1:18080/mcp (bind_addr=127.0.0.1, port=18080)` + HTTP POST `/mcp` 返 406（rmcp StreamableHttpService 对单独 initialize POST 不带 GET SSE stream context 正确拒绝，证明 server live + routing 工作）
- multi-client 同 URI 订阅 1s tick push + bind-addr 0.0.0.0 LAN 暴露 manual 验证推迟 v0.20+ cycle（依赖项 2 mcp-session-id-based identity 补全后才可精确验证）

### Findings

**P2-S6**：SSE transport manual 验证仅做到「server 启动 + HTTP 路由 live」，未跑完整 multi-client 同 URI 订阅 1s tick push + bind-addr LAN 暴露 manual 验证——因 stage 2 已知限制（项 2 P2-S4 + P2-S5）让 multi-client identity 不精确，完整 manual 验证留 v0.20+ cycle（用 mcp-session-id 补全 identity 后再跑）。当前覆盖：集成测试 `test_serve_sse_starts_and_accepts_tcp_connection` 验证监听 + stage 3 manual 验证 server 启动 banner + HTTP routing。

---

## Findings 汇总表

| ID | 主题 | 严重度 | 描述 | 建议处理 |
|---|---|---|---|---|
| P2-S4 | 项 2 multi-client 注册表 | P2 | subscribe 跨调用不做 dedup（同 client 多次 subscribe 同 URI 会在 vec push 多个 Arc<Peer> 项） | v0.20+ cycle 评估从 `RequestContext::extensions` 拿 `mcp-session-id` HTTP header（SSE 路径）作 client identity |
| P2-S5 | 项 2 multi-client 注册表 | P2 | SSE multi-client unsubscribe 清空整个 vec（A client unsubscribe 会清掉 B client 同 URI 订阅） | v0.20+ cycle 用 mcp-session-id 字符串作 secondary index 走 `vec.retain(\|p\| !identity_match)` 精确移除 |
| P2-V2 | 项 3 SSE transport | P2 | manual 验证仅做 server 启动 + HTTP 路由 live，未跑完整 multi-client 1s tick push + bind-addr LAN 暴露 | v0.20+ cycle 评估 mcp-session-id identity 补全后再跑完整 manual 验证 |
| P2-T1 | Cargo deps 引入 | P2 | v0.19 cycle 首次引入 axum + tower + tower-http web framework deps（虽与 rmcp 内部对齐无 duplicate，但 proc 编译时间增加约 30s） | 接受（与 brainstorm §决策 4 拍板对齐，axum + tower 直接 deps 与 rmcp 内部版本对齐避免类型分裂） |

**P0 0 / P1 0 / P2 4**。预期不触发 brainstorm §决策 1 自适应拆分（阈值 P0 ≥ 1 或 P1 ≥ 5）。所有 P2 都是已知限制或 manual 验证覆盖问题，无 blocker。

---

## cycle 完整性评分

| 维度 | 评分 | 说明 |
|---|---|---|
| **4 项基础设施补全全交付** | ✅ | 项 1 runtime 分支 / 项 2 multi-client 注册表 / 项 4 push task 并发 / 项 3 SSE transport 入口 全部落地 |
| **3 stage 全部 ✅** | ✅ | 1 Spike + 1 Slice + 1 Review+收尾合并段（与 v0.15/v0.16/v0.17/v0.18 cycle 同款合并模式延续） |
| **MCP tool 总数** | ✅ | 46 → 46（不变，v0.19 cycle 不新增 tool——4 项都是 transport 层与注册表基础设施补全） |
| **全量回归** | ✅ | 1447 passed / 0 failed / 4 ignored（stage 2 后基线，v0.18 基线 1425 + 12 stage 1 + 10 stage 2 = +22 新测试） |
| **ADR 落地** | ✅ | 0 份新 ADR + 1 份扩段（ADR-0027 §6 SSE transport lifecycle 4 子段：§6.1 runtime 分支 / §6.2 axum 路由 / §6.3 multi-client 注册表 / §6.4 bind-addr 安全默认） |
| **Cargo deps** | ✅ | + axum 0.7 + tower 0.5 + tower-http 0.6 + rmcp features 扩 `transport-streamable-http-server` + `transport-streamable-http-server-session`（首次引入 web framework deps，`cargo tree -d` 验证无 duplicate） |
| **fmt / clippy / build / bench** | ✅ | 全过（含 --no-default-features cfg-gate 验证） |
| **测试覆盖** | ✅ | stage 1 加 12 项 stub 测试（含 build_runtime stub + TransportKind enum 9 测试 + 静态契约改 vec 类型）/ stage 2 加 10 项真实测试（runtime 分支 match + SSE transport 集成 6 测试 + Arc::ptr_eq 源码契约 + 已知限制源码契约） |
| **文档同步** | ✅ | CONTEXT.md 加 v0.19.0 术语 5 个（TransportKind / StreamableHttpService / SessionManager / JoinSet / bind_addr）+ ADR-0027 §6 扩段 + stage-1/2.md 2 份 stage doc |
| **v0.20+ 候选方向** | ✅ | mcp-session-id-based identity / graceful shutdown / Auth Bearer / CORS / TLS / bollard prune_children / record worker 持续采样 / VT100 永久转码 CLI / options_for_version 改名 fixint_options / 主题 CEG（与 REVIEW-v0.18 §v0.19+ 候选方向延续） |

**总评**：v0.19 cycle 是 proc 历史上较小 cycle（~900 行总改动 vs v0.18 cycle ~820 行，同档轻量），3 stage 节奏紧凑。4 项基础设施补全全交付，P0 0 / P1 0 / P2 4，无 blocker。首次引入 axum + tower + tower-http web framework deps 是 v0.18 没有的复杂度，但与 rmcp 0.11 内部对齐无 duplicate。cycle 完整性良好，可 tag v0.19.0。

---

## v0.20+ 候选方向（详细评估留 v0.20 cycle brainstorm）

1. **mcp-session-id-based subscribe-push identity**（v0.19 stage 2 P2-S4 + P2-S5 + P2-V2 补全）：从 `RequestContext::extensions` 拿 `mcp-session-id` HTTP header（SSE 路径）作 client identity，让 subscribe dedup + unsubscribe precise removal 走 mcp-session-id 字符串作 secondary index
2. **graceful shutdown**（推迟 v0.20+ cycle，REVIEW-v0.18 P2-S3 + brainstorm §决策 6）：Ctrl+C 时 axum 主动关连接 + SubscribePushWorker::shutdown 实装主动停 push task
3. **Auth Bearer token**（推迟 v0.20+ cycle，brainstorm §决策 6）：生产部署时才需要，bind-addr 默认 `127.0.0.1` 已避免外网暴露
4. **CORS**（推迟 v0.20+ cycle，brainstorm §决策 6）：仅 Web dashboard 集成时才需要（浏览器 fetch 跨域），proc MCP server 主要 client 是 Claude Desktop / Cursor（非浏览器）
5. **TLS**（推迟 v0.20+ cycle，brainstorm §决策 6）：仅远程 agent 集成时才需要（公网传输），本机 + LAN 场景 bind-addr 已足够
6. **bollard prune_children 真正字段**（推迟 v0.20+ cycle，REVIEW-v0.18 §v0.19+ 第 3 项延续）：如 bollard 升级暴露或走 docker CLI 子进程路径
7. **record 暴露方案 (b) worker 持续采样路径评估**（推迟 v0.20+ cycle，REVIEW-v0.18 §v0.19+ 第 4 项延续）：如 spawn 子进程开销可感
8. **VT100 永久转码 CLI 子命令 `proc replay --convert <file>`**（推迟 v0.20+ cycle，REVIEW-v0.18 §v0.19+ 第 5 项延续）：如 agent 反馈多次转码开销可感
9. **`options_for_version` 改名 `fixint_options`**（推迟 v0.20+ cycle，REVIEW-v0.18 P2-V1）：让函数名语义更清晰（函数已不再按 version 分支）
10. **主题 C 跨平台扩展 cycle**（推迟 v0.20+ cycle，REVIEW-v0.18 §v0.19+ 第 9 项延续）：Linux/macOS 重新支持评估（与 v0.12 ADR-0022 Windows-only 决策可能翻盘）
11. **主题 E 插件系统 cycle**（推迟 v0.20+ cycle，REVIEW-v0.18 §v0.19+ 第 9 项延续）：让用户扩展 inspector tab / worker / scoring rule
12. **主题 G 分布式采集 cycle**（推迟 v0.20+ cycle，REVIEW-v0.18 §v0.19+ 第 9 项延续）：多机 proc 联合分析（与 brainstorm §主题 B 可观测性 cycle 同款方向延续）
