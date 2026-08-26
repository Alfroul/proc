# ADR-0034：RAG 历史经验召回（检索层 + 注入层 + 评估口径）

**Status**: Accepted（v0.24 stage 1 Spike 落地——D1~D5 五终判 + 附录 A prompt v3 措辞稿 + 附录 B session 语料实测盘点）

**Date**: 2026-08-24（v0.24 cycle stage 1 Spike）

**Related**: ADR-0032（eval harness——测量层 + SessionRecorder 语料源）、ADR-0033（实验纪律——方差带标尺 + prompt v3 素材 + D4 保守拍板标准）、ADR-0030（内置 agent 基座——entry-tool 架构与 token 预算纪律）

## Context

v0.23 cycle 完结（tag `v0.23.0`，2026-08-24）留下两组输入：

1. **E2B 画像固定 + 唯一剩余杠杆搁置**：GBNF 与 prompt v2 双负结论后，E2B 画像（L0 74% / L1 52% / L2 full-chain 5% / output_degraded 21）维持 v0.22 基线；v0.24 拍板**不下载**新模型（brainstorm 决策 1 D）——模型升级杠杆搁置，E2B 作为本 cycle 固定底座。
2. **方差带标尺 + 基线族即插即出**：单次 run ±3 通过数 / ±6 失败模式计数以内不可单独归因（v0.23 最重要遗产）；`eval-e2b-70q.json`（v0.22 v1 基线）+ `eval-best-70q.json`（v0.23 同配置方差列）本地留存，`--compare` 零 harness 改动出新列。

**RAG 动机**：把历史会话中的成功经验（query → tool 链 → 结论）检索出来注入 agent context，让 agent「见过一次就会第二次」。拍板口径（brainstorm 决策 2）：**机制验证为主指标**（检索准不准 + 注入有没有被用上——小语料 + E2B 也能验证），通过率增益按方差带标尺如实解读、不预设涨通过率——「E2B + RAG 收益存疑」的既定判断由实验数据闭环或维持，**「机制成立但 E2B 兑现不了」与「机制不成立」分离归档**，前者给 v0.25+ 模型底座重启决策留数据输入。

**语料现状**（附录 B 实测，2026-08-24）：session JSONL 57 文件中仅 **2 个成功 query 段**（去重后 1 个独立 query）——主语料极薄；bootstrap 备选（3 基线 run passed trace 去重）**40 独立 query**（9 场景全覆盖，33 条多步链）。

## Decision

### D1：检索方案终判——keyword BM25-lite 评分，零 deps ✅

**三路对比**（brainstorm 倾向转正）：

| 方案 | 依赖 | 规模 | 判定 |
|---|---|---|---|
| **keyword（BM25-lite）** | **+0** | ~150-250 行业务 | ✅ **终判采纳**——语料百级条目 keyword 完全够用；Cargo deps +0 纪律（v0.22/v0.23 双零）延续；stage 2 全量可单测（纯函数） |
| 本地 embedding | 新重 dep（fastembed/candle + 模型文件） | ~500 行+ | ❌ 语料量级不匹配（百级条目上向量检索收益无法兑现）+ 违背轻依赖纪律 |
| 简化 SQL（rusqlite） | 新 dep | ~300-400 行 | ❌ 同上——无结构化复杂查询需求，内存索引即可 |

**评分算法设计**（stage 2 实装规格）：

- **分词**（零 dep 口径，中英文混合）：ASCII 按非字母数字切词（lowercase）；连续 CJK 字符按 **2-gram** 切分（「弹出」→「弹出」单 bigram 命中即可，避免词典依赖）
- **评分**：`score(entry) = Σ_{t ∈ tokens(current)} idf(t) × tf(t, entry.query)`——idf 按语料条目数计算（`ln(1 + N / df)`），tf 简化计数上限 3（防单条目词频刷分）；不做长度归一化（条目 query 均短）
- **top-k = 3**（预算内最大条数），**min_score = 1.0** 门槛（低于不注入——避免无关注入吃预算）
- 排序并列时按条目 tool 链长度降序（多步经验信息量更大）

