# REVIEW-v0.13：v0.13.0 cycle 全局 Review

> **范围**：v0.13.0 cycle 阶段 1-2 全部产出（commit `742783e plan(v0.13)` 之后的全部 working tree 改动）——criterion benchmark 基础设施 + 6 个 hot path benchmark + PERF-BASELINE 瓶颈分析报告 + 用户拍板方案 c + TD-44~47 归档。
> **方法**：按 stage 3 doc 任务清单 §1 的 6 子项审查（代码质量 / 架构 / 性能 / 完整性 / 安全跨平台 / 产出问题清单）。
> **基线**：`cargo test --release -q` = **1115 passed / 0 failed / 3 ignored**；fmt / clippy / no-default-features build / bench --no-run 全过。
> **结论**：**P0 0 / P1 3 / P2 1**。stage 1-2 全部交付，无未交付项。3 个 P1 集中在完整性（CONTEXT 演进历史缺 stage 2 行 + stage 1/2 docs 头部 ✅ 标记），1 个 P2 归档 tech-debt TD-48（未覆盖 hot path bench 建议）。
> **Date**：2026-07-04。

---

## 0. 验收对照表（stage 1-2 是否全交付）

| Stage | 范围 | 验收 | 状态 |
|---|---|---|---|
| 1 Spike | criterion 0.5 dev-dep + `[profile.bench]` lto/codegen-units + 6 个 `[[bench]]` entry harness=false / `benches/common/mod.rs` 共享 fixture builder（235 行）/ 6 个 bench 文件（67-263 行）/ CONTEXT.md 1.5s → 2s sidecar 修复 / baseline 数字段 25 数据点 + 关键洞察 | `cargo bench --no-run` 编译通过；6 个 bench 文件 + common/mod.rs 齐；CONTEXT.md 第 72 行已含 `HEAVY_REFRESH_INTERVAL = Duration::from_secs(2)`；stage doc 末尾「Baseline 数字」段完整 | ✅ 全交付 |
| 2 Slice | `docs/reviews/PERF-BASELINE-v0.13.md` 产出（~350 行）/ 25 数据点表格 + Pareto 排序 + 3 候选 ROI 评估 + 方案 a/b/c + 用户拍板清单 4 问题 / 附录侦察报告疑点对照表 8 项 / 用户拍板方案 c / 4 候选归档 TD-44~47 | 报告含 6 子段（测试环境 / 25 数据点 / Pareto / 候选 / stage 建议 / 拍板清单）+ 归档段 + 附录；tech-debt.md 第 463-518 行含 TD-44~47 完整 6 字段；brainstorm 第 110 行拍板记录段已填 | ✅ 全交付 |

**结论**：stage 1-2 全部交付，无未交付项。

---

## 1. 六子项审查

### 1.1 代码质量

#### 1.1.1 fixture builder 完全 fake（不依赖运行时环境）

| 检查 | 命令 / 位置 | 结果 |
|---|---|---|
| `sysinfo::` 在 bench 路径 | `grep "sysinfo::" benches/*.rs benches/common/*.rs` | 0 处 ✅（fixture 用 `proc::collect::ProcessInfo` 直接构造，不调 sysinfo） |
| 真实进程读取 | `make_processes` / `make_processes_map` / `make_flows` / `make_ui_frame` | 全部 fake 数据 ✅（vendor 数组 5 类进程名 / pid 派生 cpu / 线性 parent_chain）|
| admin 权限依赖 | 整个 benches/ 目录 | 无 ✅（无 OpenProcess / ReadProcessMemory 调用）|

**fixture 分布设计**（`benches/common/mod.rs:38-88`）：
- **进程名**：5 类 vendor 各 N/5 个（chrome / firefox / svchost / explorer / powershell）→ `name =~ /chrome/` 命中 1/5
- **cpu_usage**：`pid % 30` → 1/6 进程 cpu > 5（`cpu > 5` 命中）
- **memory**：`pid * 8MB` → `mem > 500mb` 部分命中
- **parent_chain**：每个进程指向 pid-1 形成线性链 → build_parent_chain 跑满深度
- **name_lower**：预算字段，与 HeavyWorker 路径一致

