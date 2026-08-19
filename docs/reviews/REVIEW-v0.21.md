# REVIEW-v0.21 — v0.21 cycle Review（TUI AgentPanel + streaming chat + 写操作 confirm 通道 cycle 完结）

> **cycle 范围**：brainstorm 4 项（2026-08-18 拍板会话，8 决策全 ✅）——AgentSession 会话层 / AgentRunner 流式变体 `run_streaming` / 写操作 confirm 通道 / TUI AgentPanel；穿插 ADR-0031 + CONTEXT 术语段
>
> **Review 范围**：4 stage 全部产出（1 Spike + 2 Slice + 本 Review+收尾合并段；phased-project 自适应规则 Slice ≤ 2 时 Review 与收尾合并）
>
> **基线**：1533 passed / 0 failed / 6 ignored（v0.20.0 默认档）+ 1557 / 0 / 7（anthropic 档）→ **1607 passed / 0 failed / 8 ignored（默认档）+ 1631 passed / 0 failed / 9 ignored（`--features anthropic`）**/ fmt / clippy（双档）/ build（双档）/ bench --no-run 全过
>
> **Review 日期**：2026-08-19
>
> **Reviewer**：Claude（stage 4 会话）

---

## 概览

v0.21 cycle 是 proc 首个「UI 消费层」单主主题 cycle（~5750 行级改动：src +1951 / tests +2409 / docs +1368），4 stage 节奏（vs v0.20 的 5 stage——单主题无跨领域切换）。**agent 交互从 CLI 单轮非流式升级为 TUI 面板多轮流式 + y/n confirm**——用户在 TUI 里 Ctrl+P 搜「AI Agent」进面板，输入自然语言 query 即流式对话；agent 要执行写操作（杀进程 / 弹 U 盘 / 删容器）时面板弹出确认框，y 真执行 / n 拒绝并解释。「现场演示杀手锏」能力（brainstorm 决策 1 拍板的 F 方向）就此闭环。

- **AppMode** 9 → 10 变体（`Agent`）；MCP tool 46 不变；agent 内部 tool 47 catalog + proc_finish 不变（**本 cycle 零 tool 变更——纯消费层**）
- **Cargo deps +0**（全用既有：futures-util / tokio / ratatui / crossterm / oneshot）
- 1 份新 ADR-0031（D1~D6 六决策 + stage 1/2/3 三段落地注记）

**核心实测数字**（详见各 stage 段）：

| 验收项 | 口径 | 实测 | 结论 |
|---|---|---|---|
| stage 2 E2B 流式冒烟（风险 3） | 真实 llama-server `run_streaming` 2 query 无 Err | 2 query 35.7s（proc_ls tool 轮 + 多轮追问），stream + `tool_choice=required` + proc_finish 组合下 tool_calls 分片 / EndTurn 行为正常 | ✅ 风险 3 闭环，stage 3 无需降级方案 |
| stage 3 E2B App 端到端（`#[ignore]`） | App 层全链路（type → Enter → tick drain 循环） | 实测通过 65.8s：L0 流式 + proc_ls tool 轮正确回答 + 多轮追问（D4 生效）+ proc_help 发现 proc_kill → 确认框 → n 拒绝 → 模型解释 + Ctrl+D teardown | ✅ 自动化子集 |
| confirm 双路径 | Approved 真执行 / Denied blocked | CI 测试双 roundtrip（kill 不存在 PID 4000000 安全断言真执行非 blocked）+ E2E n 路径 | ✅ |
| teardown 生命周期 | 退出面板无 llama-server 孤儿 | teardown（Denied → interrupt → shutdown → 轮询 is_exited ≤3s → Drop）+ App::shutdown 同款；cancel 检查点密集实测远快于上限 | ✅ |
| 人工验收（y 真执行 / 视觉手感 / L2 多步演示） | 手动演示（决策 4 不设线） | y 放行语义已由 CI 测试层覆盖（不存在 PID 路径）；视觉 / L2 属演示项非阻断 | ⏸ 演示层（非验收缺口） |

**Findings 汇总**：P0 0 / P1 0 / P2 1（TD-58 归档——test_alert flaky 观察项）。stage 3 的全局键拦截缺陷在 stage 内 E2E 实测发现并闭环（含回归锚），不计遗留。

---

## 1. stage 1 Spike：ADR-0031 + 类型骨架 + palette 入口（commit `ae927c3`）

### 落地范围

ADR-0031 骨架（D1~D6 + 备选方案 4 个全否决）+ `src/agent/session.rs` 声明式骨架（SessionEvent 8 变体 + ConfirmRequest/ConfirmDecision + MAX_HISTORY_TURNS + 空 struct）+ `AppMode::Agent` 第 10 变体（编译器引导穷尽补齐 4 处）+ AgentPanelController 空骨架 + 占位渲染 + palette 入口 + 15 测试。

