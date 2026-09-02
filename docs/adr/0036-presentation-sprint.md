# ADR-0036：秋招展示冲刺（评估者视图 + 质量门禁自动化 + 测试深度）

**Status**: Accepted（v0.26 stage 1 Spike 落地——D1~D5 五组决策定稿 + 取数附录（`docs/stages/v0.26-stage-1.md` 附录 A-E））

**Date**: 2026-08-31（v0.26 cycle stage 1 Spike）

**Related**: ADR-0022（Windows-only 权衡——深挖导览第 ⑩ 条素材）、ADR-0032/0033/0034（eval harness / 实验 / RAG——AI/eval 叙事章的数字来源）、PERF-BASELINE-v0.13（bench 对照基线）

## Context

v0.26 是秋招窗口期（2026-08~11）的**展示冲刺轻 cycle**（brainstorm 5 决策拍板，零挂机，doc 为主）。定位判断：项目工程强度已足（65K src / 27K tests / 1742+1766 双档回归 / 36 ADR / 25 releases），短板在**评估者（面试官）10 分钟内看不到强项**；另一实证短板是 R2（v0.25 stage 3 clippy 漏检）——「门禁靠人工排序执行」的结构性弱点。

**Spike 动机**：展示层的每个数字都会被面试官抽查（错误数字比没有数字更伤）；gate 快档选型需要 test binary 耗时实测数据；README 重构需要既有章节底图（防破坏老用户手册价值）；竞品对比需要 as-of 数据（宁缺毋滥）。

**Spike 取数**（全部落 stage-1 doc 附录 A-E，2026-08-31 实测）：bench 7 项（附录 A，对照 PERF-BASELINE-v0.13 同机基线）/ 79 test binary 耗时分布（附录 B）/ unsafe 198 处分布 + SAFETY 注释覆盖（附录 C）/ 竞品 GitHub API 核实（附录 D）/ demo GIF 录制规格（附录 E，交用户录制）。

## Decision

### D1：README 信息架构——评估者视图加顶部，既有内容后移不删 ✅

**双受众原则**（brainstorm 风险 8）：面试官（10 分钟扫读）+ 老用户（功能手册）共存——评估者视图**插入在 `# proc` 头之后、`## 功能` 之前**，既有 825 行全部后移保留，重构以段为单位 diff 可审。

**评估者视图四件 + 一章**（stage 3 实装，顺序即阅读序）：

1. **CI badge 条**（ci.yml + release；badge 挂接前提 = 远端 CI 绿，brainstorm CI 锚）
2. **demo GIF**（附录 E 规格，用户录制；未就绪占位 `<!-- demo GIF -->` 不阻塞）
3. **架构图**（mermaid 分层：UI（TUI ratatui + CLI）→ 领域面板 → App controllers → workers（SnapshotWorker + WorkerManager 崩溃自愈）→ 采集层（sysinfo / ETW / sysinfo-delta / smartctl / DNS·NetFlow·Docker）→ 横切：security / record-replay / MCP server（46 tools）/ agent（providers + dispatch + RAG + eval））
4. **量化亮点条 + 竞品对比表**（数字全部 D5 溯源；对比表只用附录 D 核实数据 + 「proc 有」断言，不做「竞品没有」的功能矩阵断言（除 AI/MCP 已核实项））
5. **「AI Agent 与 eval 纪律」章**（六列 eval 矩阵 + 预登记门槛（净通过 ≥ +7 且 L2 方向性）+ 负结果三连归档叙事 + L2 full-chain 5% 能力边界——数字逐个对回 REVIEW-v0.23/24/25 原文）

**既有章节处置**（stage-1 任务 4 底图）：全部保留；`## Benchmark` 段（810-822 行）改为链到 `docs/performance.md`（stage 3 新建，附录 A 取数成文 + v0.13 基线回顾 + 启动时间/内存实测）。

### D2：gate 脚本两档 + hook opt-in + required checks 主防线 ✅

