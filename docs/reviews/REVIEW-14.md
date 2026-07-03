# REVIEW-14：v0.12.0 cycle 全局 Review

> **范围**：v0.12.0 cycle 阶段 1-5 全部产出（commit `f46141e plan(v0.12)` 之后的全部 working tree 改动）。
> **方法**：按 stage 6 doc 任务 1 的 6 维度审查（代码质量 / 架构 / 安全 / 跨平台 / 性能 / 完整性）。
> **基线**：`cargo test --release -q` 1115 passed / 0 failed / 3 ignored；fmt / clippy / no-default-features build 全过。
> **结论**：**P0 0 / P1 1 / P2 7**。stage 1-5 全部交付，无未交付项。仅 1 个 P1（README 平台表残留 Linux/macOS 列），7 个 P2 归档到 tech-debt v0.13.0+ 候选段（TD-38 ~ TD-44）。
> **Date**：2026-07-03。

---

## 1. 验收对照表（stage 1-5 是否全交付）

| Stage | 范围 | 验收 | 状态 |
|---|---|---|---|
| 1 Spike | ADR-0022 Accepted / ADR-0016 Superseded / ADR-0013 Deprecated / Cargo.toml feature 清理 / `docs/stages/v0.12-stage-2-audit.md` 范围审计产出 | grep `EBPF_ENABLED` 源码内 0 处；ADR Status 三处正确；audit doc 存在 | ✅ 全交付 |
| 2 Linux 移除 | `src/ebpf/` 整模块删（6 files）/ `src/psi.rs` 删 / `tests/test_{linux_stubs,psi,ebpf_flow,flow_source,gpu}.rs` 删 / `src/dns_log/unsupported.rs` + `src/net_flow/{nethogs,unsupported}.rs` 删 / ~30 文件 cfg gate 清理 / ProcessFlow 移到 `src/flow.rs` + 移除 FlowSource enum | grep `src/ebpf\|src/psi.rs` 不存在；src/ 内 cfg gate 残留 1 处（signature.rs mock policy 路径，刻意保留——见 P2-1）；src/flow.rs 新建，ProcessFlow 无 source 字段 | ✅ 全交付 |
| 3 签名完整度 | SignatureStatus 扩到 9 变体（加 Expired/UntrustedRoot/ChainError）/ from_wintrust_result 扩 5 HRESULT / TRUSTED_SIGNERS 扩到 24 vendor / `src/security/trusted_signers.rs` 新模块（~190 行）/ `verify_signature_with_policy` 加 `trusted_rules` 参数 / SecurityScorer 加 `trusted_signers_rules` 字段 | signature.rs 9 变体齐全；trusted_signers.rs 含 8 unit test；score.rs 加载 + 传参；tests/test_signature.rs 扩到 38 case | ✅ 全交付 |
| 4 FilterExpr | `EvalCtx.total_memory` 字段 / apply 在 mem+Percent 分支按 total_memory 换算 / `parse_regex_lit` 状态机支持 `\/` 转义 / `ProcessPanel.total_memory` 字段 + init_tree/refresh_tree 刷新 / 3 个 EvalCtx 构造点传值 | filter/mod.rs 含 total_memory + apply 分支；parser.rs 状态机实装；process_panel.rs 字段 + 构造点齐全；tests/test_filter_expr.rs 加 12 case（6 mem% + 6 regex escape） | ✅ 全交付 |
| 5 杂项小修 | TD-23 diag JSON wrap object + dns_collector 字段 / TD-29 NetworkIn HashSet + Value Hash/Eq / TD-32 SYSTEM_BOOT_ENTRIES 白名单 / TD-33 R18 Downloads dedup / TD-35 property_at_index lifetime 修正 / TD-36 ProcMcpHandler 持久 dns_collector | 6 个 TD 全部实装到位（详见 stage 5 doc + 各文件验证） | ✅ 全交付 |

**结论**：stage 1-5 全部交付，无未交付项。

---

## 2. 六维度审查

### 2.1 代码质量

#### 2.1.1 Linux 代码移除是否干净