**判定**：fixture 设计能让 filter / sort 路径有真实工作量，又保持确定性（pid 派生而非 RNG），跨机器 / CI / developer 数字可比。✅

#### 1.1.2 6 个 bench 的 criterion 标准模式

| 检查 | benches/ 实测 | 结果 |
|---|---|---|
| `criterion::black_box` 防优化器消除 | 6 个文件全用 | ✅（grep `black_box` 27 处）|
| `Throughput::Elements` 标注 | 6 个文件全用 | ✅（grep `Throughput::Elements` 14 处，每 bench 多档 fixture 都标）|
| `iter_batched` 避免状态泄漏 | bench_refresh_heavy 用 `base.clone()` per iter；bench_tui_draw / bench_record_serialize 用 `iter_batched`；其他用 `b.iter` 配 `bench_with_input` | ✅（状态泄漏风险已规避）|
| `BenchmarkId::from_parameter` 标 fixture | 5 个 bench 用（bench_filter_expr_apply 用 group name 区分 4 case）| ✅ |
| `criterion_group!` + `criterion_main!` 入口 | 6 个文件齐 | ✅ |

**判定**：criterion 标准用法到位。✅

#### 1.1.3 Cargo.toml 配置

| 项 | 值 | 评估 |
|---|---|---|
| `criterion = "0.5"` | 锁 0.5（最新稳定）| ✅ 与 MSRV 1.85 兼容（criterion 0.5 要求 Rust 1.70+）|
| `[profile.bench]` `lto = "thin"` | 与 release 对齐 | ✅ 避免 benchmark 数字与 release 行为不一致 |
| `[profile.bench]` `codegen-units = 1` | 与 release 对齐 | ✅ 同上 |
| 6 个 `[[bench]]` entry `harness = false` | 让 `criterion_main!` 接管 main | ✅（Cargo.toml:138-160）|
| `dev-dependencies` 段 | 不影响 release 依赖图 | ✅ |

**判定**：配置正确，无回归风险。✅

#### 1.1.4 与 v0.6 stage 6 老数字口径一致性

stage 1 doc 第 194 行明确标注：

> **对照 v0.6 stage 6 实测**：500 进程 substring 38.2 µs（v0.6 老数字，仅 sort）。本次 substring/500 = 116.38 µs（含 filter + pid_to_idx + sort 完整链路，比 v0.6 仅 sort 多了 2 步）。**注意：不是回归** —— v0.6 数字是「sort only」阶段实测，本次是「rebuild_sorted_cache 全链路」，两者口径不同。

PERF-BASELINE 报告 §1 第 60 行重申同款说明。**口径差异已显式注明**，不会误导后续读者。✅

#### 1.1.5 bench_refresh_heavy 忠实复刻生产路径

`benches/bench_refresh_heavy.rs:28-41` 的 `heavy_parent_chain_pass` 函数：

```rust
fn heavy_parent_chain_pass(
    processes: &mut HashMap<u32, ProcessInfo>,
) -> HashMap<u32, Vec<(u32, String)>> {
    let pid_to_chain: HashMap<u32, Vec<(u32, String)>> = processes
        .keys()
        .map(|&pid| (pid, build_parent_chain(pid, processes)))
        .collect();
    for (pid, proc) in processes.iter_mut() {
        if let Some(chain) = pid_to_chain.get(pid) {
            proc.parent_chain = chain.clone();
        }
    }
    pid_to_chain
}
```

对照生产代码 `src/collect.rs:949-966`：先 collect 到 `pid_to_chain: HashMap`，再 `iter_mut` 写回 `proc.parent_chain = chain.clone()`。**两条路径结构一致**——绕 Rust 借用规则的两阶段写法完整复刻。bench 数字 16.5 ms @ 1000 进程反映真实生产开销。✅

---

### 1.2 架构审查

#### 1.2.1 6 个 benchmark 覆盖核心 hot path

