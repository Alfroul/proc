# 阶段 7：全局 Review — 只产报告，不动代码

> **独立会话指令**：阅读 CONTEXT.md 和 docs/stages/stage-7.md，对整个 v0.6.0 周期进行 Code Review，产出 `docs/reviews/REVIEW-7.md`，本阶段**禁止修改任何代码**

**目标**：对 v0.6.0 阶段 1-6 全部产出做全局审查，按 P0/P1/P2 三档分级列出问题，交给阶段 8 修复。

**前置依赖**：阶段 1-6 全部完成（plan.md 阶段总览中阶段 1-6 全部 `[x]`）。

**预期代码量**：0（只产 `docs/reviews/REVIEW-7.md`，不动任何 .rs / .toml / .yml 文件）

**任务清单**：

### 任务 1：开工前回归验证

```bash
cargo test --release -q                       # 应 ~741 passed
cargo clippy --release --all-targets -- -D warnings   # 0 warnings
cargo fmt --all -- --check                            # 干净
cargo build --release --no-default-features           # 编译通过
```

如发现任何失败 → **停止 Review**，回退到修复会话（在 plan.md 加阶段 6.1 回溯修复）。

### 任务 2：审查范围与维度

按 5 个切片 + 横切维度审查：

**切片 A — 安全（阶段 2）**：
- env_mask 的 SECRET_PATTERNS 是否覆盖完整（漏报常见 secret key？）
- mask_value 的多字节字符处理是否正确
- self_mitigation 是否真的被 main 最早调用（grep 验证）
- restricted_spawn 是否覆盖了所有 elevated 时 spawn 的子进程（docker exec / nvtop 漏网？）
- 录屏强制 mask 是否在所有 Env Tab 渲染路径都生效
- EnvVar::is_secret 字段在 serde round-trip 中行为是否正确（向后兼容 0.5.0 .prec 文件？）

**切片 B — 可观测性（阶段 3）**：
- 日志 rotate 在 Windows 路径分隔符下是否工作（`\` vs `/`）
- panic hook 写 crash report 在 worker 线程 panic 时是否能触发（worker panic 不经过主线程 panic hook）
- WorkerMetrics 的 atomic 操作是否正确（CAS max_us 是否有 ABA 问题？）
- `proc diag` 在 worker 未启动时（CLI 模式）行为如何
- help_panel 的 Workers 区段在终端宽度 < 100 时是否换行正确

**切片 C — 性能（阶段 4）**：
- `Arc<str>` 的 deref 是否在所有 hot path 上正确（搜索 / sort / 显示）
- `name_lower` 字段在 ProcessInfo Clone 时是否正确共享 Arc
- rebuild_sorted_cache 优化后是否有功能回归（边界 case：空列表 / 单元素 / 全部匹配搜索）
- ProcessStatus::From sysinfo 映射是否完整（sysinfo 0.34 实际变体核对）

**切片 D — 架构（阶段 5）**：
- 3 个 Controller 拆分后 App 是否仍有上帝对象倾向
- Controller 之间的通信是否合理（直接互访 vs 通过 App 中介）
- main.rs / cli/ 拆分后是否所有 import 路径正确
- 是否有循环依赖（Controller ↔ App）
- WorkerManager::restart 是否真的能恢复崩溃的 worker

**切片 E — UX + 测试（阶段 6）**：
- 'r' / 'c' 键位的 deprecation warning 是否每次都显示（用户烦躁？）
- F5 / 'y' 在详情页的快捷键提示是否完整
- 是否需要引入 proptest（v0.7.0+ 评估；目前未引入）
- 性能回归测试是否覆盖 hot path（`tests/test_perf_baseline.rs` 是否反映真实数据分布）
- Linux stub 测试是否真的在 Linux CI 上跑（cfg-gate 正确？）

**横切 1 — 架构一致性**：
- 新代码是否遵循既有模块边界（src/metrics / src/security 等新模块的组织）
- 是否引入了未文档化的术语（CONTEXT.md 术语漂移检测）
- 是否偏离了 ADR-0001 phased-project 规则（硬性硬停止 / 跨会话规则）

**横切 2 — 文档完整性**：
- README.md 是否反映 v0.6.0 全部能力（env_reveal / self-mitigation / worker metrics / F5 / 'y'）
- CHANGELOG.md 是否每阶段都有 Added/Changed 段
- SECURITY.md 是否反映 self-mitigation 实际策略（4 项开启，Signature 留 0.7+）
- CONTRIBUTING.md 是否反映 src/cli/ 新结构
- docs/adr/0008 Status 是否真改 Accepted（阶段 2 落地后）
- docs/stages/stage-N.md 是否与实际产出一致

**横切 3 — 测试覆盖**：
- 总测试数 ~741 是否合理（vs 阶段 1-6 累计估算）
- 是否有重要模块无集成测试
- 是否有平台特化代码（#[cfg(windows)]）有 Linux 等价测试
- criterion bench 是否在 CI 中自动跑

**横切 4 — 性能基线**：
- 启动时间（cold start）vs 0.5.0 是否退化（self_mitigation 调用 + tracing-appender init 开销）
- 单帧渲染耗时（50ms tick 预算下）vs 0.5.0 是否退化
- 内存占用（WorkerMetrics / InspectorController 等新结构）
- 二进制体积（self_mitigation 加 FFI 是否增大）

**横切 5 — 安全**：
- self_mitigation 4 项策略是否实际生效（验证步骤是否在 README / SECURITY 说明）
- elevated 启动时 restricted_spawn 是否覆盖所有子进程
- 是否有未脱敏的 secret 入口（DNS 查询域名 / 进程 cmd 是否需要 mask？）

### 任务 3：产出 `docs/reviews/REVIEW-7.md`

按下方格式：

```markdown
# 全局 Review 报告 — 阶段 7（v0.6.0）

