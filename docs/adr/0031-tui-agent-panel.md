# ADR-0031：TUI AgentPanel + AgentSession 流式会话架构

**Status**: Accepted（v0.21 stage 1 Spike 落地骨架；stage 2/3 实装后扩业务段）

**Date**: 2026-08-18（v0.21 cycle stage 1）

**Related**: ADR-0030（内置 agent 基座，本 ADR 是其 UI 消费层）、ADR-0008（self-mitigation policy，写操作 confirm gate）、ADR-0029（record 暴露 + confirm 机制，`confirm: bool` 契约复用）、ADR-0012（PanelController 模式）、ADR-0010（command palette）、ADR-0019（worker 生命周期，session 线程 + channel 模式参考）

## Context

v0.20 cycle 落地了内置 AI agent 基座（ADR-0030，tag v0.20.0），留下三项与 UI 消费相关的事实：

1. **`stream()` 三 provider 全实装但零消费方**——llama-cpp SSE / anthropic SSE / mock 回放三路齐备，唯一消费是测试断言；UI 层（TUI）尚未接入。
2. **CLI ask 非交互无确认通道**——dispatch 层拦截 8 个写 tool（`WRITE_TOOL_NAMES`：proc_kill / proc_pkill / proc_usb_release / proc_docker_rm / proc_docker_image_rm / proc_docker_volume_rm / proc_record_start / proc_record_stop），返 blocked JSON 让模型转向解释。TUI 面板恰好能提供 y/n 确认。
3. **AgentRunner 是 complete-only 单轮**——`run_with_progress` 的 `StepEvent`（LlmTurn / ToolStart）事件面已就绪，但无流式文本增量、无跨 query 的 conversation history。

proc 的主入口是 TUI（`AppMode` 10 变体 + `PanelController` 模式成熟）。而 TUI 主循环是**同步 crossterm 事件循环**，agent 是 **async（tokio + reqwest SSE）**——直接在主循环 `block_on` 会卡死 UI（v0.20 风险 6 明确推迟到本 cycle 评估的跨 runtime 集成问题）。

## Decision

- **D1：AgentSession 独立会话层**——专用 `std::thread` + 自有 tokio Runtime + `std::sync::mpsc` `SessionEvent` 通道桥接同步 TUI 主循环（WorkerManager 同款模式，TUI tick 内 `try_recv` 非阻塞 drain）。async 全部封闭在 session 线程内，与 TUI 只通过 std mpsc 交互，零 runtime 纠缠。
- **D2：runner 流式变体 `run_streaming`**——消费 `provider.stream()` 逐 delta 产出（`TextDelta` 透传事件 sink）；complete-only 既有路径（`run` / `run_with_progress`）零改动，CLI ask 保持非流式批处理语义。
- **D3：写操作 confirm 通道**——dispatch 写 tool 拦截升级为 `ConfirmRequest`（tool_name + arguments + 影响摘要 + `oneshot::Sender<ConfirmDecision>` reply）：dispatch 双模式（blocked 拦截 = 无确认通道，CLI ask 用 / confirm 通道 = 有通道，TUI 用）。Approved → `confirm: true` 真执行（ADR-0008/0029 契约语义不变），Denied → blocked JSON（模型转向解释 + 等价命令行）。
- **D4：多轮 conversation**——session 持 history 滑动窗口（system prompt + 最近 12 轮，代码常量 `MAX_HISTORY_TURNS`，不预加配置项）；窗口超限截断最旧轮，ctx 溢出降级为面板 Error 事件提示开新对话。
- **D5：按需 spawn 核心约束延续**——进入面板首次 query 才 spawn llama-server，退出面板 Drop kill（v0.20 决策 6 延续，日常使用零影响）。
- **D6：streaming 渲染节流**——`TextDelta` 在 TUI tick 边界批量 drain 聚合 append 到当前 assistant 段落 + 既有 `pending_redraw` 机制，不逐 delta 全量重绘（ratatui 全量重绘模式下逐 delta 重绘浪费且闪烁）。

## stage 1 落地注记（Spike 实测）

1. **`A`（Shift+A）入口键与既有绑定冲突，走预授权 fallback**——brainstorm 决策 8 假设 `A` 无冲突，实测全仓 `Char('A')` 已被占用：`try_handle_tab_switch` 全局绑定「打开告警弹窗」（v0.7 落地，help_panel 全局段文档化），且在面板分发**之前**拦截。按 stage doc 风险 2 预授权的 fallback 执行：**palette 唯一入口**（`switch_to_agent_panel` 条目）+ brainstorm 决策 8 注记更新。stage 3 前如需直连键位，需重拍板（迁移告警弹窗键位或另选键）。
2. `ConfirmRequest` derive Debug 可行（`oneshot::Sender` 实现 Debug），stage doc 风险 4 的兜底（手写 Debug）未触发。
3. `AppMode` 实际 9 变体 → 加 `Agent` 后 10 个（brainstorm 文案「10 变体 → 11 变体」是 miscount，代码为准）。