**stage 2 实装清单预览**：`src/agent/rag/` 新模块——`corpus.rs`（`Entry { query, tools: Vec<String>, conclusion_head, source }` + 语料解析：sessions JSONL 成功段提取 / bootstrap eval JSON passed trace 提取）、`retrieve.rs`（tokenize + score + top-k + 污染排除（D4））、`mod.rs`（`RagIndex` 组装 + `inject_experience` 模板渲染（D2））。测试：`tests/test_agent_rag.rs` 集成 + 模块内联单元（语料 fixture 构造 / 命中排序 / 排除逻辑 / 空·薄语料降级返 None）。

> **stage 2 落地注记（2026-08-25）**：三模块实装与预览一致（corpus 165 / retrieve 121 / mod 196 行业务，含文档注释；测试 272 内联 + 368 集成，28 测试全绿）。补充件：`RagParams` 参数包（top_k / min_score / exclude_threshold / budget_chars 四定值打包，stage 3 `RagConfig` 映射入口）+ `RagIndex::from_entries` 公开构造（测试与调用方直喂语料）。规格歧义消解两处：① 评分公式 `Σ_{t ∈ tokens(current)}` 按 query 侧**去重集合**迭代（同 token 重复出现不重复计分）；② CJK 边界定死——CJK 判定 U+4E00~U+9FFF，中文标点等其余字符一律作分隔，单字 CJK 段产单字 token。

### D2：注入位置与 token 预算终判——per-query 预注入 ✅

**两路对比**（brainstorm 倾向转正）：

| 路线 | 机制 | 判定 |
|---|---|---|
| **per-query 预注入** | 每 query 检索 top-k 相似历史成功链，注入 user message 前缀，预算硬上限 | ✅ **终判采纳**——不依赖模型主动性：**E2B 级模型上惰性发现链已被证明不可靠**（proc_help 动态发现本身就是 L2 失败模式之一；meta tool 大概率不被调用，eval 列难出差异）；可测差异更可能产生 |
| 惰性 meta tool（proc_experience） | catalog 47→48，agent 主动调取 | ❌ 依赖模型主动发现——当前能力画像下机制验证无法开展 |

**注入格式**（query 文本层包装，user message 前缀）：

```
[历史经验参考] 以下是与当前问题相似的历史成功解法，仅供参考，不要照抄与当前问题无关的步骤：
- "列出 CPU 占用最高的进程" → proc_ls（结论：java.exe PID 13828 占用 38%…）
[当前问题] <原 query 文本>
```

- 每条目格式：`- "<query>" → tool1 → tool2（结论：<head 截断 ~80 chars>）`
- **注入位置 = user message 前缀**（非 system 尾部），三个理由：① 临近性——小模型对近期 token 注意更强；② **变量隔离**——system.md 是 prompt v1/v3 变量的载体，RAG 注入完全不碰 system.md / `build_system_prompt` 路径，stage 3 两列实验文件级解耦；③ 可观测——注入内容与 query 同段进 session log 与 eval trace，经验引用观察（D5 主指标②）直接对照
- 注入发生在 runner 的 query 入口（`run` / `run_streaming` 共用 `rag::inject_experience(query, &index, budget)` helper）；包装文本随该 query 的 user message 入 history，后续轮零特殊处理
- 无命中（min_score 门槛下零条目）→ **原文透传，零注入痕迹**（off 态与 on-无命中态行为一致）

**token 预算**：硬上限 **800 token**（brainstorm 500-1000 带内取中；中文 ~1.5 chars/token 粗估 → 实装按 **1200 chars 硬上限**近似 + 单测断言）；超出按相关性降序**整条截断**（不截半条）。注入条目数与估算 token 数进 rag 模块结构化输出（stderr log 行），stage 3 归档收集。

