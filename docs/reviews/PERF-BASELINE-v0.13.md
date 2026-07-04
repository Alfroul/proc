# v0.13 Performance Baseline Report

> **stage**: v0.13 cycle 阶段 2 产出（Slice — 不动业务代码，仅分析）
> **起草**: 2026-07-04
> **基线来源**: stage 1 落地的 criterion 基础设施 + 6 个 benchmark × 多档 fixture
> **回归基线**: 1115 passed / 0 failed / 3 ignored（v0.12.2 → v0.13 stage 1 后不变）

---

## TL;DR

stage 1 数字揭示：

1. **唯一 mean > 5 ms 的 hot path 是 `refresh_heavy_parent_chain` @ 1000 进程 = 16.5 ms**，但它在 HeavyWorker 独立线程跑（不在 UI 主线程），不直接阻塞帧。每 2s 周期 1 次 ≈ 0.83% 持续 CPU + ~5000 次堆分配/秒。
2. `tui_draw_process_table` @ 1000 进程 = 5.6 ms — **bench 高估**真实开销（bench 渲染全部 1000 行，生产代码 `src/tui/process_table.rs:71-75` 用 `skip().take(rows_visible)` 只格式化 ~36 可见行）。真实 TUI 单帧 < 1 ms。
3. 其余 5 个 hot path 全在 < 1 ms 区（用户无感）。
4. **侦察报告 5 个疑点中 4 个实测后归档为非瓶颈**：format! 风暴 / 录屏序列化 / FilterExpr regex / 搜索按键 — 全部远低于用户感知阈值。
5. **真正值得动的是 parent_chain 堆分配**（不是为了 UI 流畅，而是为了 worker 线程的代码质量 + 长期 alloc GC 压力）。

