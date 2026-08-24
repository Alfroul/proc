# REVIEW-v0.23 — v0.23 cycle Review（eval 变量实验 + record agent 侧实装 cycle 完结）

> **cycle 范围**：brainstorm 双主题（2026-08-22 拍板会话，9 决策全 ✅）——eval 变量实验（GBNF 开关 + prompt v2 措辞，漏斗式矩阵 + 最优配置拍板，均基于现有 E2B）+ proc_record_start/stop agent 侧实装（47 tool 全可用拼图补全）；穿插 ADR-0033 + 2 份 eval 归档
>
> **Review 范围**：4 stage 全部产出（1 Spike + 2 Slice + 本 Review+收尾段）
>
> **基线**：1681 passed / 0 failed / 9 ignored（v0.22.0 默认档）+ 1705 / 0 / 10（anthropic 档）→ **1689 passed / 0 failed / 9 ignored（默认档）+ 1713 passed / 0 failed / 10 ignored（`--features anthropic`）**/ fmt / clippy（双档）/ build（双档）/ bench --no-run 全过
>
> **Review 日期**：2026-08-24
>
> **Reviewer**：Claude（stage 4 会话；审查依据 CHANGELOG stage 段 + ADR-0033 落地注记 + eval 归档，不逐文件重读实现——见风险 1 口径声明）

---

## 概览

v0.23 cycle 是 proc 首个「决策型」轻 cycle（净 diff ~2000 行级：src +216/-13 / tests +412/-19 / docs +1384/-1，13 文件；prompt v2 落地 + revert 净零不入 diff）——**v0.22 建成测量层，v0.23 用测量层做受控实验拍板配置**。交付物主体不是代码而是**决策 + 数据归档**：GBNF / prompt v2 两条候选修复路径以数据双双关闭（GBNF 结构性不可用——llama-server b8685 拒绝 grammar × tools 同传；prompt v2 无明确增益——净通过增益落 E2B 方差带内，保守 revert 回 v1），**E2B 方差带首次跨 run 量化**（±3 通过数 / ±6 失败模式计数——未来所有 `--compare` 单列差异的解读标尺）；工程侧补全 agent tool 拼图最后两块（record_start/stop agent 侧实装，47 tool 全部真实可用，「不支持」清单清零）。E2B 画像（L0 74% / L1 52% / L2 full-chain 5% / degraded 30%）维持 v0.22 基线——进一步改善的杠杆只剩模型升级，推 v0.24 与 RAG cycle 一并决策（决策 2 既定）。

- **MCP tool 46 / agent catalog 47 + proc_finish 均不变**（record 落地不加——catalog 早在册，仅 dispatch 行为从「不支持」变真实执行，D6 论证 2）
- **Cargo deps +0**（零新依赖）
- 1 份新 ADR-0033（D1~D6 六决策 + 附录 A prompt v2 措辞稿 + 附录 B GBNF 冒烟判定标准与实测结论）
- 回归 +8 CI（stage 2 record 三组测试 + session 内联单元；stage 1/3 零测试改动）

**核心实测数字**（详见各 stage 段 + [`docs/eval/promptv2-70q-v0.23.md`](../eval/promptv2-70q-v0.23.md)）：