**`agent.toml [rag]` 配置**（默认 off 保基线）：

```toml
[rag]
enabled = false            # RAG 经验召回总开关（默认 off——基线隔离）
budget_tokens = 800        # 注入段 token 预算硬上限
top_k = 3                  # 检索条数上限
exclude_threshold = 0.6    # 污染排除：词元覆盖率阈值（D4）
eval_corpora = []          # bootstrap 语料：eval run JSON 路径列表（空 = 仅 session 语料）
```

**catalog 47 不变**（预注入路线不加 tool）；**Cargo deps +0**。

**stage 3 接线点预览**：`src/agent/config.rs`（`RagConfig` + Default=off）+ `src/agent/builder.rs`（读配置建 `RagIndex` 传 runner——off 态不建索引零开销）+ `src/agent/runner.rs`（query 入口注入 helper 调用）。

> **stage 3 落地注记（2026-08-25/26）**：三接线点与预览一致（`RagConfig.params()` 做 `[rag]` → `RagParams` 映射，`budget_chars = budget_tokens * 3 / 2` 整数运算——800→1200 与默认同值锚，`min_score` 定值 1.0 不暴露配置）；`AgentSession::spawn` 4→5 参（+`Option<(Arc<RagIndex>, RagParams)>`，v0.22 stage 3 同款先例）。实装口径两处明示：① **透传 = 截断 probe**——runner 侧 `rag_wrap` 先 `truncate_chars(query, 200)` 再进 `inject_experience`，无命中透传的是 probe（与 session 源侧 200 chars 同底一致；eval query 全部远短于 200 不触发）；② **stderr 报告行** `[rag] injected=<bool> entries=<n> excluded=<n> est_tokens=<n>` 每 run() 调用一行（attempt-2 重跑也计），off 态零输出。**挂机顺序硬约束**（RAG-on 列必须 prompt v1 binary——system.md include_str 嵌入无配置开关，v3 commit 必须后于 RAG-on 列挂机）。**实验终态**：RAG-on 列 +2 vs 方差列落带内 → `enabled` 默认维持 off（D5 三分支「维持 off + 数据归档」）——机制验证两主指标成立（召回 12/15 · 引用 8/15 · 干扰 0 · output_degraded -12 超带），「机制成立但 E2B 兑现不了通过率」归档 `docs/eval/rag-v3-70q-v0.24.md`；v3 列 -5/-8 落带外 → revert `7959030`（附录 A 措辞稿留档，TD-60 终态回填 stage 4）。

### D3：语料口径终判——session JSONL 主语料 + eval trace bootstrap（本 cycle 机制验证必需）✅

**索引单元**（D1 `Entry`）：

- **成功段定义**：`query_started` → 段内 `tool_start` ≥ 1 → 正常 `session_finished` 收尾且无 `error` 事件的 seq 区间
- 条目内容：query 文本 + tool 名保序序列 + 结论摘要（`final_head` 截断 ~200 chars）
- **语料筛选**：只索引成功段——空会话 / 零 tool 段 / error 段一律不进索引（风险 5 mitigate；附录 B 实证：96% 文件是零 query 空会话，筛选规则是实证必需非防御性设计）

**双语料源**：

| 语料源 | 解析对象 | 角色 |
|---|---|---|
| **session JSONL（主）** | `~/.config/proc/sessions/*.jsonl` 成功段 | **生产路径**——真实会话天然持续增长，机制先建好收益后兑（brainstorm Q3 拍板逻辑） |
| **eval run JSON（bootstrap）** | `eval_corpora` 路径列表的 passed trace（query + actual_tools + final_text_head） | **本 cycle 机制验证必需**——附录 B 实测主语料仅 2 条可索引（去重 1 独立 query），远低于 ~10 条bootstrap 触发线；40 独立 query 去重池支撑检索召回对照与 RAG-on eval |

