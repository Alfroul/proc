# REVIEW-v0.25 — v0.25 cycle Review（TD 清仓 + session 语料卫生维护型轻 cycle 完结）

> **cycle 范围**：brainstorm 5 决策（2026-08-27 拍板会话全 ✅）——决策 1 模型升级不下载（第 3 次搁置，组合设计留档附录 A）+ 决策 2 RAG 复测语料口径维持三基线 run（归档预登记）+ 决策 3 TD-55~57 无 key 不跑 + 决策 4 3 stage 节奏 + 决策 5 TD 清仓 + 语料卫生主题；1 份 ADR-0035
>
> **Review 范围**：3 stage 全部产出（1 Spike + Slice A + Slice B + 本 Review/收尾段）
>
> **基线**：1725 passed / 0 failed / 10 ignored（v0.24.0 默认档）+ 1749 / 0 / 11（anthropic 档）→ **1742 passed / 0 failed / 10 ignored（默认档）+ 1766 passed / 0 failed / 11 ignored（`--features anthropic`）**（+9 stage 2 + 8 stage 3，双档同量）/ fmt / clippy（双档）/ build（双档）/ bench --no-run 全过
>
> **Review 日期**：2026-08-30
>
> **Reviewer**：Claude（stage 3 会话；审查依据 ADR-0035 D4 验收锚 + 各 stage doc 验证记录 + CHANGELOG stage 段，按数据/测试锚审不自评——brainstorm Q5 口径）
>
> **零挂机声明**（brainstorm 风险 4）：本 cycle 无 eval 列——质量论证不靠画像对比，靠 ① tool 语义不变原则（只加响应字段不改既有字段含义，`x-deprecated` 是 hint 非删除）② 回归数字 + MCP tool 46 / catalog 47 不变锚 ③ queries.toml 70q 不涉新字段（sparkline / per-process disk_io 无对应 query）④ 回归测试锚定行为契约（三路径语料 5 例 + TD-24 状态机 3 例 + TD-40 regex 1 例 + TD-53 8 例 + TD-50 schema 1 例）

---

## 概览

v0.25 cycle 是主线条件未齐窗口期的**维护型轻 cycle**（净规模 ~1100 行级）——**「与其空转等待主线前置条件，不如清偿积压工程债 + 治理语料噪声，为模型底座重启交付干净底座」**。三组打包全交付：① **语料卫生**（D1 延迟创建——TUI 进 Agent 面板不发问不再落盘，98% 空会话现象根治，RAG 主语料停止噪声增长）② **边角清仓**（TD-24 止损状态机 + TD-40 regex 硬化 + TD-25/34 文档关闭）③ **MCP 观测补全**（TD-53 改道 sysinfo delta per-process 段 + TD-50 schema hint + TD-52/54 回填对齐）。ADR-0035 stage 1 现状核查把 cycle 规模从 ~1900 砍到 ~1100（TD-52/54 已在 v0.17 落地的重大发现）——**「设计阶段先实测现状再定方案」范式第三次兑现**（v0.24 附录 B 之后）。

- **MCP tool 46 / agent catalog 47 均不变**（TD-50 只加 hint / TD-53 是既有 tool 响应扩展——D4 不变锚）
- **Cargo deps +0**（TD 连零记录延续：sysinfo 既有字段 / serde_json / rmcp 既有 meta 属性 / regex 自带 API）
- 1 份新 ADR-0035（D1~D4 四终判 + stage 2/3 实装注记）
- 回归 +17（stage 2 +9 / stage 3 +8，双档同量）；TD 存量十余项 open → 8 项（全部既定归档 / 观察项，无新增债）

**核心实测数字**（详见各 stage 段）：