| 验收项 | 口径 | 实测 | 结论 |
|---|---|---|---|
| GBNF 冒烟（stage 1） | 判定表 4 项（ADR-0033 附录 B） | smoke1（L0）+ smoke2（L1）共 12 请求**全部 400 拒绝零生成**（`"Cannot use custom grammar constraints with tools."`）——检查项 1 即 FAIL，判定性 | ✅ 判定性负结果：GBNF 列移除，矩阵缩 2 列，省一次 ~47m 挂机 |
| prompt v2 列 ①（stage 3） | FULL 70q vs 基线，D4 主指标 | 37m07s：净通过 33 vs 基线 32；L1 +2（16 vs 14）但 L2 full-chain 1→0；degraded 21→16 | 🔶 存疑（增益与方差不可分）→ 跑列 ② 定夺 |
| prompt v2 列 ②（方差列） | 同 binary 同配置复跑 | 38m43s：净通过 35；**6 query 纯因非确定性翻转**（4 升 2 降）——±3 通过 / ±6 失败模式计数在噪声内 | ✅ 方差带量化 + 拍板依据：增益落带内 → **revert 回 v1**（`c043597`） |
| 修订 2 机制验证 | 靶点 query 逐条 | #21 列 ② 完整执行演示链（proc_help→proc_eject→proc_eject_status）/ idx63 两轮一致通过 / idx18 blocked 后解释符合措辞预期 | ✅ 写操作发现链机制可复现生效——单独记 v3 候选（TD-60） |
| record 落地不破基线 | D6 三重论证实测兑现 | 回归 1681→1689 仅 +8 新测试（0 failed）；实验列 meta 零污染；MCP 46 / catalog 47 不变 | ✅ 论证先行 + 实测验证双闭环 |
| 47 tool 全可用 | dispatch「不支持」清单 | 2 → **0**（record_start/stop 真实执行经 TUI confirm；CLI ask 维持拦截文案） | ✅ agent tool 能力拼图补全 |

**Findings 汇总**：P0 0 / P1 0 / P2 4（TD-55~57 仍 open + 新归档 TD-60 prompt v3 候选 + TD-61 GBNF 复测观察项 + 方差带标尺引用）。TD-58（test_alert flaky）整个 v0.23 cycle 各 stage 开工验证均未再现，继续观察。

---

## 1. stage 1 Spike：ADR-0033 实验设计 + GBNF 冒烟 + 双设计稿（commits `c746e14` + `1ca5295`）

### 落地范围

ADR-0033（D1 实验矩阵两变量漏斗式 + D2 run 记录文件名区分零代码 + D3 GBNF 冒烟降级路径 + D4 最优配置拍板标准 + D5 record_start/stop agent 语义六维表 + D6 落地不破 eval 基线三重论证）+ 附录 A prompt v2 措辞稿（2 处精确 diff，stage 3 才落地）+ 附录 B GBNF 冒烟判定标准 4 项与实测结论（**判定性负结果**）+ stage 1 doc；零业务代码（回归 1681/1705 与 v0.22.0 一致）。

### 4 维度审查

**代码质量** ✅：ADR 六决策先行的设计价值在后续 stage 全部兑现——D2 文件名区分零代码（YAGNI：`EvalRunMeta` 不加 grammar/prompt 字段，文件名 + `git_describe` 已可追溯）；D4 拍板标准预先锁定使 stage 3 的「保守 revert」有据可依而非临场判断；D6 三重论证把「落地不破基线」从直觉升级为可检验命题（stage 2 实测逐条兑现）。

**架构** ✅：实验设计与实装解耦——Spike 产出措辞稿（附录 A）/ 语义稿（D5）供 stage 2/3 消费，stage 1 零业务代码不污染实验变量；附录 B 判定标准 4 项**预先锁定**是冒烟结论可判定的前提（错误形态对照表让 12 次同型 400 即成判定性证据，无需 smoke3）。

**性能** N/A：零代码。冒烟本身用时 ~分钟级（12 请求即判），是「冒烟先行」成本收益的最小样本。

**完整性** ✅：GBNF 冒烟判定表逐项落（FAIL / N/A 判定 + 错误体原文逐字留档 + 版本语境绑定 llama-server b8685）；冒烟后 agent.toml 还原核对（grammar_file 回注释态，diff 为空）——实验环境零残留。风险 1 预判的坑（grammar × tools × Required × proc_finish 从未实测）以最干净的二值形态确认，无「部分兼容」中间态，补跑预案无需启用。

---

## 2. stage 2 Slice A：proc_record_start/stop agent 侧实装（commit `3defa68`）

### 落地范围

`RecordState` 新类型（`child: Arc<Mutex<Option<Child>>>` + `file_path: Arc<Mutex<Option<String>>>` 双槽）+ `start` / `stop` / `teardown_stop` 三方法薄包 MCP 既有 helper + `AgentSession::spawn` 内建状态（签名不变，builder.rs 零改动）+ `AgentRunner.record` 字段注入（CLI/eval 走 default）+ `execute_confirmed_tool` 两 tool 分支真实执行（参数映射兼容 catalog 与 MCP 风格；stop 无参语义以 start 记忆值为准）+ `teardown_agent_session` 先 stop 后 interrupt/shutdown + session_loop 退出兜底 + CLI 拦截文案更新 + 测试三组（TUI 端到端 ×2 / handle 级孤儿清理 ×1 / CLI 拦截锚 ×2）+ session.rs 内联单元 ×3。