**bootstrap 口径明示**（brainstorm 风险 3 mitigate (2) 兑现）：bootstrap 条目 `source = "eval"` 标记；**启用即受 D4 污染防护约束**（同源语料排除机制是唯一防线）；stage 3 RAG-on 列挂机配置 = `eval_corpora = ["eval-e2b-70q.json", "eval-promptv2-70q.json", "eval-best-70q.json"]`（三 run 去重后 40 条，覆盖 9 场景 × L0 20 / L1 19 / L2 1）。

**索引构建时机**：session 启动 / eval run 启动时**全量重建**（百级文件毫秒级，不做增量——简单优先）；构建失败（目录缺失 / JSON 损坏）→ 静默降级空索引 + 一次 stderr 警告（`SessionRecorder::disabled()` 同款契约）。

> **stage 2 落地注记（2026-08-25）**：成功段状态机实装口径——非 `end_turn` 收尾（max_steps / empty_after_retry / interrupted）与未收尾段（query_started 后无 session_finished）均不产出，段内任意 `error` 事件整段作废；去重规则 = 归一化 query（lowercase + 去空白）全局保首见，装载序 session 先 eval 后（真实语料优先）；bootstrap 经 `EvalRunFile` 反序列化直读（schema 与 harness 同源，零新解析代码），坏源警告跳过。session JSONL 的 query text 源侧已 200 chars 截断——Entry.query 即截断版，与排除判定同底无碍（stage 3 runner 侧注入前以同款 200 chars 截断做 exact/coverage 判定即可保持一致）。

### D4：污染防护终判——相似 query 排除 + 命中次数报告 ✅

**问题**：经验库含 eval 同款 query 的成功 trace 时，RAG-on 跑 eval 等于检索答案——增益是信息泄漏非能力提升（brainstorm 风险 3）。

**排除判定**（与 D1 分词同底，检索前过滤）：

- **exact match 一律排除**：候选条目 query 与当前 query 归一化后（lowercase + 去空白）全等
- **词元覆盖率 ≥ 0.6 排除**：`coverage = |tokens(current) ∩ tokens(entry.query)| / min(|tokens(current)|, |tokens(entry.query)|)`——**双向 min 分母**（任一方向高度覆盖即排除，保守口径：既防「同款 query 微改写」也防「长 query 包含短历史 query」型泄漏）
- 阈值 0.6 定值依据：同场景不同意图的 query（如「列出 USB 盘」vs「弹出 E 盘」）覆盖率典型 < 0.4（CJK bigram 稀疏）；同款改写（增删一两个词）典型 > 0.7——0.6 落在分离带内。stage 2 单测用 fixture 锚定三类样例（同款 / 高覆盖改写 / 同场景异意图）

**边界诚实声明**：同场景同意图不同措辞的 query 互相检索是 RAG 的**设计目的**（相似问题的历史解法），不是污染——排除阈值只挡「答案直达」型泄漏，不挡「同类经验」型参考。两类边界由 0.6 阈值切分，stage 3 机制验证报告抽样复核切分质量。

**命中次数报告**：RAG-on eval run 期间 rag 模块统计 per-query 的排除条目数 / 注入条目数 / 估算 token 数，stderr 结构化输出；stage 3 归档「排除命中次数」汇总表证明防护生效（brainstorm 风险 3 mitigate (3) 兑现）。

> **stage 2 落地注记（2026-08-25）**：coverage 实装为**去重集合**口径（`token_set` 交集 / 双向 min 分母），恰 0.6 按 `>=` 排除；排除计数 `excluded` 随 `RetrievalOutcome` 与 `InjectedQuery` 返回（`injected_entries` / `est_tokens` 同包），stage 3 stderr 结构化输出的数据源就位。三类样例（同款 exact / 高覆盖改写 / 同场景异意图）已 fixture 锚定（集成 C 组——第三类同时锚定「不排除且可命中」的同类经验参考语义）。

