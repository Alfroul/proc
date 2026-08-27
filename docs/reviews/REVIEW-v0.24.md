# REVIEW-v0.24 — v0.24 cycle Review（RAG 历史经验召回主体 + prompt v3 搭车 cycle 完结）

> **cycle 范围**：brainstorm 5 决策（2026-08-24 拍板会话全 ✅）——决策 1 不下载更强模型（E2B 维持默认，模型升级杠杆搁置）+ 决策 2 RAG 完整主体（E2B 底座机制验证口径——机制验证主指标 + 通过率增益方差带解读）+ 决策 3 prompt v3 搭车（修订 2 单独 diff）+ 决策 4 TD-55~57 不跑（无 key）+ 决策 5 4 stage；穿插 ADR-0034 + 1 份 eval 归档
>
> **Review 范围**：4 stage 全部产出（1 Spike + 2 Slice + 本 Review+收尾段）
>
> **基线**：1689 passed / 0 failed / 9 ignored（v0.23.0 默认档）+ 1713 / 0 / 10（anthropic 档）→ **1725 passed / 0 failed / 10 ignored（默认档）+ 1749 passed / 0 failed / 11 ignored（`--features anthropic`）**/ fmt / clippy（双档）/ build（双档）/ bench --no-run 全过
>
> **Review 日期**：2026-08-26
>
> **Reviewer**：Claude（stage 4 会话；审查依据 CHANGELOG stage 段 + ADR-0034 落地注记 + [`docs/eval/rag-v3-70q-v0.24.md`](../eval/rag-v3-70q-v0.24.md) 归档，不逐文件重读实现——stage doc 风险 1 口径声明）

---

## 概览

v0.24 cycle 是 proc 首个「机制验证型」cycle（净 diff ~3000 行级：src +921/-7 / tests +723/-0 / docs +1381/-0，21 文件；system.md v3 落地 + revert 净零不入 diff）——**「v0.23 建标尺，v0.24 在固定底座上建召回，机制与增益分离归档」**。RAG 历史经验召回完整落地（检索层 corpus/retrieve/index 三模块 + 注入层接线 `[rag]` 配置，业务+测试 ~1500 行级是主体）但 **`enabled` 默认维持 off**：机制验证两主指标成立（检索召回 80% / 经验引用 57% / 干扰 0）+ output_degraded -12 超带（答案质量增益），而净通过 +2 vs 方差列落 ±3 带内——**「机制成立但 E2B 兑现不了通过率」**两分归档，质量增益给 v0.25+ 模型底座重启决策留直接输入。搭车实验 prompt v3（修订 2 单变量）负结果（-5/-8 落带外向下）→ revert 回 v1，**TD-60 关闭——prompt 措辞杠杆在 E2B 上用数据穷尽**（叠加 v0.23 GBNF/prompt v2 双关闭，E2B 底座零代码/低成本杠杆全部穷尽，进一步改善路径收敛「模型升级 × RAG-on 复测」组合）。

- **MCP tool 46 / agent catalog 47 均不变**（RAG 走 per-query 预注入路线非 meta tool——决策 2 子倾向 + ADR-0034 D2 终判）
- **Cargo deps +0**（keyword BM25-lite 路线零新依赖——vs embedding / SQL 双否的选型兑现）
- 1 份新 ADR-0034（D1~D5 五终判 + 附录 A v3 措辞稿 + 附录 B session 语料盘点）
- 回归 +36 CI（stage 2 +28：检索层四组；stage 3 +8：接线 E/F/G/H 组）+ `#[ignore]` 本地召回探针 ×1

**核心实测数字**（详见各 stage 段 + [`docs/eval/rag-v3-70q-v0.24.md`](../eval/rag-v3-70q-v0.24.md)）：

