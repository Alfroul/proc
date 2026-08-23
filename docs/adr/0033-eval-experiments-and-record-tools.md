# ADR-0033：eval 变量实验（GBNF × prompt v2）+ proc_record_start/stop agent 侧支持

**Status**: Accepted（v0.23 stage 1 Spike 落地；GBNF 冒烟实测结论已回填附录 B——**不兼容，矩阵缩为 2 列**）

**Date**: 2026-08-23（v0.23 cycle stage 1 Spike）

**Related**: ADR-0032（eval harness——本 cycle 实验的测量工具）、ADR-0030（内置 agent 基座——决策 C grammar 逃生舱的主人）、ADR-0031（AgentSession/AgentPanel——record handle 持有层）、ADR-0029（MCP 层 record_handle pattern）、ADR-0008（写操作 confirm 契约）

## Context

v0.22 cycle 完结时（tag `v0.22.0`，2026-08-22）留下两组待兑现场景：

1. **E2B 基线画像已量化但修复路径未实验**（`docs/eval/e2b-70q-v0.22.md`）：FULL 70 query 得 L0 74% / L1 52% / L2 full-chain 5% + chain-step 28%；失败直方图 **output_degraded 21/70（占失败 55%）**——proc_finish 语法泄漏型为主，另有 wrong_tool 10 / chain_incomplete 7。两条候选修复路径：
   - **GBNF 逃生舱**（v0.20 决策 C，`agent.toml [llama-cpp] grammar_file = "tool_call"`，零代码）——预期消灭 proc_finish 泄漏型退化。但 grammar + `tool_choice=Required` + proc_finish 循环的完整链路**从未实测**（v0.20 stage 3a 只验证过 grammar 字段生效的极简实验，且该实验不带 tools）。
   - **prompt v2 措辞**（`src/agent/prompts/system.md` 2 处小修）——治 L2 反问缺参（#19/#25/#26/#29）与写操作发现链未触发（v0.21 观察 3 / v0.22 #21）。
2. **proc_record_start/stop agent 侧支持连续两 cycle 推迟**（v0.21 决策 8 → v0.22 决策 6），v0.22 出现复评新证据：70q 基线**零 query 依赖**这两 tool（recording 场景 expected 全是 replay/bookmark）+ catalog 47 名单不变（两 tool 早在册，仅 dispatch 返「不支持」）+ eval 走 complete 路径无 confirm 通道（写 tool 永远 blocked）——**落地不破 eval 基线**。

## Decision

### D1：实验矩阵——两变量漏斗式（冒烟后缩为 2 列）

原设计 3 列（GBNF × prompt 两变量单变量隔离 + 终验），全部 FULL 70q 同参数（attempts=2 / max_steps=10 / 同 E2B 模型 llama-server b8685 + gemma-4-E2B-it-Q4_K_M）：

| 列 | 输出文件 | 变量 | 目的 |
|---|---|---|---|
| 1 | `eval-gbnf-70q.json` | GBNF on × prompt v1 | GBNF 单变量增益 |
| 2 | `eval-promptv2-70q.json` | GBNF off × prompt v2 | prompt 单变量增益（system.md 落 v2 + rebuild 后跑） |
| 3 | `eval-best-70q.json` | 最优组合（GBNF 按列 1 数据定 + prompt v2） | 默认配置候选终验 |

**stage 1 冒烟实测后实际形态（附录 B）：GBNF 列移除，矩阵缩为 2 列**——列 ① prompt v2 × GBNF off（vs 基线 `eval-e2b-70q.json` 出增益）+ 列 ② 最优配置终验（按列 ① 数据定：v2 改善 → 复跑 v2 确认稳定性，兼测 E2B 非确定性方差；v2 无改善或退化 → 无需终验，v1 维持默认，矩阵归档 1 新列）。矩阵报告：`proc agent eval --compare eval-e2b-70q.json eval-promptv2-70q.json [eval-best-70q.json]`（compare 列顺序固定：基线 → prompt v2 → best）。