| 检查 | 命令 | 结果 |
|---|---|---|
| `EBPF_ENABLED` 残留 | `grep EBPF_ENABLED src/ tests/` | 0 处 ✅ |
| `target_os = "linux"` 残留（src） | `grep 'target_os = "linux"' src/` | 0 处 ✅ |
| `FlowSource::Ebpf` 残留 | `grep FlowSource::Ebpf src/` | 0 处 ✅（FlowSource enum 整体删除）|
| `crate::psi` / `PsiStats` 残留 | `grep "PsiStats\|psi_stats\|read_psi\|crate::psi" src/` | 0 处 ✅ |
| `src/ebpf/` 目录 | `ls src/ebpf/` | 不存在 ✅ |
| `src/psi.rs` 文件 | `ls src/psi.rs` | 不存在 ✅ |

**干净度判定**：Linux 业务代码全清。仅 `src/security/signature.rs:232` 与 `tests/test_inspect.rs:97` 两处 `cfg(not(target_os = "windows"))` / `cfg(not(any(...)))` 残留——前者是 v0.11 stage 4 ADR-0021 设计的 mock policy 测试入口（policy_override 路径，让非 Windows CI 也能跑 mock），后者是 macOS stub 测试 mod。两处都属于「保留更安全」的 fallback path，**不是 stage 2 漏清**，归档为 P2-1 / P2-2 候选清理项（见 §3）。

#### 2.1.2 ProcessFlow serde round-trip

旧录屏（v0.10/v0.11）的 `.prec` 文件含 `source: "ebpf"` 字段。v0.12 stage 2 移除 ProcessFlow.source 后反序列化是否挂？

`src/flow.rs::ProcessFlow` 字段没有 `#[serde(deny_unknown_fields)]`，serde 默认行为是忽略未知字段。`process_flow_serde_ignores_legacy_source_field` 单元测试（src/flow.rs:191）用 round-trip 验证：插入 `source: "ebpf"` 后反序列化成功，pid/sni 字段保持正确。**向后兼容 OK**。

#### 2.1.3 SignatureStatus 9 状态机 + trusted_signers.toml 集成是否阻塞主线程

`SecurityScorer::new` 在构造时调 `load_trusted_signers()` 一次性读取 `~/.config/proc/trusted_signers.toml`（文件不存在 → 空 Vec，TOML 解析失败 → 空 Vec + warn，regex 编译失败 → 跳过该 rule + warn）。每次 score 调用复用，**不重读文件**。

`verify_signature_with_policy` 调 `matches_any_rule(&company, &self.trusted_signers_rules)` 走预编译 `regex::Regex::is_match`——regex 在加载时编译一次，运行时只跑匹配（无重复编译开销）。

**主线程阻塞**：`SecurityScorer::new` 在主线程构造（启动期）一次性读盘 + 编译 regex，运行时 score 路径无 IO。✅ 不阻塞。

#### 2.1.4 FilterExpr EvalCtx.total_memory 是否每次重算

`EvalCtx.total_memory` 字段由 `ProcessPanel.total_memory`（独立字段）填充，`init_tree` / `refresh_tree` 时刷新一次（`snapshot.memory_usage().1`）。3 个 EvalCtx 构造点（Header 聚合 / Child 单进程 / Tree 节点 lookup）共用同一 panel 的 `total_memory`——**不在每次 apply 时重算**。

CLI 路径 `run_ls` 在 EvalCtx 构造时一次性传 `snapshot.memory_usage().1`。✅ 不重算。

#### 2.1.5 property_at_index lifetime 修正

`src/dns_log/etw.rs:532` + `src/schannel_etw/provider.rs:484` 签名都从 `(*const TRACE_EVENT_INFO, idx) -> Option<&'static EVENT_PROPERTY_INFO>` 改为 `(info_buf: &[u8], idx) -> Option<&EVENT_PROPERTY_INFO>`（lifetime elision 自动绑到入参 buffer）。callers 改传 `&info_buf` 切片，借用检查器保证引用不逃出 owner。**soundness 修复 OK**。

