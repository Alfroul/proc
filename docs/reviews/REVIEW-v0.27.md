# REVIEW-v0.27 — v0.27 cycle Review（CI 修复专项完结）

> **cycle 范围**：brainstorm 5 决策（2026-09-02 拍板「按建议全部做」）——决策 1 CI 修复专项定稿 / 决策 2 附录 A 第 5 次搁置（秋招窗口期，2026-11+ 重评）/ 决策 3 rmcp RUSTSEC-2026-0189 选 audit-ignore + TD-63 追踪 major / 决策 4 搭车幽灵引用 8 处 + TD-62 观察 + NT SAFETY 不并入 / 决策 5 3 stage 节奏（push / tag 用户拍板）；1 份 ADR-0037（D1~D5）
>
> **Review 范围**：3 stage 全部产出（Spike 取证 / 修复主项 / 远端验证与收尾——本 Review 同会话）
>
> **基线与终值**：1744 / 0 / 10 + 1768 / 0 / 11（双档各 80 行核对，v0.26.0 基线 → v0.27.0 终值**零变化**——env-gate 仅 CI=true 生效本地不设 + clippy 机械改写语义零变化 + ETW gate 仅 CI=true 跳过）/ fmt / clippy 双档（1.95）/ build 双档 / bench --no-run 全过；**新增 1.98 工具链 clippy --all-targets 绿**（远端 stable 同款预验）
>
> **Review 日期**：2026-09-04（stage 3 会话，tag 同日）
>
> **Reviewer**：Claude（stage 3 会话；审查依据 brainstorm 验证矩阵「远端 CI 绿」核心锚 + ADR-0037 验收锚 + 各 stage doc 验证记录，按数据/测试锚审不自评——REVIEW-v0.26 同款口径）
>
> **零挂机声明**：本 cycle 无 eval 列——质量论证四轴：① 不变锚（tool 46 / catalog 47 / runtime deps +0）② 回归双档数字（开工/终值一致）③ **远端 CI 状态机证据链**（gh API run/job/step 三级留档，4 轮迭代每轮有根因+修复+验证）④ 本 Review 溯源抽查（16 项 > 10 门槛）。

---

## 概览

v0.27 是 CI 修复专项（轻-中 cycle，workflow/依赖/小 rs 为主）——**「远端 CI 历史 18/18 全红 + check job 每次 push 6h 挂死，到首次全绿 + badge 实挂 + required checks 兑现」**。ADR-0037 五组决策（D1 测试策略 / D2 msrv 双修 / D3 check-macos 移除 / D4 audit 三修 / D5 miri+winget 声明性降级）全部实装并在远端验证。

**核心验收表**（本 Review 独立复核，非自评转写）：

| 验收项 | 口径 | 实测（2026-09-04） | 结论 |
|---|---|---|---|
| **远端 CI 全绿**（cycle 核心锚） | brainstorm 验证矩阵「远端 CI 绿」 | run `33867317029` conclusion=**success**（check ✅ / msrv ✅ / audit ✅，11:18:27–12:16:46 UTC ~58 min）——**ci.yml 历史 18/18 failure 后首次全绿** | ✅ |
| **badge 实挂**（brainstorm 组⑥） | URL 200 + SVG 内容显示 passing | `curl` badge.svg → HTTP 200 + `<title>CI - passing</title>`；README :7-10 注释符删除（占位语法零重写兑现） | ✅ |
| 回归双档终值 | 与开工基线一致（80 行核对） | **1744 / 0 / 10 + 1768 / 0 / 11**（终值会话实测；默认档首跑 TD-62 第 3 次触发，单跑绿 + 复跑绿按口径归档） | ✅ |
| 不变锚（全程） | tool 46 / catalog 47 / runtime deps +0 | grep tool 46 本 Review 复核；catalog.rs:1 47（46 MCP + proc_help）；Cargo.toml 仅 rust-version + [lints] + version bump | ✅ |
| 6h 挂死止损 | timeout 兜底确定性失败 | R2 校准 180/170 + cache-on-failure：挂点从「6h 无输出取消」变「确定性失败或完成」——R3/R4 实证（R3 ~70 min 真失败带日志，R4 ~58 min 完成） | ✅ |
| msrv / audit / 平台矩阵 | D2 / D4 / D3 | msrv windows+1.88 两轮绿（run 33827454883 + 33861156088+）；audit permissions+ignore+crossbeam 三轮绿（6 warnings 均为 warning 级 advisory）；check-macos 移除 + ADR-0022 补完注记 | ✅ |