### 4 维度审查

**代码质量** ✅：**`A` 键冲突实测是本 stage 最有价值的发现**——brainstorm 决策 8 假设 `A`（Shift+A）无冲突，实测全仓 `Char('A')` 已被 `try_handle_tab_switch` 的「打开告警弹窗」（v0.7 落地）占用，且在面板分发**之前**拦截——意味着 process_panel 的 `A` deselect_all 实际是被遮蔽的死绑定（这个考古发现本身修正了对全局键层的认知）。按 stage doc 风险 2 预授权 fallback 执行：palette 唯一入口 + `test_a_key_still_toggles_alert_popup` 回归锚，决策成本为零（未触发步 15 重拍板）。

**架构** ✅：类型骨架一次锁定（SessionEvent 8 变体形状在 stage 2/3 零改动）；`ConfirmRequest` derive Debug 可行（oneshot::Sender 实现 Debug，风险 4 兜底未触发）；AppMode 变体穷尽由编译器保证（无 `_ =>` 通配兜底，后续加变体强制更新分发点）。

**性能** N/A：纯骨架无运行时路径。

**完整性** ✅：15 测试中 4 个是回归锚（CLI ask / MCP 46 tool / A 键告警弹窗 / palette E2E 切模式）——Spike 阶段就为后续 stage 的「零回归」验证铺好锚点，这个模式值得延续。

---

## 2. stage 2 Slice A：AgentSession + run_streaming + confirm 协议层（commit `c766da1`）

### 落地范围

`run_streaming`（StreamEvent 4 变体 + cancel 检查点 3 处 + `StopCause::Interrupted`）+ dispatch 双模式（`confirm_summary` / `blocked_tool_result` / `execute_confirmed_tool`）+ AgentSession 实装（专用线程 + 自有 runtime + mpsc 双通道 + 滑动窗口 12 轮）+ builder 构造链抽共享 + 35 CI 测试 + E2B 流式冒烟。

### 4 维度审查

**代码质量** ✅：两处实测驱动的简化——(1) **ConfirmHook 签名从规划稿 `Fn(ConfirmRequest) -> Receiver` 简化为 `Fn(ConfirmRequest)`**（runner 自建 oneshot，reply 放进 ConfirmRequest 发出，rx 由 runner 持有 await），session 层闭包少一层间接；(2) **`execute_confirmed_tool` 复用 MCP 写 helper**（make_kill / make_pkill / make_usb_release / make_docker_*）而非重写执行逻辑——Approved 放行与 MCP 路径走同一份代码，写操作语义不可能漂移。安全边界收敛在 `spawn_execute(confirmed_write: bool)` 单布尔（仅 confirm Approved 后置位），审计面极小。

**架构** ✅：**D1 闭环是 v0.20 风险 6 的正式答案**——async 全封闭在 session 线程内，TUI 只通过 std mpsc 交互，零 runtime 纠缠。dispatch 双模式让 CLI ask（无通道 blocked 拦截）与 TUI（confirm 通道）共用一层，语义都来自 ADR-0008/0029 契约。complete 路径（`run` / `run_with_progress` / `execute_tool_calls`）零改动——CLI ask 回归锚在 stage 1 就铺好，本 stage 验证了它。

**性能** ✅：cancel 检查点 3 处（turn 开头 / 每 delta 后 / confirm await `tokio::select!` 100ms 轮询）——中断响应以 delta 为粒度，teardown 实测远快于 3s 上限（stage 3 注记「通常 <100ms」）。

**完整性** ✅：35 CI 测试全 ScriptedStreamProvider（零 LLM）；**风险 2 的关键测试**（confirm 挂起时 drop session 不挂死，带超时断言的收尾链：cancel → Interrupted → Sender 断开 → 线程退出）落地；E2B 流式冒烟实测通过（风险 3 闭环——v0.20 验收全走 complete，流式 tool_calls 分片行为首次实测）。

---

## 3. stage 3 Slice B：AgentPanelController + 渲染 + App 集成（commit `723a30a`）

### 落地范围

controller 三态状态机（~390 行）+ 全屏渲染（~280 行：对话滚动区底部锚定 / 输入框 / AwaitingConfirm 高亮确认框 / 状态行）+ `build_session`（build_parts 共享构造链）+ App 集成（agent_session 生命周期 + tick 每帧 drain + PanelAction::Agent dispatch）+ 24 CI 测试 + E2B App 端到端。

### 4 维度审查

**代码质量** ✅：三处设计保持了可测性——(1) **`apply_event` 是纯状态迁移**（SessionEvent → ChatEntry，测试脱离 App 直接驱动，B 组 5 测试零 UI 依赖）；(2) **副作用收进单一 `PanelAction::Agent(AgentAction)` 变体**（共享 enum 不被 Agent 专属语义污染，与 InspectorAction 内聚模式一致——brainstorm 步 11 小修范畴）；(3) **`build_parts` 共享构造链**是「AgentRunner 字段私有 + ToolRegistry 非 Clone」约束下的唯一不破坏既有 API 的复用路径（build_runner 签名行为不变，CLI ask 回归锚保住）。