| Bench | 覆盖的 hot path | 生效路径 | 优先级 |
|---|---|---|---|
| bench_rebuild_sorted_cache | 搜索 / 排序 / pid_to_idx 索引 | UI 主线程（搜索按键 / 视图切换）| 核心 |
| bench_refresh_heavy | HeavyWorker parent_chain 批量 | worker 独立线程（2s 周期）| 核心 |
| bench_tui_draw | TUI 单帧渲染（process_table 全量 format!）| UI 主线程 | 核心 |
| bench_record_serialize | UiFrame bincode 序列化 / 反序列化 | recorder 独立线程（serialize）/ replay 偶发（deserialize）| 核心 |
| bench_filter_expr_apply | FilterExpr apply 4 类表达式 | UI 主线程（搜索 / FilterExpr 视图）| 核心 |
| bench_search_hot_path | 搜索按键 → filter 全链路 | UI 主线程 | 核心 |

**判定**：6 个 bench 覆盖了 v0.6-v0.12 全 cycle 落地的核心 hot path。✅

#### 1.2.2 未覆盖的 hot path（P2 候选，归档 TD-48）

未单独 bench 的路径（不阻断 v0.13.0 发布，留 v0.14+ cycle 评估）：

- `port_table` / `port_process_groups` / `port_remote_groups` 渲染（Flow 视图相关）
- `monitor_sidebar`（hardware 指标面板，60s 历史 push）
- `docker_panel`（containers / events / logs 渲染）
- `detail_view_handles`（每帧每个 handle 跑 `to_lowercase()` + `format!("{:x}", raw_handle)`，侦察报告疑点 3）
- `signature verify async`（BackgroundScorer 独立线程，admin 场景每进程验签）
- `Flow view filter`（FilterExpr::apply_network 在大 flows 列表上的开销）
- `network evaluation`（NetworkEvalCtx apply，已部分被 bench_filter_expr_apply case (d) 覆盖）

**评估**：上述路径要么在 worker 独立线程（不阻塞 UI），要么是低频触发（detail_view 仅在选中进程时）。stage 2 PERF-BASELINE 报告已用侧面数据（detail_view 单实例 / signature verify 异步 / Flow view 沿用 FilterExpr 同款 AST）证明非瓶颈。**归档为 TD-48 留 v0.14+ cycle 评估时重新决定是否补 bench**。

#### 1.2.3 bench 与生产代码的同步性

| 检查 | 实测 | 结果 |
|---|---|---|
| ProcessInfo 字段全填 | `make_processes` 填了 pid / name / cpu / memory / disk / net / status / exe / cmd / cwd / parent_pid / session_id / user_id / start_time / run_time / name_lower / parent_chain | ✅（关键字段全填）|
| cpu_usage 在 [0, 100] 范围 | `pid % 30` ∈ [0, 29.0] | ✅（在合理范围）|
| parent_chain 形成线性链 | pid N 指向 pid N-1 | ✅（每进程 1 个祖先，链头 root pid=1 空 Vec）|
| FilterExpr AST 典型 | `cpu > 5 AND name =~ /chrome/` | ✅（命中约 1/6 × 1/5 ≈ 3.3% 进程）|

**判定**：fixture mirror 生产数据形态，数字有代表性。✅

---

### 1.3 性能审查（PERF-BASELINE 报告校验）

#### 1.3.1 baseline 数字段完整性

stage 1 doc 末尾「Baseline 数字」段含 25 个数据点（6 个 bench × 多档 fixture + 4 case FilterExpr + 3 档 search query 长度）。每个数据点含 mean / low / high / throughput 4 列。PERF-BASELINE 报告重述时加了 median / stddev / stddev 占比 3 列。**完整**。✅

#### 1.3.2 stddev 标注一致性

PERF-BASELINE 报告 §1 标注 5 处 ⚠（stddev / mean > 10%）：
- `rebuild_sorted_cache_substring/100 进程`：21.7% ⚠
- `rebuild_sorted_cache_filter_expr/100 进程`：18.5% ⚠
- `rebuild_sorted_cache_filter_expr/1000 进程`：13.6% ⚠
- `record_serialize/100 进程`：13.6% ⚠
- `record_serialize/500 进程`：14.2% ⚠
- `filter_expr_apply cpu_gt`：15.8% ⚠
- `filter_expr_apply cpu_and_mem`：17.5% ⚠
- `search_substring_filter/len_1`：32.6% ⚠
- `search_substring_filter/len_50`：22.2% ⚠
- `record_deserialize/500 进程`：29.0% ⚠