**远端验证 4 轮迭代链**（stage 3 预算内，每轮根因-修复-验证留档 stage-3 doc「远端验证记录」段）：

| 轮 | run | 结果 | 根因 | 修复 |
|---|---|---|---|---|
| R1 | `33827454883` | msrv ✅ audit ✅ check ❌（clippy 步） | 远端 stable clippy **1.98 新 lint** `chunks_exact_to_as_chunks`（本地 1.95 不报） | env.rs:170 单点 `as_chunks::<2>()` 机械改写（1.88 稳定 API；本地装 1.98 复现+修后全 target 绿；commit `575ed91`） |
| R2 | `33829331943` | clippy ✅ build×2 ✅ 但 **60 min job 上限取消** | test 步冷缓存先编译 proc 自身 79 个 test target（43.5 min 未完，**零测试执行**）——D1 预算「build ~10+test ~45」实测失真 | timeout 实测校准 60→180 / 45→170 + rust-cache `cache-on-failure: true`（本地 `cargo clean -p proc` 实测 949s 定标；commit `656c5dc`） |
| R3 | `33861156088` | timeout 生效不再取消，**首个真实测试失败** | `test_disk_io_etw::try_spawn_returns_some_on_admin_or_none_on_user` 硬时序断言——stage-1 附录 **B.3 误分类**（runner 提权 ETW 真启动走真实路径，D1.2 同族） | 仅 gate 该测试（`ci_env()` 同款模式）；宽松断言的 `spawn_collects_self_io_when_admin` 不 gate 保留 runner 冒烟信号（commit `1dc0043`） |
| R4 | `33867317029` | **全绿** ✅ | — | — |

**Findings 汇总**：P0 0 / P1 0 / P2 2（① stage 2 README 亮点条 TD 计数漏计——同 commit 新立 TD-63 但只写 61→62，本 Review 复核 `grep -c "^### TD-"` = 63 抓出，收尾 commit 修正为 63；② release badge 时点语义——release.yml 仅 tag 触发，badge 反映最近一次 run，v0.26.0 的 winget 红 history 在 v0.27.0 tag run 绿后覆盖，若观感异常需核实 badge 查询参数）。**过程注记（P3）**：Contributors 面板出现 Anthropic `claude` 账号（130 个历史 commit 带 co-author 尾注被新版 UI 计入）——用户拍板：Alfroul 身份确认无误（121 direct 归因正确）、历史不动、后续 commit 不带尾注（未 push 的 3 个 commit filter-branch 清理后 push，tree 零变化）；actions Node 20 deprecation warnings（checkout@v4）为 cosmetic，升 action 大版本留 v0.28+ 小项池。TD-62 第 3 次触发（终值会话首跑红 → 单跑绿 + 复跑绿，非回归）维持观察不修。

---

## 1. stage 1 Spike：取证补全 + ADR-0037 定稿（commit `8819ae2` 段 / 原 `6abdb85`）

**落地范围**：CI 历史 18/14/15 run 逐 job 取证（三项修正性发现：从未绿过 / 挂死自 07-01 已存在 / winget 首跑即 404）+ 80 binary 环境依赖五类分级 + msrv 16 包四链权威清单 + macOS B 路径实证否决（71/74 非 feature 相关）+ rmcp 暴露面评估 + miri/winget 移除依据 + ADR-0037 D1~D5。零业务代码。

**4 维度审查**：**代码质量** N/A（零代码）；**架构** ✅——D1~D5 全部决策有取证链支撑且 stage 2 实装时零返工（B.3 一处误分类由 stage 3 远端实证修正，属「本地无法复现」既知风险的预案内迭代，非设计缺陷）；**性能** ✅——附录 B 分级直接指导 gate 面最小化（choke-point 单点 vs 逐测试）；**完整性** ✅——附录 A-F 全部被 stage 2/3 消费（16 包清单→B 方案依据 / B.2→gate 名单 / B.4/B.5→观察项），无自创数字（本 Review §5 抽查）。

## 2. stage 2 Slice：修复主项落地（commit `398731e` 段 / 原 `1f7fe39`）

**落地范围**：choke-point env-gate（signature.rs CI=true→Unknown）+ ETW 两文件 per-test gate + timeout 60/45 + 删 check-macos + msrv 双修（windows runner + rust-version 1.88 + 步骤修正注记）+ audit 三修 + miri.yml 删 + winget job 删 + README 口径 + 幽灵引用 8 处清零 + TD-63 新立。

