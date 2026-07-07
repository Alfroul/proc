# ADR-0025a：proc_replay_search tool 的 agent 视角 schema 设计

**Status**：Accepted
**Date**：2026-07-07（v0.16.0 阶段 1 落地骨架，stage 2 Slice 实装业务逻辑）
**Related**：ADR-0009（v0.7 MCP server 字段裁剪原则）、ADR-0023（v0.15 inspect 合并设计）、ADR-0024（v0.15 handler 子 module 拆分）、ADR-0011（v0.6 FilterExpr 语言）

## 背景（Context）

proc v0.14 cycle 落地录屏 v2，其中 stage 3 落地 FilterExpr FrameField 5 维度扩展（[`src/replay/search.rs::ReplaySearch`](../../src/replay/search.rs)）：

- `: timestamp > 1759000123`（按时间戳过滤）
- `: cpu > 80`（按 CPU 占用过滤）
- `: mem > 4294967296`（按内存占用过滤）
- `: name =~ /chrome/i`（按进程名 regex 过滤）
- `: anomaly.severity == "warning"`（按异常严重度过滤）

5 维度都走 [`crate::filter::parse_frame`] + [`crate::filter::FilterExpr::apply_frame`] 路径（FrameEvalCtx 携带 `&UiFrame`）。substring 模式（无 `:` 前缀）走 [`crate::filter::build_frame_substring_expr`] 构造 `name =~ /<input>/i` 表达式（regex 元字符自动 escape 防 regex 注入）。

TUI 路径已稳定（v0.14 阶段 3 落地）：`/` 键激活搜索 → 输入即时 parse → timeline 高亮命中帧 → n/N 跳转。

v0.16 cycle 需要把这能力暴露给 LLM agent（brainstorm §v0.16 cycle 实际范围 §`proc_replay_search`）。agent 视角的 schema 设计候选：

- **方案 A**：一次性返所有命中帧（match_count 等于 matches.length）
- **方案 B**：分页 `offset + limit`，agent 多次调用拼装
- **方案 C**：仅 `limit` 截断 + `truncated` 字段标识是否还有更多（agent 想要更多时调高 limit 重试）

## 决策（Decision）

**选方案 C**：`limit: Option<usize>` 默认 **100** + 返回 `match_count`（总命中数）+ `returned`（实际返回数）+ `truncated: bool`（match_count > returned）+ 命中帧按 timestamp 升序截断。

```rust
#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReplaySearchArgs {
    pub file_path: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<usize>,
}
```

返回 JSON：
```json
{
  "ok": true,
  "query": "cpu > 80",
  "match_count": 234,
  "returned": 100,
  "truncated": true,
  "limit": 100,
  "matches": [
    { "frame_idx": 1234, "timestamp": 1759000123, "cpu_usage": 85.3,
      "memory_used": 4294967296, "matched_processes": ["chrome.exe"],
      "anomaly_severity": null },
    ...
  ]
}
```

### 关键设计点

1. **limit 默认 100**：与 `proc_flows`（默认 50）/ `proc_docker_events`（默认 100）同款数量级。100 帧足够覆盖大多数 agent 分析场景（典型命中数 < 100），过大的命中数（如 30min 高负载 session × `cpu > 50` 命中 5000+ 帧）让 agent 二次调高 limit。

2. **matches[] 字段裁剪**：每帧只返 6 字段（frame_idx / timestamp / cpu_usage / memory_used / matched_processes[] / anomaly_severity），不返完整 UiFrame（避免每帧 500+ bytes × 5000 = 2.5MB JSON 污染 LLM context）。matched_processes[] 是命中 `name =~ /xxx/` regex 的进程名列表（去重）；anomaly_severity 是该帧附带的异常等级（无异常时 null）。

3. **query 入口双路径**（与 v0.14 TUI ReplaySearch 同款契约）：
   - `: <FilterExpr>` 走 [`crate::filter::parse_frame`]（5 维度 FrameField）
   - 无 `:` 前缀走 [`crate::filter::build_frame_substring_expr`]（regex escape 元字符防注入）

4. **VT100 录屏不支持 search**：VT100 文件无结构化帧（只有 terminal escape 序列），返 `ok: false + error: "VT100 录屏不支持 search（仅 v3 UiFrame）"`。VT100 文件可调 `proc_replay_info` 查元数据，但不能 search。

