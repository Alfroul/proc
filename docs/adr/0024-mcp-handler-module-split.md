# ADR-0024：MCP handler.rs 单文件 → handler/ 子 module 拆分

**Status**：Accepted
**Date**：2026-07-06（v0.15.0 阶段 1 落地）
**Related**：ADR-0009（v0.7 MCP server 设计）

## 背景（Context）

proc v0.7.0 阶段 2 落地 MCP server 时，所有 17 个 `#[tool]` 方法 + 12 个 Args struct + 17 个 helper 函数都放在单文件 `src/mcp/handler.rs`（1156 行）。v0.6.0 阶段 5 已经历过类似问题（`src/main.rs` 1657 行 → `src/cli/` 子 module 14 个文件的拆分历史，参考 CONTEXT.md 演进历史 v0.6 阶段 6）。

v0.15 cycle 主题 D（MCP 全功能暴露）需要加 15 个新 tool（类别 1 CLI 9 个 + 类别 2 inspect 1 个含 6 tab + 类别 4 metrics 5 个）。**预期 handler.rs 涨到 ~2000+ 行**（1156 既有 + ~400 cat 1 + ~350 cat 2 + ~300 cat 4 业务代码 + ~150 测试辅助），违反单文件可读性原则：

- IDE 跳转成本：单文件 2000 行后 `#[tool]` 方法定位慢
- code review 噪音：stage 2 改 cat 1，diff 会混入 cat 4 的 import / Args 段
- 团队并行开发阻塞：cat 2 inspect 与 cat 4 metrics 是不同 Slice，应能并行 edit 不冲突

## 决策（Decision）

**拆 `src/mcp/handler.rs` → `src/mcp/handler/{mod.rs, cli.rs, inspect.rs, metrics.rs}`**：

```
src/mcp/
├── mod.rs                       (38 行，run_mcp_serve 入口，不动)
└── handler/
    ├── mod.rs                   (~1180 行，ProcMcpHandler struct + #[tool_router]
    │                             impl 含 32 个 #[tool] 方法 = 17 v0.7 既有 + 15
    │                             v0.15 新增 + ServerHandler impl + serve() +
    │                             list_tool_names() + 既有 17 helper + 公共 helper)
    ├── cli.rs                   (~80 行 stage 1 / ~480 行 stage 2，类别 1 Args +
    │                             helper，9 tool)
    ├── inspect.rs               (~50 行 stage 1 / ~400 行 stage 2，类别 2 Args +
    │                             InspectTab enum + helper，1 tool 含 6 tab)
    └── metrics.rs               (~50 行 stage 1 / ~350 行 stage 3，类别 4 Args +
                                 helper，5 tool)
```

### 关键约束：rmcp 0.11 `#[tool_router]` 不跨 module 收集

**调研结论**（context7 rmcp 官方文档，2026-07-06 验证）：

`#[tool_router]` 宏**只收集它所标注的 impl 块内**的 `#[tool]` 方法。子 module 里写 `impl ProcMcpHandler { #[tool] fn ... }` 块**不会被主 mod 的 tool_router 收集**——rmcp 编译期宏展开后，tool_router 只看当前 impl 块的 `#[tool]` 标注，不做跨 module 静态分析。

因此**所有 32 个 `#[tool]` 方法都保留在 `handler/mod.rs` 的单个 `impl ProcMcpHandler` 块里**。子 module 只放：
- Args struct（`#[derive(Deserialize, schemars::JsonSchema)]`）
- 业务 helper 函数（`pub fn make_xxx_json(...) -> Value`）

主 mod 的 `#[tool]` 方法体只 1-2 行：`ok_result(cli::make_flows_json(args.limit))`。

### 拆分边界（按 brainstorm §v0.15 cycle 模块重构 决策）

| 子 module | 类别 | tool 数 | 内容 |
|---|---|---|---|
| `cli.rs` | 1（CLI 命令暴露）+ 3（写操作） | 9 | `FlowsArgs` / `ThrottleArgs` / `ExportArgs` / `DockerEventsArgs` / `MonitorAddArgs` / `MonitorRemoveArgs` 等 Args + helper |
| `inspect.rs` | 2（详情页 Tab 合并） | 1（含 6 tab） | `ProcInspectArgs` + `InspectTab` enum + `make_inspect_json` helper |
| `metrics.rs` | 4（系统级 metrics） | 5 | `MetricsSystemArgs` / `MetricsGpuArgs` / `MetricsDiskIoArgs` / `MetricsSmartArgs` / `MetricsThermalArgs` + helper |
| `mod.rs` | 既有 17 tool | 17 + 15 stub = 32 | `ProcMcpHandler` struct + `#[tool_router] impl` 含全部 `#[tool]` 方法 + ServerHandler impl + serve() + 既有 helper |

## 备选方案（Alternatives）

### (a) 单文件保持，涨到 ~2000+ 行

**否决理由**：
- 违反单文件可读性原则（参考 v0.6 main.rs 拆分历史）
- IDE 跳转 + code review 噪音 + 并行开发阻塞（详见背景段）
- 与 v0.6 main.rs → cli/ 的拆分历史不一致（对称原则）

### (b) 用 `ToolRouter::add` 合并多个子 module 的 router

**部分否决**：rmcp 0.11 提供 `ToolRouter` impl `Add` trait（参考 context7 docs），允许 `router_a + router_b` 合并。每个子 module 独立 `#[tool_router] impl ProcMcpHandlerForCli { ... }` 块，主 mod 显式合并。