挂机时长：E2B 基线实测 47m/次（v0.22 观察 4 校准口径），2 列 ≤ ~1.6h（原 3 列预估 ~2.5h 缩减）。

### D2：run 记录方案——文件名区分，零代码

不改 `EvalRunMeta`（现有 provider/model/attempts/max_steps/git_describe 已够用）：

- prompt v2 列的 system.md 落地在独立 commit，`git_describe` 天然区分 v1/v2 run；
- 文件名显式标注变量（`eval-promptv2-70q.json` / `eval-best-70q.json`），compare 标签取文件名（ADR-0032 stage 2 注记 6）；
- compare 列顺序固定（基线 → prompt v2 → best），失败模式迁移表取首/末 run 语义不变。

不为此加 meta 字段（~20 行代码 + 测试）——两变量实验文件名 + git hash 已可追溯，YAGNI。

### D3：GBNF 冒烟降级路径——**已触发（不兼容）**

stage 1 小场景冒烟（附录 B）判定链路兼容性：**结论不兼容**——llama-server b8685 对 grammar + tools 同传的请求直接 400 拒绝（`"Cannot use custom grammar constraints with tools."`），零生成。按本决策预设降级路径执行：

- 矩阵缩为 2 列（D1 实际形态）；
- 负结果归档（附录 B）——「GBNF 与 tools 协议在该 llama-server 版本结构性互斥」本身是有价值的实验结论，关闭了 brainstorm 风险 1 的不确定性；
- ADR-0030 决策 C 的 grammar 逃生舱状态从「留口子」更新为「**tools 协议模式下结构性不可用**」——未来若要用 GBNF 约束 tool call 输出，路径是放弃 OpenAI tools 协议、改纯 JSON completion 模式（grammar 约束 + 自解析），属协议层重写非配置开关，超出本 cycle 范围。

### D4：最优配置拍板标准

- **主指标**：L0/L1 通过率 + output_degraded 次数（prompt v2 的两个靶点：缺参引导治 chain_incomplete/wrong_tool 侧、发现链措辞治写操作类 query——output_degraded 主要靶手 GBNF 已移除，v2 对退化间接收敛待观察）。
- **参考指标**：L2 双口径（多步规划是 E2B 能力边界，v2 缺参引导若生效，L2 反问缺参型失败应向 chain 命中迁移）。
- **拍板输出**：agent.toml / prompt 推荐值（README/doc 注明）+ 是否写默认值。**无改善或退化则默认配置不动**（system.md 回滚或保持 v1）；改善则 prompt v2 落地即默认（文本进代码本来就是默认路径），GBNF 开关维持注释态推荐（用户配置层，不进代码默认）。

### D5：record_start/stop agent 语义（stage 2 已实装；「录制范围」措辞按实装修订）

| 维度 | 设计 |
|---|---|
| **录制范围** | agent 调 proc_record_start 经 TUI confirm（y）后 spawn headless 录屏子进程（`proc record --no-tui`，ratatui TestBackend 合成 120x40 系统仪表盘，与 MCP `proc mcp` 路径同款语义）——录制**后台系统监控画面**（进程列表 / DNS / 指标面板），**不含 AgentPanel 对话与真实终端内容**；输出格式与手动 R 键同款 VT100 recording v2 / `.prec`。（stage 1 原稿「录制整个终端屏幕 / 后续对话被录进回放」与复用路径实装不符，按风险 4 协议以实装为准修订） |
| **句柄持有** | record handle 由 **AgentSession 层**持有（跨 tool 调用保活——query 之间不丢）；复用 MCP 层 `proc mcp` 既有 record 子进程管理（ADR-0029 record_handle pattern，不重写） |
| **停止路径 ①** | 模型调 proc_record_stop（confirm y）显式停——返回落盘路径与文件信息，模型可在 answer 里告诉用户 |
| **停止路径 ②③** | Ctrl+D teardown / 会话异常 shutdown → 自动 stop 落盘（防孤儿录制进程——复用 App::shutdown 防孤儿模式），Notice 双落点提示「录屏已自动保存至 \<path\>」（AgentPanel ChatEntry + App status_message——面板退出后 status bar 仍可见） |
| **CLI ask 拦截** | 单轮进程退出录制即死——继续拦截，文案改「录屏 tool 仅 TUI AgentPanel 会话支持（CLI 单轮进程无法保持录制）」 |
| **eval 口径** | 不受影响（eval complete 路径无 confirm 通道，两 tool 永远 blocked——D6 论证） |