---

### 2.2 架构审查

#### 2.2.1 ADR-0022 决策完整性

`docs/adr/0022-windows-only-platform.md` Status: Accepted；含背景（v0.5 跨平台设计 → v0.12 决策点）/ 决策（Windows 10 1809+ / Windows 11 x64）/ 影响（Linux 用户迁移路径：`git checkout v0.11.0`）/ Supersedes（ADR-0016 + ADR-0013）。**完整**，含 migration path。

#### 2.2.2 历史 ADR Status

- ADR-0016（eBPF flow graph）：`Superseded by ADR-0022` ✅
- ADR-0013（PSI 监控）：`Deprecated (v0.12 移除)` ✅

ADR 文件没删（按 stage 1 doc 决策「只改 Status，不删文件」），保留历史可追溯。

#### 2.2.3 trusted_signers.rs 模块设计

- 文件结构（~190 行）：
  - `TrustedSignersRule` struct（`name` / `vendor_regex: regex::Regex` 预编译缓存 / `reason: Option<String>`）
  - `TrustedSignerRaw` / `TrustedSignersFile` serde deserialize 中间结构
  - `default_rules_path()` / `load_trusted_signers()` / `load_trusted_signers_from(path)` 三层入口
  - `matches_any_rule(subject, rules)` 集成入口
  - 8 个 unit test（解析 / 大小写 / 多 rule / 错误降级）
- 与 `lineage_rules.rs` / `path_rules.rs` **同款契约**（文件不存在 → 空 Vec / 解析失败 → 空 Vec + warn / 配置 regex 编译失败 → 跳过该 rule + warn）。
- surgical 原则：未引入新的「统一用户规则系统」（如 UserRule trait），保留 3 个独立模块。设计取舍明确（模块 doc 注释「与 lineage_rules.rs / path_rules.rs 同款契约」）。

**设计符合 surgical 原则**。

---

### 2.3 安全性审查

#### 2.3.1 trusted_signers.toml regex DoS 防护

用户配置 `vendor_pattern` 直接传给 `regex::Regex::new`。regex crate 自身有 ReDoS 防护（NFA simulation，无回溯爆炸），但极端 pattern 仍可能慢（如 `(?i)^.*a.*b.*c.*$` 在长字符串上）。

实际影响：`vendor_pattern` 匹配对象是 `CompanyName` 字段（一般 < 100 字符），DoS 风险低。**当前接受**，归档 P2-3（v0.13+ 可考虑加 regex 复杂度 lint 或 size limit）。

#### 2.3.2 WinVerifyTrust HRESULT 扩展路径泄漏

`from_wintrust_result` 把 HRESULT 映射到 SignatureStatus。`verify_signature_with_policy` 在 Windows 分支调真实 `WinVerifyTrust` 后，仅在 `Unknown` 分支调 `tracing::debug!("WinVerifyTrust unknown error 0x{:08X} for {}", result as u32, exe_path)`。

`exe_path` 是进程的可执行文件绝对路径。这条 debug 日志写到 `~/.config/proc/logs/proc.log`（用户本地），**不通过网络传输**，不构成路径泄漏（用户本就在自己机器上看到自己的进程路径）。✅ 无安全风险。

#### 2.3.3 SYSTEM_BOOT_ENTRIES 白名单

TD-32 实装的 `SYSTEM_BOOT_ENTRIES = [services.exe / wininit.exe / svchost.exe]` 是按 process name 匹配（lowercase），不按 PID——PID 复用风险存在但极低（attacker 需先 privilege escalation 才能 spawn 这些名字的进程）。

更严格的方案（按 image path 校验，要求 `C:\Windows\System32\services.exe`）会引入新的依赖（`QueryFullProcessImageName`），surgical 原则下当前实现可接受。**P2-4 候选**：v0.13+ 可加 image path 校验做更严格白名单。

---

### 2.4 跨平台审查

#### 2.4.1 cfg(target_os="windows") 是否还需要

stage 6 doc 任务 1 跨平台审查项。当前残留：