### 4 维度审查

**代码质量** ✅：三处设计保持复用而非重写——(1) RecordState 三方法全部薄包 MCP `make_record_start_json` / `make_record_stop_json`（ADR-0029 record_handle pattern 的 agent 侧移植，录制语义与 MCP 路径同源）；(2) `AgentSession::spawn` 签名不变（record 状态内建于 session 构造，builder.rs 零改动——调用方零感知）；(3) teardown 双保险（App 主动 `stop_orphan_recording()` + session_loop 退出兜底 `teardown_stop()`，后者静默幂等——Handle 直接 drop 未走 App teardown 的场景也有兜底）。stop 无参语义（以 start 记忆值为准）适配 agent catalog 的 no_params schema，MCP 版 file_path 匹配校验在 agent 侧退化——单录制实例语义下合理。

**架构** ✅：句柄持有层选 AgentSession 而非 runner 是正确决策——跨 tool 调用保活的生命周期单位是「会话」不是「单次 run」（CLI ask 单轮进程退出录制即死，维持拦截；eval complete 路径 dispatch_value 层天然隔离，D6 论证 3 的结构保证）。`file_path` 记忆槽解决「stop 时模型不传参也能定位文件」——agent 侧 stop 无参化的前提。

**性能** N/A：录制子进程 spawn 是一次性开销非热路径；双保险 teardown 在 shutdown 路径多一次幂等检查，可忽略。

**完整性** ✅：测试覆盖三条关键路径（confirm Approved 后真实执行经 provider 二轮 messages 断言 / RecordState 跨线程共享孤儿清理 / CLI 拦截文案不变锚）+ 既有两「不支持」锚测试语义更新（`test_confirm_record_start_approved_executes` 等——锚随行为演进，非删锚）；一次时序 flaky（`test_session_drop_during_confirm_does_not_hang` 编译+满载下 5s 窗口）复跑全绿 + 单测隔离复跑绿，如实记录 CHANGELOG。D5「录制范围」措辞按实装修订（录后台系统监控画面非终端对话内容——stage 1 风险 4「以实装为准」协议兑现）。

---

## 3. stage 3 Slice B：实验矩阵 2 列 FULL + D4 拍板（commits `5d2ac64` + `c043597` + `590e2f2`）

### 落地范围

system.md 落 prompt v2（附录 A 2 处精确 diff 逐字落地，独立 commit 先于挂机）→ 列 ① FULL 37m07s（`eval-promptv2-70q.json`）→ 存疑跑列 ② 复跑 38m43s（`eval-best-70q.json`，兼方差测量列）→ `--compare` 三列矩阵 → 归档 `docs/eval/promptv2-70q-v0.23.md` → D4 拍板（用户拍板 A：无明确增益 **revert 回 v1**）→ ADR-0033 四处回填 + brainstorm 同步。回归 1689/1713 不变（prompt 是 include_str 编译文本，零测试改动）。

### 4 维度审查

**代码质量** ✅：D2 顺序铁律兑现——v2 独立 commit（`5d2ac64`）先于挂机，run meta `git_describe` 指向 v2 commit 且无 `-dirty` 后缀（两列 meta 同 hash，与基线列天然区分）；revert 路径干净（`c043597` 后 system.md 终态与 v1 逐字一致，净 diff 归零——「文本进代码」的实验变量可完整进可完整退，这是 prompt 类实验优于代码类实验的回滚特性）。

**架构** ✅：漏斗式矩阵的降级执行教科书式落地——GBNF 列移除后 2 列形态（列 ① 增益 + 列 ② 兼方差测量），**一列两用**（终验列兼方差列是 D1 缩列时的正确再设计，非妥协）；D4 拍板标准（主指标 + 靶点迁移 + 方差定夺三分支）使列 ① 结果「存疑」时有确定动作（跑列 ② 定夺）而非临场争论。

