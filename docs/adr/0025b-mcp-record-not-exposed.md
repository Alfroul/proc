# ADR-0025b：v0.16 cycle 决策不暴露 proc_record_start / proc_record_stop

**Status**：Accepted
**Date**：2026-07-07（v0.16.0 阶段 1 落地决策）
**Related**：ADR-0009（v0.7 MCP server 设计）、ADR-0024（v0.15 handler 子 module 拆分）、[ADR-0025a](0025a-mcp-replay-search-agent-schema.md)（v0.16 stage 1 同期 ADR）

## 背景（Context）

v0.15 brainstorm §主题 D2 锁定方向：「D2 操作 + 录屏类（v0.16）：写操作（kill / docker-rm / monitor-add-remove）+ 录屏 v2（record / replay / bookmarks）」。其中「record」一词存在歧义：

- **解读 A（启动新录屏）**：MCP tool 主动 spawn 录屏 worker，从「现在」开始记录用户 / 系统行为
- **解读 B（操作已录制的文件）**：所有 tool 都作用于**已存在**的 `.prec` 文件（replay 查元数据 / search 命中帧 / bookmarks CRUD）

proc v0.6 落地的录屏能力（[`src/cli/record.rs::run_record`](../../src/cli/record.rs)）走 TUI 路径：用户按 `R` 键启动录制 → `tui::setup_terminal() + tui::run_app()` 接管终端 → `Recorder::submit_frame` 喂 UiFrame（30 FPS）→ 按 `q` 键停止 + 写 `.prec` 文件。

MCP server 走 stdio 单工协议（client ↔ server 通过 stdin/stdout 交换 JSON-RPC），**stdio 无 TTY**——这是技术硬约束。

## 决策（Decision）

**v0.16 cycle 不暴露 `proc_record_start` / `proc_record_stop`**。7 tool 范围只覆盖：
- Replay 类别（2 tool）：`proc_replay_info` / `proc_replay_search`（操作已存在文件）
- Bookmarks 类别（4 tool）：`proc_bookmarks_list` / `add` / `edit` / `delete`（操作已存在 sidecar）
- UsbStatus 类别（1 tool）：`proc_eject_status`（用户 2026-07-07 追加，与录屏无关）

brainstorm §主题 D2 的「record」按**解读 B** 理解：所有录屏相关 tool 都作用于已录制文件，agent 视角负责「分析已有录屏」，用户视角负责「主动录屏」。

## 理由

### 1. 录屏需要 TTY（技术硬约束）

MCP server 是 stdio 单工无 TTY。proc 录屏走 TUI 路径（[`src/cli/record.rs::run_record`](../../src/cli/record.rs)）依赖：
- `crossterm::terminal::enable_raw_mode()` —— 需要 TTY
- `crossterm::execute!(stdout, EnterAlternateScreen)` —— 需要 TTY
- `EventStream::new()`（键盘事件）—— 需要 TTY

stdio 路径下这些都不可用，无法直接 attach TUI 录屏。

### 2. 替代路径（worker 持续采样）工作量大且收益边际

理论替代：MCP handler 启 worker 线程采 `SystemSnapshot → UiFrame` 喂 `Recorder::submit_frame`（绕过 TUI 路径）。但：

- **worker 持续运行成本**：30 FPS × 165 µs/frame（[`bench_record_serialize`](../../benches/bench_record_serialize.rs) 实测）= ~5% CPU 持续占用，影响 MCP server 主路径响应延迟
- **生命周期管理复杂**：recorder_worker 与 MCP stdio 生命周期解耦——client 关闭后 worker 仍跑？什么时候停止？写文件失败如何反馈给已经断连的 client？
- **agent 实际场景**：agent 需要分析「过去发生的」事件（用户已录制的 `.prec` 文件），而非「主动启动录屏等待事件发生」——后者与 proc TUI 路径冲突（用户已经在 TUI 里看实时数据，agent 同时启 worker 重复采）
- **重复实现**：proc 已有 `App::workers.heavy_worker`（2s 周期采 `SystemSnapshot`）+ `R` 键录制路径，再加 MCP worker 等于第三套采样路径

### 3. 安全 / confirm 机制待评估

