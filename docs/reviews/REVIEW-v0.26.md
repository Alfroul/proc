# REVIEW-v0.26 — v0.26 cycle Review（秋招展示冲刺 cycle 完结）

> **cycle 范围**：brainstorm 5 决策（2026-08-31 拍板「按建议全部做」）——决策 1 附录 A 不启动转展示冲刺（主线第 4 次搁置留档）/ 决策 2 六组打包定稿 / 决策 3 TD-55~57 无 key 不跑（第 7 个 cycle）/ 决策 4 搭车全并（R1 并入 stage 2）/ 决策 5 4 stage 节奏（Review 与收尾同会话，深挖导览作前半）；1 份 ADR-0036
>
> **Review 范围**：4 stage 全部产出（Spike 取数 / Slice A 门禁+测试可靠性 / Slice B 展示层 / Slice C 深挖导览 + 本 Review/收尾段）
>
> **基线**：1742 / 0 / 10 + 1766 / 0 / 11（v0.25.0，79 行）→ **1744 / 0 / 10 + 1768 / 0 / 11**（stage 2 +2 proptest 双档同量，**79 → 80 行**——test_filter_proptest 新 binary；stage 3/4 零测试变更——stage 4 的 4 处 SAFETY 注释与 2 处 clippy 机械改写均为零行为变化，终值双档复跑核对）/ fmt / clippy（双档）/ build（双档 no-default ± anthropic）/ bench --no-run 全过
>
> **Review 日期**：2026-09-02（stage 4 会话 2026-09-01 起跨日，tag 同日）
>
> **Reviewer**：Claude（stage 4 会话；审查依据 ADR-0036 D4/D5 验收锚 + 各 stage doc 验证记录 + CHANGELOG，按数据/测试锚审不自评——brainstorm Q5 口径）
>
> **零挂机声明**：本 cycle 无 eval 列——质量论证四轴（brainstorm Q1）：① 不变锚（tool 46 / catalog 47 / runtime deps +0——dev-deps +1 proptest 为 ADR-0036 D3 预登记批准）② 回归数字 + gate 脚本新增机器可验证锚（且本 Review 实证了 gate 拦截）③ 数字溯源纪律（展示内容可回溯）④ 本 Review 抽查核对（19 项 > 10 门槛）。

---

## 概览

v0.26 是秋招窗口期（2026-08~11）的**展示冲刺轻 cycle**（净 ~2200 行级，doc 为主）——**「工程强度已足（65K src / 27K tests / 1742 双档 / 36 ADR / 25 releases），短板在评估者 10 分钟内看不到；把强度从 docs 深处搬到评估者可见处，把纸面门禁变成机器强制」**。六组打包全交付：① README 评估者视图（mermaid 架构图 + 亮点条 + 竞品对比表 + badge/GIF 占位）② AI Agent 与 eval 纪律章（六列矩阵 + 方差带标尺 + 负结果三连）③ 质量门禁自动化（gate.sh 两档 + pre-push opt-in + R1 根治）④ 性能叙事（performance.md + Benchmark 段改链）⑤ 深挖导览（10 条三层）⑥ 测试深度（proptest + unsafe-audit）。

- **MCP tool 46 / agent catalog 47 / runtime deps +0 全程保持**（dev-deps +1 proptest 预登记批准）
- 1 份新 ADR-0036（D1~D5 + 三个 stage 实装注记）
- TD：R1 修复即关闭（未立 TD）+ **TD-62 新立**（负载敏感 flaky 观察项，cycle 唯一新增）

**核心实测数字**（本 Review 独立复核，非自评转写）：

