# RAG-on 列 + prompt v3 列 70 query 对比归档（v0.24 stage 3）

> ADR-0034 D2/D5 注入层实装的两列实验归档（2026-08-25/26）：**RAG-on 列**（E2B × RAG on × prompt v1，binary `b959277`）+ **v3 列**（E2B × RAG off × prompt v3，binary `e166e27`）vs 基线族（v0.22 `eval-e2b-70q.json` + v0.23 `eval-best-70q.json` 方差列）。结果 JSON 本地留存不入 commit，本 md 是精炼归档。
>
> **拍板结论（D5 三分支 + D4 保守标准）**：
> - **RAG**：通过率落方差带内（vs 方差列 +2）→ **`enabled` 默认维持 off**；机制验证两主指标成立（召回 12/15 · 链迁移 8/15 · output_degraded **-12** 超带）→「**机制成立但 E2B 兑现不了通过率**」归档，答案质量增益给 v0.25+ 模型底座重启决策留直接输入
> - **v3**：净通过 **-5 / -8 落带外向下** + L0 掉 4-6 → **revert 回 v1**（commit `7959030`），TD-60 负结果归档（终态回填 stage 4）

## run 元信息

| 项 | 基线（v1） | best（v1 方差列） | RAG-on | v3 |
|---|---|---|---|---|
| run（UTC） | 2026-08-22T02:46:12Z | 2026-08-23T13:19:04Z | 2026-08-25T14:01:25Z | 2026-08-26T00:50:39Z |
| git | `v0.21.0-5-gb210dc3` | `v0.22.0-4-g5d2ac64` | `v0.23.0-7-gb959277` | `v0.23.0-8-ge166e27` |
| 时长 | 47m 19s | 38m 43s | **25m 20s** | 54m 53s |
| 输出 | `eval-e2b-70q.json` | `eval-best-70q.json` | `eval-rag-on-70q.json` | `eval-v3-70q.json` |

四列 provider/参数一致：llama-cpp（llama-server b8685 CUDA 12.4 + gemma-4-E2B-it-Q4_K_M），attempts=2，max_steps=10，70 query（L0 23 / L1 27 / L2 20），GBNF off。**挂机顺序硬约束兑现**：RAG-on 列 `git_describe v0.23.0-7-gb959277` = 接线 commit（system.md 仍 v1），v3 列 `v0.23.0-8-ge166e27` = v3 commit（`[rag]` 还原 off，log 零 `[rag]` 行核对）——两列各自单变量。

**RAG-on 注入配置**（`agent.toml [rag]`）：`enabled=true` + `eval_corpora` 三基线 JSON（session 主语料 + bootstrap 40 条去重池）；budget 800 token（1200 chars）/ top_k 3 / threshold 0.6 代码默认。

**环境一致性**：Docker daemon 四列均未运行（docker 场景 1/10 · 2/10 · 3/10 · 2/10，final_text 均为「Docker 未运行」应对）；时长 25m-55m 波动为机器负载差异（E2B 正常域，注入未拖慢单 query——rag-on 列反而是四列最快）。

## 对比矩阵（四列）

| 指标 | 基线 v1 | best 方差列 | **RAG-on** | **v3** | 判读 |
|---|---|---|---|---|---|
| L0 | 17/23（74%） | 19/23 | **20/23（87%）** | 13/23（57%） | RAG 三列最优（+1 vs 方差列，带内）；v3 掉 4-6 |
| L1 | 14/27（52%） | 16/27 | **17/27（63%）** | 14/27 | RAG +1 vs 方差列（带内）；v3 = 基线平 |
| L2 full-chain | 1/20 | 0/20 | 0/20 | 0/20 | 三列平——E2B 多步规划画像不动 |
| L2 chain-step | 12/43 | 13/43 | 14/43 | 12/43 | 带内漂移 |
| output_degraded | 21 | 19 | **9** | 24 | **RAG -10 vs 方差列 / -12 vs 基线——超 ±6 带的最大信号**；v3 +3/+5 恶化 |
| wrong_tool | 10 | 11* | 11 | 10 | 平 |
| chain_incomplete | 7 | 11* | 12 | 8 | RAG +1 vs 方差列（带内）；degraded 改善的镜像（失败转移） |
| 净通过 | 32 | 35 | **37** | **27** | RAG +5 / +2；v3 -5 / -8 |

*best 方差列直方图取自 v0.23 归档（`eval-best-70q.json` 报告段）。

失败模式迁移（基线→RAG-on）：output_degraded 21→9（**-12**）；chain_incomplete 7→12（+5，带内）——退化减少的构成是「失败质量」改善（答案更完整可读）混合「失败转移」，与 v0.23 v2 列的同款拆解口径一致。

## 机制验证主指标 ①：检索召回对照（15 抽样，离线挂机前跑）

抽样 `[0,8,12,15,18,21,27,31,37,42,46,52,57,62,67]`（覆盖 9 场景 × L0 6/L1 4/L2 5），`#[ignore]` 探针（`tests/test_agent_rag.rs::local_recall_probe_prints_top3_for_sampled_queries`）输出 top-3 后人工标注相关条目集合：