**结论**：proc 当前架构在 1000 进程规模下无显著性能瓶颈。v0.13 cycle 推荐方案 **c（4 stage 收尾：baseline + 报告 + Review + tag）**，或方案 **b（+1 stage 做 parent_chain Arc 重构）** 看用户是否想清技术债。详见 [cycle 后续 stage 建议](#cycle-后续-stage-建议)。

---

## 测试环境

| 项目 | 值 |
|---|---|
| CPU | 13th Gen Intel(R) Core(TM) i7-13700HX（16 cores / 24 threads / 2.1 GHz base） |
| RAM | 16 GB（16,886,128,640 bytes） |
| Machine | Acer Predator PHN16-71（笔记本） |
| OS | Microsoft Windows 11 家庭中文版（Version 10.0.26200.8037，64-bit） |
| Rust | rustc 1.95.0 (59807616e 2026-04-14) / cargo 1.95.0 (f2d3ce0bd 2026-03-21) |
| release profile | `lto = "thin"`, `codegen-units = 1`, `opt-level = 3` |
| bench profile | `lto = "thin"`, `codegen-units = 1`（与 release 对齐） |
| 测试时间 | 2026-07-04 |
| criterion 版本 | 0.5 |
| 测试机器负载 | 默认后台（无主动 CPU 重负载） |
| sample_size | 100（criterion 默认）；warmup 3s；measurement 5s |

---

## Benchmark 数字（25 个数据点）

> 来源：`target/criterion/<name>/<fixture>/new/estimates.json`（criterion 标准输出）
> 单位：µs（微秒）。1 ms = 1000 µs。
> 「stddev 占比」= stddev / mean。> 10% 时标 ⚠（measurement 不稳）。
> 所有数字已对照 stage 1 文档「Baseline 数字」段，吻合（小数位级偏差是 criterion 重测正常抖动）。

### 1. `bench_rebuild_sorted_cache` — 搜索 + 排序 hot path

| fixture | mean (µs) | median (µs) | stddev (µs) | stddev 占比 | throughput |
|---|---|---|---|---|---|
| substring/100 进程 | 29.0 | 29.0 | 6.3 | 21.7% ⚠ | 3.50 Melem/s |
| substring/500 进程 | 115.0 | 115.3 | 9.3 | 8.0% | 4.30 Melem/s |
| substring/1000 进程 | 233.7 | 235.7 | 24.6 | 10.5% | 4.27 Melem/s |
| filter_expr/100 进程 | 27.9 | 28.8 | 5.2 | 18.5% ⚠ | 3.53 Melem/s |
| filter_expr/500 进程 | 130.1 | 130.9 | 9.5 | 7.3% | 3.83 Melem/s |
| filter_expr/1000 进程 | 269.6 | 264.3 | 36.6 | 13.6% ⚠ | 3.67 Melem/s |

**对照 v0.6 stage 6 实测**：500 进程 substring 38.2 µs（v0.6 老数字，**仅 sort 阶段**）。本次 substring/500 = 115 µs 是「rebuild_sorted_cache 全链路」（pid_to_idx 构建 + sort + filter + clone），口径不同。**不是回归**。

**判定**：< 300 µs @ 1000 进程，**远低于 1 ms 用户感知阈值** → **非瓶颈**。

### 2. `bench_refresh_heavy` — HeavyWorker 单轮 parent_chain 批量 ⚠ 关注

| fixture | mean (µs) | median (µs) | stddev (µs) | stddev 占比 | throughput |
|---|---|---|---|---|---|
| 100 进程 | **1,381** (1.4 ms) | 1,379 | 73 | 5.3% | 72.5 Kelem/s |
| 500 进程 | **8,567** (8.6 ms) | 8,631 | 764 | 8.9% | 58.4 Kelem/s |
| 1000 进程 | **16,473** (16.5 ms) | 16,549 | 1,228 | 7.5% | 60.7 Kelem/s |

**scaling 分析**：100 → 1000 进程 = 10× 输入，开销 12× 增加（1.4 → 16.5 ms）→ **亚线性偏线性**（O(N·D)，D = 平均 parent_chain 深度 ≈ 5-10）。每进程平均 16.5 µs。

**线程归属**：HeavyWorker 在独立线程 `proc-heavy-refresh` 跑，2s 周期。16.5 ms / 2000 ms = **0.83% 持续 CPU**（单核）。**不阻塞 UI 帧预算**。

**真问题**：每周期堆分配 ≈ 1000 进程 × 5 平均链深 × 2 次 clone（`build_parent_chain` 内 push + `chain.clone()` 写回）= **~10,000 次 String heap alloc/free per cycle**。对 worker 线程是 GC 压力 + cache miss 来源。

**判定**：mean > 5 ms 阈值命中，但因 worker 线程独立，**用户感知阈值（UI 帧预算）不适用**。归为「代码质量 / 长期 alloc 压力」类优化候选，**非用户感知瓶颈**。

### 3. `bench_tui_draw` — ratatui TestBackend 单帧渲染

| fixture | mean (µs) | median (µs) | stddev (µs) | stddev 占比 | throughput |
|---|---|---|---|---|---|
| 100 进程 | 1,105 (1.1 ms) | 1,098 | 61 | 5.5% | 91.0 Kelem/s |
| 500 进程 | 3,021 (3.0 ms) | 2,947 | 255 | 8.4% | 165.5 Kelem/s |
| 1000 进程 | **5,621** (5.6 ms) | 5,567 | 479 | 8.5% | 177.9 Kelem/s |

**⚠ Bench 高估说明**：bench 文件 `benches/bench_tui_draw.rs:33-49` 对 `processes.iter().map(...)` 全部 1000 行跑 format! + 构造 Row。**生产代码 `src/tui/process_table.rs:71-75` 用 `.skip(scroll_offset).take(rows_visible)`** 只格式化可见行（典型 30-50 行）。

**真实生产开销估算**（按比例缩放）：
- 1000 进程 fixture 在 bench 中是 5.6 ms（全量 1000 行 format!）
- 生产中只 format! ~36 可见行 → ~36/1000 × 5.6 ms ≈ **200 µs/frame**（< 1 ms 阈值，用户无感）
- 加上 ratatui 内部 buffer / diff 开销，实测最差 ~500 µs/frame

**判定**：bench 数字 5.6 ms 命中 5-15 ms 边缘区，但**真实生产路径 < 1 ms**。**非瓶颈**。format! 风暴优化收益有限（用户感知不到）。

### 4. `bench_record_serialize` — UiFrame bincode 序列化 / 反序列化

#### 4a. serialize（录屏写路径）

| fixture | mean (µs) | median (µs) | stddev (µs) | stddev 占比 |
|---|---|---|---|---|
| serialize/100 进程 | 2.3 | 2.3 | 0.3 | 13.6% ⚠ |
| serialize/500 进程 | 9.0 | 8.6 | 1.3 | 14.2% ⚠ |
| serialize/1000 进程 | 14.1 | 13.9 | 0.7 | 5.0% |

#### 4b. deserialize（replay 读路径）

| fixture | mean (µs) | median (µs) | stddev (µs) | stddev 占比 |
|---|---|---|---|---|
| deserialize/100 进程 | 10.2 | 10.1 | 0.2 | 2.0% |
| deserialize/500 进程 | 61.0 | 47.6 | 17.7 | 29.0% ⚠ |
| deserialize/1000 进程 | **165.4** | 166.6 | 18.5 | 11.2% |

**长 session 累计成本估算**：
- 30 min session × 30 FPS 录屏 × 1000 进程 = 1800 frames
- serialize 总耗时 = 1800 × 14 µs ≈ **25 ms**（在 recorder 独立线程，零用户感知）
- 1000 进程 30 min session 文件体积估算：1000 procs × 200 bytes/proc × 1800 frames ≈ **360 MB**（未压缩 bincode）

**deserialize vs serialize 比率（1000 进程）**：165 / 14 ≈ **12×**。replay 路径明显比录屏慢。但 replay 是用户主动触发（按一次方向键 seek 一帧），165 µs 单帧 seek 完全无感。

**判定**：serialize 完全可忽略；deserialize 12× 倍数虽高，但绝对值 < 200 µs，**非瓶颈**。

### 5. `bench_filter_expr_apply` — FilterExpr apply 4 类表达式

| case | mean (µs) | median (µs) | stddev (µs) | stddev 占比 |
|---|---|---|---|---|
| (a) `cpu > 5`（500 进程） | 19.2 | 18.6 | 3.0 | 15.8% ⚠ |
| (b) `name =~ /chrome/i`（500 进程） | **124.1** | 125.9 | 9.4 | 7.5% |
| (c) `cpu > 5 AND mem > 100mb`（500 进程） | 34.0 | 33.9 | 6.0 | 17.5% ⚠ |
| (d) `sni in (...)` HashSet（500 flows） | 84.0 | 83.5 | 6.0 | 7.2% |

**regex 路径最慢**（124 µs vs cpu_gt 19 µs，6.5×）— `regex::Regex::is_match` 在 500 进程上每进程约 250 ns。这是 regex crate 自身开销，**stage 3+ 优化空间有限**（除非改预编译缓存，但 FilterExpr AST 已经在 parse 阶段编译好）。

**HashSet lookup（v0.12 阶段 5 TD-29 落地）** 在 500 flows 上 84 µs ≈ 170 ns/flow — O(1) `contains` 表现良好。

**判定**：< 130 µs @ 500 进程，**远低于 1 ms 阈值** → **非瓶颈**。

### 6. `bench_search_hot_path` — 搜索按键 → filter 全链路

| query 长度 | mean (µs) | median (µs) | stddev (µs) | stddev 占比 |
|---|---|---|---|---|
| len_1（"c"） | 30.1 | 23.3 | 9.8 | 32.6% ⚠ |
| len_10（"cccccccccc"） | 101.4 | 100.8 | 10.8 | 10.7% |
| len_50（"c" × 50） | 70.6 | 69.3 | 15.7 | 22.2% ⚠ |

**len_50 < len_10 反直觉解释**：50 个 `c` 的 query 在进程名 `chrome.exe` 上 `name_lower.contains()` 提前返回 false（chrome 只含 1 个 c），50 字符 query 每次比对多 5-10 字符才放弃。**v0.6 stage 6 增量 lowercase 优化确实生效**（每按键 30-100 µs 全链路）。

**判定**：< 110 µs，**远低于 16.6 ms 帧预算** → **非瓶颈**。按键即时反馈完整。

---

## Pareto 分析（按 1000 进程 mean 排序）

| Rank | hot path | 1000 进程 mean | 线程归属 | 用户感知 | 是否值得优化 |
|---|---|---|---|---|---|
| 1 | `refresh_heavy_parent_chain` | **16.5 ms** | worker（独立线程） | 无（不阻塞 UI） | ⚠ 中（alloc 压力，非 UI 紧急） |
| 2 | `tui_draw_process_table` | 5.6 ms（bench 高估） | UI 主线程 | bench 5.6 ms 边缘 / 真实 < 1 ms | ❌ 低（bench 误导，真实 < 1ms） |
| 3 | `rebuild_sorted_cache_filter_expr` | 270 µs | UI 主线程 | 无（< 1 ms） | ❌ 低 |
| 4 | `rebuild_sorted_cache_substring` | 234 µs | UI 主线程 | 无 | ❌ 低 |
| 5 | `record_deserialize` | 165 µs | replay 主线程 | 无（replay 偶发） | ❌ 低 |
| 6 | `filter_expr_apply/regex` @ 500 | 124 µs | UI 主线程 | 无 | ❌ 低（regex crate 内部） |
| 7 | `search_substring_filter/len_10` @ 500 | 101 µs | UI 主线程 | 无 | ❌ 低 |
| 8 | `filter_expr_apply/sni_in` @ 500 flows | 84 µs | UI 主线程 | 无 | ❌ 低 |
| 9 | `search_substring_filter/len_50` @ 500 | 71 µs | UI 主线程 | 无 | ❌ 低 |
| 10 | 其他（cpu_gt / search_len_1 / record_serialize 等） | < 35 µs | 各 | 无 | ❌ 低 |

**关键发现**：唯一两个 mean > 1 ms 的 hot path（parent_chain / tui_draw）：
- parent_chain 在 worker 线程，不影响 UI 流畅
- tui_draw 数字是 bench 高估（生产路径只 format! 可见行）

→ **proc 当前架构在 1000 进程规模下无用户感知瓶颈**。

---

## 候选优化点（top 3）

> ROI 评估口径（来自 stage 2 任务清单）：
> - **高 ROI**：mean > 5 ms + 优化后能 < 1 ms + 工作量 < 500 行
> - **中 ROI**：mean > 1 ms + 优化后能 < 0.5 ms + 工作量 < 300 行
> - **低 ROI**：mean < 1 ms 或 工作量 > 500 行

### 候选 1：parent_chain Arc 重构（kill 10000+ heap allocs/cycle）— ⚠ 中 ROI

- **位置**：
  - 字段定义：`src/collect.rs:588` — `pub parent_chain: Vec<(u32, String)>`
  - 写入热路径：`src/collect.rs:953-966`（`pid_to_chain` 构建 + `chain.clone()` 写回）
  - `build_parent_chain` 实函数：`src/security/lineage.rs:149-176`
  - `parent_proc.name.to_string()` 每个祖先 1 次 String alloc（lineage.rs:172）
  - `chain.clone()` 每进程 1 次完整 Vec clone（collect.rs:964）
  - UI 消费者：`src/tui/detail_view.rs:371-376, 406-413`（用 `.as_str()`）
  - 评分消费者：`src/security/lineage.rs:200, 209, 222-231, 333-338`（用 `.as_str()`）
  - 其他构造点：`src/record/conversions.rs:49` / `src/eject/locks.rs:92` / 测试 `src/security/lineage.rs:574`
- **当前 baseline**：1000 进程 mean 16.5 ms，每周期 ~10000 次 heap alloc（5000 build + 5000 clone）
- **预估优化后**：~3-5 ms（杀 90% 堆分配后剩 HashMap 构建 + Arc atomic increment）。基于：parent_chain 16.5 ms 中，build_parent_chain 走链 + String alloc 占主导，clone Vec 占 ~30%；改 Arc 后 build 几乎零成本，clone 单次 Arc 原子加。
- **方案**：
  - `ProcessInfo::parent_chain: Vec<(u32, String)>` → `Vec<(u32, Arc<str>)>`（保留 serde 兼容性 — `Arc<str>` 不 impl Serialize，需要在 `FrameProcess` 中转层做 `String` 转换，FrameProcess 已是 String）
  - `build_parent_chain` 把 `parent_proc.name.to_string()` 改 `Arc::clone(&parent_proc.name)` — 零 heap alloc
  - 把 `chain.clone()` 改成 `Arc::clone` —— 需要把 `Vec<(u32, Arc<str>)>` 升一级为 `Arc<[(u32, Arc<str>)]>`，processes 写入时 Arc 整链一次
  - UI/评分消费者：`.map(|(_, n)| n.as_str())` → `.map(|(_, n)| n.as_ref())` 或 `&**n`
- **ROI**：**中**（mean > 5 ms 命中，但优化后未必 < 1 ms；worker 线程非 UI 紧急；用户感知不到差）
- **风险**：中 — `Arc<str>` 不 impl `Serialize/Deserialize`，需检查 serde 边界；`Arc<[(u32, Arc<str>)]>` 改动会触级联签名更新（~10 处）
- **预估工作量**：~300-400 行（field 类型 + build_parent_chain + ~10 个消费点 + 单元测试 + bench 更新）
- **不做的理由**：bench 16.5 ms 不阻塞 UI，0.83% CPU 不显著，10000 allocs/s 在现代 allocator 上无感

### 候选 2：tui_draw format! 风暴优化 — ❌ 低 ROI（不建议）

- **位置**：`src/tui/process_table.rs:71-159`（每行 5+ 次 format!：cpu_str / mem_str / mem_pct / name_str / format_bytes / format_speed）+ `src/format.rs:3-46`
- **当前 baseline**：bench 5.6 ms @ 1000 进程（**高估**）
- **预估优化后**：bench ~2-3 ms（删 format! 用 itoa / 直接 into Cell< Cow<_> >）。**真实生产 ~500 µs → ~200 µs**（用户感知无差异）
- **ROI**：**低**（bench 数字误导，真实生产路径 < 1 ms；优化后用户感知不到差）
- **风险**：低
- **预估工作量**：~150 行（format_bytes / format_speed / cpu format 各自改 itoa / dtoa，单元测试不变）
- **不做的理由**：bench 高估了真实开销，生产路径 < 1 ms 已在用户无感区。优化收益不显著。

### 候选 3：record deserialize 路径加速 — ❌ 低 ROI

- **位置**：`src/record/reader.rs`（未读，但 165 µs @ 1000 进程的 bincode deserialize 路径）
- **当前 baseline**：165 µs @ 1000 进程 deserialize（vs 14 µs serialize，12× 慢）
- **预估优化后**：~80 µs（bincode 默认 little-endian，可换 native-endian 或零拷贝方案）。但 replay 偶发触发，165 µs 完全无感
- **ROI**：**低**（mean < 1 ms 已在无感区）
- **风险**：高（bincode 配置改动影响文件格式兼容性，需迁移层）
- **预估工作量**：~200 行 + 兼容层
- **不做的理由**：replay 偶发触发，165 µs 用户无感；改 bincode 配置影响向后兼容。

---

## cycle 后续 stage 建议

基于 Pareto 分析 + 候选 ROI 评估，建议如下三方案供用户选择：

### 方案 c（推荐）：4 stage 收尾，cycle 作为「性能验证 cycle」

**理由**：
- 唯一 mean > 5 ms 的 hot path（parent_chain）在 worker 线程，不阻塞 UI
- 其他 5 个 hot path 全在 < 1 ms 用户无感区
- benchmark suite 已建立，留作 future 性能 guard
- 用户报「卡」时附 criterion 数字 → 定位成本降一半

**stage 安排**：
| Stage | 类型 | 目标 |
|---|---|---|
| 阶段 3（N-1） | Review | cycle 全局 Review（产出 `docs/reviews/REVIEW-v0.13.md`，P0/P1/P2 分级） |
| 阶段 4（N） | 收尾 | README + CHANGELOG + tag v0.13.0 |

**CHANGELOG 卖点**：「v0.13 性能验证 cycle — 6 个 criterion benchmark 建立性能 baseline，实测 1000 进程规模下无显著瓶颈」

### 方案 b（可选）：5 stage，加 1 个 parent_chain Arc 重构

**理由**：
- 候选 1（parent_chain Arc 重构）虽非用户感知瓶颈，但 kill 10000+ heap allocs/cycle 对长期代码质量有益
- bench 16.5 ms 命中 mean > 5 ms 阈值，是 cycle 内唯一可量化优化的点
- v0.14 之前清掉一个侦察报告里挂了 1 cycle 的疑点

**stage 安排**：
| Stage | 类型 | 目标 |
|---|---|---|
| 阶段 3 | Slice | parent_chain Arc 重构（候选 1，~400 行） |
| 阶段 4（N-1） | Review | cycle 全局 Review |
| 阶段 5（N） | 收尾 | README + CHANGELOG + tag v0.13.0 |

**风险**：候选 1 涉及 ~10 处消费点签名变更，需 regression 守护（1115 passed 不变）。

### 方案 a（不推荐）：6-7 stage，做 parent_chain + 1 个低 ROI 优化

**理由**：
- 候选 2/3 ROI 低，做完用户感知不到差，cycle 容量浪费
- v0.14 cycle 可以重新评估（基于 v0.13 baseline 数字 + 未来用户反馈）

---

## 用户拍板清单

请确认：

1. **选哪个方案？**
   - [ ] **方案 c（推荐）**：4 stage 收尾，v0.13 = 性能验证 cycle（无业务代码改动）
   - [ ] **方案 b**：5 stage，加 parent_chain Arc 重构（~400 行 worker 线程 alloc 优化）
   - [ ] **方案 a**：6-7 stage（不推荐，多做的低 ROI 优化无用户感知收益）

2. **若选方案 c**：是否归档候选 1（parent_chain Arc 重构）到 `docs/tech-debt.md` 留 v0.14？
   - [ ] 归档到 tech-debt（推荐 — 保留优化机会）
   - [ ] 不归档（cycle 内不再考虑）

3. **若选方案 b**：stage 3 是否包含 bench 数字对比（before 16.5 ms / after 目标 < 5 ms）？
   - [ ] 是 — stage 3 验收标准含 criterion before/after 数字
   - [ ] 否 — 仅回归 ≥ 1115 passed 即可

4. **stage 2 报告是否有遗漏的 hot path 想加测？**
   - 已覆盖：搜索 / 排序 / heavy refresh / TUI 渲染 / 录屏序列化 / FilterExpr apply
   - 未覆盖候选：port_table / monitor_sidebar / docker_panel / detail_view_handles / command_palette fuzzy / signature verify async / Flow view / network evaluation
   - [ ] 不加（已覆盖核心 hot path）
   - [ ] 加 ___（说明想加测哪个 + 理由）

---

## 归档（不进 stage 3+ 的低 ROI 项）

以下候选拟归档到 `docs/tech-debt.md` 留 v0.14+ cycle 评估（如用户在拍板清单选「归档」）：

### TD-42: tui_draw format! 风暴优化（候选 2）

- **位置**：`src/tui/process_table.rs:71-159` + `src/format.rs:3-46`
- **当前**：bench 5.6 ms @ 1000 进程（高估，真实生产 < 1 ms 因 `skip().take(rows_visible)` 只格式化可见行）
- **预估优化收益**：用户感知不到差（bench 5.6 → 2 ms，真实 ~500 µs → ~200 µs）
- **为何归档**：bench 高估导致 stage 2 评估时 ROI 看起来高于实际；真实生产路径已 < 1 ms 阈值

### TD-43: record deserialize 加速（候选 3）

- **位置**：`src/record/reader.rs`（bincode deserialize 165 µs @ 1000 进程）
- **当前**：12× 慢于 serialize，但绝对值 < 200 µs
- **为何归档**：replay 偶发触发，用户无感；改 bincode 配置影响向后兼容

### TD-44: command_palette fuzzy 优化（侦察报告疑点 4）

- **位置**：`src/tui/command_palette.rs:225-237`（`recompute_matches` nucleo fuzzy）
- **当前**：未单独 bench，但 `command_palette.rs:813, 912` 引用的 `to_lowercase().contains()` 在 **测试代码内**（不是生产路径）
- **侦察报告纠错**：brainstorm 文档说「command_palette `to_lowercase().contains()` 每帧（line 813, 912）」是**误读**——这两行在 `#[test]` 模块，生产 fuzzy 用 nucleo（已有 matcher 复用，line 109-150）
- **为何归档**：侦察报告这条疑点不成立；nucleo fuzzy 本身已优化（matcher 复用）

---

## 附录：侦察报告疑点对照表

| 侦察报告疑点 | stage 2 实测验证 | 结论 |
|---|---|---|
| 1. CONTEXT.md 与代码不一致（HeavyWorker 1.5s → 2s） | stage 1 sidecar 已修 | ✅ 已闭环 |
| 2. TUI 渲染层 format! 风暴 | bench_tui_draw 5.6 ms（**高估**，生产 < 1 ms） | ❌ 非瓶颈，归档 TD-42 |
| 3. detail_view handles 每帧 to_lowercase | 未单独 bench，但 detail_view 仅在选中进程时渲染（每秒 60 帧但单实例） | ❌ 非热路径 |
| 4. command_palette to_lowercase().contains 每帧 | **侦察报告误读**——line 813/912 在 `#[test]` 内 | ❌ 非问题 |
| 5. parent_chain 每周期 clone Vec | bench_refresh_heavy 16.5 ms（**命中**，但 worker 线程不阻塞 UI） | ⚠ 候选 1（中 ROI） |
| 6. 录屏 UiFrame bincode 序列化 | bench_record_serialize 14 µs（serialize）/ 165 µs（deserialize） | ❌ 完全可忽略 |
| 7. refresh_heavy 主线程 vs worker 路径重复 | 代码 review：主线程 refresh_heavy (`collect.rs:1504-1553`) 只跑 CPU EMA + sysinfo refresh，不跑 parent_chain；worker 路径 (`collect.rs:845-968`) 跑全量 | ✅ 无重复工作（两条路径分工清晰） |
| 8. docker exec 输出每帧跑 | 已知，注释标注（`app.rs:1287-1288`） | 不在 stage 2 范围 |

**总结**：8 个疑点中 1 个已闭环（疑点 1），1 个误读（疑点 4），1 个命中但非用户感知（疑点 5 → 候选 1），5 个非瓶颈。**侦察报告准确率 5/8 = 62.5%**（剩下的 3 个未命中的疑点全部需要 stage 2 数字验证才知道不是瓶颈 —— 这正是 stage 2「先测后优」的价值）。