| 验收项 | 口径 | 实测 | 结论 |
|---|---|---|---|
| 数字溯源抽查（D5 锚 ≥ 10） | stage-3 核对表 20 项抽 19 项对回原文 | 18 项零自创；1 项发现**源文档算术错误**（引用率 57% 实为 8/15 = 53%，见 Findings ①） | ✅ 无 P0（自创）；溯源纪律兑现且抽查抓到真问题 |
| gate 有效性实证 | 注入 fmt 违规 → `bash scripts/gate.sh` | `[1/4] cargo fmt --check` 检出 Diff → **exit 1** → 还原后复绿（probe log 留档） | ✅ 门禁机器强制实证（R2 结构性修复闭环） |
| 展示层链接完整性 | 4 份展示 doc 相对链接存在性脚本核验 | 修复 3 处：深挖/audit `../` 前缀错误（本 Review 自查）+ README ADR-0027 既有死链（stale）；余全绿（demo.gif 占位注释预期） | ✅ 且证明「对深挖导览如同审其他 stage 产出」不是空话 |
| 深挖导览（D4 锚） | 10 条封闭 / 单条 30-45 行 / 三层深挖 / 中性命名 | 10 条全保无砍单；**条目①证据源修正**——D4 表引 ADR-0003 是幽灵引用（git 全历史查证该文件从未入库），改用代码层五处 (pid, start_time) 键控证据 + 诚实边界（process_cache 单键权衡 / TD-21 / ADR-0005） | ✅ 比照抄 D4 表更强的溯源执行 |
| unsafe 审计 | 附录 C 数据成文 + ≤5 处注释小修 | `docs/unsafe-audit.md`（198 分布 + 59→63 SAFETY 覆盖判读 + edition 2024 已合规 + miri 如实声明不可用）+ 4 处注释（handles ×2 / estats / collect，回归双档数字不变） | ✅ 注释写 invariant 不写废话 |
| 回归终值（零行为变化声明） | stage 4 改动 = 4 注释 + 2 clippy 机械改写 | 双档 **1744 / 0 / 10 + 1768 / 0 / 11**（80 行核对）与 stage 2/3 终值一致 | ✅ 零行为变化由数字背书 |
| 不变锚（全程） | tool 46 / catalog 47 / deps +0 | grep tool 46 本 Review 复核；catalog 47；Cargo.lock 无 runtime 变更 | ✅ 三不变锚保持 |

**Findings 汇总**：P0 0 / P1 0 / P2 4（① RAG 引用率算术错误——v0.24 归档表格「引用 8/15」与汇总行「57%」自相矛盾（8/15 = 53%，8/14 才 = 57%），stage 3 忠实转写了汇总行；README 三处 + 深挖⑨已改「8/15 = 53%」并注勘误，归档原文不改（冻结历史，与 REVIEW-v0.25 勘误段同款处置）② ADR-0003 幽灵引用四处（app.rs:181 / CONTEXT.md PID 词条 / ADR-0021 Related / ADR-0036 D4①）把 PID 复用键控引到从未入库的文件——深挖①已改代码锚，四处引用点修正留 v0.27 ③ TD-62 新立（见概览）④ 亮点条规模数字时点漂移——65,074/26,968 是 stage-1 附录 F 时点值，stage 2 getter/proptest 代码演进后 HEAD 实为 65,099/27,480，README 已更新并注 as-of）。TD-55~57（第 7 个 cycle）/ TD-58 / TD-59 / TD-61 维持观察。

---

## 1. stage 1 Spike：取数底座 + ADR-0036 定稿（commit `bfb0a14`）

**落地范围**：bench 7 项 25 数据点实测（分批纪律）/ 79 binary 耗时摸底（gate 快档选型）/ unsafe 198 分布 + SAFETY 覆盖 / README 章节底图 / 竞品 GitHub API 核实 / GIF 录制规格 + ADR-0036（D1~D5）。零业务代码。

**4 维度审查**：**代码质量** N/A（零代码）；**架构** ✅——D4 十条清单封闭与 D5 溯源纪律把后续 stage 的蔓延与诚信两个最大风险前置关死；**性能** ✅——附录 A 是 stage 3/4 数字唯一来源的纪律设计（转写非记忆）；**完整性** ✅——附录 B 实测数据直接支撑 gate 快档名单（21 binary 稳态 29s < 5 min 目标），Spike 取数全部被下游消费。

## 2. stage 2 Slice A：门禁 + 测试可靠性（commit `7a62821`）

**落地范围**：R1 根治（PID 断言替代全局 tasklist，连跑 3 轮稳定绿）+ `scripts/gate.sh` 两档 + `.githooks/pre-push` opt-in + required checks 设置说明 + proptest roundtrip ×2（256 cases 0.05s）+ proc_smart 口径连改 + TD 回填。

**4 维度审查**：**代码质量** ✅——R1 修复面最小（两个 getter + 断言改写），gate.sh `set -euo pipefail` 任一步失败即中止（首跑即拦截 1 处 fmt 违规的生效实证）；**架构** ✅——防线分层（脚本可本地复现 > hook opt-in 不强推 > required checks 主防线）与「砍名单不砍纪律」的选型纪律；**性能** ✅——快档稳态 29s / 首跑 569s 两档数字如实入档（含「名单大小非主要变量」的归因，不粉饰）；**完整性** ✅——proptest 语义等价用行为等价判定（无 PartialEq 的正确替代），proc_smart 锚定断言连改保持 tool 46。

## 3. stage 3 Slice B：展示层主项（commit `a2d98de`）

**落地范围**：README 评估者视图（+114/-1 既有 825 行后移不删）+ AI Agent 与 eval 纪律章（六列矩阵 + 判读纪律五条）+ `docs/performance.md` + Benchmark 段改链。零 rs 变更。