| 验收项 | 口径 | 实测 | 结论 |
|---|---|---|---|
| session 语料盘点（stage 1） | 成功段状态机全量扫描 | 57 文件仅 **2 个成功 query 段**（去重 1 独立 query，96% 空会话）→ bootstrap 升级为机制验证必需（3 基线 run passed trace 去重 **40 独立 query**，L0 20 / L1 19 / L2 1，9 场景全覆盖） | ✅ 诚实数据驱动设计修订（D3 从「倾向」变「数据结论」） |
| RAG-on 列（stage 3） | FULL 70q vs 基线族，D5 主指标 + 方差带 | 25m20s（四列最快）：净通过 **37**（+5 vs 基线 / **+2 vs 方差列落 ±3 带内**）；L0 20/23（87%）/ L1 17/27（63%）双双四列最优但增量带内；L2 三列平（0/20）；**output_degraded 19→9（-12 vs 基线，超 ±6 带最大信号）** | 🔶→✅ D5 三分支「维持 off + 数据归档」——通过率不归因增益，质量增益超带归档 |
| 机制验证主指标 ① 检索召回 | 15 抽样（9 场景 × L0 6/L1 4/L2 5）top-3 人工标注 | **12/15 = 80%**；3 miss 均语料覆盖制约（docker 域 bootstrap 池仅 2 条 × 2 +「为什么」模板泛匹配 × 1），非算法缺陷（同域充足时首位命中） | ✅ 检索准 |
| 机制验证主指标 ② 经验引用 | 15 抽样三分类（链迁移/结构采纳） | **引用 8 / 无视 6 / 干扰 0**；idx31 前缀采纳 + idx67 整链采纳教科书级；引用但未过 3 例均 L2（链已迁移、能力边界制约） | ✅ 注入被用上且零干扰（D4 排除 +「仅供参考」措辞生效） |
| 污染防护（D4） | `[rag]` stderr 行聚合 | 103 行（injected 99 / 透传 4）；**42/70 query（60%）首尝试即有排除命中**；est_tokens 均值 **296**（预算 800 的 37%，1200 chars 上限从未触顶） | ✅ 防线实际拦截规模充分——干净归因前提成立 |
| prompt v3 列（stage 3） | FULL 70q vs 同底座 v1 基线族，D4 保守标准 | 54m53s：净通过 **27**（**-5 / -8 落带外向下**）；L0 13/23 掉 4-6 + degraded 24 双向一致恶化；机制单 query 冒烟三段链完整复现 | ❌ 负结果 → **revert 回 v1**（`7959030`），TD-60 关闭 |
| RAG off 默认不破基线 | off 态行为 = v0.22/v0.23 run 同口径 | 回归 1717/1741（stage 2 零接线）→ 1725/1749（stage 3 接线后，off 默认）0 failed；run meta `git_describe` 逐列核对 | ✅ 默认 off 基线隔离兑现（D6 同款先行论证路径） |

**Findings 汇总**：P0 0 / P1 0 / P2 1（TD-55~57 仍 open，第 5 个 cycle）+ TD-60 **本 cycle 关闭**（v3 负结果）+ TD-58/59/61 维持观察 + RAG 语料密度观察项（非 TD）。TD-58（test_alert flaky）整个 v0.24 cycle 各 stage 开工回归 0 failed 未再现。

---

## 1. stage 1 Spike：ADR-0034 设计定稿 + 语料盘点 + v3 措辞稿（commits `70ba776` + `820d874`）

### 落地范围

ADR-0034（D1 检索 keyword BM25-lite 零 deps + D2 注入 per-query 预注入 user 前缀 800 token 硬上限 + D3 语料 session 主语料 + eval trace bootstrap + D4 污染防护相似 query 排除 + D5 评估机制验证主指标 + 方差带分级拍板）+ 附录 A prompt v3 措辞稿（修订 2 单独 diff 不进代码）+ 附录 B session 语料实测盘点（57 文件仅 2 成功段 → bootstrap 升级必需）+ stage 1 doc；零业务代码（回归 1689/1713 与 v0.23.0 一致）。

### 4 维度审查