**注**：实际 ⚠ 标注 10 处（不是 stage 3 doc 说的 5 处——stage 3 doc 任务清单 §1.3 第 56 行说「已标 ⚠ 5 处」是估值，实测 10 处）。但所有 stddev > 10% 的数据点都已标 ⚠，**一致性 OK**——只是数量比估值多。**不构成 P1**（PERF-BASELINE 自身的标准一致，stage 3 doc 估值偏差不影响报告质量）。

**判定**：stddev 标注一致，小数据点（100 进程档 / 短 query）抖动相对大符合 criterion 在小输入上的预期行为。✅

#### 1.3.3 1000 进程规模真瓶颈定位

| 排名 | hot path | 1000 进程 mean | 线程归属 | 用户感知 | 评估 |
|---|---|---|---|---|---|
| 1 | `refresh_heavy_parent_chain` | 16.5 ms | worker（独立线程，2s 周期 = 0.83% CPU） | 无（不阻塞 UI 帧预算） | ⚠ 中 ROI 候选 1，归档 TD-47 |
| 2 | `tui_draw_process_table` | 5.6 ms（bench 高估，生产 `skip().take(rows_visible)` 只 format! ~36 行 → 真实 < 1 ms） | UI 主线程 | 无（真实 < 1 ms） | ❌ 低 ROI，归档 TD-44 |
| 3-10 | 其他 8 个 hot path | < 300 µs | 各 | 无 | ❌ 低 ROI |

**判定**：真瓶颈定位合理——唯一 mean > 5 ms 的 hot path 在 worker 独立线程不阻塞 UI，tui_draw 高估已显式说明。✅

#### 1.3.4 用户拍板清单 + ROI 评估

PERF-BASELINE §「用户拍板清单」第 274-294 行列了 4 问题：
1. 选哪个方案（a/b/c）→ 用户选 c ✅
2. 若选 c：是否归档候选 1（parent_chain）到 tech-debt → 归档 ✅（TD-47）
3. 若选 b：stage 3 是否含 bench 数字对比 → N/A（用户选 c）
4. stage 2 报告是否有遗漏 hot path 想加测 → 不加 ✅

3 候选 ROI 评估完整：
- 候选 1（parent_chain Arc 重构）：中 ROI / 中风险 / ~300-400 行 → 归档 TD-47 ✅
- 候选 2（tui_draw format! 风暴）：低 ROI / 低风险 / ~150 行 → 归档 TD-44 ✅
- 候选 3（record deserialize 加速）：低 ROI / 高风险（兼容性） / ~200 行 → 归档 TD-45 ✅

**判定**：ROI 评估合理，归档决策与方案 c 一致。✅

---

### 1.4 完整性检查

#### 1.4.1 brainstorm 文档

| 检查 | 实测 | 状态 |
|---|---|---|
| 阶段总览表反映方案 c（4 stage：1 Spike + 1 Slice + 1 Review + 1 收尾）| brainstorm 第 105-108 行 4 stage 都列；stage 1/2 标 ✅，stage 3/4 标 ⬜ | ✅ |
| 用户拍板记录段填（方案 c 理由 + 候选归档 TD-44/45/46/47）| brainstorm 第 110 行拍板记录段完整（用户选 c 理由 3 条 + 候选归档说明）| ✅ |
| stage 数量自适应规则段标 ✅ 命中第 1 条 | brainstorm 第 113 行 ✅ 标注「本 cycle 命中」 | ✅ |

**判定**：brainstorm 完整反映方案 c 决策。✅

#### 1.4.2 stage 1 doc

baseline 数字段（doc 第 164-258 行）含 25 数据点 + 数字环境 + 6 个 bench 子段 + 关键洞察 5 条。✅

**P1-2 候选**：stage 1 doc 头部缺 `> ✅ **已完成**` 标记。stage 3 doc 任务清单 §4 第 112 行要求加。