**4 维度审查**：**代码质量** ✅——env-gate 单点设计经受远端验证（R3/R4 全量 1744 在 runner 零 WinVerifyTrust 挂点复现）；两项实施发现（let-chain 37 处 / --all-targets 编不过测试目标）均按手册边界处置（[lints] 过渡 + 步骤修正 + ADR 注记）而非扩面；**架构** ✅——D1.4「全量不裁剪」在 R4 兑现（远端与本地同口径 1744）；**性能** ✅——timeout 初值 60/45 依据「build ~10 min」估算偏乐观，由 stage 3 实测校准（949s 定标）——估算-实测-校准链完整；**完整性** ✅——回归双档与开工基线一致 + 本地 CI env 为空的零激活前提实证。

## 3. stage 3 Slice + Review：远端验证 4 轮迭代 + 收尾（本 commit）

**落地范围**：push 拍板执行（3 commit 尾注清理后 push）+ R1-R4 观察-修复循环（2 处代码修复 + 1 处 workflow 校准 + 1 处测试 gate）+ badge 激活（URL 200 + SVG passing 实证）+ required checks 设置说明兑现（3 job 名单更新）+ REVIEW + CHANGELOG 0.27.0 + Cargo bump + tag + v0.28+ 候选评估。

**4 维度审查**：**代码质量** ✅——两处修复均为最小面（单点 lint 改写 / 单测试 gate），每处有本地复现-修复-验证闭环（1.98 工具链 rsproxy 19s 装成复现远端同款；CI=true 模拟 gate 触发实证）；**架构** ✅——timeout 校准用本地实测（cargo clean -p proc 949s）推算 runner 带宽而非拍脑袋，`--tests` 等价性验证（79 executable 同集）后才决定命令零改动；**性能** ✅——R4 暖缓存全绿 58 min（前置 11 min + 执行 47 min），后续 push 稳态时长可预期；**完整性** ✅——B.5 时序断言类（perf_baseline / search_perf / scorer sleep）与 kill_by_name 降级路径在 R4 全量跑过**零触发**（stage-1 观察项全部通过，无需归档 TD）。

---

## 5. 数字溯源抽查详表（≥ 10 门槛，实抽 16）

| # | 数字（出处） | 来源核对 | 判定 |
|---|---|---|---|
| 1 | ci.yml 历史 18/18 failure（stage-1 附录 A.1） | stage-1 gh API 取证留档 + 本 cycle 4 run 追加（18→22 run，R4 首绿） | ✅ |
| 2 | R1 `33827454883`：msrv/audit ✅ + check clippy ❌ | run jobs API + --log-failed（chunks_exact_to_as_chunks @ env.rs:170:10） | ✅ |
| 3 | clippy lint 名与位置 | R1 失败日志原文 vs commit `575ed91` diff（env.rs:170 唯一挂点，全仓 grep 佐证） | ✅ |
| 4 | R2 `33829331943` 60 min 取消 + test 步 43.5 min 零执行 | job log 时间线（02:40:00 步起 / 02:40:50 Compiling proc 后零输出 / 03:23:27 cancel） | ✅ |
| 5 | 本地 proc test-target 冷编译 **949s** | 本会话 `cargo clean -p proc` + `cargo test --release --no-run` 计时（推算 runner 32-63 min 的定标依据） | ✅ |
| 6 | timeout 180/170 + cache-on-failure | ci.yml diff（commit `656c5dc`）；R3/R4 生效实证 | ✅ |
| 7 | R3 `33861156088` ~70 min 失败 + disk_io_etw:44 left 5 right 0 | run API + --log-failed 原文 | ✅ |
| 8 | gate 仅 1 测试（spawn_collects 不 gate） | commit `1dc0043` diff + CI=true 模拟 SKIP 输出实证 | ✅ |
| 9 | R4 `33867317029` success ~58 min（11:18:27–12:16:46） | run API conclusion/jobs/updatedAt 三字段 | ✅ |
| 10 | badge HTTP 200 + `CI - passing` | curl badge.svg `<title>` 提取（R4 前为 failing 对照） | ✅ |
| 11 | 回归双档终值 1744/0/10 + 1768/0/11（各 80 行） | 本会话 tee 落盘 awk 聚合 + 行数核对（TD-62 第 3 次触发单跑绿+复跑绿） | ✅ |
| 12 | MCP tool 46 / catalog 47 | `grep "name = \"proc_"` = 46；catalog.rs:1 注释 47（46+proc_help）+ 回归绿背书 | ✅ |
| 13 | README 亮点条 65,114 / 27,518 / 80 binaries / 27 tags / 37 ADR / 63 TD | 本会话实测：`find src -name '*.rs' \| xargs wc -l` / tests 同 / `git tag \| wc -l`（26+v0.27.0）/ ls docs/adr（38 文件含 README 索引）/ `grep -c "^### TD-"` | ✅（63 处为 Findings ① 修正值） |
| 14 | msrv 1.88 双修实证 | `cargo +1.88.0 check --no-default-features` 34.6s（stage 2）+ 远端 msrv job 三轮绿 | ✅ |
| 15 | audit 6 warnings 不挡 | R1/R3/R4 audit job conclusion=success + 注记（warning 级 advisory：unmaintained/unsound/yanked） | ✅ |
| 16 | release 分发链 | release.yml 单 build job（winget 已删）+ [package.metadata.binstall] 在位（stage-2 验证）；v0.27.0 tag run 绿 + draft release 待用户 publish | ✅ |