| verdict | query | 说明 |
|---|---|---|
| ✓ 命中 | idx0/8/12/15/18/31/37/42/46/52/62/67（**12/15 = 80%**） | top-3 含 ≥1 标注相关条目；平均命中相关条数 ~1.6 |
| ✗ miss | idx21（nginx 容器健康）| 检索到 chrome/DNS 条目——docker 语料仅 2 条且词元重叠低 |
| ✗ miss | idx27（postgres 容器查询）| 同上，docker 域检索盲区 |
| ✗ miss | idx57（TCP 重传率）| 「为什么」模板泛匹配到无关诊断条目 |

docker 域 2 条 miss 与语料分布（docker 场景 bootstrap 池仅 2 条）直接相关——**检索准度受语料覆盖制约，不是算法缺陷**（同域条目充足时 idx31/idx46/idx67 均首位命中）。

## 机制验证主指标 ②：经验引用观察（15 抽样三分类：引用 / 无视 / 干扰）

| 分类 | 数量 | 案例 |
|---|---|---|
| **引用（链迁移/结构采纳）** | **8/15** | idx31 教科书级：注入「E 盘为什么不能弹出 → proc_help → proc_eject_status → …」，实际链 `proc_help → proc_eject_status`（前缀采纳）；idx67 整链采纳：注入「最近 10 分钟域名 → proc_help → proc_dns」，实际 `proc_help → proc_dns`；idx18：注入「父子链可疑 → proc_ls → proc_inspect」，实际 `proc_ls → proc_inspect ×2`；idx0/12/15/62 部分采纳 |
| 无视 | 6/15 | 检索 miss 的 idx21/27/57 + idx8/52/46（无链交集，按 query 自身语义走） |
| **干扰（注入误导）** | **0/15** | 无一案例因注入偏航——D4 排除机制 + 「仅供参考，不要照抄无关步骤」措辞生效 |

引用但未过（idx18/37/46，均 L2）：链已向经验迁移但答案质量/多步规划仍受 E2B 能力边界制约——「机制成立但兑现不了」的直接微观证据。idx46 无视注入且 final 泄漏 `proc_finish{answer:...}` JSON（degraded 个案）。

**run 级间接佐证**：output_degraded 19→9（-10，超 ±6 带）——注入条目的结论样例（结构化中文答案 head）在 run 级显著改善答案质量，与微观链迁移互为印证。

## 排除命中次数汇总（D4 防护实效，`[rag]` stderr 行聚合）

- **103 行**（70 query × attempts 重跑），`injected=true` 99 行 / 透传 4 行（2 query 各两 attempt）
- 排除分布（行级）：`excluded=0` ×53 / `=1` ×42 / `=2` ×4 / `=3` ×4——**42/70 query（60%）首尝试即有污染排除命中**，防线实际拦截规模充分
- 透传 query（min_score 零命中，非排除全灭）：idx38「eject_status → kill → eject_status 反馈循环演示」（英文 tool 名 query，中文语料词元零匹配）、idx48「Word 启动了 cmd，是不是 macro 攻击？」（Word/macro 词元语料稀疏）
- `est_tokens` 均值 **296**（预算 800 的 37%）——预算无压力，1200 chars 上限从未触顶

## 通过率增益方差带解读 + 拍板（预登记标准执行）

**RAG**（D5 三分支）：

1. 净通过差 **+2 vs 方差列**（+5 vs v1 基线）——落 ±3 带内 → **「维持 off + 数据归档」分支**，`enabled` 默认值不动（agent.toml 注释态推荐，不强制改默认）
2. L0 20 / L1 17 双双四列最优但增量带内；L2 三列平（0-1/20）——E2B 多步规划画像不动
3. **两分结论归档：「机制成立但 E2B 兑现不了（通过率）」**——主指标 ①② 成立（召回 80% / 引用 57% / 干扰 0）+ degraded -12 超带 + 通过率带内。v0.25+ 模型底座重启决策（brainstorm 决策 1 归档候选表）的直接输入：更强底座上 RAG-on 是首选复测列（质量增益若保持、通过率上限打开，改默认门槛即有解）

**v3**（D4 保守标准）：

1. 净通过 **-5 vs 基线 / -8 vs 方差列**——落带外向下，非噪声主导（L0 13 掉 4-6 + degraded 24 高于基线带 19-21 双向一致恶化）
2. 机制层面单 query 冒烟发现链三段完整复现（`proc_help` 找 tool → `proc_kill` 带参调用 → blocked 后解释+给命令行，v0.23 修订 2 证据延续），但 70q 规模负向：发现链措辞让简单 query 绕路（L0 受伤最重），且 v3 列无 RAG 经验缓冲
3. **revert 回 v1**（commit `7959030`），system.md 终态 = v1；TD-60 终态回填「修订 2 单变量亦负，prompt 措辞杠杆在 E2B 上用数据关闭」留 stage 4

## 与 v0.23 结论的合并视图

v0.23 关闭了 GBNF 与 prompt v2（捆绑）两条路径；v0.24 关闭 prompt v3（修订 2 单变量）并证明 RAG 机制成立但通过率带内——**E2B 底座上零代码/低成本杠杆全部穷尽，进一步改善的路径收敛为「模型升级 × RAG-on 复测」组合**（v0.25+ 候选池首位，决策 1 归档候选表 + 本归档 degraded -12 数据互为输入）。