**否决理由**：
- boilerplate 多：每个子 module 要写独立 handler struct + trait impl + 合并代码
- 子 module handler struct 与主 `ProcMcpHandler` 字段共享（如 `dns_collector` Arc）需要繁琐的 Deref / AsRef 实现
- rmcp 0.11 的 `Add` impl 文档稀少，stage 1 Spike 验证成本高
- 主 mod.rs impl 块结构变化大（既有 17 tool 也要拆），违反 surgical 原则

### (c) 用 `SyncTool` / `AsyncTool` trait-based 模式

**否决理由**：
- 每个 tool 独立 struct impl `SyncTool` trait，boilerplate 最多（32 tool × ~30 行 = ~960 行 wrapper 代码）
- 与 v0.7 既有的 macro 模式不一致（既有 17 tool 用 `#[tool]` macro）
- trait 模式优势在 dynamic dispatch（如 plugin 系统），proc MCP 不需要

### (d) 把 32 个 `#[tool]` 方法都搬进子 module 的 `impl ProcMcpHandler` 块，主 mod 用 `use cli::*; use inspect::*;`

**否决**：rmcp 0.11 不收集子 module impl 块的 `#[tool]` 方法，编译失败（决策调研已验证）。

### (e) v0.7 既有 17 tool 也按类别拆到子 module

**否决**：违反 surgical 原则。既有 17 tool 没有「类别」划分（按 brainstorm 拆法，多数会落入 `cli.rs`），把它们物理迁移收益小（位置变了逻辑不变）+ 风险高（迁移过程可能漏 use 语句 / Args 漏 re-export）。stage 1 仅拆新 15 tool 的 Args + helper，既有 17 tool 留在 mod.rs。

## 结果（Consequences）

### 正面

- **可读性提升**：32 个 `#[tool]` 方法集中在 mod.rs 的 impl 块（业务分发层），每个 tool 体只 1-2 行；业务逻辑（cli.rs / inspect.rs / metrics.rs）独立可读
- **并行开发友好**：stage 2 改 cli.rs / inspect.rs，stage 3 改 metrics.rs，diff 不交叉
- **既有 17 tool 零回归**：impl 块结构不变（仅在末尾追加 15 个新 `#[tool]` 方法），v0.7 schema / 行为完全保留
- **未来扩展对称**：v0.16+ cycle 加新 tool 时按类别归入对应子 module，主 mod.rs impl 块只追加新 `#[tool]` 方法

### 负面

- **mod.rs 仍是单文件 ~1180 行**：32 个 `#[tool]` 方法必须集中在 impl 块，无法进一步拆。rmcp 0.11 限制决定，未来 rmcp 升级（如支持 cross-module collection）才能优化
- **子 module 文件互相依赖**：cli.rs 的 helper 可能需要 mod.rs 的公共 helper（如 `ok_result` / `err`），需 `use super::*;` 引入
- **use 语句膨胀**：mod.rs 顶部加 `mod cli; mod inspect; mod metrics;` + `use cli::*; use inspect::*; use metrics::*;`（+6 行）

### 中性

- **handler.rs → handler/mod.rs Git 历史**：`git mv` 显式 rename 让 Git 自动追踪（`git log --follow`），v0.7 以来的 commit history 不丢
- **CONTEXT.md 术语段**：加 `McpToolCategory` 描述子 module 边界（cli/inspect/metrics 3 类）

## Migration path

| 视角 | 影响 |
|---|---|
| MCP client（Claude Desktop / Cursor） | 零感知——tool list / schema / 行为完全不变 |
| proc 内部代码 | `crate::mcp::handler::ProcMcpHandler` 路径不变（mod.rs 替代 handler.rs 自动生效）；新增 `crate::mcp::handler::cli::FlowsArgs` 等子 module 路径 |
| 测试代码 | `tests/test_mcp_server.rs` 用 `use proc::mcp::handler;` 路径不变；`list_tool_names()` / helper 函数（`make_processes_json` 等）路径不变 |
| 贡献者 | 新加 v0.15+ tool 时：Args + helper 写到对应子 module（cli/inspect/metrics），`#[tool]` 方法追加到 mod.rs impl 块末尾 |

## 实装路径

- **stage 1（v0.15 Spike，本 ADR 落地）**：
  1. `git mv src/mcp/handler.rs src/mcp/handler/mod.rs`（保留 1156 行原样）
  2. 创建 `src/mcp/handler/{cli.rs, inspect.rs, metrics.rs}` 三个空骨架（含 `//!` doc comment）
  3. mod.rs 顶部加 `mod cli; mod inspect; mod metrics;` + `use cli::*; use inspect::*; use metrics::*;`
  4. 加 15 个新 `#[tool]` stub 方法（impl 块末尾追加，调子 module 的 stub helper）
  5. 子 module 各放 9 + 1 + 5 个 Args struct + stub helper
- **stage 2（v0.15 Slice）**：填 `cli.rs` + `inspect.rs` 业务逻辑（替换 stub helper）
- **stage 3（v0.15 Slice）**：填 `metrics.rs` 业务逻辑

## 相关 ADR

- **ADR-0009（v0.7 MCP server）**：本 ADR 是其文件结构演进的延续，工具暴露原则（thin wrapper / 字段裁剪）不变
- **ADR-0023（proc_inspect tool 合并）**：依赖本 ADR 的子 module 拆分（`inspect.rs` 文件落地）
- **v0.6 main.rs → cli/ 子模块拆分**（CONTEXT.md 演进历史 v0.6 阶段 6）：本 ADR 是同款 surgical 拆分原则的 MCP 路径应用
