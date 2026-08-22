# REVIEW-v0.22 — v0.22 cycle Review（Eval harness + session Observability cycle 完结）

> **cycle 范围**：brainstorm 4 项（2026-08-19 拍板会话，7 决策全 ✅）——`proc agent eval` 评测 harness（70 query 数据文件化 + per-query 结果 JSON + 失败模式分类 + 跨 run/模型对比报告）/ L2 多步系统验收（双口径首跑）/ session observability 全套（SessionRecorder JSONL + 指标提取 + session-info CLI）/ E2B FULL 70 query 实跑归档；穿插 ADR-0032 + CONTEXT 术语段
>
> **Review 范围**：4 stage 全部产出（1 Spike + 2 Slice + 本 Review+收尾合并段；phased-project 自适应规则 Slice ≤ 2 时 Review 与收尾合并）
>
> **基线**：1607 passed / 0 failed / 8 ignored（v0.21.0 默认档）+ 1631 / 0 / 9（anthropic 档）→ **1681 passed / 0 failed / 9 ignored（默认档）+ 1705 passed / 0 failed / 10 ignored（`--features anthropic`）**/ fmt / clippy（双档）/ build（双档）/ bench --no-run 全过
>
> **Review 日期**：2026-08-22
>
> **Reviewer**：Claude（stage 4 会话）

---

## 概览

v0.22 cycle 是 proc 首个「测量与观测层」单主主题 cycle（~6000 行级改动：src +2183/-8 / tests +2256 / docs +1543，26 files），4 stage 节奏（与 v0.21 同款）。**agent 能力从「可演示」升级为「可测量」**——`proc agent eval` 让 70 query 基准、失败模式分类、跨 run/模型对比成为一条命令；session observability 让 TUI 面板的每次对话留档 JSONL、TTFT / 生成时长 / confirm 行为可回溯可量化。E2B FULL 70 query 首跑同时完成：**47m19s 零中断，L0 74% / L1 52% / L2 full-chain 5% + chain-step 28%**——这组数字是未来所有模型 / prompt / GBNF 变量的对比基线（`--compare` 即插即出）。

- **MCP tool 46 / agent catalog 47 + proc_finish 均不变**（本 cycle 零 tool 变更——eval 基线纯净性要求，决策 6）
- **Cargo deps +0**（serde / serde_json / toml 全既有）
- 1 份新 ADR-0032（D1~D5 五决策 + stage 1/2/3 三段落地注记 + markdown 报告样式附录）
- `AgentSub` 2 → 4 变体（Eval / SessionInfo）

**核心实测数字**（详见各 stage 段 + [`docs/eval/e2b-70q-v0.22.md`](../eval/e2b-70q-v0.22.md)）：

| 验收项 | 口径 | 实测 | 结论 |
|---|---|---|---|
| E2B QUICK 冒烟（26 query） | 链路确认后挂 FULL | 18m46s：L0 7/9 / L1 4/9 / L2 full-chain 0/8、chain-step 4/17 | ✅ 风险 1 mitigate 4 兑现（先冒烟再挂机） |
| E2B FULL 70 query | attempts=2 / max_steps=10，挂机 | **47m19s 零中断零 LlmError**：L0 17/23（74%）· L1 14/27（52%）· L2 full-chain 1/20 + chain-step 12/43（28%） | ✅ harness 全链路验证 + E2B 画像落位 |
| 失败模式直方图 | 38 失败 / 70 query | **output_degraded 21（55%）** / wrong_tool 10 / chain_incomplete 7 / 其余 0 | ✅ 风险 6 量化闭环（30% 退化率） |
| 与 v0.20 验收口径差 | 17/23 vs 23/23 是否回归 | text_ok 新增退化检测 + Docker daemon 未运行（10 query 全败）+ E2B 非确定性；排除 docker 后 L0 17/20（85%）/ L1 14/24（58%） | ✅ 非能力回归（口径与环境差） |
| session observability 真实链路 | `#[ignore]` E2B 冒烟 | build_session → TUI 路径 1 query 12.7s end_turn → 真实 sessions/ 目录 JSONL → analyze TTFT Some | ✅ D5 全链路 |
| eval 链路 stage 3 改动后回归 | E2B 冒烟 | performance-diagnose L0 3/3 PASS（28s） | ✅ 零回归 |

**Findings 汇总**：P0 0 / P1 0 / P2 2（TD-55~57 仍 open + 新归档 TD-59 session log 累积观察项）。TD-58（test_alert flaky）整个 v0.22 cycle 未再现，继续观察。