### D5：评估口径终判——机制验证主指标 + 增益方差带解读 ✅

**主指标（机制验证，不依赖通过率）**：

1. **检索准不准——召回对照**：从 queries.toml 70 条抽 ~15 条（覆盖 9 场景 × 3 level），人工标注每条的相关经验条目集合（bootstrap 40 条池内分组）；对照 top-3 检索结果——命中率 = top-3 含 ≥1 标注相关条目的 query 占比 + 平均命中相关条数。**离线可跑**（挂机前，不耗 LLM）
2. **注入有没有被用上——经验引用观察**：RAG-on run 的 final_text / tool 链 vs 注入经验对照——模型 answer 是否引用经验结论（数字 / 结论复述）、tool 链是否向经验链迁移（同 tool 序列子序列命中比例）；抽样 ~15 query 人工判读，三分类：引用 / 无视 / 干扰（注入误导）

**通过率增益**：按方差带标尺（±3 通过数 / ±6 失败模式计数）如实解读；**两分结论框架**——「机制成立但 E2B 兑现不了」（主指标 ①② 成立 + 通过率落带内）vs「机制不成立」（检索不准或模型无视注入）——前者给模型底座重启决策留数据输入（决策 1 归档候选表呼应），后者关闭或重设计 RAG 路线。

**off 态复用基线族的可比性论证**（stage 3 compare 合法性依据）：

- v0.24 binary 默认配置（RAG off + prompt v1 + E2B + attempts=2 / max_steps=10）与 v0.22 `eval-e2b-70q.json` / v0.23 `eval-best-70q.json` run **同口径**——差异仅 binary `git_describe`（代码演进但 off 态零行为变更：`enabled=false` 时零检索零注入零开销，stage 2/3 回归 1689/1713 不动是佐证）
- 跨 binary 版本复用基线的先例：v0.23 prompt v2 列直接对比 v0.22 归档的 `eval-e2b-70q.json`（REVIEW-v0.23 口径）；v0.24 延续——**off 态不新增挂机列**（brainstorm 风险 7 mitigate 兑现）

**RAG 默认 off 不破基线**：`RagConfig` Default=off；off 态不建索引、不走注入路径；回归基线 1689 / 1713 不动是 stage 2/3 完工验收项（D6 三重论证同款先行论证）。

**stage 3 拍板标准**（ADR-0033 D4 保守标准同款 + 方差带分级）：

- 净通过差 **≥ +7（显著超带）** 且主指标 ①② 成立 → `enabled` 默认值改 `true` 候选（改默认与换模型同级决策，用同级门槛）
- 净通过差 **+4 ~ +6（超带但未显著）** → 复跑 1 列方差定夺（D4 三分支同款）
- 落带内（±3）→ 默认维持 off + 数据归档（agent.toml 注释态推荐，不强制改默认）——「机制成立但 E2B 兑现不了」归档路径

## Consequences

- Cargo deps +0（keyword 路线）；agent catalog 47 不变（预注入，不加 meta tool）；MCP 46 不变
- RAG off 默认不破 eval 基线（1689/1713 回归锚不动）；on 态单变量新增 1 列，off 态复用基线族零新增挂机
- bootstrap 语料启用使 D4 污染排除从防御性设计变为**归因生命线**——排除命中次数报告进 stage 3 必归档项
- 机制验证与增益分离归档：无论通过率涨不涨，检索召回对照 + 经验引用观察都有独立结论，v0.25+ 模型底座重启决策有数据输入
- prompt v3（附录 A）与本 ADR 主体在 stage 3 同矩阵但文件级解耦（v3 只碰 system.md，RAG 不碰）

## 与既有 ADR 关系

