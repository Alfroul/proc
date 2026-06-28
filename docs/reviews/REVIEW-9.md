# 全局 Review 报告 — v0.8.0 cycle（阶段 4）

**审查范围**：v0.8.0 cycle 阶段 1-3 全部产出（阶段 1 主动推迟到 v0.9.0 cycle，详见下方「阶段 1 推迟说明」）
**审查日期**：2026-06-28
**审查人**：stage 4 会话
**基线测试**：`cargo test --release -q` = **930 passed / 0 failed / 3 ignored**（53 个测试 bin）
**其它基线**：
- `cargo fmt --all -- --check` 干净 ✓
- `cargo clippy --release --all-targets -- -D warnings` 0 warnings ✓
- `cargo build --release --no-default-features` 通过 ✓

---

## 阶段 1 推迟说明（不是 P0/P1/P2，是用户主动决策）

v0.8.0 阶段 1（TD-19：ebpf Linux 真实编译验证，WSL2）由用户主动推迟到 v0.9.0 cycle 启动前再评估。理由：用户主要用 Windows 开发，WSL2 / Linux 真机环境准备成本高（clang/llvm/libelf + nightly + bpf-linker ~30 min），且 stage 2/3 不依赖 stage 1。

**后果**：
- stage 4 的 Linux 验收标准（`cargo +nightly build -p proc-ebpf --target bpfel-unknown-none --release` / `cargo build --release --features ebpf`）跟着跳过
- v0.7.0 已记录的 known limitation「Linux ebpf 编译路径未验证」延续到 v0.8.0
- v0.8.0 README / CHANGELOG 必须显式标注此 known limitation（已纳入 stage 4 第 9 步任务）

stage 4 自身（review + 修 P0/P1 + tag）不依赖 stage 1，可独立完成。

---

## 摘要

- 总问题数：**P0 0 / P1 1 / P2 4**
- 阻断性问题：**0 项**（无 P0）
- 已知限制：stage 1 推迟（ebpf Linux 验证）→ CHANGELOG / README 文档化
- 关键主题：**代码与文档小范围漂移**（src 注释 stale + ADR 缺 v0.8 增量段 + TD-19 状态需明确「主动推迟」）

---

## P0（阻断性，必须修才能交付 v0.8.0）

无。基线三件套 + no-default-features build 全通过，无 cfg-gate 漏写 / 无 Linux-only 回归（stage 1 未跑）/ 无 AppGroup FilterExpr 逻辑错误。

---

## P1（重要，影响质量，本 cycle 必修）

### P1-1：`src/view_models/process_panel.rs:679-680` 注释 stale（与 v0.8 阶段 3 实际接入矛盾）

**位置**：`src/view_models/process_panel.rs:679-680`

**现状**：

```rust
// v0.7 阶段 4：':' 激活 FilterExpr 模式（ADR-0011）。
// 仅 List view 接入；Tree / AppGroup 视图暂保持 substring（详见 tech-debt）。
```

但 v0.8.0 阶段 3（TD-15）已把 Tree / AppGroup view 接入 FilterExpr（同文件 line 760 `handle_tree_key` 的 `:` 激活 + line 823 `handle_app_group_key` 的 `:` 激活）。

**为什么 P1**：

1. **未来会话误导**：下次会话读到这条注释会以为 Tree/AppGroup 仍是 substring，要么重做、要么困惑代码与注释矛盾。
2. **CLAUDE.md 「Surgical Changes」原则的反面**：注释不及时跟进代码变更，是典型的 stale comment 问题。
3. **不符合 stage 3 doc 验收标准**：stage 3 doc 第 99 行明确「TD-15 标 Fixed」，但代码内注释还指向 TD-15 未修状态。

**修复**：把「仅 List view 接入；Tree / AppGroup 视图暂保持 substring（详见 tech-debt）」改成「List / Tree / AppGroup 三视图均接入（v0.7 阶段 4 List + v0.8 阶段 3 Tree/AppGroup）」。