---

## 1. stage 1 Spike：ADR-0032 + queries.toml + 类型骨架 + CLI stub（commit `7f071c1`）

### 落地范围

ADR-0032 骨架（D1~D5 + markdown 报告样式附录——风险 4 mitigate 2「样式在 stage 1 锁定锚」）+ `src/agent/eval/queries.toml` 70 query（L0 23 + L1 27 逐字迁移 QUERY_TABLE + L2 20 seed 迁移 + expected 链 authoring）+ serde 类型锁定（QuerySpec / FailureMode / QueryResult / EvalReport / LevelSummary）+ 纯函数三件（`classify_failure` / `tools_subsequence_hit` / `is_degraded_output`）+ 加载校验 6 项 + `AgentSub` 4 变体 stub + 22 集成测试 + 1 QUERY_TABLE 冻结锚 + 6 单元。

### 4 维度审查

**代码质量** ✅：**判定口径一次锁定**是本 stage 最有价值的设计——serde schema 即结果 JSON contract（roundtrip 测试锚字段名），stage 2 runner 只是填执行循环不碰口径。**风险 6 防线的时间线价值**：`is_degraded_output`（特殊 token 名单 + 重复阈值）与 OutputDegraded 优先归类在 2026-08-20 手动观察入档后、8-22 FULL 量化前就位——FULL 实跑直接产出 21 次退化数据而非误判 Pass（若沿用纯「非空」口径，eval 虚高 30%）。

**架构** ✅：数据契约（queries.toml）/ 类型契约（serde）/ 判定口径（纯函数）三层分离，每层独立可测。QUERY_TABLE 冻结锚 vs queries.toml 演进数据源的双数据源取舍（风险 5）用跨文件逐字比对锚测试锁死——锚漂即测试红。

**性能** N/A：纯骨架无运行时路径（include_str! 编译期嵌入）。

**完整性** ✅：加载校验 6 项（总数 / 分布 / 去重 / scenario 域 / L2 链长 / catalog 47 名单）把 queries.toml 的数据质量断言在 CI；L2 链 authoring 规则文档化（「关键路径」非「唯一路径」——保序子序列口径，风险 3 mitigate）。

---

## 2. stage 2 Slice A：`proc agent eval` runner 实装（commit `fd4183a`）

### 落地范围

`run_eval` 执行循环（attempts 重试记末次状态 + 单 query LlmError 不中断 + progress 回调内 JSON 每 query 全量重写实时落盘）+ `parse_levels` / `select_queries`（QUICK 26 条）+ `build_report` 聚合单一实现 + `render_markdown` / `render_compare_markdown`（按 ADR-0032 附录样式不重新设计）+ CLI 接线（`--provider`/`--model` 接 builder 链）+ 24 CI 测试 + E2B 冒烟 3/3。

### 4 维度审查

**代码质量** ✅：两处实测驱动的修正——(1) **classify 优先级修正**（mock CLI 冒烟 3/3 误判发现：max_steps 兜底文案是 runner 合成的重复 tool 名列表，天然触发重复退化检测——修正为 LlmError → MaxSteps → OutputDegraded，OutputDegraded 只归类**模型产出**的退化文本；stage-1 测试零破坏 + stage-2 正反测试锁定）；(2) **MockProvider 回放多轮语义确认**——录制 fixture 只覆盖首轮流，complete() 回放在 required+proc_finish 循环下重复执行同一 tool 至 max_steps；这不是缺陷而是分工确认：**MockProvider 用途 = 管线确定性验证，通过率数据必须来自真实 provider**。

**架构** ✅：report-only 不 gate（决策 2）与 stage_3b 既有 `#[ignore]` 验收测试双入口分工明确——runner 若也 gate 两套阈值必然漂移；`build_report` 单一实现让 compare 与单 run 报告不可能口径漂移；UTC 时间戳不引 chrono（record.rs 同款 civil 算法，零新 deps 兑现）。

**性能** ✅：实时落盘 = 每 query 全量重写（70 query 末态几百 KB / 总写量 ~20MB 级）——简单正确优先，append-only 优化不预实现；这个选择在 FULL 挂机场景的价值是「中途崩已跑数据不丢」。

