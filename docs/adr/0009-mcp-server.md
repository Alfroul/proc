# ADR-0009: MCP Server via rmcp + stdio transport

## Status

**Accepted** — v0.7.0 阶段 2 引入

## Context

v0.6.0 的 proc 已经把 17+ 个 CLI 子命令（ls / tree / port / kill / dns / handles / docker ps / ...）做成了 thin CLI 调用同一套采集层。这些子命令天然是结构化的进程/网络/DNS 信息入口，但只能由人敲命令使用，LLM agent 无法直接调用。

2025-2026 年 MCP（Model Context Protocol）成为 LLM agent 接入外部能力的标准协议，已有 5 个进程管理类 MCP server（mcp-system-monitor / devops-mcp / process-mcp / mcp-process / @anonx3247/process-mcp），但都没 proc 的 14 项安全评分 + DNS 日志 + per-process 网络流量。

如果不暴露 MCP 接口，proc 在 LLM 运维生态里就是"只能给终端用户用"，错过了 agent 自动化场景（如 Claude Code 让 agent 查端口占用 + kill 进程 + 查 DNS 历史）。

## Decision

**用 `rmcp` 官方 Rust SDK（modelcontextprotocol/rust-sdk），第一版只做 stdio transport，每个 CLI 子命令映射为 `proc_<subcommand>` tool**。

具体决策：

1. **库选 rmcp**（不选 rust-mcp-sdk / prism-mcp-rs / 自写协议）
   - rmcp 是 MCP 协议官方维护，rust-code 在用，rust-mcp-stack/rust-mcp-sdk 是第三方
   - 官方意味着协议版本升级延迟最短

2. **stdio transport only**（第一版）
   - Claude Desktop / Cursor / 99% MCP client 默认用 stdio
   - HTTP/SSE 留 v0.8.0+ 评估（增加攻击面，单机不必要）

3. **tool 命名 `proc_<subcommand>`**（如 `proc_ls` / `proc_port` / `proc_kill`）
   - 命名空间清晰，LLM 看 schema 一眼知道是 proc 的 tool
   - 避免与其他 MCP server 冲突

4. **每个 tool 是 thin wrapper**：直接调 `crate::cli::run_subcommand` 同款函数
   - 不重写实现，复用 v0.6 已有 17+ 子命令
   - 输出格式不同：CLI 给人看（表格），MCP 给 LLM 看（JSON）

5. **不暴露的工具**：`proc_record` / `proc_replay` / `proc_export`
   - 这些是给人用录屏回放的，LLM 没意义

6. **异步 tokio runtime 隔离**
   - rmcp 用 tokio，proc 主线程是同步
   - MCP server 走独立子命令分支，`block_on` 启动 tokio runtime，不影响 TUI 路径

## Alternatives Considered

### A. rust-mcp-stack/rust-mcp-sdk（第三方 Rust MCP SDK）

**否决理由**：
- 非官方维护，协议升级可能滞后
- 236 个 code snippets vs rmcp 的 17000+（生态差距）
- rust-code（已知在用 rmcp）的反向佐证：成熟项目都选 rmcp

### B. 自写 MCP 协议（JSON-RPC over stdio）

**否决理由**：
- 协议本身不复杂，但 schema 生成 / tool dispatch / 错误处理 boilerplate 多
- 重复造轮子，且容易在协议版本升级时坏
- 没有官方 SDK 的 `#[tool_router]` / `#[tool_handler]` 宏便利

### C. HTTP/SSE transport（第一版就上）

**否决理由**：
- 增加 attack surface（需要认证、限流、CORS）
- 99% MCP client 默认 stdio，HTTP 没有立即价值
- 留 v0.8.0+ 评估

### D. 暴露所有 CLI 工具（包括 record / replay）

**否决理由**：
- 录屏回放对 LLM 无意义
- 反而让 tool list 冗长，LLM 选择困难

## Consequences

### 正面

- **生态卡位**：Claude Code / Cursor / Windsurf 等开箱即用
- **零成本扩展**：v0.7 阶段 6/7/8 落地后追加 `proc_psi` / `proc_throttle` / `proc_disk_io` / `proc_flows` 即可
- **复用 v0.6 实现**：thin wrapper 不重写业务逻辑
- **官方协议保证**：未来 MCP 新版本（V2026_X）升级阻力小

### 负面

- **依赖增加 ~600KB**（rmcp + schemars + async-trait）
- **tokio runtime 进入**：proc 主进程原本是同步，MCP 分支引入 tokio multi-thread runtime
- **17+ tool wrapper 的 boilerplate**：每个 tool ~50 行，总计 ~1000 行
- **测试复杂度**：MCP handler 是 async trait，集成测试需要 tokio runtime + in-process channel

### 缓解

- 包装 `cargo build --no-default-features` 仍编译通过（验证 rmcp 进 default features 后不破坏 cfg-gate）
- 用 `#[tool_router]` 宏压缩 boilerplate
- 集成测试用 rmcp 提供的 in-process channel（不真起 stdio）

## Implementation Notes

- 入口：`src/cli/def.rs` 加 `Command::Mcp { subcommand: McpSub }`，`McpSub::Serve`
- 主逻辑：`src/mcp/{mod.rs,handler.rs,tools.rs}`
- 测试：`tests/test_mcp_server.rs`（list_tools 数量 ≥ 17 + 调用 proc_ls 拿到 JSON）
- 用户接入 Claude Desktop：`~/.config/claude/claude-desktop-config.json` 加 `"proc": {"command": "proc", "args": ["mcp", "serve"]}`

## References

- [Model Context Protocol](https://modelcontextprotocol.io/)
- [rmcp 官方 Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)
- [rust-code（用 rmcp 的参考实现）](https://github.com/fortunto2/rust-code)
- proc v0.6.0 阶段 5 #6（main.rs 拆 cli/ 子模块的对称操作）