v0.6 落地的 `pending_record_confirm` 确认弹窗（[`src/record/mod.rs`](../../src/record/)）让用户在启动录屏前明确同意「即将记录终端内容（可能含 secret）」。MCP 路径需要 MCP-level confirm 机制让用户授权 agent 启动录屏——但 rmcp 0.11 的 sampling / elicitation 机制文档稀少（context7 rmcp 官方文档 2026-07-07 验证），需要单独 cycle 评估。

与 brainstorm FAQ Q2 「写操作 confirm 机制」、§主题 D2 「docker-rm 写操作」一并留 v0.17+ cycle 评估。

### 4. agent 视角与用户视角分工

| 视角 | 责任 | 工具 |
|---|---|---|
| **用户视角** | 主动在 TUI 启动录屏（`R` 键 + `b` 键加书签 + `q` 键停止） | TUI 路径，不暴露给 MCP |
| **agent 视角** | 分析已存在的 `.prec` 文件（查元数据 / 搜索命中帧 / 操作书签） | MCP 7 tool（v0.16 cycle） |

这是更合理的协作分工——用户在终端「现场」，agent 在「事后分析」。用户主动决定「这段值得录」，agent 帮忙分析已录制内容。

## 备选方案（Alternatives）

### (a) 暴露 record 走 TUI 子进程

**否决**：MCP stdio 无 TTY，子进程 spawn TUI 会立即失败（`enable_raw_mode` 返回错误）。即使绕过 TTY 检查，TUI 输出会污染 MCP stdio（terminal escape 序列混入 JSON-RPC 流）。

### (b) 暴露 record 走 worker 持续采样

**否决**：理由 2 详述（成本 + 复杂度 + 收益边际）。

### (c) 推迟到 v0.17+ cycle 评估（**本 ADR 选此**）

**接受**：v0.16 cycle 聚焦 replay / bookmarks / usb_status 7 tool，cycle 内聚强。v0.17+ cycle 评估：
- 方案 (a) 变体：spawn `proc record` 子进程（无 TTY，子进程独立写 `.prec`）—— 但子进程无 TUI 输入，需要 stdin 命令控制（start / stop / bookmark）
- 方案 (b) 变体：MCP-level confirm + worker 持续采样路径（与 brainstorm FAQ Q2 confirm 机制一并评估）
- 方案 (d)：rmcp 0.11+ elicitation / sampling 机制成熟后评估

## 结果（Consequences）

- **v0.16 cycle 7 tool 范围明确**：6 录屏 v2（操作已存在文件）+ 1 USB status，cycle 内聚强
- **agent 与用户分工清晰**：用户主动录屏，agent 分析已录内容
- **MCP 实现路径简单**：所有录屏 tool 都是「打开 .prec 文件 → 读 / 操作」，无 worker / 无生命周期管理
- **未来评估路径明确**：v0.17+ cycle 候选方案 (a) / (b) / (d) 已记录，等用户反馈驱动优先级

### 负面（Trade-offs）

- **agent 无法主动录屏**：用户必须先在 TUI 录屏，agent 才能分析——若用户希望 agent 「定时录屏」或「事件触发录屏」（如 CPU 飙升时自动启录），需要 v0.17+ cycle 评估方案 (b) worker 路径
- **录屏敏感数据保护机制待评估**：MCP-level confirm 机制缺失，agent 启动录屏会绕过用户的 `pending_record_confirm` 确认——v0.17+ cycle 必须先解决 confirm 才能暴露 record

## Migration path

- **v0.16 cycle**（本 ADR 落地）：7 tool 范围不含 record，brainstorm §决策 1 文档化此决策
- **v0.17+ cycle**：评估方案 (a) / (b) / (d)，根据用户反馈驱动优先级；若决定暴露 record，先评估 confirm 机制（与 brainstorm FAQ Q2 一并处理）

## 相关 ADR / 文档

- [ADR-0009](0009-mcp-server.md)：v0.7 MCP server 设计（agent 视角字段裁剪原则延续）
- [ADR-0024](0024-mcp-handler-module-split.md)：v0.15 handler 子 module 拆分（v0.16 record.rs 子 module 容器延续）
- [ADR-0025a](0025a-mcp-replay-search-agent-schema.md)：v0.16 stage 1 同期 ADR（agent 视角 schema 设计参考）
- [`docs/stages/v0.16-brainstorm.md`](../stages/v0.16-brainstorm.md) §决策 1（record 不暴露理由段）+ §FAQ Q1（v0.16 cycle 范围排除 docker-rm 写操作同款推迟理由）