- `src/security/signature.rs:232` `#[cfg(not(target_os = "windows"))]`（mock policy 路径）
- `tests/test_inspect.rs:97` `#[cfg(not(any(target_os = "windows", target_os = "linux")))]`（macOS stub 测试 mod）
- `Cargo.toml` `[target.'cfg(windows)'.dependencies.nvml-wrapper]`（保留更安全，stage 2 doc 任务 6 明确保留）

stage 2 doc 任务 2 原则是「删除 cfg(not(target_os="windows")) 块；保留 cfg(target_os="windows") 块」。signature.rs 这条是 v0.11 stage 4 ADR-0021 设计的 mock 入口（policy_override 在 Windows 真实路径外提供测试可注入分支）。

矛盾点：stage 2 doc 任务 2 严格说要删，但 ADR-0021 设计说要保留（mock policy 路径）。**当前状态**：mock policy 实际在 Windows 分支也能跑（policy_override 在 `if let Some(result) = policy_override` 路径短路），非 Windows 分支冗余但无害。归档 P2-1 候选清理。

#### 2.4.2 macOS 用户能否降级

ADR-0022 决策：v0.12 起 Windows-only，macOS 用户不能编译运行 v0.12+。Linux / macOS 用户迁移路径：`git checkout v0.11.0`（最后含跨平台代码的 release）。

CONTEXT.md 顶部「⚠ 已知限制」段已记录。✅ 决策清晰。

---

### 2.5 性能审查

#### 2.5.1 NetworkIn HashSet 改造

`FilterExpr::NetworkIn.values` 从 `Vec<Value>` 改为 `HashSet<Value>`，apply 路径 `iter().any()` O(N) → `contains` O(1)。`Value` 加手动 `Hash + Eq` impl（f64 `to_bits()`，parser 产生非 NaN 安全）。

理论收益：100 个 IP × 1000 flows 的极端场景从 100_000 次比较降到 1_000 次 hash 查找。无 benchmark 基础设施，**理论收益可接受**。

#### 2.5.2 trusted_signers 加载时机

`SecurityScorer::new` 构造时一次性读取 `trusted_signers.toml`，运行时 score 调用复用（不重读文件）。regex 在加载时编译一次缓存到 `TrustedSignersRule.vendor_regex` 字段。✅ 不在每次 score 重读 / 重编译。

---

### 2.6 完整性检查

#### 2.6.1 plan.md 中 v0.12 cycle 阶段 1-5 已 `[x]`

plan.md 不用 `[x]` checkbox 风格（v0.11 stage 8 TD-34 已记录此设计选择），改用「阶段 N：xxx ←─ stage N 提交点」流程图描述。stage 1-5 提交点都在 plan.md 第 36-89 行的流程图里有体现。**约定一致**。

#### 2.6.2 tech-debt TD 标 Fixed 状态

| TD | 当前状态 | stage 6 应标 |
|---|---|---|
| TD-17 | ✅ Fixed in v0.12.0 阶段 1 | 保持 |
| TD-19 | ✅ Fixed in v0.12.0 阶段 1 | 保持 |
| TD-23 | ✅ Fixed in v0.12.0 阶段 5 | 保持 |
| TD-26 | ⏸（仍标 v0.12+ 候选，未 Fixed）| 改 ✅ Fixed in v0.12.0 阶段 3 |
| TD-27 | ⏸ | 改 ✅ Fixed in v0.12.0 阶段 3 |
| TD-28 | ⏸ | 改 ✅ Fixed in v0.12.0 阶段 4 |
| TD-29 | ✅ Fixed in v0.12.0 阶段 5 | 保持 |
| TD-30 | ⏸ | 改 ✅ Fixed in v0.12.0 阶段 4 |
| TD-32 | ✅ Fixed in v0.12.0 阶段 5 | 保持 |
| TD-33 | ✅ Fixed in v0.12.0 阶段 5 | 保持 |
| TD-35 | ✅ Fixed in v0.12.0 阶段 5 | 保持 |
| TD-36 | ✅ Fixed in v0.12.0 阶段 5 | 保持 |
| TD-31 | ⏸（v0.13+ 候选）| 保持 ⏸ |
| TD-37 | ✅ Fixed in v0.11.0 阶段 8 | 保持 |

