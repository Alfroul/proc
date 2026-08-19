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

## stage 2 落地注记（Slice A 业务实装，2026-08-18）

1. **ConfirmHook 签名简化**：runner 自建 oneshot（reply 放进 `ConfirmRequest` 发出），hook 只负责发事件（`Fn(ConfirmRequest)`）——rx 由 runner 持有 await，session 层闭包无需包装 rx（规划稿的 `Fn(ConfirmRequest) -> Receiver` 少一层间接）。
2. **confirm 真实执行复用 MCP 写 helper**：`execute_confirmed_tool`（Approved 后注入 `confirm: true`）调 `make_kill_json` / `make_pkill_json` / `make_usb_release_json` / `make_docker_{rm,image_rm,volume_rm}_json`；record_start/stop 需跨调用子进程保活持久 handle（ADR-0029），Approved 也返「不支持」（决策 8）。安全边界：`spawn_execute(confirmed_write: bool)` 仅 confirm Approved 后置位，无通道时一律走既有 `execute_tool` blocked 拦截。
3. **E2B 流式冒烟实测通过**（风险 3 闭环）：真实 llama-server（b8685 + gemma-4-E2B）`run_streaming` 2 query（proc_ls tool 轮 + 多轮 history 追问）35.7s 无 Err、事件面有产出、最终回答非空——stream + `tool_choice=required` + proc_finish 组合下 tool_calls 分片与 EndTurn 行为正常，stage 3 无需降级方案。
4. **session 线程可观测性**：`SessionHandle::is_exited`（线程退出标志）+ events 通道随线程终结断开（drain 返 None）——stage 3 面板可感知会话死亡；confirm 挂起时 drop 的收尾链（cancel → Interrupted → Sender 断开 → 线程退出）由 D 组集成测试带超时断言覆盖。
5. **`StopCause::Interrupted` 新变体**：`SessionFinished{stop}` 复用（Interrupted 不入 history，已完成轮保留）；`label()` 返 `"interrupted"`。

## stage 3 落地注记（Slice B 面板实装，2026-08-19）

1. **副作用通道走 `PanelAction::Agent(AgentAction)`**：controller 保持 ADR-0012 的 `(key, ctx) -> PanelAction` 签名（与既有 5 个 controller 一致），Agent 专有副作用收进单一变体（SendQuery / Interrupt / ExitPanel）——App 持 `SessionHandle` 执行，controller 是纯状态机（confirm 的 reply send 例外：oneshot 通道由面板 `pending_confirm` 持有，y/n 在 `resolve_confirm` 内直接回传不经 App）。
2. **`build_session` 共享构造链**：`AgentRunner` 三字段私有且 `ToolRegistry` 非 Clone，`AgentSession::spawn` 需要 provider/registry/options 三件套——抽 `build_parts` 内部函数让 `build_runner`（CLI ask，签名行为不变）与 `build_session`（TUI 进面板）共享。
3. **会话生命周期防 llama-server 孤儿**：退出面板 / App shutdown 时 teardown = pending confirm 发 Denied → interrupt → shutdown → 轮询 `is_exited` ≤3s → Drop（cancel 检查点密集通常 <100ms）——进程退出强杀线程时 `LlamaServerHandle::Drop` 不会执行，有界等待是唯一防线。
4. **`AgentPanel::apply_event` 是纯状态迁移**：SessionEvent → `ChatEntry` 对话流缓冲（TextDelta 末段 append / tool 后新段；proc_finish answer 不经 TextDelta，SessionFinished 时落 AssistantFinal；Interrupted 落 Notice 不落占位文本）——测试可脱离 App 直接驱动。
5. **E2B 端到端自动化**（`test_f_e2b_app_e2e_smoke` `#[ignore]`，实测通过 65.8s）：App 层全链路（type → Enter → `app.tick()` drain 循环）真实 llama-server 验证 streaming + 多轮 + confirm n 路径 + teardown；y 真执行路径与视觉手感留人工验收。**实测踩坑 1 修复**：Agent 模式必须豁免全局键（数字 1-6 tab-switch / R 录屏 / D 清崩溃）——否则含数字的 query 在面板分发前被切走面板；**实测观察 2 条**：llama-server 中途连接失败时面板 Error 提示后 provider 惰性重 spawn 自愈；E2B 对写操作若不经 proc_help 发现链（entry 工具集不含写 tool）倾向文字解释——确认框的触发依赖两层架构发现路径，属模型能力边界非面板缺陷。

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