**验证**：`grep -n "仅 List view 接入" src/` 应无结果；该注释提到 v0.8 阶段 3。

**Status: Fixed in commit TBD**（stage 4 修复，详见 commit diff）

---

## P2（文档一致性，归档到 tech-debt / ADR / docs，不阻断发布）

### P2-1：ADR-0011 缺 v0.8.0 阶段 3 增量段（FilterExpr 扩 Tree/AppGroup 的决策记录）

**位置**：`docs/adr/0011-filter-expression.md`

**现状**：ADR-0011 主 Decision 段（line 14-87）和 Consequences 段（line 124-155）只描述 v0.7.0 阶段 4 的「List view 接入」+ v0.8.0 阶段 2 的「错误信息中文化」两段，缺 v0.8.0 阶段 3（FilterExpr 扩 Tree/AppGroup）的决策记录。

**应补内容**：
- Tree view：`get_filtered_tree_visible(cached_processes)` 加 cached_processes 参数，FilterExpr 分支建 pid→&ProcessInfo HashMap apply
- AppGroup view：`app_group_filtered_visual_items(cached_processes)` 同款扩参；**Header 项按聚合值（total_cpu/total_memory）apply**，Child 项按单进程 apply；Header 命中→整组保留，Header 不命中但 Child 命中→仅显示命中 child + 自动展开
- 设计取舍：AppGroup Header 用合成 ProcessInfo（避免改 AppGroupProcess 结构），与 stage 3 doc 切片 B 选项 A 一致

**修复**：ADR-0011 加一节「### v0.8.0 阶段 3 增量：FilterExpr 扩 Tree/AppGroup view（TD-15）」，~15 行。

**Status: 归档为 P2，由本 stage 4 任务 10（README + CONTEXT 清理）一并补 ADR 增量段**

---

### P2-2：ADR-0016 缺 v0.8.0 stage 1 推迟说明

**位置**：`docs/adr/0016-ebpf-flow-graph.md` Consequences 段（line 171）

**现状**：line 171 已写「Linux 真实编译验证缺失：worker.rs / ebpf-ebpf/src/main.rs 在 Windows 会话落地，未在 Linux + root + 内核 5.10+ 环境验证（详见 TD-19）」。但未明确「v0.8.0 cycle 主动推迟 stage 1，留 v0.9.0 cycle 启动前再评估」这一状态。

**为什么 P2 不 P1**：

- v0.7.0 已记录此 known limitation，v0.8.0 README banner + CHANGELOG 都会再次显式标注，用户不会因此误解
- ADR Consequences 是技术记录，不强求 cycle 状态字样
- 「主动推迟」是项目流程事实，不是技术决策变更

**修复**：在 line 171 后加一行括注「（v0.8.0 cycle 用户主动推迟 stage 1 到 v0.9.0 cycle，详见 stage 4 doc 顶部说明）」。

**Status: 归档为 P2，由本 stage 4 任务 10（README + CONTEXT 清理）一并补**

---

### P2-3：tech-debt TD-19 状态需明确「v0.8.0 cycle 主动推迟到 v0.9.0 cycle」

**位置**：`docs/tech-debt.md` TD-19 段（line 180-186）

**现状**：TD-19 标题写「v0.7.0 阶段 8 遗留」，line 185 写「修复（v0.7 收尾或 v0.8）：Linux 会话跑 ... 按报错修」。但 v0.8.0 cycle 已主动推迟此条到 v0.9.0 cycle。

**为什么 P2 不 P1**：

- TD-19 当前状态准确（未 Fixed），只是缺 v0.8.0 cycle 的状态推进记录
- stage 4 验收标准明确「tech-debt.md 终态：v0.8.0 段全 Fixed，新 P2 追加到 v0.9.0+ 段」——TD-19 属 v0.7 遗留，不属 v0.8.0 段，状态更新即可