**代码质量** ✅：零业务代码但设计质量三处亮眼——(1) **D5 预登记三分支拍板标准**（涨超带换默认 / 带内维持 off 归档 / 带外向下 revert）使 stage 3 拍板不受结果偏好影响（实际执行：RAG 带内→off、v3 带外向下→revert，两分支都是标准的机械执行）；(2) D2 换算口径定死（800 token ≈ 1200 chars，中文 ~1.5 chars/token 粗估）避免实装自由发挥；(3) 附录 A 措辞稿与 system.md v1 态 line 20 逐字核对——stage 3 变量隔离的前置。

**架构** ✅：**语料盘点把 D3 从「倾向」变「数据结论」**——brainstorm 原口径「session 主语料 + bootstrap 备选」，实测 96% 空会话后 bootstrap 升级为「机制验证必需」并明示污染防护约束（风险 5 预案兑现：数字如实归档，不硬撑原设计）。五终判给 stage 2/3 实施级输入（模块结构 / 参数定值 / 接线点预览——v0.23 D5 同款「设计稿带实装清单预览」模式的第二次兑现）。

**性能** N/A：零代码。

**完整性** ✅：附录 B 语料盘点方法论完整留档（成功段状态机口径 + 去重规则 + 3 run 基线族构成）——「设计阶段先实测数据现状再定方案」的范式样本；v3 措辞稿独立成稿（修订 1 / 修订 2 分离，v0.23 捆绑实验无法归因的教训直接吸收）。

---

## 2. stage 2 Slice A：RAG 检索层三模块（commits `e465a1f` + `556c969`）

### 落地范围

`src/agent/rag/` 新模块三文件——corpus.rs（`Entry`/`EntrySource` 索引单元 + session JSONL 成功段状态机 + bootstrap `EvalRunFile` 直读 + 归一化去重）、retrieve.rs（CJK 2-gram / ASCII 分词 + idf 加权评分 + 污染排除）、mod.rs（`RagIndex` 全量重建 + `RagParams` + `inject_experience` 模板渲染）——482 行业务 + 640 行测试 28 个全绿；**库形态零接线**（runner / config / builder / agent.toml diff 为空，有验证锚）。

### 4 维度审查

**代码质量** ✅：库形态零接线的接口纪律——`RagIndex::build` 路径参数由调用方传（stage 3 才接 builder/config），本阶段不改任何既有文件（`src/agent/mod.rs` 一行 `pub mod rag` 除外）；静默降级契约与 `SessionRecorder::disabled()` 同款（目录缺失 / 坏源一次 stderr 警告继续，绝不 panic）；`EvalRunFile` 反序列化直读零新解析代码（serde Deserialize 已派生——复用而非重写）。

**架构** ✅：检索层先于注入层（可独立单测，注入依赖检索产物——brainstorm 阶段排序理由兑现）；**tempdir fixture 全构造**（不依赖真实 `~/.config/proc/sessions/` 与本地 eval JSON——CI 确定性）；分词 / 评分 / 排除全纯函数（`is_polluted` / `score_entry` 可独立边界测试）。

**性能** ✅（设计口径）：百级文件全量重建毫秒级（D3 全量重建不做增量的依据）——设计论证充分，未单独 bench（如实标注：非热路径，每次 agent 构造一次）。

**完整性** ✅：28 测试四组覆盖 D1/D3/D4 规格——污染排除**三类样例锚定**（同款 exact / 高覆盖改写 / 同场景异意图不排除且可命中——D4 0.6 阈值标定的最终判定锚）；规格歧义在 ADR 落地注记消解（query 侧去重集合计分 / CJK 边界定值 U+4E00~9FFF / session query 200 chars 截断同底声明）——「规格 → 实装 → 注记回写」闭环。

---

## 3. stage 3 Slice B：RAG 注入接线 + eval 矩阵两列 + 机制验证拍板（commits `754a79d` + `b959277` + `e166e27` + `7959030` + `463a64a`）

### 落地范围