**4 维度审查**：**代码质量** N/A（纯 doc）；**架构** ✅——双受众原则兑现（评估者视图插入不删旧内容，风险 8）；「—」格注明「REVIEW 未单列该维度」不自创；**性能** ✅——performance.md 归因只写 TD-47 `4c7e294`（其余提升明确不写归因——宁缺毋滥纪律的正面示范）；**完整性** ✅（在 stage 4 抽查下复核）——20 项溯源自查在位，但**自查未覆盖两点**：引用率算术（见到源文档汇总行 57% 即算过，未对表格 8/15 复算）与规模数字时点（stage 2 代码演进后附录 F 值漂移）——两处均由本 Review 抽查抓出并修正（Findings ①④）。**结论**：自查 + 独立抽查双层制的价值实证。

## 4. stage 4 Slice C + Review：深挖导览 + unsafe-audit + 收尾（本 commit）

**落地范围**：`docs/architecture-deep-dive.md`（10 条全保）+ `docs/unsafe-audit.md` + 4 处 SAFETY 注释 + 2 处 clippy 1.98 漂移搭车修（用户拍板）+ 本 Review + CHANGELOG 0.26.0 + Cargo bump + README banner + tag。

**4 维度审查**：**代码质量** ✅——4 处注释全部声明可核对 invariant（buffer 同源 / 字节预算校验 / 全零合法 POD / dwSize 契约），非模板废话；2 处 clippy 改写机械零语义（本地 1.95 双档仍绿 + 注释说明远端标准语境）；**架构** ✅——深挖①的幽灵引用修正把「清单封闭」升级为「清单 + 证据双重校验」：D4 表照抄会传播过时引用，产出时逐条点开验证（引用编号会漂移，见 CONTEXT.md 幽灵引用词条的教训沉淀）；**性能** ✅——零行为变化由回归双档终值背书（数字不变）；**完整性** ✅——开工基线异常（anthropic 档连续 2 次同断言红）按手册「单跑确认后复跑」处置 + 归因（机器后台负载实测：两个 java + 两个 claude 进程）+ TD-62 归档，不静默重跑也不无限重试。

---

## 5. 数字溯源抽查详表（D5 锚：≥ 10，实抽 19）

| # | 数字（出处） | 来源核对 | 判定 |
|---|---|---|---|
| 1 | 亮点条 src/tests 行数 | 附录 F（65,074/26,968）vs 本会话 HEAD 实测（65,099/27,480）——时点漂移，README 已更新 + as-of 注 | 🔶→✅（Findings ④） |
| 2 | 1744 + 1768 双档 | stage 2 完工 + 本会话三次默认档（开工/含 flaky 复跑/终值）+ 三次 anthropic 档实测 | ✅ |
| 3 | 80 test binaries | 本会话每轮 grep -c = 80 | ✅ |
| 4 | MCP tools 46 | `grep 'name = "proc_" \| wc -l` = 46（本 Review 复核） | ✅ |
| 5 | 36 ADR / 61 TD / 25 releases | ls / grep / `git tag \| wc -l`（本 Review 复核） | ✅ |
| 6 | 竞品四行（stars/语言/release/AI-MCP 无） | 附录 D 逐行（34,329 / 13,954 / 15,732 / 33,483 + as-of 2026-08-31 标注在位） | ✅ |
| 7 | btop4win 注脚 | 附录 D（最后 push 2025-10-12） | ✅ |
| 8 | E2B 基线 32 / 17(74%) / 14(52%) / 1(5%) + 12/43 / degraded 21 | REVIEW-v0.22 概览表 + REVIEW-v0.23 表（基线 32）逐项 | ✅ |
| 9 | QUICK 18m46s / 7/9 / 4/9 / 0/8 + 4/17 | REVIEW-v0.22 概览表 | ✅ |
| 10 | promptv2 33 / best 35 / 37m07s / 38m43s / v2 L1 16 / degraded 16 | REVIEW-v0.23 概览表 line 29-30 | ✅ |
| 11 | best 列 L0 19 | rag-v3 归档四列表（19/23）+ REVIEW-v0.23 | ✅ |
| 12 | rag-on 37 / 20(87%) / 17(63%) / degraded 19→9 / 25m20s | REVIEW-v0.24 概览表 line 29 | ✅ |
| 13 | 召回 12/15 = 80% / 干扰 0/15 | REVIEW-v0.24 line 30/98 + rag 归档 | ✅ |
| 14 | **引用率** | 归档表格「引用 8/15」vs 汇总行「57%」——8/15 = 53.3%，8/14 才 = 57%；三分类行合计 14/15 本身有 1 例未归类（归档内部记账模糊） | ❌ **算术错误**（Findings ①，已修） |
| 15 | v3 27 / -5/-8 / L0 13(57%) / degraded 24 / 54m53s | REVIEW-v0.24 line 33 + rag 归档（13/23 = 56.5% → 57% 此处四舍五入正确） | ✅ |
| 16 | GBNF 12 请求 400 + 错误体原文 | REVIEW-v0.23 line 28 + ADR-0033 附录 B | ✅ |
| 17 | 预登记门槛 ≥ +7 且 L2 方向性 | REVIEW-v0.24 line 100（观察 1） | ✅ |
| 18 | bench 25 数据点 + refresh_heavy 2.9×（16.47→5.69 ms）→ TD-47 `4c7e294` | performance.md 对附录 A 逐行抽对（6 表 25 格全对）+ 归因段只写已溯源项 | ✅ |
| 19 | 启动 2291/2527/2875 ms + 56.7 MB | performance.md 口径注记在位（合并口径 + 单机单次样本非基准声明） | ✅ |