**两档设计**（`scripts/gate.sh`，stage 2 实装）：

| 档 | 内容 | 目标时长 | 用途 |
|---|---|---|---|
| **快档** | `cargo fmt --all -- --check` + clippy 双档（默认 + `--features anthropic`）+ **测试子集**（附录 B 耗时摸底选定：覆盖核心语义的最快 binary 组合） | **< 3-5 min** | pre-push hook / 日常开发 |
| **全档** | 快档 + 全量回归双档（79 binary，~10 min 级） | ~15 min | stage 完工 / 合并前 |

**防线分层**：① pre-push hook 是 **opt-in**（`git config core.hooksPath .githooks` + doc 说明安装）不强推（brainstorm 风险 5：hook 被绕过不如没有）；② **GitHub required checks 是主防线**（仓库 Settings 手动项，stage 2 doc 写操作说明）；③ gate.sh 快档选型以附录 B 实测为准，失衡时 stage 2 实测调整（砍名单不砍纪律）。

**R2 结构性修复**：gate 把「fmt → clippy → build → test」从人工排序变成脚本强制顺序——「变更落在验证之后」的窗口被关闭。

### D3：proptest dev-dep 预登记（runtime deps +0 纪律延续）✅

- **runtime deps +0** 纪律延续（v0.22 以来不变锚）
- **dev-deps +1**：`proptest`（本 cycle 显式批准——brainstorm 打包清单 ⑥ 组预登记改判）
- 实装（stage 2）：`tests/test_filter_proptest.rs` FilterExpr roundtrip 属性测试——`parse(format(expr))` 语义等价 + 乱输入不 panic；cases **~256**（16GB 机器秒级预期；超时则降 cases 不加 feature gate，Simplicity First）

### D4：深挖 10 条清单封闭（只砍不加）✅

`docs/architecture-deep-dive.md`（stage 4）每条 30-45 行预算，超限砍条目不砍深度：

| # | 条目 | 证据源 |
|---|---|---|
| ① | PID 复用键控（(pid, start_time) 元组） | ADR-0003 |
| ② | 采集三路 + ETW→sysinfo delta 改道三理由 | ADR-0035 D2 |
| ③ | worker 指数退避 + TD-24 止损 | ADR-0019 + TD-24 |
| ④ | D1 延迟创建 vs Drop 三缺陷 | ADR-0035 D1 |
| ⑤ | VT100 vs UiFrame 双格式录屏 | record/ 模块 |
| ⑥ | 安全纵深四件套（评分/self-mitigation/restricted spawn/env mask） | v0.6 ADR-0008 |
| ⑦ | MCP 46 不变锚 + x-deprecated 双轨 + snapshot 复用 | ADR-0026 + v0.25 |
| ⑧ | OpenAI tools over llama.cpp + GBNF 否决证据链 | REVIEW-v0.22 |
| ⑨ | RAG 全链 + 判读纪律（±3/±6 带） | ADR-0034 + REVIEW-v0.23 |
| ⑩ | ADR-0022 Windows-only 权衡 + eval 科学方法论 | ADR-0022 + ADR-0033 |

**中性命名纪律**：文件名 `architecture-deep-dive.md`、行文「设计深挖导览」——内容本身是架构叙事，评估者看到是加分（brainstorm FAQ Q3：备考价值与展示价值同构）。

### D5：数字溯源纪律（诚信风险——面试场景放大）✅

- README / `docs/performance.md` / `docs/architecture-deep-dive.md` / `docs/unsafe-audit.md` 中**每一个数字必须有来源**：REVIEW 原文 / CHANGELOG / bench log（本 cycle 实测转写，非记忆值）/ git 数据（`git log --oneline \| wc -l` 等）/ 附录 A-E
- **bench 数字**：必须转写自附录 A（= criterion 输出），不引 v0.13 基线记忆值（对照时可引 PERF-BASELINE-v0.13 原文行）
- **Review 验收**（stage 4）：抽查 **≥ 10 个数字**回溯核对；发现自创数字 = P0
- **竞品数字**：只用附录 D（GitHub API 2026-08-31 实测）+ as-of 标注