接线三处（config `RagConfig` 五字段默认 off + runner `rag_wrap` 注入入口 stderr 报告行 + builder `build_rag` 构造链接线 + `AgentSession::spawn` 4→5 参）+ 测试 E/F/G/H 组（+8 过 + 1 `#[ignore]` 探针）→ **挂机顺序硬约束执行**（RAG-on 列跑 prompt v1 binary → v3 commit → v3 列）→ 四列 `--compare` 矩阵 + 机制验证两主指标 + 排除命中聚合 → D5/D4 拍板（RAG 维持 off / v3 revert）→ 归档 `docs/eval/rag-v3-70q-v0.24.md`。回归 1725/1749（+8，off 默认不破 1717/1741 基线）。

### 4 维度审查

**代码质量** ✅：off 默认零开销锚（`build_rag` 返 None 不建索引——off 态连索引构建成本都为零，有测试锚）；`rag_wrap` 200 chars 同底截断（与 session 源侧 query 截断版排除判定两侧一致——D3 口径的实装细节）；revert 干净（`7959030` 后 system.md 终态与 v1 逐字一致，净 diff 归零——v0.23 同款回滚特性）。

**架构** ✅：**挂机顺序硬约束兑现**是本 stage 最关键的结构决策——system.md 经 include_str 嵌入 binary 无配置开关，RAG-on 列必须先于 v3 commit 跑；两列 `git_describe` 归档核对（`v0.23.0-7-gb959277` = prompt v1 / `v0.23.0-8-ge166e27` = v3），**各列单变量成立**（RAG 列 v1 prompt、v3 列 RAG off——交互组合不跑）；eval harness 零改动出新列（ADR-0032 延续，`--compare` 四列即插即出）。

**性能** ✅：RAG-on 列 25m20s **四列最快**（vs 基线 47m19s）——注入未拖慢单 query 的 run 级佐证（est_tokens 均值 296 预算无压力的宏观印证；时长波动本身在 E2B 正常域，归档已如实标注机器负载差异）。

**完整性** ✅：机制验证两主指标完整归档——离线召回探针（`#[ignore]` 15 抽样 top-3 打印 + 人工标注 verdict 表）+ 经验引用三分类（逐案例：idx31 前缀采纳 / idx67 整链采纳 / 引用未过 3 例均 L2 的能力边界微观证据）；排除命中 103 行聚合（分布 + 透传 query 明示）；Docker 环境一致性四列核对（daemon 均未运行）——v0.23 同款单变量隔离纪律延续。

---

## 实测观察归档

### 观察 1：机制验证结论——「机制成立但 E2B 兑现不了通过率」的两分归档

**现象**：RAG-on 列机制侧全绿（召回 12/15 = 80%，3 miss 均语料覆盖制约非算法缺陷；经验引用 8/15 链迁移可观察、**干扰 0/15**；degraded 19→9 = **-12 超 ±6 带**），通过率侧 +2 vs 方差列落 ±3 带内（L0/L1 四列最优但增量带内、L2 三列平 0/20）。

**两分结论的价值**：「机制不成立」（检索不准或模型无视注入——RAG 路线关闭）与「机制成立但兑现不了」（机制可用、能力边界制约收益）是**两种不同结论**——本 cycle 实测落在后者：引用但未过的 3 例（idx18/37/46 均 L2）是直接微观证据（链已向经验迁移、答案质量/多步规划仍受 E2B 制约）；degraded -12 与微观链迁移互为印证（注入条目的结构化结论样例改善 run 级答案质量）。**该结论给 v0.25+ 模型底座重启决策留直接输入**（brainstorm 风险 1 mitigate 口径的兑现）：更强底座上 RAG-on 是首选复测列——质量增益若保持 + 通过率上限打开，改默认门槛（≥+7 且 L2 方向性改善）即有解。

**归档教训**：机制验证主指标（召回对照 + 引用观察）与通过率增益**分离归档**是「小模型上做 RAG」的正确口径——只看通过率会误判「RAG 无用」；只看机制不看数字会误判「马上改默认」。两轴各配标尺（召回人工标注 / 通过率方差带），结论才可复用。