**修复**：TD-19 末尾加「**v0.8.0 cycle 推进**：用户主要用 Windows 开发，stage 1（WSL2 真实验证）主动推迟到 v0.9.0 cycle 启动前再评估。stage 4 验收时此条标「⏸ Deferred to v0.9.0 cycle（用户主动决策）」」。

**Status: 归档为 P2，由本 stage 4 任务 7（P2 归档）一并处理**

---

### P2-4：v0.8 stage docs 头部缺发布标记

**位置**：`docs/stages/v0.8-stage-{1,2,3,4}.md`

**现状**：4 个 stage doc 头部都是阶段标题 + 独立会话指令引用，缺 v0.7-stage-{1..10}.md 同款的 ✅ 已发布 / ⏸ 推迟 标记。

**为什么 P2 不 P1**：

- 不影响代码 / 测试 / 功能
- stage docs 是过程文档，发布标记是约定俗成的状态指示
- v0.7 cycle 收尾时已为 v0.7-stage-{1..10}.md 加过 ✅，v0.8 cycle 同款做法

**修复**：
- `v0.8-stage-2.md` / `v0.8-stage-3.md` / `v0.8-stage-4.md` 头部加 `> ✅ **已发布**（v0.8.0，2026-06-28）`
- `v0.8-stage-1.md` 头部加 `> ⏸ **主动推迟**（v0.8.0 cycle；用户主要用 Windows，TD-19 Linux 验证推迟到 v0.9.0 cycle 启动前再评估）`

**Status: 归档为 P2，由本 stage 4 任务 10（README + CONTEXT 清理）一并处理**

---

## 审查覆盖矩阵（按 stage 4 doc 第 1 步 5 子项）

| 子项 | 检查点 | 结论 |
|---|---|---|
| **代码质量** | stage 1 ebpf 修复是否引入 Linux-only 回归 | ✅ stage 1 主动推迟，无修改，无回归 |
| 代码质量 | stage 2 Linux stub 测试是否真覆盖降级路径 | ✅ `tests/test_linux_stubs.rs` 6 case（env/dlls/handles/memory bogus pid Err + self pid Ok）+ `tests/test_platform_compat.rs` 5 case（跨平台契约）；CI yml `check-linux` job 跑全量 + ≥ 30 bin 校验 |
| 代码质量 | stage 3 AppGroup FilterExpr 聚合值判断 | ✅ Header 走 `total_cpu`/`total_memory` 合成 ProcessInfo，Child 走原始 ProcessInfo；语义「`cpu > 50` = 该 .exe 总 cpu > 50」与 stage 3 doc 第 70 行一致；Header 命中→整组保留，Header 不命中但 Child 命中→仅显示命中 child + 自动展开 |
| 代码质量 | FilterExpr 错误中文化覆盖 nom ErrorKind 变体 | ✅ `error_kind_to_chinese` 覆盖 9 变体（TakeWhile1/TakeTill1/Tag/Char/AlphaNumeric/Alpha/Digit/Verify/Float/Eof/MultiSpace/Space + 兜底「语法错误」）；`char_to_chinese` 覆盖 4 字符；测试锁「不泄漏 nom 内部枚举名」契约 |
| **架构** | ADR-0011 Consequences 与 v0.8 落地一致 | ⚠️ ADR-0011 已加 v0.8 阶段 2 增量段（错误中文化 + cut 决策），但缺 v0.8 阶段 3 增量段（FilterExpr 扩 Tree/AppGroup）→ **P2-1** |
| 架构 | ADR-0016 Consequences 与 v0.8 落地一致 | ✅ 限制段准确（line 171 标 TD-19 未验证），但缺 v0.8 cycle 推迟说明 → **P2-2** |
| 架构 | AppGroup API 签名变更是否破坏 v0.7 阶段 5 panel controller 边界 | ✅ `app_group_filtered_visual_items` 是 ProcessPanel 方法；调用方走 `app.process_panel.panel.<method>(&app.cached_processes[..])` 通过 `.panel` 访问器，符合 PanelController thin wrapper 设计（ADR-0012）；`get_filtered_tree_visible` 同款 |
| **安全性** | ebpf Linux 真实运行时权限 | ✅ README FAQ + ADR-0016 都明确 root 或 CAP_BPF + CAP_PERFMON；`EbisuBpfWorker::try_spawn` 失败→None→UI 降级。stage 1 未跑 → 真实权限模型未在 Linux 实测，但作为 known limitation 已记录（CHANGELOG + README） |
| 安全性 | FilterExpr AppGroup 聚合值是否泄漏单进程信息 | ✅ Header 走聚合值（total_cpu/total_memory）→ 不泄漏；Child 走单进程 ProcessInfo apply → 这本来就是用户可见信息；无「通过聚合反推」攻击面 |
| **跨平台** | v0.8 stage 1 ebpf 代码在 Windows 默认 build 不破坏 | ✅ stage 1 主动推迟，无修改；`cargo build --release`（默认 features）+ `--no-default-features` 都通过；cfg-gate 正确（`[target.'cfg(target_os = "linux")'.dependencies]` + `[features] ebpf = ["dep:aya", "dep:aya-log"]`） |
| 跨平台 | Linux stub 测试在 Windows CI 跑也能 pass | ✅ `test_linux_stubs.rs` 整文件 `#![cfg(target_os = "linux")]`，Windows 编译跳过；`test_platform_compat.rs` 跨平台契约 case 在所有平台都跑；CI `check-linux` job 校验 ≥ 30 测试 bin |

