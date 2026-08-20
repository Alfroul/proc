# ADR-0032：`proc agent eval` 评测 harness + session observability

**Status**: Accepted（v0.22 stage 1 Spike 落地骨架；runner 实装 stage 2，observability stage 3）

**Date**: 2026-08-20（v0.22 cycle stage 1 Spike）

**Related**: ADR-0030（内置 AI agent 基座——eval 测量的对象）、ADR-0031（AgentSession/AgentPanel——session 观测的事件源）、ADR-0008（self-mitigation policy，隐私语境）

## Context

v0.21 cycle 完结时（tag `v0.21.0`，2026-08-19），agent 能力的测量与观测存在四项缺口：

1. **QUERY_TABLE 验收 runner 以 `#[ignore]` test 形态存在**——50 query（L0 23 + L1 27）是测试文件常量（`tests/test_agent_v0_20_stage_3b.rs`），输出走 eprintln，判定 = expected_tool trace 命中 + final_text 非空。泛化（数据文件化 / 结果持久化 / 失败分类 / 跨模型对比）超出 test harness 形态：测试不该产报告、参数化（provider/model/attempts）别扭、`--compare` 跨 run 对比无法表达。
2. **L2 20 query seed 从未系统实跑**——fixtures 70 条中 L2 占 20 条（9 场景多步链），v0.21 决策 4 定性「系统验收留 v0.22 C-cycle」。E2B 在多步 ReAct 链上的能力边界没有量化数据。
3. **session 事件流仅进 TUI 不留档**——v0.21 AgentSession 落地后，TTFT / streaming / confirm 行为首次可观测，但 SessionEvent 只被 AgentPanel drain 消费，跑完即散，无指标提取入口。
4. **REVIEW-v0.21 观察 3（E2B 写操作发现链边界）待量化**——写 tool 不在 entry 4 集内，query 措辞不引导 proc_help 发现链时 E2B 倾向直接文字解释不触发确认框；两层架构发现路径是前置，属模型能力边界非工程缺陷，需要 eval 数据说话。

另有 **2026-08-20 实测观察（brainstorm 风险 6）**：E2B 在异常上下文（连接错误恢复后 / 长 history）下出现输出退化——形近字乱码 / tool call 语法泄漏成正文（`<tool_call|>` 字面量）/ `<eos>` 字面量数百次重复。退化文本非空，纯「非空」的 text_ok 判定会误判 Pass、eval 虚高。

## Decision

- **D1：eval runner = CLI 子命令**——`proc agent eval`（`AgentSub::Eval`）。eval 是测量工具不是测试：独立可执行、参数化（`--level` / `--scenario` / `--quick` / `--attempts` / `--max-steps` / `--output` / `--compare`）、复用 builder 构造链（`--provider` / `--model` stage 2 按 ask 变体同款接入）。执行走 complete 路径（与 stage_3b 验收同款），不动 `run` / `run_streaming`。
- **D2：70 query 数据文件编译进 binary**——`src/agent/eval/queries.toml` + `include_str!`（与 GBNF grammar 同款模式）：版本与 binary 强一致、零运行时文件依赖、git 版本化天然。`[[query]] = {scenario, level, text, expected_tools}`；L0 23 + L1 27 逐字迁移 QUERY_TABLE（锚测试锁一致），L2 20 迁移 fixtures seed + expected tool 链 authoring（2-3 个有序 tool，「关键路径」非「唯一路径」——保序子序列判定）。
- **D3：判定与失败分类确定性**——tool 命中 = expected_tools **保序子序列**在实际 steps trace 中命中（允许中间插其他 tool）；text_ok = final_text 非空、非 nudge 兜底文案、**且非退化输出**（`is_degraded_output`：特殊 token 字面量名单 `<eos>` / `<end_of_turn>` / `<tool_call` / `<turn|>` 任一命中，或同一片段连续重复 ≥ 8 次）。FailureMode 7+1 变体（Pass / NoToolCall / WrongTool / ChainIncomplete / EmptyAnswer / MaxSteps / LlmError / **OutputDegraded 优先归类**——tool 命中但文本退化仍记失败）从 RunnerOutcome 确定性判定；不上 LLM-as-judge（本地无 judge 模型，确定性口径可复现可 diff）。eval 默认 report-only 不 gate（L0/L1 硬线保留在 stage_3b 既有 `#[ignore]` 验收测试）。
- **D4：L2 双口径不设硬线**——full-chain（expected 链全部命中 / 20）+ chain-step（链步命中数 / 总链步数）双口径报告。E2B 预期低通过率（L1 三步链已暴露边界），设线只会 flaky；数据价值 > 门槛价值，更强模型复测（v0.23+）时这组数字才是对比基线。
- **D5：observability 在 session 层旁路**——SessionLogEntry 时间戳包装记录（**SessionEvent 8 变体形状零改动**，apply_event 测试零破坏）+ SessionRecorder 写 JSONL（`dirs_config_dir()/sessions/<yyyyMMdd-HHmmss>-<provider>.jsonl`，agent.toml `[session].log` 默认 true 可关，写入失败静默降级）+ `analyze_session_log` 指标提取（TTFT / 生成时长 / delta 数 / tool 轮数 / confirm 行为，JSONL 后处理零运行时开销）+ `proc agent session-info` 薄 CLI 展示。运行时 UI 零改动；不做轮转/清理（不预实现）。