| 验收项 | 口径 | 实测 | 结论 |
|---|---|---|---|
| 空会话治理（D1，stage 2） | ADR-0035 D1 回归测试口径（三路径） | 三路径 5 例测试全绿：TUI 语义不发问无文件（recorder 层 + session 层）/ Error-only 2 行诊断文件 / query 进行中退出文件保留 / ask·eval 不落盘现状锚；发问首两行（session_start → query_started）由既有 v0_22_stage_3 生命周期测试覆盖 | ✅ 102/100 现象根治（机制归因 stage 1 修正：TUI 进面板即落盘，非 eval run） |
| TD-24 止损（stage 2） | TD 原文 mock 口径：spawn_one 恒 false → MAX_RETRIES 后 permanent_failure | 非 canonical thread_name 直插 `restart_history` 确定性失败，三次 tick（5s/30s/300s）后 retry_count=3 + `PermanentFailure` banner + 止损不再尝试 | ✅ 原「retry_count 永不增长、banner 永远 Restarting」无限重试消除 |
| TD-40 硬化（stage 2） | 极端 regex 编译期拒绝 | `(a{300})^{300}`（展开 ~90K 指令）被 `RegexBuilder::size_limit(64KB)` 拒绝，普通 `^Adobe` 存活 | ✅ 理论 ReDoS 面关闭 |
| TD-53 改道（stage 3） | ADR-0035 D2 改道规格（delta 口径 + per_process 段 + source 声明） | 单元 4 例（差分精确断言 / 首观测基线 / counter 回退 saturating / 基线重建 + PID 复用键控）+ 响应段 2 例（source: "sysinfo-delta" / 降序 / top 截断 / 既有字段不缺）全绿；`proc_ls` disk_read_bps/write_bps 随 worker 填充附带受益 | ✅ 避开 NT Kernel Logger 单实例陷阱（原方案否决理由成立），与 TUI 非管理员档同口径 |
| TD-50 hint（stage 3） | schema 层 `_meta.x-deprecated`（运行时断言） | `proc_smart_tool_attr().meta` 含 `x-deprecated: true` + `proc_metrics_smart` 阴性对照 None；MCP tool 46 运行时锚 | ✅ 与 v0.17 文本层 hint 双轨，tool 不删保外部 client |
| TD-52/54 回填（stage 3） | ADR-0035 D2 现状核查（v0.17 已落地） | tech-debt.md 状态与代码对齐（system_history + proc_metrics_history tool / snapshot 持久字段 + 1s tick worker）；TD-54 开销锚（连续 metrics_* 调用复用 snapshot 不重建）由 v0.17 warm-up 测试族为锚 | ✅ 状态滞后修正，无代码 |
| D4 不变锚（全程） | tool 46 / catalog 47 / deps +0 | `grep "name = \"proc_"` 46 + `list_tool_names().len() == 46` 运行时断言；`load_eval_queries` 47 校验全绿；Cargo.toml/Cargo.lock 零依赖 diff（lock 仅 proc 自身版本行） | ✅ 三不变锚全程保持 |

**Findings 汇总**：P0 0 / P1 0 / P2 3（① TD-55~57 仍 open 第 7 个 cycle（无 key）② `proc_smart` description 内 "will be removed in v0.18+" 文本过时——v0.17 静态 grep 测试锚定该字符串，更新需连测试一起改，记 v0.26+ 文档候选 ③ TD-54 关闭注记如实记录 `proc_flows` 仍走 `App::new` ~2s warm-up（flows 数据源是 Schannel worker 非 SystemSnapshot，不在 snapshot 复用范围——若未来 agent 高频调 flows 再评估））+ TD-58/59/61 维持观察。

---

## 1. stage 1 Spike：空会话归因 + ADR-0035 定稿 + TD 逐项终判（commit `e278a37`）

### 落地范围

ADR-0035（D1 延迟创建终判（弃退出清理——Drop 语义不可靠）/ D2 MCP 持久化现状核查（**TD-52/54 v0.17 已落地重大发现**——stage 3 实装从 ~450 砍到 ~100-150）+ TD-53 改道终判（ETW worker 方案三理由否决 → sysinfo delta）+ D3 TD 逐项终判表（12 项，清单封闭只砍不加）/ D4 验收锚）+ 空会话机制归因修正（ask/eval 走 build_runner 无 recorder；空文件唯一来源是 TUI AgentPanel 进面板即 `SessionRecorder::start` 构造即 File::create——brainstorm「与 eval run 时间吻合」是相关非因果）+ TD-34 判废 / TD-22 已修回填 / TD-25 ADR-0019 追加段；零业务代码。

### 4 维度审查

**代码质量** ✅：零代码但两个归因修正质量关键——① 空会话来源归因从「eval run 副作用」修正为「TUI 进面板即落盘」（成对出现 15-30s 间隔 + `-llama-cpp` 后缀的文件级证据 → 源码级 `enter_agent_session` 调用链证据）；② TD-52/54 状态与代码不一致的发现让 stage 3 实装缩水 2/3。「先实测再定方案」避免了重复实装已落地功能。

**架构** ✅：D1 两路对比表（延迟创建 vs 退出清理）判定理由完整（Drop 语义三缺陷枚举）；D3 终判表逐项有 Spike 实测依据（位置 + 现状 + 理由仍立核查），清单封闭兑现（只砍不加，无新发现债混入——TD-22 是已修项回填非新债）。

**性能** N/A：零代码。

**完整性** ✅：D4 验收锚预登记（不变锚 + 递增预期 + tool 语义不变原则）让 stage 2/3 的验收是机械核对非自由发挥；TD-53 改道方案的 fallback 路径清单 + 单实例冲突场景两项预登记标准实际执行核对（ADR D2 记录）。

---

## 2. stage 2 Slice A：语料卫生 + 边角清仓（commit `43300d9`）

### 落地范围