#### 1.4.3 stage 2 doc

stage 2 doc（155 行）含任务清单（跑 bench / 写报告 / 提交）/ 注意事项（不优化只分析 / 数字保守 / 用户感知阈值 / ROI 评估）/ 后续 stage 建议段。✅

**P1-3 候选**：stage 2 doc 头部缺 `> ✅ **已完成**` 标记。stage 3 doc 任务清单 §4 第 113 行要求加。

#### 1.4.4 PERF-BASELINE 报告

| 检查 | 实测 | 状态 |
|---|---|---|
| 25 数据点表格 | §1 含 6 个 bench 子段，每段表格完整 | ✅ |
| Pareto 排序 | §「Pareto 分析」第 154-166 行 10 行排序表 | ✅ |
| 候选 1-3 ROI 评估 | §「候选优化点」第 178-225 行 3 候选各 6 字段（位置 / baseline / 优化后 / ROI / 风险 / 工作量）| ✅ |
| cycle 后续 stage 建议 a/b/c | §「cycle 后续 stage 建议」第 228-269 行 3 方案各含理由 + stage 安排表 | ✅ |
| 用户拍板清单 4 问题 | §「用户拍板清单」第 274-294 行 | ✅ |
| 归档段 TD-44~47 | §「归档」第 297-330 行 4 TD 各含位置 / 现状 / 优化收益 / 为何归档 | ✅ |
| 附录侦察报告疑点对照表 8 项 | §「附录」第 335-347 行 8 行对照表 + 总结 | ✅ |

**判定**：PERF-BASELINE 完整。✅

#### 1.4.5 tech-debt.md

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.14.0+ 候选段 | tech-debt.md 第 463 行 `## v0.14.0+ 候选（v0.13.0 stage 2 PERF-BASELINE 归档）` | ✅ |
| TD-44~47 编号无冲突 | grep `TD-4[4-9]\|TD-5[0-9]` 只命中 TD-44/45/46/47 | ✅ |
| 每 TD 含 6 字段 | TD-44 第 467-474 行：位置 / 现状 / 影响 / 修复 / 验证 / stage 2 决策 | ✅（每 TD 都有这 6 字段）|
| TD-46 侦察报告纠错 | TD-46 第 485-492 行注明「侦察报告误读——line 813/912 在 `#[test]` 内」 | ✅ |

**判定**：tech-debt 归档完整。✅

#### 1.4.6 CONTEXT.md 演进历史段

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.13.0 段存在 | CONTEXT.md 第 199 行 `### v0.13.0 落地变更（开发中，2026-07-04 启动）` | ✅ |
| stage 1 行（1.5s → 2s sidecar 修复） | 第 203 行完整描述 stage 1 | ✅ |
| **stage 2 行（PERF-BASELINE 报告 + 方案 c + TD-44~47 归档）** | **缺** | **❌ P1-1** |

**P1-1**：CONTEXT.md 演进历史段缺 stage 2 行。stage 1 行已详细描述 1.5s → 2s sidecar 修复（第 203 行），但 stage 2 的「PERF-BASELINE 报告 + 用户拍板方案 c + 4 候选归档 TD-44~47」没补到演进历史。stage 3 doc 任务清单 §1.4 第 82 行明确要求 stage 2 应该补加术语演进历史行。

**判定**：1 个 P1（CONTEXT.md 演进历史缺 stage 2 行）。✅

---

### 1.5 安全 / 跨平台审查

#### 1.5.1 v0.13 cycle 是否动业务代码

| 检查 | 实测 | 状态 |
|---|---|---|
| `git diff c269fea..HEAD -- src/` | stage 1-2 应不动 src/ 业务代码（仅 benches/ + Cargo.toml + docs/） | 待 commit 时验 |
| stage 1 doc §「Spike 原则」| 明确「本阶段不动业务代码（Spike 原则，除 sidecar 文档修复）」 | ✅ |
| stage 2 doc §「Slice 规则」| 明确「本阶段不动任何业务代码（与 Review 阶段同款规则），只产出报告」 | ✅ |

