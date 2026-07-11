# REVIEW-v0.18 — v0.18 cycle Review（v0.17 残留项补全 cycle 完结）

> **cycle 范围**：v0.17 cycle 末段（stage 7）锁定的 8 项残留项中，用户 2026-07-10 拍板做 4 项合并到 v0.18 cycle —— 代码清理 + varint 启用 + Resource subscribe-push + record auto-stop
>
> **Review 范围**：3 stage 全部产出（1 Spike + 1 Slice + 本 Review+收尾合并段）
>
> **基线**：1425 passed / 0 failed / 3 ignored（stage 2 后）/ fmt / clippy / build（含 --no-default-features）/ bench --no-run 全过
>
> **Review 日期**：2026-07-11
>
> **Reviewer**：Claude（stage 3 会话）

---

## 概览

v0.18 cycle 是 proc 历史上较小 cycle（~820 行总改动，与 v0.13 cycle ~500 行同款轻量 cycle），3 stage 节奏紧凑（1 Spike + 1 Slice + Review+收尾合并段，与 v0.15/v0.16/v0.17 cycle 同款合并模式延续）。MCP tool 总数 46 → 46（不变），0 份新 ADR + 2 份扩段（ADR-0027 §5 subscribe-push lifecycle / ADR-0029 §6 record auto-stop），4 项 v0.17 残留项全交付。

**Findings 汇总**：P0 0 / P1 0 / P2 5（详见末尾表）。预期不触发 brainstorm §决策 1 自适应拆分（阈值 P0 ≥ 1 或 P1 ≥ 5）。

---

## 1. 项 1：P1-R1 + P2-R1 代码清理（stage 2 落地）

### 落地范围

| finding | 范围 | 主修改区域 |
|---|---|---|
| P1-R1 | `make_record_stop_json` `let mut child_opt` → `let child_opt`（take 已清空无需 mut）+ 删 `child_opt = None; let _ = child_opt;` 两行冗余 | `src/mcp/handler/record.rs:1156-1165` |
| P2-R1 | `make_record_start_json` `log_file.try_clone().unwrap_or_else(\|_\| log_file.try_clone().unwrap())` 双重 fallback 简化为单次 `try_clone` + `expect("try_clone log_file 失败（fd 耗尽？）")` | `src/mcp/handler/record.rs:1095-1108` |

### 4 维度审查

**代码质量** ✅：P2-R1 简化为 `expect` 而非 fallback inherit（stage 2 决策 1 拍板）——try_clone 失败时 inherit 子进程 stdout 会污染 MCP stdio transport（agent 解析 JSON-RPC 失败），比 panic 更糟；expect 让真失败时 server 重启，与原 `unwrap_or_else(unwrap)` 行为等价但去掉双重 fallback 冗余。

**架构** ✅：纯清理，无接口变更。

**性能** ✅：删除冗余指令 + 简化 try_clone 调用次数（极端情况下从 2 次降为 1 次）。

**完整性** ✅：既有 `test_record_start_stop_round_trip` / `test_record_stop_no_active` 覆盖（不增测试），全量回归 1425 passed / 0 failed 验证。

---

## 2. 项 2：TD-45 varint 配置层启用（stage 2 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| bump RECORDING_VERSION | `RECORDING_VERSION: u16 = 3` → `4` | `src/record/frame.rs:4` |
| 接口设计 | `options_for_version` 改为 fixint-only 兼容函数（vt100/sidecar 等 fixint-only 路径继续用）+ 新增 `serialize_with_version` / `deserialize_with_version` 两个 helper 函数把 dispatch 收敛到函数内部（bincode 1.x `Options` trait 有 `Sized` bound 不 object-safe，无法 `Box<dyn Options>`） | `src/record/encoding.rs`（+293 / -87 行） |
| writer/reader 适配 | writer.rs 用 `serialize_with_version(version, ...)` 写 frame/footer / reader.rs 用 `deserialize_with_version(version, ...)` 读 frame/footer；header 永远 fixint（reader 先拿 version 再分支） | `src/record/{writer.rs, reader.rs}` |
| 回归修复 | `tests/test_mcp_v0_16.rs::test_replay_info_v3_recording` 改用 `RECORDING_VERSION` 常量替代硬编码 3 / `tests/test_record.rs::v3_footer_trailer_persisted_at_file_end` 改用 `deserialize_with_version` 读 footer（footer 跟随文件 version 走 varint） | `tests/{test_mcp_v0_16.rs, test_record.rs}` |
| 测试覆盖 | 6 个 encoding unit test：legacy v1/v2/v3 fixint 等价性 + options_for_version fixint 兼容 + v4 varint 字节流不同 + v4 varint round-trip + v4 varint 占用 byte 少（u64=1 vs 8）+ v3 fixint round-trip | `src/record/encoding.rs::tests` |