- **建立在 ADR-0032 之上**——eval 测量层（`--compare` 零改动出新列）+ SessionRecorder JSONL 格式（主语料的数据结构来源，10 kind 事件流）
- **延续 ADR-0033 实验纪律**——方差带标尺（±3/±6）作增益解读前提 + D4 保守拍板标准 + 文件名区分 run（`eval-rag-on-70q.json` / `eval-v3-70q.json`）
- **不更新 ADR-0030 Status**——entry-tool 架构不动（预注入不加 catalog），token 预算纪律从 entry 4 tool ~600 扩展到「+ RAG 注入 ≤800」双预算口径

## 附录 A：prompt v3 措辞稿（修订 2 单独 diff，本 stage 只定稿不进代码）

> 对象：`src/agent/prompts/system.md`。**只取 ADR-0033 附录 A 修订 2（写操作发现链），不含修订 1（缺参引导）**——v0.23 D4 拍板已证修订 2 机制可复现生效（idx63 两轮过 / #21 完整执行演示链）、修订 1 无明确靶点（靶点场景缺真正的无参枚举 tool）。v3 = 修订 2 单变量，stage 3 变量隔离需要。
>
> 本 stage 已核对 system.md 当前 v1 态（revert `c043597` 后）：line 20 与下列 diff 上下文逐字一致。**stage 3 落地独立 commit 先于挂机**（v0.23 列 ② 同款流程），落地时再核对一次。

**修订 2（写操作发现链——治 v0.21 观察 3 / v0.22 #21 直接文字解释不走发现链）**，替换 line 20：

```diff
- - 写操作（kill / 删容器 / 释放 USB）已被平台拦截：在答案里解释影响并给出等价 proc 命令行，让用户自己执行。
+ - 需要执行写操作（kill / 删容器 / 释放 USB / 录屏）时：先调 proc_help 找到对应 tool 并正常调用（带完整参数）；调用被平台拦截（blocked）后，再在答案里解释影响并给出等价 proc 命令行，让用户自己执行。不要未经调用就直接声明「无法执行」。
```

**authoring 约束**：单处替换不动结构（类别路由表 / 快照段 / 字数约束全不动——v0.20 stage 3b few-shot 教训延续）。修订 2 措辞已按 v0.23 stage 2 record 实装核对过一致性（ADR-0033 D5 落地注记末条），「录屏」枚举与 47 tool 现状一致，无需再改。

**stage 3 v3 列口径**：E2B × RAG off × v3——system.md 落本修订 + rebuild 后跑 `eval-v3-70q.json`，对比同底座基线族（`eval-e2b-70q.json` + `eval-best-70q.json` 方差列），无需多挂基线列；存疑时按 ADR-0033 D4 三分支复跑定夺。

## 附录 B：session 语料实测盘点（2026-08-24 stage 1）

**对象**：`~/.config/proc/sessions/*.jsonl`（brainstorm 拍板时点 53 文件，stage 1 执行时点 **57 文件**——同日 +4，语料持续增长中但增长主体是空会话）。盘点脚本本地一次性（python），不入 commit。

**总量与构成**：

| 指标 | 数值 |
|---|---|
| 文件数 | 57（日期分布：08-21 ×4 / 08-22 ×12 / 08-23 ×19 / 08-24 ×22） |
| 事件总行数 | 69 |
| provider | llama-cpp 57/57 |
| **query 段总数** | **2**（55 文件零 query——AgentPanel 进入即写 `session_start`，多数未发 query 即退出，**空会话占 96%**） |
| 成功段（≥1 tool + end_turn + 无 error） | **2 / 2**（100%） |
| **可索引条目** | **2 段，去重后 1 个独立 query** |
| 每段 tool 数 | 均值 1.0（max 1） |
| tool 错误 / error 事件 | 0 / 0 |
| text_delta 事件 | 0（直接 tool → proc_finish 形态，与 E2B TTFT=None 既有观察一致） |