**完整性** ✅：24 CI 测试覆盖分类 8 变体全覆盖 + retry 语义 + LlmError 不中断 + select_queries 边界（含选空报错）+ 报告 roundtrip + MockProvider seed 全管线 L0 子集；E2B 冒烟 3/3 PASS（28s）确认真实链路。

---

## 3. stage 3 Slice B：session observability 全套 + E2B FULL 归档（commits `b210dc3` + `2439d53`）

### 落地范围

`src/agent/session_log.rs`（新 ~510 行：SessionLogEntry serde schema 10 kind + SessionRecorder + TextDelta 聚合 + confirm 决策旁路 + analyze_session_log + format_session_metrics）+ `[session].log` 配置 + `AgentSession::spawn` 3→4 参接线 + `proc agent session-info` CLI + 21 CI + 1 E2B `#[ignore]` + E2B QUICK/FULL 实跑与归档 `docs/eval/e2b-70q-v0.22.md`。

### 4 维度审查

**代码质量** ✅：三处设计保持了「形状零改动」承诺——(1) **SessionEvent 8 变体形状零改动**（时间戳在 session 层 send 前包装旁路记录，两个既有测试文件全绿即验证锚）；(2) **confirm 决策旁路**（`ConfirmDecision` 不在 SessionEvent 流里——session 层换出 `req.reply` oneshot 包一层转发线程，决策先记录后转发保日志序；y/n/drop 三条路径不泄漏不挂死，RecvError → Denied 既有语义不变）；(3) **TextDelta 聚合 ts 取首 delta 时间**（`(首 ts, 累计 chars)` 缓冲设计让 TTFT 精度不受聚合影响——这是把聚合做成两元组而非 flush 时取 ts 的原因，测试断言 200×4 chars → ≤14 条）。

**架构** ✅：D5 完整兑现——观测是离线诉求，运行时 UI 零改动；JSONL 用 serde tag+flatten 每行人类可读（后处理单遍扫描）；`build_session` 双读 agent.toml（小文件读两次比改 build_parts 签名干净）；TTFT 忠实语义（无流式文本的 query TTFT=None——直接 tool→proc_finish 形态的忠实表达非缺失，E2B 冒烟实测恰有此形态）。

**性能** ✅：per-line flush 崩溃安全（会话崩溃时已写行完整）；聚合上限测试锁行数；指标提取是 JSONL 后处理零运行时开销。

**完整性** ✅：21 CI（recorder seq 单调 / ts 非降 / 8 变体映射 / analyze 数字断言 / confirm 旁路 roundtrip / CLI / config 三态）+ E2B 真实链路 14.8s；**E2B FULL 70 query 挂机完成**（QUICK 26 条冒烟 → FULL 47m19s，零 LlmError 零中断——风险 1 未触发，llama-server 全程稳定惰性 spawn 一次到底）。

---

## 实测观察归档（E2B FULL 70 query 真实数据）

### 观察 1：OutputDegraded 30% 画像——E2B 最大瓶颈是「收尾语法」不是「调工具」

**现象**：21/70 query（30%）输出退化，占失败的 55%。典型形态：`proc_finish{answer:...}` 语法泄漏成正文（模型想发结构化 tool call 但走了文本通道，随后 `<tool_call|>` + `<eos>` 字面量连发）；`<eos>` 重复循环（停止 token 被当文本吐出）。**退化文本非空且常含正确答案**（#8「Docker 未运行」解释本身正确但外壳泄漏）——按 D3 口径仍记失败。

**归因**：tool 通道大部分 query 正常（L0 排除退化/环境后 ~85%），问题集中在「收尾时把 finish 调用写成文本」。**两条现成修复路径**（v0.22 零行为变更基线内不动，数据留给 v0.23+ 决策）：GBNF 逃生舱（`agent.toml grammar_file = "tool_call"`，v0.20 决策 C 留的口子）预期直接消灭 proc_finish 泄漏型退化；更强模型是另一条路（同时改善 wrong_tool 10 + chain_incomplete 7）。eval json 本地留存即两路的对比基线。

### 观察 2：classify 优先级实测修正（MaxSteps 提前）

**现象**：stage 2 mock CLI 冒烟 3/3 全部误判 OutputDegraded——max_steps 的兜底文案是 runner 合成的 tool 名列表（模型循环调同一 tool 至上限时 `proc_ls, proc_ls, …`×10），天然触发重复退化检测。

**修复**：优先级修正为 LlmError → MaxSteps → OutputDegraded → …，OutputDegraded 只归类**模型产出**的退化文本（EndTurn 路径「tool 命中但文本退化仍记失败」不变）。