### 观察 2：通过率增益方差带解读——「四列最优」不等于「增益成立」

**现象**：RAG-on 净通过 37（+5 vs v1 基线 / +2 vs 方差列）——**+2 落 ±3 带内**，不可单独归因；L0 20/23（87%）/ L1 17/27（63%）双双四列最优但增量均带内；L2 三列平（0-1/20，E2B 多步规划画像不动）。

**方差带标尺（v0.23 遗产）的首次大规模应用**：四列矩阵中基线 32 / 方差列 35 本身就展示 3 通过数的 run 间漂移——RAG-on 的 +2 若无标尺会被误读为「RAG 提升通过率」；带标尺后正确结论是「通过率不动，质量信号（degraded -12）超带另归档」。**机制指标与通过率指标分离归档的必要性实证**：单轴解读必错一个方向。

**顺带验证**：`--compare` 四列即插即出（基线族 JSON 跨三个 cycle 复用，零 harness 改动）——v0.22 建 eval 资产、v0.23 建基线族、v0.24 基线族直接当对照列用的三层兑现。

### 观察 3：污染防护实效——干净归因的前提设施

**现象**：103 行 `[rag]` stderr 聚合（injected 99 / 透传 4），排除分布 `=0` ×53 / `=1` ×42 / `=2` ×4 / `=3` ×4——**42/70 query（60%）首尝试即有污染排除命中**；est_tokens 均值 296（预算 800 的 37%）。

**为什么这是前提而非细节**：bootstrap 语料含 3 基线 run 的 passed trace（与 eval 同款 queries.toml）——没有 D4 排除机制（exact + 覆盖率 ≥0.6 双向 min），「RAG-on 通过率 +2」可能就是检索答案的信息泄漏而非任何能力信号；**排除命中规模充分（60% query 首尝试拦截）证明防线实际在工作**，通过率带内结论才可信。透传 2 query（英文 tool 名 query 词元零匹配 / Word 语料稀疏）如实归档为检索盲区——语料覆盖制约的又一佐证。

**归档教训**：评估口径必须包含「防护实效报告」（排除命中次数进 D4 必归档清单）——防护机制存在 ≠ 防护生效，规模数据才是归因干净的证据。

### 观察 4：v3 负结果与 E2B 零代码杠杆穷尽——路径收敛

**现象**：v3 列（修订 2 写操作发现链，单变量）净通过 27（**-5/-8 落带外向下**）+ L0 13/23 掉 4-6 + degraded 24 双向一致恶化 → revert `7959030`；机制单 query 冒烟三段链完整复现（proc_help 找 tool → proc_kill 带参调用 → blocked 后解释+给命令行）但 70q 规模发现链措辞让简单 query 绕路（L0 受伤最重），且 v3 列无 RAG 经验缓冲。

**杠杆穷尽的全景**：v0.23 关闭 GBNF（结构性互斥）+ prompt v2（捆绑）；v0.24 关闭 prompt v3（单变量）+ RAG 通过率（带内）——**E2B 底座上零代码/低成本杠杆全部穷尽**，进一步改善路径收敛为「模型升级 × RAG-on 复测」组合：决策 1 归档候选表（E4B 同家族 ~3GB 低风险 / Qwen 7-8B 需 QUICK 三判定 / 14B 已排除 / 换默认门槛 净通过差 ≥+7 且 L2 方向性改善）+ 本 cycle degraded -12 质量增益**互为输入**——模型列与 RAG-on 列同矩阵跑，一次挂机回答两个问题。

**归档教训**：负结果的措辞杠杆实验（v2 捆绑 → v3 单变量两轮）展示了「候选拆分到最小变量再关闭」的纪律——v3 负结果后 TD-60 终态不是「措辞永远没用」而是「在 E2B 上用数据关闭」（更强模型上措辞敏感度可能不同，但已非独立候选）。

---

## Findings 表