**stage 2 实装清单预览 → 落地注记（2026-08-23 实装完成）**：

- `src/agent/session.rs`：新类型 `RecordState`（`child: Arc<Mutex<Option<Child>>>` + `file_path: Arc<Mutex<Option<String>>>` 双槽——file_path 记忆 start 落盘路径）+ `start` / `stop` / `teardown_stop` 三方法（全部薄包 MCP `make_record_start_json` / `make_record_stop_json`）；`AgentSession::spawn` 内建状态（**签名不变，builder.rs 零改动**），session 线程 clone 喂 runner + 循环退出兜底 teardown，`SessionHandle` clone 供 `stop_orphan_recording()`
- `src/agent/runner.rs`：`AgentRunner` 加 `record` 字段 + `with_record_state` 注入（CLI ask / eval 走 default——complete 路径 dispatch_value 层拦截，永不触达）
- `src/agent/tools/dispatch.rs`：`execute_confirmed_tool(call, &RecordState)` 两分支真实执行（catalog schema 参数 `output`/`duration`，兼容 MCP 风格 `file_path`/`duration_secs`）；**stop 无参语义**——agent catalog 的 `proc_record_stop` 是 no_params，忽略模型参数以 start 记忆值为准（MCP 版的 file_path 匹配校验在 agent 侧退化；无录制返业务错误非 is_error）；CLI 拦截文案已按上表落地
- `src/app.rs`：`teardown_agent_session` 在 interrupt/shutdown 前调 `stop_orphan_recording()`（App::shutdown 同路径覆盖 Ctrl+C）；session_loop 退出兜底 `teardown_stop()`（Handle 直接 drop 未走 App teardown 的场景，静默幂等双保险）
- confirm_summary 两行（dispatch.rs L126-127）已就位不动；测试三组（`tests/test_agent_v0_23_stage_2.rs` 端到端 + handle 级孤儿清理 + CLI 拦截锚，`src/agent/session.rs` 内联单元组 fake child kill / 幂等 / 无录制）
- **附录 A 修订 2 核对**：「录屏」在写操作枚举中的引导（先调 proc_help 发现 → 正常调用 → blocked 才文字解释）与实装一致，**不动**（措辞引导调用行为，不依赖录制内容语义）

### D6：record 落地不破 eval 基线的依据归档

三重论证作为决策 6（v0.22 brainstorm）的正式记录：

1. **零 query 依赖**：70 query 中 recording 场景 5 条（L0 2 + L1 2 + L2 1）的 expected_tools 全是 `proc_replay_info` / `proc_replay_search` / `proc_bookmarks_list`，无一条依赖 proc_record_start/stop；
2. **catalog 名单不变**：两 tool 早在 catalog 47 名单内（v0.20 起在册），仅 dispatch 返「不支持」——落地改的是 dispatch 行为，catalog/schema 零变化，模型可见面不变；
3. **eval 无 confirm 通道**：eval 走 complete 路径，写 tool 一律 blocked JSON——record tool 在 eval 口径下永远不真实执行，行为变更对基线不可见。

## Consequences