D1 延迟创建（`RecorderInner` 持 `path` + `pending_start` + `writer: Option`，物化收敛 `write_entry` 单点）+ TD-24 `on_respawn_failed` 状态机 + TD-40 `RegexBuilder::size_limit(64KB)` + 新测试 9 例（`tests/test_agent_v0_25_stage_2.rs` 三路径 5 + TD-24 单测 2/集成 1 + TD-40 单测 1）；业务 ~90 行 + 测试 ~180 行 + doc ~150 行。

### 4 维度审查

**代码质量** ✅：D1 物化收敛单点（`write_entry`）是设计亮点——TextDelta 聚合段经 `flush_pending` → `write_entry` 同样触发，触发口径天然一致；`is_enabled()` 语义不变（构造时目录检查保留，仅文件创建延迟）让既有调用方零改动。TD-24 mock 口径规避环境不确定性（非 canonical thread_name 走 `_ => false` 确定性失败，不依赖管理员权限）。

**架构** ✅：延迟创建对「TUI 中途退出」边界的覆盖是结构性的（无 query = 无文件，不依赖 Drop）；测试锚定「ask/eval 不落盘」现状（防未来 recorder 接进 build_runner 时口径漂移）比只测新行为更有防御价值。

**性能** ✅：零热路径影响——session_log 在 agent 会话路径（非 UI 帧）；`pending_start` 暂存是一次 Option 存储。

**完整性** ✅：三路径口径全落地（ADR D1 stage 2 回归测试口径逐条）；`tests/test_agent_rag.rs` 全绿实证 D1 成功段口径核对（无 QueryStarted 的文件在 corpus 状态机下不产语料）。

---

## 3. stage 3 Slice B + Review：MCP 观测补全 + cycle 收尾（本 commit）

### 落地范围

TD-53 改道三层接线（`process_cache_mut()` 访问器 + `compute_process_disk_speeds` 纯函数 + worker 局部基线 + `per_process` 响应段）+ TD-50 `_meta.x-deprecated` hint + dispatch/既有测试调用点机械适配 + TD 批量回填（6 项关闭 + 清仓盘点总检）+ 本 Review + CHANGELOG 0.25.0 + Cargo bump + tag；业务 ~110 行 + 测试 ~230 行 + doc ~450 行。

### 4 维度审查

**代码质量** ✅：`compute_process_disk_speeds` 纯函数化（操作 `HashMap<u32, ProcessInfo>` 而非 SystemSnapshot）让单元测试合成数据零 sysinfo 依赖——测试数值确定性断言（`checked_sub` 构造精确 elapsed，差分除法精确到 assert_eq）；「只在 `Ok(true)` 时算 delta」复刻 TUI 的触发条件（pending tick cache 未变重算会把速度刷成 0——这是本 stage 最重要的正确性细节）；`per_process` 排序用 `process_cache()` 引用（省 `cached_processes_vec()` 全量 clone）。TD-50 测试升级：发现 `#[tool]` 宏生成 `pub fn {fn}_tool_attr() -> rmcp::model::Tool` 可直接调用——比 v0.17 源码 grep 静态断言更强的运行时 schema 断言。

**架构** ✅：delta 状态 worker 局部（`prev_disk`/`prev_disk_at`）不进 handler 字段——brainstorm 风险 2（持久字段并发与生命周期）的结构性规避（无锁竞争、无新鲜度问题——速度与 snapshot 同 tick 一致）；`source: "sysinfo-delta"` 字段履行 ADR D2 精度声明义务（agent 可判读 IO counters 含非磁盘 IO）。TD-53 改道方案与 ADR D2 终判一致（原 ETW 方案三理由否决——单实例互抢 + 非提权恒 None + 启动延迟）。

**性能** ✅：worker 每 heavy tick（~2s）一次 O(n) 遍历（n = 进程数）+ HashMap 重建，与 TUI `update_disk_speeds` 同量级（TUI 已在生产验证）；`per_process` 段仅在 tool call 时排序 truncate（top-N 小排序）；响应增量 ~10 条 JSON 对象在 8K chars 截断预算内。

**完整性** ✅：D4 验收锚逐项核对（见概览表）；TD 回填 6 项 + 总检段（open 收敛 8 项枚举完整）；agent 内部分发路径（dispatch.rs）同步适配（MCP schema 与 agent catalog 语义一致）；`test_mcp_v0_15.rs` 既有断言零改动（只补参数——tool 语义不变锚的回归面证据）。

---

## 4. 零挂机可比性论证（brainstorm 风险 4 口径声明）

本 cycle 无 eval 列，「不破既有画像」论证按预登记四轴：