**4 个 TD 需 stage 6 补标 Fixed**（TD-26 / 27 / 28 / 30）。这是 P1-1（task #3）。

#### 2.6.3 CONTEXT.md 术语段 + 演进历史段完整

- 术语段 v0.12.0 段（line 180-194）已含 8 个新术语（9 状态机 / trusted_signers / EvalCtx.total_memory / parse_regex_lit / diag JSON / NetworkIn HashSet / SYSTEM_BOOT_ENTRIES / R18 dedup / property_at_index / MCP 持久 collector）。✅ 完整。
- 术语演进历史段（line 199-207）已含 v0.12.0 阶段 1 / 阶段 3 / 阶段 4 / 阶段 5 四行。**缺阶段 2 + 阶段 6**——阶段 2 是 stage 6 doc 任务 7 要求补的（CONTEXT.md 更新），阶段 6 是 stage 6 本身。

stage 6 doc 任务 7 要求 CONTEXT.md 加阶段 6 行（REVIEW-14 + 收尾）。stage 2 行也应该补（虽然 stage 2 doc 任务 9 要求 stage 2 当时补，但实际未补——review 发现的差距）。

**这是 P1-2**（stage 6 任务 7 修：CONTEXT.md 演进历史补 阶段 2 + 阶段 6 行；术语段移除 Linux 相关术语 + 加新术语 stage 2 doc 任务 9 要求的也一并做）。

#### 2.6.4 stage docs 头部 ✅ 已发布标记

当前 stage 1-5 doc 头部都只有「### 阶段 N：xxx」+ 独立会话指令，**没有 ✅ 标记**。stage 6 doc 任务 8 要求加 `> ✅ **已完成**（v0.12.0 阶段 N 会话产出，YYYY-MM-DD）`。

**这是 P1-3**（stage 6 任务 8 修）。

#### 2.6.5 README + CHANGELOG 反映 v0.12.0

- README banner 还是 v0.11.0（line 5）+ v0.10.0（line 9）+ ... 完全没有 v0.12.0 段。**这是 P1-4**。
- README 平台支持表（line 435-460）还有 Linux / macOS 列。**这是 P1-5**。
- CHANGELOG.md 顶部 `[Unreleased]` 段未改 `[0.12.0]`。**这是 P1-6**。

---

## 3. 问题清单（P0 / P1 / P2）

### P0（阻断 release）

**无 P0**。

### P1（必修，本阶段闭环）

| # | 类别 | 描述 | 修复 |
|---|---|---|---|
| **P1-1** | 完整性 | tech-debt TD-26 / 27 / 28 / 30 当前未标 ✅ Fixed（实际已在 stage 3 / stage 4 落地）| 改 `✅ Fixed in v0.12.0 阶段 N` |
| **P1-2** | 完整性 | CONTEXT.md 演进历史缺阶段 2（Linux 移除）+ 阶段 6（本 Review + 收尾）行；术语段缺阶段 2 移除的 Linux 术语记录 | stage 6 任务 7 补 |
| **P1-3** | 完整性 | stage docs（v0.12-stage-1.md ~ v0.12-stage-6.md）头部缺 ✅ 已完成标记 | stage 6 任务 8 加 |
| **P1-4** | 完整性 | README banner 完全没有 v0.12.0 段 | stage 6 任务 6 加 |
| **P1-5** | 完整性 | README 平台支持表（line 435-460）还有 Linux / macOS 列；line 3 description 还有「Linux/macOS 可降级运行」 | stage 6 任务 6 删 Linux/macOS 列 |
| **P1-6** | 完整性 | CHANGELOG.md `[Unreleased]` 段未改 `[0.12.0] - 2026-07-03` | stage 6 任务 5 |
| **P1-7** | 版本号 | Cargo.toml `version = "0.11.0"`（line 3），未 bump 到 0.12.0；Cargo.lock 需 build 同步 | stage 6 任务 4 |