**普适教训**：确定性分类器要区分「模型产出的文本」与「runner 合成的兜底文案」——后者不是模型行为，进退化口径会污染画像。冒烟测试（哪怕是 mock）在分类逻辑上的价值不亚于真实跑。

### 观察 3：QUICK 26 条 ≠ 27（数据源边界的文档化）

**现象**：QUICK 每 (scenario, level) 抽 1 条，理论 9×3=27，实际 26——monitor 场景无 L2 query（v0.20 fixtures `monitor-l2.jsonl` 是空文件保结构，L2 迁移源无此组）。

**处置**：brainstorm「≈27」是约数，测试锁 26；ADR-0032 注记 3 文档化根因。微小但值得归档——eval 数字进报告时「分母是什么」必须无歧义（QUICK 26 vs FULL 70 的对比口径）。

### 观察 4：挂机 47m19s vs 预估 4.5-6.5h（预估校准）

**现象**：FULL 70 query（attempts=2）实测 47m19s（均 ~40.6s/query），远低于 brainstorm 预估 4.5-6.5h——该预估沿用 v0.20 慢速实测（2.7-4h/50 query），本轮大量 query 3-15s 即完成。

**校准意义**：v0.23+ 更强模型（更大 GGUF，生成速度更低）eval 时长预估应按「query 均时 × 70 × attempts 折叠率」重算而非沿用小时级挂机假设；QUICK（~19m）作为链路冒烟的定位不变。

**附：与 v0.20 验收数字的口径差归档（17/23 vs 23/23 非回归）**——三处差均非 agent 能力退化：(1) text_ok 加了退化检测（21 条 output_degraded 多数在 v0.20 口径下是 Pass）；(2) Docker daemon 未运行（docker 场景 10 query 全败——模型正确识别「未运行」并解释，但部分外壳退化、部分 expected tool 无对象可调；v0.20 验收时已知同样问题）；(3) E2B 非确定性（同 query 两次 attempt 行为可不同）。排除 docker 10 条后近似口径：L0 17/20（85%）/ L1 14/24（58%）。

---

## Findings 表

| 级别 | # | 内容 | 处置 |
|---|---|---|---|
| P0 | — | 无 | — |
| P1 | — | 无 | — |
| P2 | TD-55~57 | Sonnet 50 query 对照 / model ID 真实 API 验证 / nudge 路径实测（v0.20 归档，无 `ANTHROPIC_API_KEY`） | 仍 open；有 key 后 **eval runner 1 条命令闭环三连**（`ANTHROPIC_API_KEY=... proc agent eval --provider anthropic --output ...` 即出 Sonnet 70 query 对比列，TD-55 顺带闭环 TD-56/57）——v0.22 harness 让这三个 TD 的闭环成本从「跑测试」降为「跑 eval」 |
| P2 | TD-59（新） | session log 累积无轮转 / 清理——每次 TUI 面板会话一个 JSONL（单会话几百 KB 量级），长期使用目录单调增长 | 归档 `docs/tech-debt.md` 观察项：brainstorm 决策 3 明确「不预实现；累积成问题再治理」——触发条件是磁盘占用可感知（届时加最简清理：启动时删 N 天前文件，或 sessions 目录体积上限）。**不是缺陷是延后决策**，归档为了不让「不预实现」变成「永不处理」 |
| — | TD-58 | `test_alert::test_metric_extract_process_cpu` 并发 flaky（v0.21 归档观察项） | 整个 v0.22 cycle 4 次基线验证（stage 1/2/3/4 开工双档）均未再现——继续观察，连续 cycle 再现才修的原判不动 |

---

## cycle 数据汇总