**性能** ✅：两列挂机 37m07s + 38m43s，均低于基线 47m19s——v2 列 query 均时更快本身是方差的佐证（挂机时长波动与通过数波动同源）；「结果 JSON 每 query 实时落盘」设计再次兑现挂机安全。

**完整性** ✅：Docker 环境一致性三轮核对（daemon 均未运行，docker 场景 1/10 / 2/10 / 2/10 同基线——单变量隔离成立，逐轮核对入归档）；靶点 query 逐条 verdict 完整（修订 1 两轮 1/4 迁移 + 根因分析——靶点场景缺真正的无参枚举 tool，引导语义被理解但无处落地；修订 2 三处证据）；失败模式迁移的定性拆解（「失败质量」与「失败转移」混合，非全净收益）诚实归档。

---

## 实测观察归档

### 观察 1：GBNF 判定性负结果的时间线价值——冒烟先行的成本收益极值

**现象**：stage 1 冒烟 12 请求（performance-diagnose L0 3×2 + usb L1 3×2）**全部** 400 拒绝（`"Cannot use custom grammar constraints with tools."`），零生成——llama-server b8685 对 grammar × tools 同传是请求级硬校验，与 query 内容/场景/步数无关。

**时间线价值**：该结论在矩阵挂机**之前**落地，直接关闭 GBNF 列——省一次 ~47m FULL 挂机 + 关闭 brainstorm 风险 1 的全部不确定性 + ADR-0030 决策 C 逃生舱状态落定（「tools 协议模式下结构性不可用」，未来路径是协议层重写非配置开关）。12 请求分钟级成本换一整列挂机，是「冒烟先行」成本收益的极值样本。

**归档教训**：(1) 冒烟要能**判定**——附录 B 的 4 项判定标准（含错误形态对照）预先锁定，12 次同型错误即成判定性证据，无需跑 smoke3；(2) 错误在请求校验层时与场景无关，单场景重复即可判，多场景矩阵冒烟是浪费；(3) 负结果价值成立的前提是结论**绑定版本语境**（llama-server b8685）+ 复测路径留档（升级后按判定表重跑，TD-61）。

### 观察 2：prompt v2 双列实验 + E2B 方差带首次量化——单列数据不可单独归因

**现象**：同 binary 同配置复跑（列 ① vs 列 ②）**6 query 纯因非确定性翻转**（4 升 idx16/34/42/56、2 降 idx14/46）；失败模式计数轮间自差 degraded 3 / wrong_tool 6（直方图轮间漂移最大项）。

**方差带标尺**：单次 run 的 **±3 通过数 / ±6 失败模式计数**在噪声内——v2 净通过 +1/+3 落带内，增益无法与方差区分（D4 保守拍板的直接依据）。**这是 v0.24 模型底座决策的解读前提**：未来所有 `--compare` 单列差异小于此量级时不可单独归因，必须复跑定夺。

**靶点与机制的分离判读**：整体增益不成立 ≠ 机制无效——修订 2（写操作发现链）三处证据可复现（#21 列 ② 完整执行演示链 / idx63 两轮一致通过 / idx18 blocked 后解释），机制生效但整体数字被修订 1 的 L2 回退嫌疑（chain_incomplete +3/+4 两轮同向）与方差淹没。捆绑实验无法分离归因 → 修订 2 单独记 v3 候选（TD-60，单独 diff 单变量再实验）。

### 观察 3：record 落地与 eval 口径隔离——D6 三重论证的实测兑现

**现象**：stage 2 在实验矩阵（stage 3）**之前**落地 record 真实执行，但实验列零污染——回归 1681→1689 仅 +8 新测试（0 failed）、两列 run meta 干净、MCP 46 / catalog 47 不变。

**兑现链**：D6 三重论证（零 query 依赖 / catalog 名单不变 / eval 无 confirm 通道——写 tool 永远 blocked）每条都在实测中兑现：70 query 无一触达 record 分支；模型可见面（catalog/schema）零变化；eval complete 路径的 dispatch_value 层拦截使行为变更在 eval 口径下不可见。**「论证先行 + 实测验证」双闭环**是本轮双主题（工程 + 实验）互不干扰的结构保证——也是「先做后做都不污染实验」（决策 9 排序理由）的事后验证。