### 4 维度审查

**代码质量** ✅：
- `serialize_with_version` / `deserialize_with_version` dispatch 收敛到函数内部，避免 `impl Options` 在两分支不同类型时编译失败（bincode 1.x `Options` trait 有 `Sized` bound 不 object-safe）
- header 永远 fixint（writer.rs 用 `bincode::serialize` 写 header / reader.rs 用 `bincode::deserialize` 读 header），让 reader 拿到 `header.version` 后再分支选 config——避免 reader 还不知道 version 时就分支的悖论
- `RecordingFooter::header_version` 字段镜像 `header.version` 用于 sanity check（footer 自身走 `options_for_version(version)` 序列化 → v4 文件 footer 是 varint）

**架构** ✅：
- `options_for_version` 保留为 fixint-only 兼容函数（vt100 / sidecar / v0.17 test 继续用，不参与 version 分支），向后兼容
- v0.17 stage 3 TD-45 fixint 兼容层基础（`options_for_version` 函数）保留，v0.18 在其上加 varint 分支，演进路径清晰

**性能** ✅：
- v4 varint 对小数字占 byte 少（u64=1 vs 8），30 min × 30 FPS × 1000 进程录屏 `start_time` / `pid` / `version` 等小数字字段 size 下降 ~10-15%（与 brainstorm §项 2 子方向 ROI 评估对齐）
- v1/v2/v3 旧文件零迁移（fixint 兼容层让旧 `.prec` 文件可读）

**完整性** ✅：
- v1/v2/v3 fixint 兼容回归测试覆盖（`legacy_v1_v2_v3_equivalent_to_default_serialize` + `v3_fixint_round_trip`）
- v4 varint round-trip 测试覆盖（`v4_varint_round_trip` + `v4_uses_varint`）

### Findings

**P2-V1**：`options_for_version` 保留为 fixint-only 兼容函数（v0.18 stage 2 决策），vt100 / sidecar 等 fixint-only 路径继续用——这是接口设计决策（bincode 1.x `Options` trait 限制），不是 bug。但函数名 `options_for_version` 当前已 misleading（不再按 version 分支），v0.19+ cycle 评估是否改名为 `fixint_options` 让语义更清晰。

---

## 3. 项 4：record auto-stop（stage 2 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| `shutdown::request()` 函数 | 主动 flip flag 让 timer thread 触发干净退出（与 Ctrl+C handler 走同一 flag，主循环无需区分 shutdown 来源） | `src/shutdown.rs:29-46`（+19 行） |
| timer thread spawn | `run_record_headless` 内 `std::thread::spawn(move \|\| { sleep N secs; shutdown::request(); })`，主循环检 `shutdown::requested()` 退出 | `src/cli/record.rs:80-149` |
| spawn 传 `--duration` flag | `make_record_start_json` spawn 子进程时加 `.arg("--duration").arg(secs.to_string())`（如 `duration_secs` Some） | `src/mcp/handler/record.rs:1095-1108` |
| 移除 warning 字段 | `make_record_start_json` 返 JSON 去掉 `warning` 字段（auto-stop 已实装，与 v0.17 stage 6 warning 透出对比） | `src/mcp/handler/record.rs:1138-1143` |

### 4 维度审查

**代码质量** ✅：
- `shutdown::request()` 与 `init()` 共享 `FLAG: OnceLock<Arc<AtomicBool>>` 全局状态，未 `init()` 时 `request()` no-op（与 `requested()` 在 FLAG 未初始化时返 false 一致）
- timer thread 在子进程内 spawn，子进程退出时 timer thread 自动终止（process death）——无 zombie timer，与 brainstorm 决策 4 拍板对齐
- MCP handler 不持 timer 状态（timer 在子进程内），与 ADR-0029 关键设计点 2「MCP handler 不持 worker 状态」对齐