| 级别 | # | 内容 | 处置 |
|---|---|---|---|
| P0 | — | 无 | — |
| P1 | — | 无 | — |
| P2 | TD-55~57 | Sonnet 70 query 对照 / model ID 真实 API 验证 / nudge 路径实测（v0.20 归档，无 `ANTHROPIC_API_KEY`） | 仍 open，第 5 个 cycle（v0.24 决策 4 不跑）；有 key 后 eval runner 1 条命令三连闭环；v0.25+ 模型底座若启动，Sonnet 云端档并入同矩阵自然到期 |
| — | TD-60 | prompt v3 候选——**本 cycle 实验关闭**：v3 列（修订 2 单变量）-5/-8 落带外向下，revert `7959030` 回 v1 | **终态回填 `docs/tech-debt.md`**（已关闭）；措辞杠杆随「模型升级 × RAG-on 复测」组合才可能重开（更强底座措辞敏感度不同） |
| — | TD-61 | GBNF grammar × tools 复测（llama-server b8685 互斥，v0.23 归档观察项） | 维持观察——v0.24 未升级 llama-server，节点未触发 |
| — | TD-58 | `test_alert::test_metric_extract_process_cpu` 并发 flaky（v0.21 归档观察项） | v0.24 cycle 各 stage 开工回归 0 failed 均未再现，继续观察，原判不动 |
| — | TD-59 | session log 累积无轮转（v0.22 归档观察项） | 维持观察，未触发；**注**：RAG 索引依赖 session 语料，未来若治理轮转需与 RAG 语料口径联动 |
| — | RAG 语料密度 | session 主语料 96% 空会话（57 文件仅 2 成功段），机制验证靠 bootstrap 40 条支撑；docker 域 2 条 / 英文 query 零匹配为已知盲区 | **观察项非 TD**（是语料现状非债务）：语料随日常使用自动增长；v0.25+ RAG-on 复测时重评主语料占比与域覆盖 |

---

## cycle 数据汇总

| 维度 | 数字 |
|---|---|
| stage 数 | 4（1 Spike + 2 Slice + Review+收尾段；无回溯修复子阶段、无 Checkpoint 接力——容量风险全部未触发，铁律 19 上限余量充足） |
| commits | `91e6ba9`（plan brainstorm）/ `70ba776`+`820d874`（stage 1）/ `e465a1f`+`556c969`（stage 2）/ `754a79d`+`b959277`+`e166e27`+`7959030`+`463a64a`（stage 3：plan + 接线 + v3 落地 + revert + 归档）/ `18bea3f`（plan stage 4）+ stage 4（本段）+ tag `v0.24.0` |
| 全量回归 | 1689 → **1725**（默认）/ 1713 → **1749**（anthropic），0 failed 全程；ignored 9→10 / 10→11（+1 `#[ignore]` 本地召回探针） |
| 新增测试 | +36 CI（stage 2 +28：检索层四组 + 模块内联；stage 3 +8：接线 E/F/G/H 组）+ `#[ignore]` 探针 ×1（挂机前离线跑） |
| MCP tool / agent catalog | 46 / 47 + proc_finish **均不变**（RAG 预注入路线不加 tool——D2 终判 meta tool 双否） |
| ADR | 新 1 份（ADR-0034，D1~D5 + 附录 A v3 措辞稿 + 附录 B 语料盘点） |
| Cargo deps | +0（keyword BM25-lite 路线——v0.22/v0.23/v0.24 三连零） |
| 新文件 | `docs/adr/0034-rag-experience-recall.md` + `docs/eval/rag-v3-70q-v0.24.md` + `src/agent/rag/{mod,corpus,retrieve}.rs` + `tests/test_agent_rag.rs` + `docs/stages/v0.24-{brainstorm,stage-1..4}.md` |
| 行数（insertions 口径） | src +921/-7（8 文件：rag/ 三模块 787 + 接线四文件——vs 预估业务 ~600，1.5×，注释与模板字符串占比合理）；tests +723/-0（5 文件——vs 预估 ~500，1.4×，fixture 构造重是 CI 确定性的代价）；docs +1381/-0（8 文件——vs 预估 ~900，1.5×，brainstorm 381 + stage docs 664 + ADR 224 + eval 归档 89 + CHANGELOG 等 23）；合计 **+3025/-7**（vs 预估 ~2000 行级，1.5×——同量级漂移） |
| 实验挂机 | FULL 2 列 **80m13s**（RAG-on 25m20s + v3 54m53s，vs 预估 ~1.5-2.5h 兑现下沿）+ 离线召回探针分钟级；off 态零新增挂机（复用基线族两列） |