- v0.23 stage 3 挂机清单缩减为 2 次 FULL（≤~1.6h，原 3 次 ~2.5h）；
- GBNF 逃生舱关闭（tools 协议模式下）：未来 tool call 输出约束的路径是协议层重写（纯 JSON completion + 自解析）或 llama-server 升级后复测（附录 B 版本语境）；
- 实验列（prompt v2 增益 + 可选终验）成为 v0.24 RAG cycle 的模型底座决策输入；
- record 落地后 47 tool 全部真实可用（「不支持」清单零项）——agent tool 能力拼图补全；
- prompt v2 若拍板落地即成默认 system prompt（回滚路径 = git revert + rebuild）。

## 与既有 ADR 关系

- **建立在 ADR-0032（eval harness）之上**——实验矩阵全部用既有 `proc agent eval` / `--compare` 能力，零 harness 改动；
- **更新 ADR-0030 决策 C**——grammar 逃生舱从「留口子」进入「实测验证：tools 协议下结构性不可用」状态（ADR-0030 本文不改 Status，其决策 C 的口子仍留给纯 completion 模式的未来路径）；
- **D5 参考 ADR-0029**（record_handle pattern 的 agent 侧复用）；
- **写操作 confirm 契约沿用 ADR-0008/0029**（record_start/stop 走 confirm 通道，与 kill/eject 同款）。

## 附录 A：prompt v2 措辞稿（stage 3 任务 ② 落地，本 stage 只定稿不进代码）

> 对象：`src/agent/prompts/system.md`。只做 2 处小修不动结构（类别路由表 / 快照段 / 字数约束全不动）——v0.20 stage 3b few-shot 教训（prompt 大改全局回归）的 mitigate。变量隔离需要 prompt v1 先跑完 GBNF 列……GBNF 列已移除，但隔离逻辑不变：**v1 基线列已有（v0.22 归档），stage 3 改 v2 + rebuild 后跑 v2 列**。

### 修订 1（缺参引导——治 L2 反问缺参 #19/#25/#26/#29）

在「推理类问题」行（line 17）之后插入一行：

```diff
  - 推理类问题（如「为什么卡」）先用 proc_metrics_system 看全局，再用 proc_ls 深挖，最后给建议。
+ - 用户问题缺少具体参数（盘符 / PID / 容器名等）时，先用无参或列表型 tool 枚举可用对象（如 proc_eject_status 列出所有盘、proc_ls 列出进程），从结果中定位目标后再继续——不要直接反问用户要参数。
```

### 修订 2（写操作发现链——治 v0.21 观察 3 / v0.22 #21 直接文字解释不走发现链）

替换 line 20：

```diff
- - 写操作（kill / 删容器 / 释放 USB）已被平台拦截：在答案里解释影响并给出等价 proc 命令行，让用户自己执行。
+ - 需要执行写操作（kill / 删容器 / 释放 USB / 录屏）时：先调 proc_help 找到对应 tool 并正常调用（带完整参数）；调用被平台拦截（blocked）后，再在答案里解释影响并给出等价 proc 命令行，让用户自己执行。不要未经调用就直接声明「无法执行」。
```

修订 2 顺带把「录屏」纳入写操作枚举（stage 2 record 落地后的措辞一致性——风险 4：若 stage 2 实装语义与附录 A 漂移，以 stage 2 实装为准同步更新本附录）。

## 附录 B：GBNF 冒烟判定标准与实测结论

### 判定标准（兼容 = 4 项全满足）

| # | 检查项 | 兼容证据 | 不兼容证据 |
|---|---|---|---|
| 1 | llama-server 接受请求 | 无 LlmError（grammar 未被拒绝） | 连接/400 错误 |
| 2 | tool_calls 协议解析正常 | actual_tools 有真实调用（grammar 约束的 JSON 被 llama-server 转为结构化 tool_calls 而非文本） | 大量 NoToolCall + final_text 是裸 `{"tool_calls":...}` 文本 |
| 3 | proc_finish 提取正常 | final_text 是自然语言答案（answer 字段提取成功） | EmptyAnswer / final_text 含 JSON 外壳 |
| 4 | 退化转移观察 | output_degraded 次数相对基线场景不升 | JSON 字段内退化（新形态，记失败仍可检测） |