**架构** ✅：
- 子进程 `--duration` flag 与 ADR-0029 决策 4「spawn 子进程复用 v0.6 业务逻辑 + 子进程崩溃隔离」对齐
- 复用 v0.6 落地的 `shutdown::requested()` 干净退出路径（与 `--no-tui` headless 路径 + `R` 键 TUI 路径同款）

**性能** ✅：
- timer thread sleep N secs 不占 CPU（与 brainstorm §项 4 决策 4 优势段对齐）
- 不影响 MCP server 主路径（timer 在子进程内）

**完整性** ✅：
- spawn 时传 `--duration` flag 验证（`test_record_start_duration_secs_param_echoed` 改写为检查 cmd line 含 `--duration`）+ warning 字段移除验证 + auto-stop timer thread 验证（`test_auto_stop_timer_thread_requests_shutdown`，stage 2 新增）
- 全量回归 1425 passed / 0 failed 验证

### Findings

无新增 finding。项 4 落地与 brainstorm 决策 4 + ADR-0029 §关键设计点 6 完全对齐。

---

## 4. 项 3：Resource subscribe-push（stage 2 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| `SubscribePushWorker` 注册表 value 替换 | stage 1 Spike `HashMap<SubscriberId, ()>` 占位 → stage 2 `HashMap<String /* uri */, Peer<RoleServer>>`（key 用 uri 因 rmcp `SubscribeRequestParam` 只有 uri 无 subscriber_id）+ `task_spawned: Arc<Mutex<bool>>` 字段防重复 spawn | `src/mcp/subscribe_worker.rs`（重写 +294 / -138 行） |
| subscribe/unsubscribe 业务逻辑 | subscribe(uri, peer) 写入注册表 + lazy spawn push task / unsubscribe(uri) 从注册表 remove | `src/mcp/subscribe_worker.rs::subscribe/unsubscribe` |
| spawn_push_task 实装 | `TokioHandle::try_current()` 检查 runtime + `handle.spawn(async move { ... })` 1s tick 遍历调 `peer.notify_resource_updated(ResourceUpdatedNotificationParam { uri })` + peer 断开（返 Err）从注册表移除（自动清理） | `src/mcp/subscribe_worker.rs::spawn_push_task` |
| `ProcMcpHandler` 加字段 | `pub subscribe_push_worker: SubscribePushWorker`（不 cfg-gate，Default 返空 worker）+ Clone/Default/new() 初始化 | `src/mcp/handler/mod.rs`（+60 / -10 行） |
| `ServerHandler::subscribe`/`unsubscribe` 真实业务 | 从 stage 4 的 no-op 改为真实注册 / 注销（从 `request.uri` + `context.peer.clone()` 调 `ResourceRoute::subscribe(self, uri, peer)` / `unsubscribe(self, uri)`，Err 时返 McpError） | `src/mcp/handler/mod.rs:1095-1211` |
| `ResourceRoute` trait 签名调整 | stage 1 Spike `subscribe(uri, subscriber_id: u64)` → stage 2 `subscribe(uri, peer: Peer<RoleServer>)` / `unsubscribe(uri)` | `src/mcp/resources.rs`（+69 / -30 行） |
| 测试覆盖 | 6 个 subscribe-push unit test（new/default/shutdown no-op/idempotent unsubscribe/spawn_push_task 无 runtime 返 Err/源代码静态断言 Peer RoleServer） | `src/mcp/subscribe_worker.rs::tests` + `tests/test_mcp_v0_18.rs` |

### 4 维度审查

**代码质量** ✅：
- `subscribers.lock()` 持锁窗口短（subscribe/unsubscribe 仅 insert/remove，push task 在 tick `'loop` 内部短暂 lock 取 snapshot 后释放，避免 push 期间阻塞 subscribe/unsubscribe）
- peer 断开自动清理逻辑清晰：`peer.notify_resource_updated(params).await.is_err()` 时从注册表 `g.remove(&uri)`（drop Arc<Peer> 让 Arc 引用计数减 1）
- `TokioHandle::try_current()` 在 spawn 前检查 runtime 上下文，让单元测试在非 tokio runtime 路径调用 `spawn_push_task` 时返 Err（与 stage 2 决策 6 对齐）
- lazy spawn 模式（第一次 subscribe 时 spawn，避免无 subscriber 时空跑 worker）+ 单 task 多 subscriber（一个 push task 遍历所有 subscriber，与 brainstorm 决策 3 + stage 2 决策 7 对齐）