## Consequences

1. **展示层可信**：所有对外数字可回溯——面试官抽查不塌（对比「错误数字比没有更伤」）
2. **门禁机器化**：R2 类「人工排序漏检」结构性关闭；gate 快档把日常验证压到 3-5 min 内
3. **测试深度叙事补齐**：proptest 属性测试 + unsafe audit doc（198 处分布 + SAFETY 覆盖现状 + edition 2024 lint 已合规结论）成为「测试文化」的可展示证据
4. **成本**：~2100 行级（doc 为主）；demo GIF 依赖用户录制（占位不阻塞）；README 变长（评估者视图 ~300-400 行新增，双受众取舍已拍板）

## stage 2 实装注记

（2026-08-31 stage 2 会话回填）

- **R1 修复锚**：`LlamaServerHandle::pid()` + `LlamaCppProvider::server_pid()`（getter 小改，未触及 spawn 生命周期——风险 7 边界保持）；e2e 清理断言改 `tasklist /FI "PID eq N"` 查自身子进程退出。验收：`--test test_llama_cpp_provider` 连跑 3 轮稳定绿（27/27 × 3，真实 server 并行场景）。TD 台账回填（v0.26 cycle 追踪段，未立 TD-62）
- **gate.sh 快档实测**（i7-13700HX / 16GB / Win11）：稳态（无变更）**29s**——远低于 < 5 min 目标 ✅；src 变更后首跑 **~9.5 min（569s）**——clippy 双档编译 + 21 binary release thin-LTO 链接主导，名单大小非主要变量（砍名单不显著缩短，纪律保持不砍）。两档数字如实入档（gate.sh 头注释同步）
- **proptest cases 实测**：256 cases × 2 属性 **0.05s**（风险 4 秒级预期兑现，无需降 cases）；解析版本 proptest 1.11.0（dev-dep +1，runtime deps +0 兑现）
- **快档名单定稿**：附录 B 20 binary + 本阶段新增 `test_filter_proptest` = **21**（单次 cargo 调用 `--test` × 21 串行，省 spawn 开销）
- **首跑拦截实证**：stage 2 会话首次跑 gate 快档即拦截 1 处 fmt 违规（rustfmt 对新增测试文件的换行改写）——门禁生效的直接证据

## stage 3 实装注记

（2026-09-01 stage 3 会话回填）

- **README 重构落地**（+114/-1，既有 825 行后移不删）：`## 项目概览` 插入首段定位之后（badge 注释占位 + GIF 注释占位 + mermaid 分层架构图 + 亮点条 3×3 + 竞品对比表）；`### AI Agent 与 eval 纪律` 章插在既有 AI Agent 段尾（六列矩阵 + 判读纪律五条）；`## Benchmark` 段改链 performance.md + 性能摘要三点
- **badge 占位根因（用户拍板 2026-09-01）**：远端 CI 三 workflow 全红且本地领先 origin/master 7 commits 未 push——CI stable 1.98.0 vs 本地 1.95.0 clippy 漂移 2 处（`hash_cache.rs:146` / `smart/mod.rs:186`）+ check-macos cfg-gate unused imports + audit 2 漏洞（rmcp 0.11.0 RUSTSEC-2026-0189 / crossbeam-epoch 0.9.18）+ miri 历史全红（E0433）+ release winget action 上游删库。**「远端 CI 绿」前提不成立 → badge 注释占位**（注释体含完整 badge 语法，转绿即启用）；修复清单留档 stage-3 doc「已知阻塞」段，处置留 stage 4 搭车评估或 v0.27
- **GIF 挂载状态**：`docs/assets/demo.gif` 未录制（用户协作项未启动）→ 按风险 1 占位不阻塞；亮点条口径改「23 顶层子命令」（本会话 `--help` 实测，替代旧文 v0.15 时代「32 子命令」过时数）
- **performance.md 落地**（新 ~120 行）：附录 A 逐行转写 + v0.13 对照 + 归因只写 TD-47 `4c7e294`（其余提升明确不写归因——D5 兑现）+ 启动/内存实测（`proc ls` 2291-2875 ms 合并口径 + record headless 峰值 56.7 MB，单机单次样本非基准声明）
- **eval 章数字溯源**：六列矩阵每个数字对回 REVIEW-v0.22/23/24 + rag 归档原文；「—」格注明来源未单列不自创；proc stars 如实写 1（gh api 2026-09-01）
- **验收**：回归双档 1744/0/10 + 1768/0/11（80 行，零 rs 变更开工即终值）；tool 46 / catalog 47 / runtime deps +0 不变锚在位；数字溯源 20 项全 ✅（stage-3 doc 核对表）