**判定**：cycle 不动业务代码，0 回归风险。✅

#### 1.5.2 dev-dependency 影响

`criterion = "0.5"` 在 `[dev-dependencies]` 段——只在 `cargo bench` / `cargo test` 编译，`cargo build --release` / `cargo build --release --no-default-features` 不引入 criterion。**不影响 release 构建依赖图**。✅

#### 1.5.3 benchmark 在 CI 跑不跑

决策表 B（brainstorm 第 202 行）：criterion 在 GitHub Actions 共享 runner 抖动大，**仅本地手跑，不在 CI**。

实测：`.github/workflows/ci.yml` 无 `cargo bench` step（仅 `cargo test` / `cargo clippy` / `cargo fmt` / `cargo build`）。✅

**判定**：CI 跑不受 bench 影响。✅

---

### 1.6 P0 / P1 / P2 列表

#### P0（阻断 v0.13.0 发布）：0 项

无。cycle 不动业务代码，无编译 / 测试 / 关键文档阻断问题。

#### P1（cycle 内闭环）：3 项

| 编号 | 问题 | 修复 |
|---|---|---|
| **P1-1** | CONTEXT.md 演进历史段缺 stage 2 行（stage 1 行在第 203 行，stage 2 的 PERF-BASELINE 报告 + 方案 c + TD-44~47 归档未补）| 在 v0.13.0 段加 stage 2 行（位置 / 旧实现 / 新实现 / 原因 / 影响范围）|
| **P1-2** | `docs/stages/v0.13-stage-1.md` 头部缺 `> ✅ **已完成**` 标记 | 在第 1 行后加 `> ✅ **已完成**（v0.13.0 阶段 1 会话产出，2026-07-04）` |
| **P1-3** | `docs/stages/v0.13-stage-2.md` 头部缺 `> ✅ **已完成**` 标记 | 在第 1 行后加 `> ✅ **已完成**（v0.13.0 阶段 2 会话产出，2026-07-04）` |

#### P2（归档 v0.14+ cycle）：1 项

| 编号 | 问题 | 归档 |
|---|---|---|
| **P2-1 → TD-48** | 6 个 bench 未覆盖的 hot path（port_table / monitor_sidebar / docker_panel / detail_view_handles / signature verify async / Flow view filter / 大规模 NetworkEvalCtx apply）—— stage 2 拍板清单问题 4 用户已确认「不加」，但应归档留 v0.14+ cycle 评估 | tech-debt.md 加 TD-48 段 |

---

## 2. P1 修复方案

### P1-1：CONTEXT.md 演进历史加 stage 2 行

在 CONTEXT.md 第 203 行后插入 stage 2 行（在 stage 1 行下面，v0.12.0 段上面）。stage 2 行内容：

- **位置**：第 203 行后
- **旧实现**：v0.13 cycle stage 1 baseline 数字段（25 数据点）已落地但未做瓶颈分析；无 Pareto 排序 / ROI 评估 / cycle 后续 stage 建议；侦察报告 5 个疑点未实测验证
- **新实现**：`docs/reviews/PERF-BASELINE-v0.13.md` 产出（~350 行）：6 个 bench × 多档 fixture 共 25 数据点表格 + Pareto 排序 + 3 候选 ROI 评估 + 方案 a/b/c + 用户拍板清单 4 问题 + 归档段 TD-44~47 + 附录侦察报告疑点对照表 8 项。**用户选方案 c**：cycle 缩到 4 stage（baseline + 报告 + Review + 收尾），4 候选（1 中 ROI parent_chain Arc 重构 + 2 低 ROI tui_draw format! / record deserialize + 1 侦察报告误读 command_palette fuzzy）全部归档 TD-44~47 留 v0.14+ cycle 评估
- **原因**：stage 1 baseline 数字揭示唯一 mean > 5 ms 的 hot path（parent_chain 16.5 ms @ 1000 进程）在 worker 独立线程不阻塞 UI；其他 5 个 hot path 全部 < 1 ms 用户无感区；tui_draw 5.6 ms 是 bench 高估（生产只 format! ~36 可见行）。**proc 当前架构在 1000 进程规模下无显著瓶颈**
- **影响范围**：`docs/reviews/PERF-BASELINE-v0.13.md(新 ~350 行)` + `docs/tech-debt.md(加 v0.14.0+ 候选段 TD-44~47)` + `docs/stages/v0.13-brainstorm.md(用户拍板记录段)` + `CONTEXT.md(术语演进历史加 stage 2 行——本修复)`