### 观察 4：TD-55~57 终态确认——无 key 第 4 个 cycle open

**现象**：`ANTHROPIC_API_KEY` 自 v0.20 归档起持续缺席，TD-55（Sonnet 对照）/ TD-56（model ID 验证）/ TD-57（nudge 路径）连续 4 个 cycle open（v0.20 归档 → v0.21 不跑 → v0.22 不跑 → v0.23 决策 3 不跑）。

**终态口径**：闭环路径不变且成本进一步降低——有 key 后 `ANTHROPIC_API_KEY=... proc agent eval --provider anthropic --output ...` 一条命令三连闭环（Sonnet 70 query 对比列 + model ID 真实验证 + nudge 路径覆盖）；v0.24 若启动「更强模型底座」决策，云端档（Sonnet）与本地档（E4B/Qwen）天然合并进同一对比矩阵，TD 到期即闭环。

---

## Findings 表

| 级别 | # | 内容 | 处置 |
|---|---|---|---|
| P0 | — | 无 | — |
| P1 | — | 无 | — |
| P2 | TD-55~57 | Sonnet 70 query 对照 / model ID 真实 API 验证 / nudge 路径实测（v0.20 归档，无 `ANTHROPIC_API_KEY`） | 仍 open（终态确认见观察 4）；有 key 后 eval runner 1 条命令三连闭环；v0.24 模型底座决策若启动则自然到期 |
| P2 | TD-60（新） | prompt v3 候选——修订 2（写操作发现链）单独实验验证：本轮 3 处证据表明机制生效，但与修订 1 捆绑时整体增益落方差带内无法分离归因 | 归档 `docs/tech-debt.md`；v0.24+ 与 RAG cycle 一并评估（单独 diff 修订 2 → QUICK 冒烟 → FULL vs 基线，方差带标尺解读） |
| P2 | TD-61（新） | GBNF grammar × tools 复测——llama-server b8685 互斥结论绑定版本，升级 llama.cpp 后按附录 B 判定表重跑 | 归档 `docs/tech-debt.md` 观察项；挂 llama-server 升级节点，不主动排期 |
| — | 方差带标尺 | ±3 通过数 / ±6 失败模式计数（本 cycle 首次量化） | 非 TD 是标尺——归档 `docs/eval/promptv2-70q-v0.23.md`；**v0.24 模型底座 `--compare` 解读必须引用**（单列差异小于此量级不可单独归因） |
| — | TD-58 | `test_alert::test_metric_extract_process_cpu` 并发 flaky（v0.21 归档观察项） | v0.23 cycle 各 stage 开工验证均未再现，继续观察，原判不动（stage 2 一次 `test_session_drop_during_confirm` 满载 flaky 系另一测试，复跑绿已记录 CHANGELOG，不并入本条） |
| — | TD-59 | session log 累积无轮转（v0.22 归档观察项） | 维持观察，未触发（磁盘占用可感知 / 目录上万文件才治理） |

---

## cycle 数据汇总