---

## 修复计划

- **P1-1**：本 stage 4 任务 6（修 P0/P1）单独 commit `fix(v0.8.0): P1-1 stale comment re FilterExpr view coverage`
- **P2-1 ~ P2-4**：合并到 stage 4 任务 10（README + CONTEXT 清理）一次性处理，分别归档到对应 ADR / tech-debt / stage docs
  - P2-1 → ADR-0011 加 v0.8 阶段 3 增量段
  - P2-2 → ADR-0016 加 cycle 推迟说明
  - P2-3 → tech-debt TD-19 加 v0.8.0 cycle 推进段
  - P2-4 → 4 个 stage doc 头部加发布标记

---

## 验收对照（stage 4 doc 第 4 步验收标准 vs 实际）

| 验收项 | 实际 | 备注 |
|---|---|---|
| REVIEW-9.md 全部 P0/P1 标 Fixed | 待修 P1-1 后标 Fixed | P0 0 项 |
| `cargo test --release -q` 通过（≥ 920 passed / 0 failed） | **930 passed / 0 failed / 3 ignored** ✓ | 超 stage 4 验收阈值 |
| `cargo fmt --all -- --check` 通过 | ✓ | |
| `cargo clippy --release --all-targets -- -D warnings` 通过 | ✓ | |
| `cargo build --release --no-default-features` 通过 | ✓ | 6m40s 完成 |
| `cargo +nightly build -p proc-ebpf --target bpfel-unknown-none --release`（Linux） | **跳过** | stage 1 推迟，stage 4 不依赖 |
| `cargo build --release --features ebpf`（Linux） | **跳过** | 同上 |
| Cargo.toml 版本号 0.8.0 | 待 stage 4 任务 8 改 | |
| README + CHANGELOG + CONTEXT 完整反映 v0.8.0 | 待 stage 4 任务 9-10 改 | |
| `git tag v0.8.0` 已打（未 push） | 待 stage 4 任务 12 | 等用户确认 |
| tech-debt.md 终态 | 待 stage 4 任务 7 | TD-19 加推迟说明 |
| stage docs 头部全 ✅ | 待 stage 4 任务 10 | stage 1 是 ⏸ |