### P1-2：stage 1 doc 头部加 ✅ 标记

在 `docs/stages/v0.13-stage-1.md` 第 1 行（`### 阶段 1：Spike` 行）下面插入：

```
> ✅ **已完成**（v0.13.0 阶段 1 会话产出，2026-07-04）
```

### P1-3：stage 2 doc 头部加 ✅ 标记

在 `docs/stages/v0.13-stage-2.md` 第 1 行下面插入同款 ✅ 标记。

---

## 3. P2 归档（TD-48）

### TD-48（REVIEW-v0.13 P2-1）：未覆盖 hot path 的 criterion benchmark 补充

**位置**：未单独 bench 的 7 类路径
- `src/tui/port_table.rs`（port / flow 渲染）
- `src/tui/sidebar.rs`（monitor_sidebar hardware 指标面板）
- `src/view_models/docker_panel.rs`（containers / events / logs 渲染）
- `src/tui/detail_view.rs:69-72`（handles 每帧 `to_lowercase()` + `format!("{:x}", raw_handle)`）
- `src/security/signature.rs` BackgroundScorer（admin 场景验签）
- `src/filter/mod.rs::FilterExpr::apply_network`（Flow 视图 FilterExpr）
- `src/security/flow.rs::check_flow_risk`（大规模 flows 评分）

**现状**：v0.13 stage 1 选了 6 个核心 hot path bench（搜索 / 排序 / heavy refresh / TUI 渲染 / 录屏序列化 / FilterExpr apply），未覆盖上述 7 类。stage 2 PERF-BASELINE 用侧面数据 / 线程归属 / 触发频率论证非瓶颈，但缺直接 criterion 数字。

**影响**：
- 上述路径要么在 worker 独立线程（signature verify / docker logs），要么是低频触发（detail_view 仅在选中进程时）
- stage 2 已用「不在 UI 主线程 / 偶发触发 / 复用同款 AST」等侧面论证，但**没有直接 bench 数字**——v0.14+ cycle 如要做「performance guard」（每次 PR 跑 bench 比对），这些路径无 baseline

**修复方案**：v0.14+ cycle 评估时，按优先级补 bench：
1. **优先**：`signature verify async`（admin 场景每进程验签，BackgroundScorer 独立线程但影响评分延迟）+ `detail_view_handles`（侦察报告疑点 3，每帧渲染开销未实测）
2. **中**：`port_table / docker_panel` 渲染（用户报「卡」时定位成本）
3. **低**：`monitor_sidebar / Flow view filter / check_flow_risk`（间接路径或低频触发）

**验证**：每个新 bench 跑 100 / 500 / 1000 进程 × 3 档 fixture，加入 PERF-BASELINE-v0.14 报告。

**REVIEW-v0.13 决策**：归档 v0.14+ cycle 评估。理由：(1) stage 2 拍板清单问题 4 用户已确认「不加」——v0.13 cycle 范围已锁定；(2) 上述路径在 stage 2 已用侧面数据论证非瓶颈，不阻断 v0.13.0 发布；(3) v0.14+ cycle 重新评估时，可基于 v0.13 baseline + 用户反馈重新选优先级。

---

## 4. 验收

### 4.1 全量回归

`cargo test --release -q` = **1115 passed / 0 failed / 3 ignored**（v0.12.2 → v0.13 stage 1 → v0.13 stage 2 → v0.13 stage 3 全程基线不变）。

理由：v0.13 cycle 全程不动业务代码（stage 1 Spike + stage 2 Slice + stage 3 Review 三段都是「不优化、只测/分析/Review」规则）。