**审查范围**：v0.6.0 阶段 1-6 全部产出
**审查日期**：YYYY-MM-DD
**基线测试**：cargo test --release = X passed / 0 failed / N ignored

## 摘要

- 总问题数：P0 X / P1 Y / P2 Z
- 阻断性问题（必须修才能发 v0.6.0）：N 项
- 已知限制（不阻断但需文档化）：M 项

## P0（阻断性，必须修才能交付 v0.6.0）

- [问题描述] — `文件路径:行号` — 建议修复方式
  - 影响：用户 / 安全 / 性能 具体后果
  - 验证：修复后如何确认

## P1（重要，影响质量）

- [问题描述] — `文件路径:行号` — 建议修复方式

## P2（建议，长期改善）

- [问题描述] — 影响评估

## 切片 A — 安全审查

### A1: ...

### A2: ...

## 切片 B — 可观测性审查

...（依此类推）

## 横切维度

### 架构一致性
...

### 文档完整性
...

### 测试覆盖
...

### 性能基线
...

### 安全
...

## 建议归档到 tech-debt.md 的 P2（v0.7.0+）

- ...
```

### 任务 4：归档 P2 到 `docs/tech-debt.md`

如果 P2 较多（> 10 项），创建 `docs/tech-debt.md` 按 v0.7.0 / v0.8.0+ 分组归档：

```markdown
# 技术债归档 — v0.6.0 Review 产出

## v0.7.0 候选

- P2-1: ... — 影响 / 修复建议
- P2-2: ...

## v0.8.0+ 候选

- P2-10: ProcessSignature Policy 评估（需审计所有 native 依赖签名）
- P2-11: Linux eBPF net_flow provider（替代 nethogs 子进程）
...
```

### 验收命令

```bash
# 1. 本阶段未修改任何代码（除新增 REVIEW 文件外）
git diff --stat | grep -v "docs/reviews/REVIEW-7.md" | grep -v "docs/tech-debt.md"
# 上述命令应该输出空（除 REVIEW / tech-debt 文件外无任何代码改动）

# 2. REVIEW 文件存在且按 P0/P1/P2 分级
test -f docs/reviews/REVIEW-7.md
grep -c "^## P[012]" docs/reviews/REVIEW-7.md   # 应该 ≥ 3

# 3. 测试基线未动
cargo test --release -q    # 仍是 ~741 passed
```

**验收标准**：
- `docs/reviews/REVIEW-7.md` 已产出，按 P0/P1/P2 三档分级
- 至少覆盖 5 个切片 + 5 个横切维度
- 本阶段**未修改任何代码**（`git diff` 应只显示新增的 REVIEW / tech-debt 文件）
- 全量测试基线未动（~741 passed）
- 如 P2 > 10 项，已归档到 `docs/tech-debt.md`

**主修改区域**：
- `docs/reviews/REVIEW-7.md`（新）
- `docs/tech-debt.md`（新，可选）
- **不动任何 .rs / .toml / .yml 代码文件**