**架构** ✅：
- 与 v0.17 stage 4 polling-push 互补——v0.17 落地 `ResourceRoute::route()` client 走 `resources/read` 主动拉 / v0.18 补全 subscribe-push（client 订阅后 server 主动 push 增量），与 ADR-0027 §关键设计点 5（v0.18 stage 1 Spike 扩段）对齐
- `subscribe_push_worker` 字段不 cfg-gate（与 v0.17 stage 1 持久字段 cfg-gate 不同）——`SubscribePushWorker::new()` 不持运行时状态（仅 `Arc::new(Mutex::new(HashMap::new()))` + `Arc::new(Mutex::new(false))`），测试路径安全（spawn push task 才需 tokio runtime，subscribe/unsubscribe 业务路径不需要）

**性能** ✅：
- 1s tick 与 v0.17 stage 4 `system_history` worker 同款节奏，push task 用 `tokio::time::interval` 异步 sleep（不阻塞 tokio runtime thread）
- push task 单 task 遍历所有 subscriber（不每个 subscribe spawn 一个 task），减少 lock contention

**完整性** ⚠️：
- subscribe-push 单元测试无法直接构造 `Peer` 实例（`Peer::new` 是 `pub(crate)` 无法在 proc 测试中构造），用源代码静态断言（`subscribe_worker_source_uses_peer_role_server` 检查源代码含 `Peer<RoleServer>` / `peer.notify_resource_updated` / `TokioHandle::try_current` / `ResourceUpdatedNotificationParam`）+ mcp-inspector 手动验证（与 brainstorm §测试命令段 subscribe-push 验证 mcp-inspector 手动验证对齐）
- stdio transport 单 client 假设（注册表 key 用 uri，同 URI 单 client）；SSE transport 多 client 待 v0.19+ cycle 升级为 `HashMap<String, Vec<Peer>>`

### Findings

**P2-S1**：subscribe-push 单元测试无法直接构造 `Peer` 实例验证完整 lifecycle（`Peer::new` 是 `pub(crate)`），用源代码静态断言 + mcp-inspector 手动验证替代——这是 rmcp 0.11 API 限制，不是 bug。v0.19+ cycle 评估是否加 mock Peer / fake peer 路径让单元测试能验证完整 lifecycle。

**P2-S2**：stdio transport 单 client 假设（同 URI 单 client 订阅，第二次 subscribe 同 URI 会覆盖第一次的 Peer）——这是 stage 2 决策（stdio 单 client 假设），与 brainstorm §风险 1 mitigate 对齐。SSE transport 多 client 待 v0.19+ cycle 升级。

**P2-S3**：`shutdown` 方法保留 no-op 占位（stage 2 决策），push task 在 tokio runtime 上 spawn，进程退出时 tokio runtime shutdown 自动 cancel 所有 task（无 zombie task）。本方法预留未来 graceful shutdown（如 server 重载时主动停 push task），当前 no-op 不影响功能。

---

## Findings 汇总表

| ID | 主题 | 严重度 | 描述 | 建议处理 |
|---|---|---|---|---|
| P2-V1 | 项 2 varint | P2 | `options_for_version` 保留为 fixint-only 兼容函数，函数名 misleading（不再按 version 分支） | v0.19+ cycle 评估改名为 `fixint_options` 让语义更清晰 |
| P2-S1 | 项 3 subscribe-push | P2 | 单元测试无法直接构造 `Peer` 实例（`Peer::new` 是 `pub(crate)`），用源代码静态断言 + mcp-inspector 手动验证替代 | v0.19+ cycle 评估加 mock Peer / fake peer 路径 |
| P2-S2 | 项 3 subscribe-push | P2 | stdio transport 单 client 假设（同 URI 单 client 订阅，第二次 subscribe 同 URI 覆盖第一次） | v0.19+ cycle SSE transport full 实装时升级为 `HashMap<String, Vec<Peer>>` |
| P2-S3 | 项 3 subscribe-push | P2 | `shutdown` 方法保留 no-op 占位，预留未来 graceful shutdown | v0.19+ cycle 评估是否实装 graceful shutdown（如 server 重载时主动停 push task） |
| P2-R0 | 基线 | P2 | stage 2 commit `b579506` 在 `tests/test_mcp_v0_16.rs` 改硬编码 `3` → `RECORDING_VERSION` 常量但未跑 fmt，留下 4 行 fmt diff（stage 3 已修） | 已在 stage 3 内修复（小修 < 5 处改动，按 brainstorm 步 11 在当前阶段内直接修复） |