1. **tool 语义不变原则**：TD-53 只加 `per_process` 响应字段（既有 `total/per_disk/disks` 字段含义与结构不变——回归测试锚定「既有字段不缺」）；TD-50 `x-deprecated` 是 schema hint 非删除（tool 46 不变锚运行时断言）；D1 空会话治理在 agent 行为面之外（session 落盘是观测侧非决策侧）。
2. **回归数字 + 不变锚**：+17 测试 0 failed 双档；MCP tool 46 / catalog 47 / deps +0 全程保持。
3. **queries.toml 70q 不涉新字段**：sparkline / per-process disk_io 无对应 query（queries 面向系统状态查询，`proc_metrics_disk_io` 既有字段已覆盖）；附录 A 启动时若需验证可加对照列（brainstorm 风险 4 mitigate 3 预登记）。
4. **行为契约测试锚**：三路径语料 / TD-24 状态机 / TD-40 regex / TD-53 delta + 响应 / TD-50 schema 共 17 例新增测试把「工具语义」从文档契约变成可回归断言。

**结论**：零挂机 cycle 的质量论证成立——不是「没测所以不知道」，而是「变更面被不变锚 + 契约测试双向夹紧」。

---

## 5. v0.26+ 候选方向评估（brainstorm 备注段预登记清单）

| 优先级 | 方向 | 现状评估（2026-08-30） | 建议 |
|---|---|---|---|
| 1 | **附录 A 模型升级 × RAG-on 复测组合**（v0.25 决策 1 第 3 次搁置留档） | 重启条件四项全未变：库存仍仅 E2B 两变体 / VRAM 5.9GB 空闲容 E4B / 无 key / llama.cpp 仍 b8685——**设计完整留档 brainstorm 附录 A，重启只需重评「下载意愿/磁盘/网络」**（决策 1 归档语境）；v0.25 语料卫生让未来 RAG 复测的主语料更干净（空会话停止增长） | 主线首选——条件变化（用户下载意愿 / key 到位 / llama.cpp 升级）即启动，QUICK 三判定先行 |
| 2 | **Multi-agent 协作（方向 D）** | 维持单 agent 上限先拉满判断（依赖候选 1 结果——E2B 上单 agent 画像 L2 full-chain 5% 是能力边界，多 agent 编排在同底座上预期放大而非缓解） | 排队候选 1 之后 |
| 3 | **MonitorManager 持久化（TD-51）** | feature 非 debt（brainstorm 不打包理由仍立）；monitors.toml 持久化需与 TUI 路径一致性评估 | 独立小主题可搭车（v0.26 若是轻 cycle） |
| 4 | **FilterExpr 跨 ctx（TD-31）/ replay 增强（TD-49）** | 单特性候选，各自 ~50 / ~1000 行级，与维护型定位不符 | v0.26+ 主题评估再定 |
| 5 | **本 cycle 新增文档候选** | `proc_smart` description "v0.18+" 文本过时（Findings P2 ②）——改字符串需连 v0_17 静态 grep 测试一起改 | 搭车项（任意下个 cycle 顺手） |

**TD 观察项存量**（触发条件未到，无行动）：TD-55~57（key）/ TD-58（flaky 未再现第 5 个 cycle）/ TD-59（轮转未触发）/ TD-61（llama-server 未升级——若候选 1 启动顺手并入）。

---

## 6. cycle 完结核对

- 3 stage 全 ✅（brainstorm 总览表唯一勾选点）；tag `v0.25.0`
- 打包清单三组全清（① 语料卫生 / ② 边角清仓 / ③ MCP 观测补全）；D3 终判表 12 项全部落地或显式关闭
- 容量检查：stage 2 ~400 行 / stage 3 ~790 行（含 Review doc）——远低于 1500 行铁律，无 Checkpoint 触发
- 手册执行：每 stage 独立会话 + 开工基线验证（回归双档 78/79 行核对 + 三件套）+ 完工报告 + 启动指令包——全流程兑现

---

## 勘误（2026-08-31，v0.26 拍板会话补记）

> 本 Review 头部基线声明「fmt / clippy（双档）/ build（双档）/ bench --no-run 全过」**在 tag v0.25.0 上不成立**：`tests/test_mcp_v0_25_stage_3.rs:24`（stage 3 收尾 commit `2297738` 引入）触发 clippy `field_reassign_with_default`，默认档 `cargo clippy --release --all-targets -- -D warnings` 退出码 101。定因：stage 3 会话 clippy 跑在该测试文件写入之前（过程 miss 非业务缺陷；工具链 1.95.0 未变，非 lint 漂移）。处置：v0.26 拍板会话（2026-08-31）按 clippy 自荐机械修复（struct 字面量替换，零行为变化，该二进制 8/8 复跑绿），修复随 `plan(v0.26)` commit 入库；流程改进（gate 脚本 + pre-push hook 机器门禁）立项为 v0.26 stage 2 主项——详见 [`docs/stages/v0.26-brainstorm.md`](../stages/v0.26-brainstorm.md) R2 段。