**唯一可索引条目原文**：「列出当前 CPU 占用最高的 3 个进程」→ `proc_ls` → end_turn（final_head 242 chars 正常中文答案，含 java.exe PID/CPU 数据）。

**bootstrap 备选语料盘点**（3 基线 run passed trace 去重）：

| 指标 | 数值 |
|---|---|
| passed trace 原始 | 100（e2b 32 + promptv2 33 + best 35，含跨 run 重复） |
| **去重后独立 query** | **40** |
| level 分布 | L0 ×20 / L1 ×19 / L2 ×1 |
| 场景分布 | process-diagnose 8 / performance-diagnose 6 / security 6 / usb 5 / flow 4 / monitor 4 / dns 4 / docker 2 / recording 1（9 场景全覆盖） |
| 含 ≥2 tool 链 | 33 条（多步经验主体） |

**结论**：

1. **主语料可索引条目 2 条（去重 1）——远低于 ~10 条 bootstrap 触发线**（stage doc 风险 2 预设阈值）。session 语料的经验密度受 AgentPanel 使用频率制约（真实 query 才产生经验，空会话不产生），按当前使用节奏短期内无法支撑检索层验证。
2. **D3 口径结论：bootstrap 从「备选」升级为「本 cycle 机制验证必需」**——session 主语料机制照建（面向持续增长的生产路径，`source` 字段区分），stage 3 检索召回对照与 RAG-on eval 的语料主体是 bootstrap 40 条池。
3. 空会话占 96% 实证了 D3 语料筛选规则（只索引成功段）的必要性——索引器跳过 55 个零 query 文件是常态路径非边界情况。
4. 40 条 bootstrap 池 + D4 排除（exact + 覆盖率 0.6）后，每个 eval query 的可检索池仍有数十条异 query 经验（同场景异意图条目为主）——机制验证（检索准不准 / 被用上）语料可行。

## Migration path

- **v0.24 stage 1 Spike**（本 ADR 落地）：D1~D5 五终判 + 附录 A prompt v3 措辞稿 + 附录 B 语料实测盘点
- **v0.24 stage 2 Slice A**：RAG 检索层实装（`src/agent/rag/` corpus / retrieve / mod + 单测——D1 规格 + D4 排除逻辑）✅（2026-08-25 完成——482 行业务 + 640 行测试（28 测试全绿），库形态零接线（runner / config / builder diff 为空）、Cargo deps +0）
- **v0.24 stage 3 Slice B**：注入层接线（D2 规格 + `[rag]` 配置默认 off）+ eval 矩阵挂机（RAG-on 列 + v3 列，附录 A / D5 口径）+ 机制验证报告 + 拍板（D5 标准）✅（2026-08-26 完成——接线 `b959277` + v3 落地 `e166e27` + revert `7959030`；拍板：RAG 维持 off（机制成立但通过率带内）+ v3 负结果归档，详见 `docs/eval/rag-v3-70q-v0.24.md`）
- **v0.24 stage 4**：REVIEW-v0.24 归档（机制验证结论 + 增益方差带解读 + 污染防护实效三段落）
- **v0.25+**：RAG 结论（机制成立与否）作为模型底座重启决策（决策 1 归档候选表）的输入

## References

- [`docs/stages/v0.24-brainstorm.md`](../stages/v0.24-brainstorm.md)：cycle 总览 + 5 决策（本 ADR 是决策 2/3 的展开）
- [`docs/eval/promptv2-70q-v0.23.md`](../eval/promptv2-70q-v0.23.md)：方差带 ±3/±6 数据来源 + D4 保守拍板实录（D5 依据）
- [`docs/adr/0032-eval-harness.md`](0032-eval-harness.md)：eval harness + SessionRecorder（语料数据结构来源）
- [`docs/adr/0033-eval-experiments-and-record-tools.md`](0033-eval-experiments-and-record-tools.md)：附录 A 修订 2 原文（v3 素材）+ 方差带标尺（增益解读纪律）