| 维度 | 数字 |
|---|---|
| stage 数 | 4（1 Spike + 2 Slice + Review+收尾段；无回溯修复子阶段、无 Checkpoint 接力——容量风险全部未触发） |
| commits | `c746e14`（plan）/ `1ca5295`（stage 1）/ `3defa68`（stage 2）/ `5d2ac64`（stage 3 v2 落地）/ `c043597`（stage 3 数据拍板 revert）/ `590e2f2`（stage 3 归档收尾）+ stage 4（本段）+ tag `v0.23.0` |
| 全量回归 | 1681 → **1689**（默认）/ 1705 → **1713**（anthropic），0 failed 全程；ignored 9/10 不变 |
| 新增测试 | +8 CI（stage 2：TUI 端到端 ×2 + handle 级孤儿清理 ×1 + CLI 拦截锚 ×2 + session 内联单元 ×3）；stage 1/3 零测试改动 |
| MCP tool / agent catalog | 46 / 47 + proc_finish **均不变**（record 落地不加——catalog 早在册；零 tool 变更延续 v0.22 干净基线口径） |
| ADR | 新 1 份（ADR-0033，D1~D6 + 附录 A 措辞稿 + 附录 B 冒烟判定标准与实测） |
| Cargo deps | +0 |
| 新文件 | `docs/adr/0033-eval-experiments-and-record-tools.md` + `docs/eval/promptv2-70q-v0.23.md` + `tests/test_agent_v0_23_stage_2.rs` + `docs/stages/v0.23-{brainstorm,stage-1..4}.md` |
| 行数（insertions 口径） | src +216/-13（vs 预估业务 ~100，~2×——RecordState 三方法 + teardown 双保险全在 session.rs 151 行内，D5 落地注记口径，合理漂移）；tests +412/-19（vs 预估 ~200+250，~0.9×——TUI 端到端 + 孤儿清理 + CLI 锚三组 + 既有锚语义更新）；docs +1384/-1（vs 预估 ~950，~1.05×——brainstorm 392 本体 + 4 份 stage docs 903 + ADR 180 + eval 归档 71 + CHANGELOG/ADR README 28） |
| 实验挂机 | GBNF 冒烟 ~分钟级（12 请求判定性）+ FULL 2 列 37m07s + 38m43s（vs 原设计 3 列 ~2.5h——缩列后 ≤1.6h 预估兑现，实际 ~1.3h） |

**与 v0.22 cycle 对比**：4 stage 同款节奏；~2000 vs ~6000 行级（决策型轻 cycle vs 测量层重 cycle）；deps +0 双零；tool 层双零变更。**「能力维度」**：v0.22 交付测量层（能力可量化），v0.23 交付**决策 + 标尺**（测量层的第一次实际使用：两条修复路径以数据关闭 + E2B 方差带量化——「单列差异不可单独归因」从此是项目纪律而非直觉）；**「拼图维度」**：47 tool 全部真实可用（v0.20 建立以来的「不支持」清单清零）。数据资产续扩：`eval-promptv2-70q.json` / `eval-best-70q.json`（本地留存）与 `eval-e2b-70q.json` 构成三 run 基线族，`--compare` 即插即出。

---

## v0.24+ 候选方向

brainstorm 备注（cycle 末段评估）+ 本 Review 归档综合（决策 2/7/8 既定排期锚定）：

| 优先级 | 方向 | 依据 | 规模预估 |
|---|---|---|---|
| 1 | **RAG 历史经验召回主体（方向 B）+ 更强模型底座一并决策** | 决策 2/7 既定：更强模型列（Gemma 4 E4B / Qwen 系，需下载）推 v0.24 与 RAG 一并评估——RAG 注入增大 context 负担，模型底座先定才有干净归因；session JSONL 数据源就位（v0.22）+ 方差带标尺（v0.23）使模型对比列首次**可解读**（±3/±6 以下的差异不再误读为增益） | ~1500 行+ 完整 cycle（检索方案选型 / 注入位置与预算 / 评估口径） |
| 2 | prompt v3（TD-60） | 修订 2 机制证据（3 处）+ 本 cycle 归档的实验路径：单独 diff 修订 2 单变量 → QUICK → FULL，可与方向 1 的模型列同矩阵跑 | ~0 行（文本）+ 挂机 2 列 |
| 3 | TD-55~57 补验 | 有 `ANTHROPIC_API_KEY` 即 1 条命令三连闭环；Sonnet 云端档可与方向 1 的本地档合并进同一对比矩阵 | ~0 行 |
| 4 | GBNF 复测（TD-61） | 挂 llama-server 升级节点（观察项，不主动排期）；复测路径 = 附录 B 判定表重跑 | ~0 行（配置） |
| 5 | Multi-agent 协作（方向 D） | 维持 RAG 之后（决策 8 澄清口径——协作 agent 的价值依赖单 agent 能力上限先拉满） | — |

**v0.24 模型底座决策的解读纪律**（本 cycle 最重要遗产）：任何 `--compare` 单列差异 < ±3 通过数 / ±6 失败模式计数时**不可单独归因**——必须复跑定夺（本轮 prompt v2 拍板的直接方法论）；更强模型列的预期信号量级应显著超带（如 E4B 预期 L2 双位数改善）才值得换默认。