## Consequences

- `AgentSub` 2 → 4 变体（Eval / SessionInfo）
- eval 结果 JSON 与 markdown 报告成为 cycle 间可比资产；`--compare` 是未来模型对比的即插接口（v0.22 只产 E2B 一列，用户拍板不下载更强模型）
- session log 默认写盘（本地工具零上传、单会话几百 KB，与 ADR-0008 隐私架构一致；`[session].log = false` 一键关）
- QUERY_TABLE 冻结锚与 queries.toml 并存的取舍：QUERY_TABLE 是 v0.20 验收的**冻结证据**（回归锚，永不演进），queries.toml 是 **eval 演进数据源**——stage 1 锚测试锁 50 query 逐字一致，锚漂即测试红（brainstorm 风险 5 mitigate 1）
- 零 tool 变更基线（决策 6）：catalog 47 / MCP 46 不动，跨 cycle 数据可比
- 零新 deps：serde / serde_json / toml 全既有

## 与既有 ADR 关系

- **建立在 ADR-0030（内置 agent 基座）+ ADR-0031（AgentSession/面板）之上**——本 ADR 是测量与观测层：0030 定义了被测对象（AgentRunner + 两层 tool registry），0031 定义了被观测对象（SessionEvent 事件流）
- **复用 ADR-0008 self-mitigation 语境**——eval 不触碰写 tool；confirm 行为仅观测不触发

## 附录：markdown 报告样式草案（stage 2 实施以此为准，不重新设计）

> brainstorm 风险 4 mitigate 2——报告格式在 stage 1 锁定锚，stage 2 实施不吸收超预期打磨时间。

### 单 run 报告（`eval-<provider>-<ts>.md`）

```markdown
# proc agent eval 报告

- run: 2026-08-21T02:30:00+08:00
- provider: llama-cpp (gemma-4-E2B-it-Q4_K_M.gguf)
- 参数: attempts=2, max_steps=10, git describe v0.22.0-xxx
- 总时长: 4h 32m 15s

## 通过率（per level）

| Level | 通过 | 总数 | 通过率 |
|---|---|---|---|
| L0 | 23 | 23 | ██████████ 100% |
| L1 | 21 | 27 | ███████▊▌ 78% |
| L2 full-chain | 3 | 20 | █▌ 15% |
| L2 chain-step | 31/48 | — | ██████▌ 65%（链步命中 31 / 总链步 48） |

## 失败模式直方图

| 失败模式 | 次数 | 分布 |
|---|---|---|
| ChainIncomplete | 12 | ████████████ 60% |
| WrongTool | 4 | ████ 20% |
| NoToolCall | 2 | ██ 10% |
| OutputDegraded | 1 | █ 5% |
| LlmError | 1 | █ 5% |

## 失败 query 明细

| # | Level | Scenario | Query | 失败模式 | 链命中 | stop | final_text（截断） |
|---|---|---|---|---|---|---|---|
| 1 | L1 | docker | postgres 容器为什么 unhealthy？ | WrongTool | — | end_turn | 检查了容器列表后发现… |
```

### 对比报告（`--compare a.json b.json`）

```markdown
# proc agent eval 对比报告

| run | provider | L0 | L1 | L2 full-chain | L2 chain-step | OutputDegraded |
|---|---|---|---|---|---|---|
| a（基线） | llama-cpp E2B | 23/23 | 21/27 | 3/20 | 31/48 | 1 |
| b（对照） | anthropic sonnet | 23/23 | 26/27 | 14/20 | 39/48 | 0 |

## 失败模式迁移（a → b）

| 失败模式 | a | b | Δ |
|---|---|---|---|
| ChainIncomplete | 12 | 5 | -7 |
| WrongTool | 4 | 1 | -3 |
```

## Migration path

- **v0.22 stage 1 Spike**（本 ADR 落地）：D1~D5 骨架 + queries.toml 70 query + 类型锁定（QuerySpec / FailureMode / QueryResult / EvalReport）+ `classify_failure` / `tools_subsequence_hit` / `is_degraded_output` 纯函数 + CLI 变体 stub（友好拦截）
- **v0.22 stage 2 Slice A**：runner 实装（执行循环 + attempts 重试 + JSON 输出 + markdown 报告按本附录样式 + `--compare`）+ MockProvider seed 确定性 CI 全覆盖 + E2B QUICK 手动冒烟
- **v0.22 stage 3 Slice B**：observability 全套（SessionRecorder + analyze_session_log + session-info）+ E2B FULL 70 query 用户挂机实跑 + 报告归档 `docs/eval/`
- **v0.23+**：更强模型下载后 `--compare` 即出对比列；queries.toml 可扩 query（QUERY_TABLE 锚不动）

## References

- [`docs/stages/v0.22-brainstorm.md`](../stages/v0.22-brainstorm.md)：cycle 总览 + 7 决策 + 风险 6（E2B 输出退化实测）
- [`tests/test_agent_v0_20_stage_3b.rs::QUERY_TABLE`](../../tests/test_agent_v0_20_stage_3b.rs)：L0/L1 迁移源（冻结锚）
- [`tests/fixtures/agent/*-l2.jsonl`](../../tests/fixtures/agent/)：L2 query 迁移源