**与 v0.23 cycle 对比**：4 stage 同款节奏；~3025 vs ~2000 行级（机制验证型 vs 决策型轻 cycle——RAG 主体使工程量升档）；deps +0 三连零；tool 层零变更延续。**「能力维度」**：v0.23 交付决策 + 标尺（关闭两条路径 + 方差带量化），v0.24 交付**机制 + 组合候选**（RAG 机制两主指标验证成立、默认 off 归档质量增益；TD-60 措辞杠杆关闭）——两 cycle 合并视图：**E2B 底座零代码杠杆穷尽，「模型升级 × RAG-on 复测」是唯一剩余主线**。数据资产续扩：`eval-rag-on-70q.json` / `eval-v3-70q.json` 本地留存（不入 commit），四列矩阵 `--compare` 即插即出。

---

## v0.25+ 候选方向

brainstorm 备注（cycle 末段评估）+ 本 Review 归档综合（决策 1 归档候选表 + RAG 机制结论互为输入）：

| 优先级 | 方向 | 依据 | 规模预估 |
|---|---|---|---|
| 1 | **模型升级 × RAG-on 复测（组合）** | 决策 1 归档候选表（E4B 同家族 ~3GB 低风险 / Qwen 7-8B 偏紧可行需 QUICK 三判定（模板渲染 / tool_calls 解析 / 内存实测）/ 14B 按 6GB VRAM 排除 / 换默认门槛 净通过差 ≥+7 且 L2 方向性改善）+ 本 cycle「机制成立但 E2B 兑现不了」+ degraded -12 质量增益——更强底座上 RAG-on 是首选复测列：模型列与 RAG-on 列同矩阵，一次挂机回答「底座是否换默认」与「RAG 是否改默认」两个问题；RAG 语料密度观察项同场重评 | 下载 3-4.5GB + 挂机 2-3 列 |
| 2 | TD-55~57 补验 | 有 `ANTHROPIC_API_KEY` 即 1 条命令三连闭环；Sonnet 云端档并入模型光谱（E2B / E4B / Sonnet 三档一次出齐，<10min） | ~0 行 |
| 3 | GBNF 复测（TD-61） | 挂 llama-server 升级节点维持观察；复测路径 = ADR-0033 附录 B 判定表重跑（smoke1 L0 3×2 即可判定） | ~0 行（配置） |
| 4 | RAG 深化（语料密度 / 检索盲区） | session 主语料 96% 空 + docker 域 2 条 + 英文 query 零匹配——语料随日常使用积累后重评主语料占比与域覆盖；机制已验证（本 cycle 两主指标），深化是语料工程非代码工程 | 视数据，~0 行起 |
| 5 | Multi-agent 协作（方向 D） | 维持单 agent 能力上限先拉满（决策 8 口径）——上限不动则协作无增益，依赖候选 1 结果 | — |

**v0.25 模型底座决策的解读纪律**（v0.23 标尺 + 本 cycle 应用验证）：任何 `--compare` 单列差异 < ±3 通过数 / ±6 失败模式计数时**不可单独归因**（本 cycle RAG-on +2 带内的解读实证）；模型列换默认需**净通过差 ≥+7 且 L2 方向性改善**（决策 1 归档门槛——预期信号量级应显著超带才值得换）；RAG-on 复测列同场观察 degraded 质量信号是否跨底座保持（-12 是本 cycle 唯一超带增益，跨底座保持则「质量型默认 on」成为独立讨论项）。