| 维度 | 数字 |
|---|---|
| stage 数 | 4（1 Spike + 2 Slice + Review+收尾合并段；无回溯修复子阶段、无 Checkpoint 接力——容量风险 4 未触发） |
| commits | `53de133`（plan）/ `e7abc75`（plan 小修：风险 6 入档）/ `7f071c1`（stage 1）/ `fd4183a`（stage 2）/ `b210dc3`（stage 3 工程）/ `2439d53`（stage 3 FULL 归档）+ stage 4（本段）+ tag `v0.22.0` |
| 全量回归 | 1607 → 1681（默认）/ 1631 → 1705（anthropic），0 failed 全程；ignored 8→9 / 9→10（+1 E2B session log 真实测试） |
| 新增测试 | +74 CI（stage 1 29 含冻结锚与单元 / stage 2 24 / stage 3 21）+ 1 个 `#[ignore]` E2B 真实测试 |
| MCP tool / agent catalog | 46 / 47 + proc_finish **均不变**（零 tool 变更干净基线，决策 6） |
| ADR | 新 1 份（ADR-0032，D1~D5 + 三段 stage 落地注记 + markdown 样式附录） |
| Cargo deps | +0（serde / serde_json / toml 全既有） |
| 新文件 | `src/agent/eval/{mod.rs, queries.toml, runner.rs, report.rs}` + `src/agent/session_log.rs` + `tests/{test_agent_v0_22_stage_1, test_agent_v0_22_stage_2, test_agent_v0_22_stage_3}.rs` + `docs/eval/e2b-70q-v0.22.md`（首个 eval 归档） |
| 行数（insertions 口径） | src +2183/-8（vs 预估业务 ~1260，~1.7×——queries.toml 数据文件 + serde 类型层 + 纯函数 + CLI 接线 + session_log 全套，合理漂移）；tests +2256（vs 预估 ~730，~3×——分类 8 变体全覆盖 + 报告 roundtrip + session 端到端 + recorder 真文件，与 v0.21 测试超 2.8× 同趋势：测试 infra 重是项目惯例）；docs +1543（vs 预估 ~700-1000 偏上——ADR 三段注记 + eval 归档 + 4 份 stage docs） |

**与 v0.21 cycle 对比**：4 stage vs 4 stage（同款节奏）；~6000 vs ~5750 行级（同档）；deps +0 vs +0；tool 层双双零变更。「能力维度」：v0.21 交付 UI 消费层（用户能对话），v0.22 交付测量与观测层（**能力可量化**——E2B 画像数字 L0 74% / L1 52% / L2 5%+28% / 退化 30% 从此是所有后续变更的对比基线）；「资产维度」：eval json + session JSONL 是 cycle 间可比数据资产（`--compare` 即插即出），这是 v0.21 所没有的新类别。

---

## v0.23+ 候选方向

brainstorm 备注（cycle 末段评估 v0.23+ 候选）+ 本 Review 归档综合：

| 优先级 | 方向 | 依据 | 规模预估 |
|---|---|---|---|
| 1 | **GBNF 开关 vs 更强模型**（eval 数据驱动决策） | 观察 1：output_degraded 21 次以 proc_finish 泄漏型为主——GBNF（`grammar_file = "tool_call"`，零代码）预期直接消灭；更强模型（Gemma 4 E4B / Qwen 14B，需下载）同时改善 wrong_tool 10 + chain_incomplete 7（L2 多步规划）。两路都零 harness 改动——改一个变量跑 FULL，`--compare eval-e2b-70q.json new.json` 即出对比列 | ~0 行（配置层）/ 下载 |
| 2 | TD-55~57 补验 | 有 `ANTHROPIC_API_KEY` 即 1 条命令（Sonnet 70 query 对比列 + model ID 验证 + nudge 路径覆盖三连闭环） | ~0 行 |
| 3 | proc_record_start/stop agent 侧支持 | v0.21 决策 8 / v0.22 决策 6 连续推迟；**v0.22 基线已落**——「catalog 一变数据不可比」的代价现在可精确计算（一次 FULL 重跑 47m 即出新基线列），推迟理由弱化；语义边界（录制范围 / 停止时机）需单独设计 | ~100 行 |
| 4 | prompt 措辞优化（system.md 引导） | L2 反问缺参 4 条（#19/#25/#26/#29「请提供盘符/PID」是合理行为但 eval 记失败）+ REVIEW-v0.21 观察 3（写操作发现链措辞敏感）——system prompt 的发现链引导措辞是零成本变量；与方向 1 正交可组合 | ~0 行（prompt 文本） |
| 5 | RAG 历史经验召回（方向 B） | v0.20 brainstorm 既定 v0.23+；session JSONL 留档（v0.22）为 RAG 提供了数据源基础 | — |
| 6 | Multi-agent 协作（方向 D） | v0.20 brainstorm 既定 v0.24+ | — |

**queries.toml 演进**（ADR-0032 Migration path）：v0.23+ 可扩 query（QUERY_TABLE 锚不动）；L2 链标注主观性（风险 3）在更强模型数据出来后可复核——若某链「合理行为但记失败」占比高，链 authoring 是第一修订对象。