## Consequences

- AppMode 10 变体（ProcessList / PortMap / UsbAssistant / MonitorPanel / DockerPanel / ProcessDetail / ContainerExec / Replay / Help / Agent）；App 加 `agent_panel: AgentPanelController` 字段。
- dispatch 双模式（blocked 拦截 / confirm 通道）——CLI ask 与 TUI 面板共用 dispatch 层，语义都来自 ADR-0008/0029 的 `confirm: bool` 契约。
- 上下文窗口管理成为运行时关注点（滑动窗口截断 + 溢出降级提示）。
- confirm 挂起时的生命周期需显式收尾：退出面板 / Ctrl+C → 对 pending ConfirmRequest 发 Denied + 置 cancel flag，runner 走收尾路径（stage 2 集成测试覆盖「confirm 未决时 drop session 不挂死」）。
- 写操作放行后真实执行不可逆——ConfirmRequest 强制展示影响摘要（kill → 目标进程列表 / docker_rm → 容器名 / usb_release → 盘符），`confirm: true` 仅在用户显式 y 后传入。

## 与既有 ADR 关系

- **建立在 ADR-0030 之上**：本 ADR 是 v0.20 内置 agent 基座的 UI 消费层（stream() 首个真实消费方 / dispatch 拦截的确认通道升级 / AgentRunner 的流式变体）。
- **复用 ADR-0008 + ADR-0029**：写操作 confirm 语义（`confirm: bool` 必传契约），Approved 即用户代传 `confirm: true`。
- **复用 ADR-0012**：AgentPanelController 与既有 5 个 XxxPanelController 同款包装结构。
- **复用 ADR-0010**：palette 入口注册（`switch_to_agent_panel` 条目，`CommandAction::SwitchPanel(AppMode::Agent)`）。
- **参考 ADR-0019**：session 线程 + channel + tick drain 的 worker 生命周期模式（WorkerManager 同款）。

## Alternatives Considered

### A. TUI 主循环内直接 block_on async agent

**否决**：crossterm 同步事件循环内 block_on 会卡死 UI（无 tick 即无按键处理）；引入 async 事件源改造主循环是全 TUI 重写级改动。

### B. 每次查询 spawn 临时 runtime（`Runtime::new().block_on`）

**否决**：llama-server 句柄跨调用复用（v0.20 stage 3a 惰性 spawn 设计）需要常驻 async 上下文；临时 runtime 每查询重建连接池 + spawn/kill server 抖动。

### C. confirm 通道复用 MCP 层 confirm 参数（dispatch 直传 true）

**否决**：违背 ADR-0008 self-mitigation 契约——写操作必须有人工确认环节，直传 true 等于绕过 gate；TUI 的价值恰是提供这个确认环节。

### D. 逐 delta 全量重绘（不做 D6 节流）

**否决**：ratatui 全量重绘模式 + E2B 流式高频 delta = 每秒数十次全帧重绘，浪费且闪烁；tick 批量 append 用户感知仍是逐字流出。

## Migration path

- **v0.21 stage 1 Spike（本 ADR 落地）**：`src/agent/session.rs` 类型骨架（SessionEvent 8 变体 + ConfirmRequest/ConfirmDecision + MAX_HISTORY_TURNS + 空 struct）+ `AppMode::Agent` + AgentPanelController 空骨架 + 占位渲染 + palette 入口。
- **v0.21 stage 2 Slice A**：AgentSession 实装（专用线程 runtime + mpsc 桥接 + 多轮滑动窗口 + 中断）+ `run_streaming` 流式变体 + confirm 协议层（dispatch 双模式 + 影响摘要）。
- **v0.21 stage 3 Slice B**：TUI AgentPanel 实装（controller 状态机 + 全屏渲染 + streaming 增量 + confirm y/n UI + App/tick/palette 集成）+ E2B 真实手动验收。
- **v0.22+**：session 观测素材（session log / TTFT / streaming 指标 / confirm 行为）纳入 C-cycle eval 设计。

## References

- [`docs/stages/v0.21-brainstorm.md`](../stages/v0.21-brainstorm.md)：cycle 总览 + 8 决策 + 风险 + 验证矩阵
- [`docs/stages/v0.21-stage-1.md`](../stages/v0.21-stage-1.md)：本 ADR 落地的任务清单
- WorkerManager 线程 + channel 模式：`src/workers/manager.rs`（v0.6 起）
- ratatui 全量重绘模型：`src/tui/mod.rs::run_app`（pending_redraw 机制）