**架构** ✅：controller 纯状态机 / App 持 session 执行副作用 / 渲染只读状态零副作用——三层职责与 ADR-0012 既有 5 个 controller 完全一致。teardown 有界等待（≤3s）防 llama-server 孤儿是 D5 在进程退出场景的必要补丁（进程强杀线程时不执行 Drop）。confirm 的 reply oneshot 由面板 `pending_confirm` 持有、y/n 在 `resolve_confirm` 直接回传不经 App——例外但有理由（oneshot 单次回传语义放 App 层反而多一次转发）。

**性能** ✅：D6 兑现——tick 内 drain 全部事件批量 apply、单次 `pending_redraw`（E2B 端到端实测流式视觉流畅）；渲染底部锚定只格式化 viewport 内行（`start = total_lines - viewport_h - scroll_from_bottom` 切片）。

**完整性** ✅：24 CI 测试（A 键位 7 / B apply_event 5 / C session 端到端含 confirm 双路径 4 / D TestBackend 渲染 3 / E App 集成 5）；**全局键豁免回归锚**（E 组）是本 stage E2E 实测踩坑的直接产物（见下节观察 1）。E2B App 端到端 `#[ignore]` 实测 65.8s 全链路通过。

---

## stage 3 实测观察归档（E2E 真实 llama-server）

### 观察 1：全局键拦截层 vs 文本输入面板（踩坑→修复）

**现象**：E2E 实测「列出 top 3」query 时，输入的数字 `3` 在面板分发**之前**被 `try_handle_tab_switch` 的数字 1-6 tab-switch 全局绑定捕获——面板被切走。同类被全局层拦截的还有 R（录屏）/ D（清崩溃）/ t·c·A·?（tab-switch）。

**修复**：Agent 模式豁免全局键捕获（输入框接收全部字符），附 E 组回归锚测试（Agent 模式下数字 / R / t / c / D / A / ? 不被全局层拦截）。

**普适教训**：proc 的全局键层设计于「浏览型面板」时代（所有面板都是光标导航语义），新增「接收任意文本输入」的面板时必须显式豁免——数字键是最意外的一类（query 含数字极常见）。后续若再加文本输入面板（如 palette 内联参数输入），此锚测试模式直接复用。

### 观察 2：llama-server 连接级故障自愈

**现象**：E2E 实测中 llama-server 中途连接失败 → 面板 Error 提示 → provider 惰性重 spawn 自愈 → 后续 query 正常继续。

**归因**：v0.20 stage 3a 的 `Arc<Mutex<Option<Handle>>>` 惰性 spawn 设计（本意是「跨调用复用 + 按需 spawn」）在会话场景产生意外收益——连接级故障（server 崩溃 / 端口失效）不需要用户重启面板，下一次请求自动重建 server。会话层零为此写的代码。

### 观察 3：E2B 写操作发现链边界（模型能力边界，非面板缺陷）

**现象**：写 tool（proc_kill 等 8 个）不在 entry 4 集内；query 措辞不引导 proc_help 发现链时（如直接「杀掉 chrome」），E2B 倾向直接文字解释「请用任务管理器……」而不发起 proc_help → proc_kill 链——确认框不触发。措辞引导（「你需要时可以调用进程管理工具」）时发现链正常走通（E2E 的确认框 n 路径即由引导触发）。

**归因**：两层架构的发现路径（entry 4 → proc_help(category) → 动态扩 tools）是确认框的前置；2B 模型对「先发现再调用」的多步规划能力有限。**属模型能力边界非工程缺陷**——v0.22 C-cycle 的直接 eval 素材（更强模型复测预期改善；system prompt 措辞优化也是候选方向）。

---

## Findings 表

| 级别 | # | 内容 | 处置 |
|---|---|---|---|
| P0 | — | 无 | — |
| P1 | — | 无（stage 3 全局键拦截缺陷在 stage 内 E2E 发现并闭环 + 回归锚，不计遗留） | — |
| P2 | TD-58 | `test_alert::test_metric_extract_process_cpu` 并发 flaky（stage 1 开工首跑 1 failed，重跑 / 单跑 / anthropic 档均过——真实 SystemSnapshot 采集在 CI 并发下偶尔慢，环境时序敏感非回归；与 stage 2 注记 A2 决策同款根因——该决策已把 A2 测试改用 proc_help(meta) 零 IO 查询规避） | 归档 `docs/tech-debt.md`：观察项——连续 cycle 再现才值得修（改零 IO 断言或加 retry） |
| P2 | TD-55~57 | Sonnet 50 query 对照 / model ID 真实 API 验证 / nudge 连续 user 消息实测（v0.20 归档，无 `ANTHROPIC_API_KEY`） | 仍 open；有 key 后 1 条命令闭环（v0.22 C-cycle 素材） |