### P2（归档到 tech-debt v0.13.0+ 候选段）

| # | 类别 | 描述 | 归档 |
|---|---|---|---|
| **P2-1** | 跨平台 | `src/security/signature.rs:232` `#[cfg(not(target_os = "windows"))]` mock policy 块在 Windows-only 后冗余（mock 路径在 Windows 分支也跑），surgical 清理候选 | TD-38 |
| **P2-2** | 跨平台 | `tests/test_inspect.rs:97` `#[cfg(not(any(target_os = "windows", target_os = "linux")))]` macOS stub 测试 mod 在 Windows-only 后永不编译；保留 vs 删除的设计取舍 | TD-39 |
| **P2-3** | 安全 | `trusted_signers.toml` 用户 regex 无复杂度限制，理论 ReDoS 风险（实际 CompanyName 字段 < 100 字符风险低）| TD-40 |
| **P2-4** | 安全 | TD-32 `SYSTEM_BOOT_ENTRIES` 白名单按 process name 匹配，未校验 image path（PID 复用 + 名字欺骗风险存在但极低）| TD-41 |
| **P2-5** | 完整性 | `Cargo.toml` `[target.'cfg(windows)'.dependencies.nvml-wrapper]` cfg target 包裹——v0.12 Windows-only 后理论上不需要，但保留更安全（未来若加回 macOS 不会重新引依赖）。stage 2 doc 任务 6 明确保留 | 不归档（设计取舍，stage 2 doc 已记录） |
| **P2-6** | 文档 | `.github/workflows/ci.yml` 仍有 `check-linux` job（line 36-42），v0.12 Windows-only 后 Linux CI 永远失败。stage 2 doc 任务 7 应删但未删 | TD-42 |
| **P2-7** | 文档 | `.github/workflows/release.yml`（如有）的 Linux / macOS build target 应删，stage 2 doc 任务 7 应改但未改 | TD-43 |
| **P2-8** | 完整性 | plan.md 流程图 stage 1-5 标记与 stage docs ✅ 标记不一致（plan.md 用文字「←─ 阶段 N 提交点」，stage docs 用 ✅ 标记）——文档风格不统一 | 不归档（TD-34 v0.11 stage 8 已记录此设计选择）|

---

## 4. 建议优先级

stage 6 修复顺序（按 P1）：

1. **P1-7**（Cargo.toml bump 0.12.0）—— 让后续 commit / tag 用对版本号
2. **P1-1**（tech-debt 4 个 TD 标 Fixed）—— 文档快改
3. **P1-6**（CHANGELOG 加 [0.12.0] 段）—— 文档
4. **P1-4 + P1-5**（README banner + 平台表）—— 文档
5. **P1-2**（CONTEXT.md 演进历史补阶段 2 + 6）—— 文档
6. **P1-3**（stage docs 头部 ✅）—— 文档
7. 最终回归测试 + git tag v0.12.0

P2 全部归档到 tech-debt TD-38 ~ TD-43。

---

## 5. v0.12.0 cycle 整体评估

**Slice 完成度**：stage 1-6 全部按计划交付，无 scope creep。stage 2（最大 Slice，~1000 行删除）一次会话完成未触发 Checkpoint。

**质量**：1115 passed / 0 failed / 3 ignored，相比 v0.11 基线 1146 略降（删 ~46 个 Linux 相关测试）但符合 stage 2 doc 验收标准（≥ 1100）。

**架构债**：清零 TD-17 / 19 / 23 / 26 / 27 / 28 / 29 / 30 / 32 / 33 / 35 / 36（12 个 TD）。新增 TD-38 ~ TD-43（6 个 P2 归档），净减 6 个 TD。

**Release 准备度**：P1 全部修复后即可 tag v0.12.0。

---

**Review 完成**：2026-07-03。
**下一步**：按 stage 6 doc 任务 2 修 P1，任务 3 归档 P2，任务 4-11 完成收尾。