**判定**：19 项抽对，18 项零自创，1 项源文档算术错误被忠实转写（P2 非 P0——溯源链完整，错在源头；修正后展示层全部数字可对回原文）。**D5 纪律的元结论**：溯源抽查能抓住「转写忠实但源头错」的盲区——「见到来源」与「数字正确」是两道独立检查。

## 6. v0.27+ 候选方向评估（REVIEW-v0.26 惯例段）

| 优先级 | 方向 | 现状评估（2026-09-01） | 建议 |
|---|---|---|---|
| 1 | **CI 修复专项**（stage 3 发现 + stage 4 拍板项） | clippy 2 处已搭车修（本 commit）——**push 后首验远端 check job 是否转绿**；剩余：rmcp 0.11.0 RUSTSEC-2026-0189（DNS rebinding，修复 = 0.11→1.4 major 升级大工程）+ crossbeam-epoch 0.9.18（`cargo update -p` 可解）+ check-macos cfg-gate unused imports（~10+ 文件清理）+ miri workflow E0433（历史全红）+ winget action 上游删库；badge 激活的前置 = ci.yml 全 job 绿 | v0.27 首选主题候选（展示冲刺的收尾——badge 转绿 + README 启用占位语法零重写） |
| 2 | **附录 A 模型升级 × RAG-on 复测组合**（第 4 次搁置留档） | 重启纪律不变（只重评下载意愿/磁盘/网络）；秋招后（2026-11+）是自然重评窗口；v0.25 语料卫生 + v0.26 展示冲刺都已交付，底座干净 | 秋招后重评（窗口期内继续搁置） |
| 3 | **Multi-agent 协作（方向 D）** | 依赖候选 2 结果（E2B 单 agent L2 5% 边界未变） | 排队候选 2 之后 |
| 4 | **TD-51 / TD-31 / TD-49** | MonitorManager 持久化 / FilterExpr 跨 ctx / replay 增强——单特性候选（~100-300 / ~50 / ~1000 行级） | v0.27+ 主题评估再定 |
| 5 | **本 cycle 新增小项** | TD-62（flaky 观察项，复现再修）/ 幽灵引用四处修正（doc 小修）/ NT API 层 SAFETY 全量注释（~93 处专项） | 搭车项池（任意下 cycle 顺手） |

**TD 观察项存量**：TD-55~57（key，第 7 个 cycle）/ TD-58（未再现）/ TD-59（轮转未触发）/ TD-61（llama-server 未升级）/ **TD-62（首触发归档）**。

## 7. cycle 完结核对

- 4 stage 全 ✅（brainstorm 总览表唯一勾选点）；tag `v0.26.0`；push 9 commits + tag（用户拍板 2026-09-01）
- 打包清单六组全清（① 评估者视图 ② eval 纪律章 ③ 门禁自动化 ④ 性能叙事 ⑤ 深挖导览 ⑥ 测试深度）+ 搭车全并（R1 / proc_smart / R2 / 勘误）
- 容量检查：stage 4 ~700 行（深挖 ~230 / audit ~120 / Review ~180 / 收尾 doc）——低于 1500 行铁律，无 Checkpoint 触发
- demo GIF：**已挂载**（`docs/assets/demo.gif`，1.34 MB / 1920×1140 @16.7fps / 35.1s——ShareX 采集（内置录屏同开，● REC 角标入镜）；内容 = TUI 主界面 → AI Agent 面板一次 tool-use 完整链路（query → tool 步骤行 → streaming 回答）；README 注释占位替换为 `![demo]` + 说明行；答案含一处 E2B 幻觉数字（claude.exe 内存 31.06 GB，16GB 机器不可能）——与项目「E2B 能力边界」诚实叙事一致，缩放后不可读，用户知情拍板直接用）
- 手册执行：每 stage 独立会话 + 开工基线验证（回归双档 + 三件套；stage 4 含 flaky 处置记录）+ 完工报告 + 启动指令包——全流程兑现