## stage 4 实装注记

（2026-09-01 stage 4 会话回填）

- **深挖导览落地**：`docs/architecture-deep-dive.md` **10 条全保**（无砍单，单条 30-45 行预算内）；每条 L1/L2/L3 三层 + 证据链接 + 一句话结论。**条目①证据源修正**：D4 表引「ADR-0003」经 git 全历史查证是**幽灵引用**（`0003-pid-reuse-start-time-key.md` 从未入库，现行 0003 是 smartctl 选型）——产出改用代码层真实证据（app.rs / score.rs / mcp handler / detail_view / flow 五处 (pid, start_time) 键控 + TD-21/ADR-0005 边界 + process_cache 单键权衡的诚实声明）；幽灵引用归档 TD（v0.27 顺手修四处引用点）
- **unsafe-audit 落地**：独立 doc `docs/unsafe-audit.md`（~120 行，非 deep-dive 附录）+ **4 处 SAFETY 注释小修**（handles ×2 / estats ×1 / collect ×1——动态加载 NT 函数 / 灵活数组 / 两段式 sizing / ToolHelp 四模式 invariant；纯注释零行为变化，回归双档数字不变）；NT API 层其余 ~93 处留 v0.27+ 专项（注释写错比没有更伤）
- **Review 抽查**：数字溯源抽查 14 项（> 10 门槛）——**发现 1 处算术错误**（v0.24 归档汇总行「引用 57%」实际是 8/15=53%，8/14 才=57%；README 三处 + 深挖⑨已改 53% 并注勘误）+ 1 处漂移（亮点条 65,074/26,968 是 stage-1 时点值，收尾态实测 65,099/27,480 已更新）；**零自创数字**（P0 无）。gate 有效性实证：fmt 违规注入 → gate.sh [1/4] 步拦截 exit≠0 → 还原复绿
- **CI 议题拍板（用户 2026-09-01）**：① clippy 1.98 漂移 2 处（hash_cache question_mark / smart manual_filter）**搭车修**（机械改写语义零变化，本地 1.95 clippy 双档仍绿）；② 收尾 commit + tag 后 **push 9 commits + tag 到 origin/master**（badge 维持占位——audit/mirii/macos 仍红，CI 修复专项留 v0.27）；③ demo GIF **用户本窗口录制并挂载**（`docs/assets/demo.gif` 1.34 MB / 35.1s，ShareX 采集，README 注释占位已替换）
- **TD-62 新立**：session confirm 中断测试负载敏感 flaky（开工基线 anthropic 档连续 2 次同断言红→复跑绿→单跑绿）——观察项归档（不在 gate 快档名单，TD-58 同款处置）
- **验收终值**：回归双档 1744/0/10 + 1768/0/11（80 行核对）；MCP tool 46 / catalog 47 / runtime deps +0 不变锚全程保持；CHANGELOG 0.26.0 + Cargo bump + README v0.26 banner + tag `v0.26.0` 三处版本一致