### 实测结论（2026-08-23，stage 1 冒烟）：**不兼容——检查项 1 即 FAIL（判定性）**

**版本语境**：llama-server `b8685`（win-cuda-12.4-x64，`D:\llama.cpp\bin\llama-b8685-bin-win-cuda-12.4-x64\llama-server.exe`）+ gemma-4-E2B-it-Q4_K_M。结论绑定此版本——未来升级 llama.cpp 后可复测（llama-server 对 grammar × tools 的互斥校验若放开，本附录结论可重开）。

**实测数据**：

| 冒烟 | 场景 | query × attempts | 结果 |
|---|---|---|---|
| smoke1 | performance-diagnose L0（单步形态） | 3 × 2 = 6 请求 | 全部 `llm_error`，0 步，0 生成 |
| smoke2 | usb L1（发现链形态） | 3 × 2 = 6 请求 | 全部 `llm_error`，0 步，0 生成 |
| smoke3 | usb L2（多步链形态） | **未跑**——错误在请求校验层（server-side validation），与 query 内容/场景/步数无关，12 次同型请求已是判定性证据 | — |

**错误体原文**（每次请求完全一致）：

```
status=400 body={"error":{"code":400,"message":"Cannot use custom grammar constraints with tools.","type":"invalid_request_error"}}
```

**判定表逐项**：

| # | 检查项 | 判定 | 证据 |
|---|---|---|---|
| 1 | llama-server 接受请求 | **FAIL** | 400 invalid_request_error，显式消息「Cannot use custom grammar constraints with tools.」 |
| 2 | tool_calls 解析 | N/A | 零生成（请求即被拒） |
| 3 | proc_finish 提取 | N/A | 同上 |
| 4 | 退化转移 | N/A | 同上 |

**结构性结论**：llama-server b8685 在 `/v1/chat/completions` 上**显式禁止 grammar 与 tools 同传**——不是解析优先级问题或输出形态问题，是请求级硬校验。v0.20 stage 3a 极简实验（`root ::= "yes"`）当时能生效是因为**不带 tools**（纯文本生成）；带 tools 的完整 agent 循环（grammar + `tool_choice=Required` + proc_finish）从未实测——brainstorm 风险 1 预判的坑实测确认，且以最干净的二值形态（无「部分兼容」中间态，风险 1 的补跑预案无需启用）。

**冒烟输出留档**：`eval-gbnf-smoke1.json` / `eval-gbnf-smoke2.json` 本地留存（同 v0.22 惯例不入 commit）；agent.toml 冒烟后已还原（grammar_file 回注释态，与备份 diff 为空）。

## Migration path

- **v0.23 stage 1 Spike**（本 ADR 落地）：D1~D6 + 附录 A prompt v2 措辞稿 + 附录 B 冒烟实测结论
- **v0.23 stage 2 Slice A**（✅ 2026-08-23 完成）：record_start/stop agent 侧实装（D5 语义 + 上方落地注记）
- **v0.23 stage 3 Slice B**：实验矩阵 2 列 FULL 挂机（prompt v2 / 可选终验）+ `--compare` 矩阵报告 + 归档 `docs/eval/` + 最优配置拍板（D4 标准）
- **v0.24+**：实验列作 RAG cycle 模型底座决策输入；GBNF 复测挂在 llama-server 升级节点

## References

- [`docs/stages/v0.23-brainstorm.md`](../stages/v0.23-brainstorm.md)：cycle 总览 + 9 决策（本 ADR 是决策 4/5/6 的展开）
- [`docs/eval/e2b-70q-v0.22.md`](../eval/e2b-70q-v0.22.md)：E2B 基线画像（实验假设的数据来源）
- [`src/agent/grammars/tool_call.gbnf`](../../src/agent/grammars/tool_call.gbnf)：GBNF 内容（v0.20 stage 3a 实测修正版）
- [`src/agent/builder.rs`](../../src/agent/builder.rs)：决策 C grammar 逃生舱接线（~L81-103）