5. **长录屏性能可接受**：30 min × 30 FPS × 165 µs/frame（[`bench_record_serialize`](../../benches/bench_record_serialize.rs) 实测）= ~9 秒一次性扫描。MCP 协议本身没强制 timeout（stdio transport 由 client 决定），主流 client（Claude Desktop / Cursor / mcp-inspector）默认 timeout 通常 30s+，9 秒可接受。agent 低频调用（不是热路径），与 brainstorm FAQ Q3 同款判断。

6. **截断时按 timestamp 升序**：与 timeline 视觉顺序一致，agent 拿到的前 100 帧是时间最早的 100 个命中（不是「随机 100 个」）。

## 备选方案（Alternatives）

### (a) 一次性返所有命中帧

**否决**：payload 过大风险。30 min × `cpu > 50` 命中 5000+ 帧 × ~500B/帧 = 2.5MB JSON，会让 LLM context 溢出（typical context window 100K-200K tokens，2.5MB JSON ≈ 500K+ tokens）。

### (b) 分页 `offset + limit`

**否决**：agent 多次调用复杂度过早优化。agent 视角更自然的交互是「拿到 100 帧看一眼，需要更多时调高 limit」而非「拿到第 1-100 帧后调 offset=100 拿 101-200」——后者需要 agent 维护游标状态，违反「MCP tool 无状态」原则（与既有 `proc_flows` / `proc_docker_events` 同款无状态契约）。

### (d) 异步路径 + progress notification

**否决**：rmcp 0.11 的 progress notification 文档稀少（context7 rmcp 官方文档 2026-07-07 验证），实装成本高 + LLM agent 端支持不确定。留 v0.17+ cycle 评估（与 brainstorm FAQ Q3 「未来考虑」段同款推迟理由）。

## 结果（Consequences）

- **agent 视角 schema 简单**：limit + truncated 两字段，无 offset 游标状态
- **与既有 tool 一致**：`proc_flows` / `proc_docker_events` 同款 limit 模式，agent 学一次用多次
- **长录屏性能可接受**：~9s/30min session，agent 低频调用不构成瓶颈
- **VT100 录屏的 agent 路径降级**：VT100 文件可调 `proc_replay_info` 查元数据，但不能 search——文档化在 description 字段让 agent 知道限制

### 负面（Trade-offs）

- **agent 想看 200+ 帧需二次调用**：典型场景不需要，但极端 case 下多一次 round-trip
- **truncated=true 时 agent 看不到所有命中**：agent 必须显式调高 limit 才能拿到全部（与 brainstorm §决策 5 「未来考虑 offset 分页」段评估推迟）
- **regex escape 限制 substring 灵活度**：用户输入 `chrome.exe` 时会自动 escape `.`，与「raw regex」语义不同——但有文档说明（description 字段）

## Migration path

- v0.14 TUI 路径（[`src/replay/search.rs::ReplaySearch`](../../src/replay/search.rs)）**不动**——MCP 路径独立 helper，TUI 路径仍走 `ReplaySearch` 状态机
- v0.16 stage 2 在 [`crate::mcp::handler::record`]（[`src/mcp/handler/record.rs`](../../src/mcp/handler/record.rs)）实装 `make_replay_search_json(file_path, query, limit)` 业务逻辑，调 [`crate::filter::parse_frame`] / [`crate::filter::build_frame_substring_expr`] + `Player::frame_at` 遍历
- v0.16 stage 1（本 ADR 落地时）`make_replay_search_json` 是 stub（返 placeholder JSON），见 [`src/mcp/handler/record.rs`](../../src/mcp/handler/record.rs)

## 相关 ADR / 文档

- [ADR-0009](0009-mcp-server.md)：v0.7 MCP server 字段裁剪原则（agent 视角「拿必要字段」）
- [ADR-0011](0011-filter-expression.md)：v0.6 FilterExpr 语言设计（含 FrameField 5 维度 v0.14 扩展）
- [ADR-0023](0023-mcp-inspect-tool-merge.md)：v0.15 inspect tool 合并设计（agent 视角 schema 设计参考）
- [ADR-0024](0024-mcp-handler-module-split.md)：v0.15 handler 子 module 拆分决策（v0.16 record.rs 子 module 延续）
- [`docs/stages/v0.16-brainstorm.md`](../stages/v0.16-brainstorm.md) §决策 5（limit 默认 100 + truncated）
- [`docs/stages/v0.16-stage-2.md`](../stages/v0.16-stage-2.md)（stage 2 业务逻辑实装任务清单，本 ADR 落地后由 stage 1 启动指令包生成）