**判定**：16 项抽对全部可回溯（gh API run/job/step 三级 + 本地命令复跑 + commit diff），零自创数字；1 项上游漏计（stage 2 的 TD 62 计数）被本 Review 复核抓出修正——REVIEW-v0.26「自查+独立抽查双层制」延续有效。

## 6. v0.28+ 候选方向评估（惯例段）

| 优先级 | 方向 | 现状评估（2026-09-04） | 建议 |
|---|---|---|---|
| 1 | **附录 A 模型组合重启** | 第 5 次搁置（v0.27 决策 2）；秋招窗口 2026-11+ 自然重评；底座比 v0.26 更干净（CI 绿 + badge 实挂） | 2026-11+ 重评（重启纪律不变：下载意愿/磁盘/网络） |
| 2 | **TD-63 rmcp 0.11→≥1.4 major** | RUSTSEC-2026-0189 ignore 在位（stdio 不受影响 + SSE 三条件窄暴露 + confirm 契约缓解，ADR-0037 D4/附录 E）；46 MCP tool 底座 breaking 升级 | 独立 cycle 候选（与 Multi-agent 都动 agent 栈，二选一先评估） |
| 3 | **Multi-agent 协作（方向 D）** | 依赖附录 A 结果（E2B 单 agent L2 5% 边界未变） | 排队附录 A 之后 |
| 4 | **let-chain 现代化 37 处** | stage 2 [lints] collapsible_if=allow 过渡 + TD 归档 v0.28+——1.88 已解锁，纯机械重构 + 回归双档背书 | 轻 cycle 候选 / 搭车项 |
| 5 | **TD-51 / TD-31 / TD-49** | MonitorManager 持久化（~100-300 行）/ FilterExpr 跨 ctx（~50 行）/ replay 增强（~1000 行级） | 单特性候选池 |
| 6 | **NT API 层 SAFETY 93 处** | unsafe-audit §4 自评独立专项（「注释写错比没有更伤」） | 专项候选（不搭车） |
| 7 | 小项池 | actions/checkout@v5（Node 20 deprecation）/ TD-62（第 3 次触发，复现再修）/ TD-55~57（key，第 9 个 cycle）/ TD-58 / TD-59 / TD-61 | 搭车项池 |

## 7. cycle 完结核对

- 3 stage 全 ✅（brainstorm 总览表唯一勾选点）；tag `v0.27.0`（用户拍板同日）；push 6 commits + tag
- 打包清单：主项六组中①②③④⑤ 全在 stage 2 落地、⑥ badge+required checks 在 stage 3 兑现；搭车（幽灵引用 8 处 / TD-63 / TD 台账）全清
- 远端状态：ci.yml 全绿（R4）+ badge passing + release.yml 待 tag 触发验证（build job 历史 10/10 绿，winget job 已删）
- 手册执行：每 stage 独立会话 + 开工基线（回归双档 + 三件套，首跑红按 TD-62 口径处置）+ 完工报告 + 启动指令包——全流程兑现；push（2 次：初次 3 commit + 迭代随修随推）与 tag 均用户拍板
- required checks：设置说明更新为 3 job（check / msrv / audit）落 stage-3 doc——仓库手动项，用户按需设置（Claude 无法代办）

## License

MIT（仓库根目录 [`LICENSE`](../../LICENSE) 文件）