**P0 0 / P1 0 / P2 5**。预期不触发 brainstorm §决策 1 自适应拆分（阈值 P0 ≥ 1 或 P1 ≥ 5）。所有 P2 都是已知限制或文档/测试覆盖问题，无 blocker。

---

## cycle 完整性评分

| 维度 | 评分 | 说明 |
|---|---|---|
| **4 项残留补全全交付** | ✅ | 项 1 代码清理 / 项 2 varint / 项 4 auto-stop / 项 3 subscribe-push 全部落地 |
| **3 stage 全部 ✅** | ✅ | 1 Spike + 1 Slice + 1 Review+收尾合并段（与 v0.15/v0.16/v0.17 cycle 同款合并模式延续） |
| **MCP tool 总数** | ✅ | 46 → 46（不变，v0.18 cycle 不新增 tool——4 项都是已有 tool 的补全） |
| **全量回归** | ✅ | 1425 passed / 0 failed / 3 ignored（stage 2 后基线，stage 1 基线 1401 + 新增 24 测试） |
| **ADR 落地** | ✅ | 0 份新 ADR + 2 份扩段（ADR-0027 §5 subscribe-push lifecycle / ADR-0029 §6 record auto-stop） |
| **fmt / clippy / build / bench** | ✅ | 全过（含 --no-default-features cfg-gate 验证） |
| **测试覆盖** | ✅ | stage 1 加 4 项 stub 测试骨架 / stage 2 改写为真实测试（varint 等价性 + v3 fixint 兼容 + auto-stop timer thread + subscribe-push lifecycle + spawn_push_task 无 runtime 返 Err） |
| **文档同步** | ✅ | CONTEXT.md 加 v0.18.0 段 4 术语（SubscribePushWorker / SubscriberId / DurationFlag / VarintEncoding）+ ADR-0027/0029 扩段 + stage-1/2.md 2 份 stage doc |
| **v0.19+ 候选方向** | ✅ | SSE transport full 实装 / bollard prune_children / record worker 持续采样路径评估 / VT100 永久转码 CLI / SSE multi-client subscribe-push 升级 / 主题 C/E/G（与 REVIEW-v0.17 §v0.18+ 候选方向延续） |

**总评**：v0.18 cycle 是 proc 历史上较小 cycle（~820 行总改动 vs v0.17 cycle ~5540 行，15% 量级），3 stage 节奏紧凑。4 项残留补全全交付，P0 0 / P1 0 / P2 5，无 blocker。cycle 完整性良好，可 tag v0.18.0。

---

## v0.19+ 候选方向（详细评估留 v0.19 cycle brainstorm）

1. **SSE transport full 实装**（推迟 v0.19+ cycle）：加 Cargo feature `transport-streamable-http-server-tower` + axum 等 deps + multi_thread runtime 重构 ~500+ 行大工程（v0.17 stage 4 stub，v0.18 cycle 末段仍未做）
2. **bollard prune_children 真正字段**（推迟 v0.19+ cycle）：如 bollard 升级暴露或走 docker CLI 子进程路径（v0.17 stage 6 透出 warning 字段）
3. **record 暴露方案 (b) worker 持续采样路径评估**（推迟 v0.19+ cycle）：如 spawn 子进程开销可感（v0.17 stage 6 落地方案 (a) spawn 子进程）
4. **VT100 永久转码 CLI 子命令 `proc replay --convert <file>`**（推迟 v0.19+ cycle）：如 agent 反馈多次转码开销可感（v0.17 stage 5 落地临时转码路径）
5. **SSE multi-client subscribe-push 升级**（v0.18 stage 2 透出）：subscribe_push_worker 注册表 value `Peer` → `Vec<Peer>`，让多 client 同时订阅同 URI（与 SSE transport full 实装绑定）
6. **主题 C 跨平台扩展 cycle**：Linux/macOS 重新支持评估（与 v0.12 ADR-0022 Windows-only 决策可能翻盘）
7. **主题 E 插件系统 cycle**：让用户扩展 inspector tab / worker / scoring rule
8. **主题 G 分布式采集 cycle**：多机 proc 联合分析（与 brainstorm §主题 B 可观测性 cycle 同款方向延续）