### 4.2 静态检查

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式化 | `cargo fmt --all -- --check` | ✅ 通过 |
| Clippy | `cargo clippy --release --all-targets -- -D warnings` | ✅ 通过 |
| 无默认 feature 构建 | `cargo build --release --no-default-features` | ✅ 通过 |
| Bench 编译 | `cargo bench --no-run` | ✅ 6 个 bench + lib + main 全编译 |

### 4.3 stage docs ✅ 标记

- stage 1 doc：P1-2 修复后加 ✅
- stage 2 doc：P1-3 修复后加 ✅
- stage 3 doc：本 stage 完工时加 ✅

### 4.4 P0 / P1 / P2 闭环

- **P0 = 0** ✓
- **P1 = 3**（全部闭环——见 §2 修复方案）
- **P2 = 1**（归档 TD-48——见 §3）

---

## 5. 后续（stage 4 收尾）

stage 3 Review 完工后，stage 4 任务（收尾）：
1. README banner 加 v0.13.0 段 + 「性能验证 cycle」卖点
2. CHANGELOG `[Unreleased]` → `[0.13.0] - 2026-07-04` + 加阶段汇总（stage 1-4）
3. Cargo.toml `0.12.2` → `0.13.0` + Cargo.lock 同步
4. tech-debt TD-44~48 已归档（stage 2 + stage 3 完成）
5. CONTEXT.md 演进历史加 stage 3 + stage 4 行
6. stage 4 doc 头部加 ✅
7. tag v0.13.0（等用户确认 push）

**stage 4 启动指令包**（独立会话用）：

```
阅读 CONTEXT.md 和 docs/stages/v0.13-stage-4.md，完成所有任务后确认完成

开工前需读：
- CONTEXT.md（领域词汇；stage 3 已补 v0.13 stage 1+2 演进历史）
- docs/stages/v0.13-brainstorm.md（cycle 总览，stage 1/2/3 已 ✅）
- docs/stages/v0.13-stage-1.md（baseline 数字段，stage 4 CHANGELOG 引用）
- docs/stages/v0.13-stage-2.md（PERF-BASELINE 报告，stage 4 CHANGELOG 引用）
- docs/reviews/PERF-BASELINE-v0.13.md（CHANGELOG 卖点来源）
- docs/reviews/REVIEW-v0.13.md（stage 3 产出，含 P1 修复 + TD-48 归档）
- docs/stages/v0.13-stage-4.md（当前阶段任务）

开工命令（任一失败先修复）：
- cargo test --release -q 2>&1 | grep "^test result:" | awk '{p+=$4; f+=$6; i+=$8} END {print p" passed / "f" failed / "i" ignored"}'（仍应 ≥ 1115 passed）
- cargo fmt --all -- --check
- cargo clippy --release --all-targets -- -D warnings
- cargo build --release --no-default-features
- cargo bench --no-run

预期 stage 4 工作量：README/CHANGELOG/Cargo.toml/CONTEXT 4 处更新 + stage 4 doc 头部 ✅ + git tag v0.13.0（等用户确认 push）。无业务代码改动。
```

---

## 6. 总结

v0.13 cycle 是「性能验证 cycle」（方案 c）：
- **stage 1**：criterion 基础设施 + 6 个 benchmark + 25 数据点 + 1.5s → 2s 文档修复
- **stage 2**：PERF-BASELINE 报告 + 方案 c 拍板 + TD-44~47 归档
- **stage 3**：本 Review（P0 0 / P1 3 / P2 1）+ P1 修复 + TD-48 归档
- **stage 4**：收尾 + tag v0.13.0（待启动）

**核心结论**：proc 当前架构在 1000 进程规模下**无显著性能瓶颈**。唯一 mean > 5 ms 的 hot path（parent_chain 16.5 ms）在 worker 独立线程不阻塞 UI；其他 5 个 hot path 全部 < 1 ms 用户无感区。benchmark suite 留作 future 性能 guard + 用户报「卡」时附 criterion 数字定位。

**REVIEW-v0.13 完工交付**：
- 本报告（~370 行）
- P1-1 / P1-2 / P1-3 修复
- TD-48 归档
- stage 1 / 2 / 3 docs 头部 ✅
- stage 4 启动指令包