---

## cycle 数据汇总

| 维度 | 数字 |
|---|---|
| stage 数 | 4（1 Spike + 2 Slice + Review+收尾合并段；无回溯修复子阶段、无 Checkpoint 接力——容量风险 6 未触发） |
| commits | `3dfee04`（plan）/ `ae927c3`（stage 1）/ `c766da1`（stage 2）/ `6365496`（stage 2 docs）/ `723a30a`（stage 3）+ stage 4（本段）+ tag `v0.21.0` |
| 全量回归 | 1533 → 1607（默认）/ 1557 → 1631（anthropic），0 failed 全程 |
| 新增测试 | +74 CI（stage 1 15 + stage 2 35 + stage 3 24）+ 2 个 `#[ignore]` E2B 真实测试（stage 2 流式冒烟 + stage 3 App 端到端） |
| MCP tool | 46 → 46（不变）；agent catalog 47 + proc_finish 不变 |
| ADR | 新 1 份（ADR-0031，D1~D6 + 三段 stage 落地注记） |
| Cargo deps | +0（全用既有） |
| 新文件 | `src/agent/{session.rs, builder.rs}` + `src/view_models/agent_panel_controller.rs` + `src/tui/agent_panel.rs` + `tests/{test_agent_v0_21_stage_1, test_agent_v0_21_stage_2, test_agent_panel}.rs` |
| 行数（insertions 口径） | src +1951 / -230（vs 预估业务 ~1600，+22% 合理漂移）；tests +2409（vs 预估 ~850，超 2.8×——ScriptedStreamProvider 在 stage 2 / stage 3 两文件各复刻一份（测试文件不跨 import 的项目惯例）+ C/D 组端到端较重）；docs +1368（ADR + 3 stage docs + brainstorm + CHANGELOG） |

**与 v0.20 cycle 对比**：4 stage vs 5 stage（单主题无跨领域切换，无 stage 拆分）；~5750 vs ~3000+ 行级（本 cycle 测试占比更高——UI 消费层的端到端测试天然重）；deps +0 vs +2；tool 层零变更 vs 新建 47 catalog。「演示能力」维度：v0.20 交付 CLI 单轮（`proc agent ask` 出 Markdown），v0.21 交付 TUI 面板（流式逐字 + tool 步骤实时 + y/n 确认 + 多轮追问）——面试现场演示从「跑一条命令看输出」升级为「交互式对话操作自己的机器」。

---

## v0.22+ 候选方向

brainstorm「v0.22 C-cycle 预拍板记录」（2026-08-18 拍板会话定稿，v0.22 brainstorm 直接引用）+ 本 Review 归档综合：

| 优先级 | 方向 | 依据 | 规模预估 |
|---|---|---|---|
| 1 | **Eval + Observability**（方向 C ⭐⭐⭐⭐⭐，v0.22 主主题） | 决策 1 / 6 预拍板；eval 雏形（QUERY_TABLE 验收 runner）已存在 | ~1500 行 |
| 2 | L2 多步系统验收（20 query seed 已录） | 决策 4——系统验收本质是 eval 素材；E2B 多步能力边界实测 | 归入 C-cycle |
| 3 | 更强本地模型复测 L1（Gemma 4 E4B / Qwen 14B） | 决策 7 预拍板推迟 v0.22；L1 78% → 80%+ 验证可移植性；**本 Review 观察 3（写操作发现链边界）是直接复测素材** | ~0 行（配置层） |
| 4 | TUI session 观测素材纳入 eval 设计 | 决策 1 理由 4——session log / TTFT / streaming 指标 / confirm 行为只有 F 落地后才能纳入 | 归入 C-cycle |
| 5 | TD-55~57 补验 | 有 `ANTHROPIC_API_KEY` 即 1 条命令 | ~0 行 |
| 6 | proc_record_start/stop agent 侧支持 | 决策 8 推迟 v0.22+（confirm 通道落地后成本 ~100 行，语义边界需单独设计） | ~100 行 |
| 7 | RAG 历史经验召回（方向 B） | v0.20 brainstorm 既定 v0.23+ | — |
| 8 | Multi-agent 协作（方向 D） | v0.20 brainstorm 既定 v0.24+ | — |

**eval 基准（决策 6 预拍板）**：复用 70 query 泛化 harness（QUERY_TABLE 验收 runner 泛化：per-query 结果 JSON + 失败模式分类 + 模型对比报告）；τ²-bench 仅对照引用（proc 是 Windows 系统运维域，工具集与外部基准不匹配，完整适配成本高收益低）。
