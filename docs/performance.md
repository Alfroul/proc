# proc 性能实测（performance.md）

> **口径声明**：本文全部数字转写自 **v0.26 stage 1 Spike 的 criterion 实测**（2026-08-31，原始输出 `/tmp/bench1-6.log` 逐行转写，非记忆值），对照列为 [PERF-BASELINE-v0.13](reviews/PERF-BASELINE-v0.13.md)（2026-07-04 **同机**实测 mean）。机器：13th Gen Intel i7-13700HX / 16GB RAM / Windows 11 / rustc 1.95.0。criterion 默认参数（sample_size=100 / warmup 3s / measurement 5s），6 个 bench 文件逐文件跑（分批纪律，不整包）。
>
> 本地复现：`cargo bench`（或单跑 `cargo bench --bench bench_refresh_heavy`）。**不在 CI 跑**——criterion 在 GitHub Actions 共享 runner 抖动大，仅本地手跑。

## 总结论

- **25 数据点全无回归**（v0.13 → v0.26 对照，最差 +7% 为 deserialize/100，同量级噪声）
- **显著提升项**：refresh_heavy 1000 进程 **2.9×**（16.47 → 5.69 ms，归因见下）；FilterExpr apply ~2× / 搜索 hot path ~2× / 反序列化 1.9× / TUI 渲染 1.2-1.5×
- 1000 进程规模下全部核心 hot path < 6 ms；搜索 + 排序全链路 235 µs，远低于 1 ms 感知阈值

## 1. bench_rebuild_sorted_cache（搜索按键 → 过滤 + 排序全链路）

| fixture | v0.26 mean (µs) | v0.13 mean (µs) | 变化 |
|---|---|---|---|
| substring/100 | 27.5 | 29.0 | -5% |
| substring/500 | 113.4 | 115.0 | -1% |
| substring/1000 | 235.4 | 233.7 | +1% |
| filter_expr/100 | 25.2 | 27.9 | -10% |
| filter_expr/500 | 127.8 | 130.1 | -2% |
| filter_expr/1000 | 255.5 | 269.6 | -5% |

全 fixture ±10% 内同量级，无回归。1000 进程 235 µs 远低于 1 ms 感知阈值（v0.13 结论延续）。

## 2. bench_refresh_heavy（HeavyWorker 单轮 parent_chain 批量）

| fixture | v0.26 mean | v0.13 mean | 变化 |
|---|---|---|---|
| 100 进程 | 698 µs | 1,381 µs | **-49%** |
| 500 进程 | 4.13 ms | 8.57 ms | **-52%** |
| 1000 进程 | **5.69 ms** | 16.47 ms | **-66%（2.9×）** |

**归因（已溯源）**：commit [`4c7e294`](https://github.com/Alfroul/proc/commit/4c7e294)（v0.17 stage 2，TD-47）——`parent_chain: Vec<(u32, Arc<str>>)` 重构消除单轮 ~10,000 次 String heap alloc（PERF-BASELINE-v0.13 候选 1 的落地，当时判中 ROI，v0.17 cycle 实装）。

## 3. bench_tui_draw（ratatui TestBackend 单帧渲染 + format_bytes 对照）

| fixture | v0.26 mean | v0.13 mean | 变化 |
|---|---|---|---|
| tui_draw/100 | 894 µs | 1,105 µs | -19% |
| tui_draw/500 | 2.08 ms | 3.02 ms | -31% |
| tui_draw/1000 | 3.66 ms | 5.62 ms | -35% |
| format_bytes itoa / std_format | 104.6 ns / 105.1 ns | （v0.13 报告未单列） | 两者无显著差异 |

bench 高估说明延续：bench 全量 `format!` 1000 行，生产 `.skip().take()` 只格式化可见 ~36 行 → 真实 < 500 µs/frame。

## 4. bench_record_serialize（UiFrame bincode 序列化 / 反序列化）

| fixture | v0.26 mean | v0.13 mean | 变化 |
|---|---|---|---|
| serialize/100 | 1.34 µs | 2.3 µs | -42% |
| serialize/500 | 7.31 µs | 9.0 µs | -19% |
| serialize/1000 | 14.6 µs | 14.1 µs | +4% |
| deserialize/100 | 10.9 µs | 10.2 µs | +7% |
| deserialize/500 | 46.8 µs | 61.0 µs | -23% |
| deserialize/1000 | **87.7 µs** | 165.4 µs | **-47%（1.9×）** |

serialize 全线 ≤ v0.13；deserialize/1000 快 1.9×（单帧 seek 88 µs 无感结论不变）。

## 5. bench_filter_expr_apply（FilterExpr apply，500 进程 / 500 flows）

| case | v0.26 mean | v0.13 mean | 变化 |
|---|---|---|---|
| `cpu > 5` | 10.4 µs | 19.2 µs | -46% |
| `name =~ /chrome/i` | 61.6 µs | 124.1 µs | **-50%** |
| `cpu > 5 AND mem > 100mb` | 14.9 µs | 34.0 µs | -56% |
| `sni in (...)` | 48.6 µs | 84.0 µs | -42% |

4 case 全部 ~2× 提升。regex 路径仍最慢（61.6 vs 10.4，6× 比）——相对结论与 v0.13 一致。

## 6. bench_search_hot_path（搜索按键 → filter 全链路）

| query | v0.26 mean | v0.13 mean | 变化 |
|---|---|---|---|
| len_1 | 23.6 µs | 30.1 µs | -22% |
| len_10 | 55.8 µs | 101.4 µs | -45% |
| len_50 | 34.2 µs | 70.6 µs | -52% |

len_50 < len_10 反直觉现象延续（提前放弃机制）。

## 提升归因说明（宁缺毋滥）

**只写已溯源项**：refresh_heavy 2.9× → TD-47 `4c7e294`（v0.17 stage 2，上表 2 节内注）。

其余提升（FilterExpr apply ~2× / search ~2× / deserialize 1.9× / tui_draw 1.2-1.5×）**不写归因**——两代实测跨越 v0.13 → v0.26 十二个 release（依赖升级 / 编译器 1.95 全重编译 / 多次代码变更综合作用），未做单变量归因实验的数字不做因果断言。

## 启动时间 / 内存（2026-09-01 实测）

**方法**（v0.26 stage 3 会话，同机同工具链，release build）：

- **CLI 冷数据路径**：`proc ls --limit 5` wall time（进程 spawn + sysinfo 全量进程首轮采集 + 表格渲染 + 退出，bash `date +%s%N` 差值）——3 次实测 **2291 / 2527 / 2875 ms**（约 2.3-2.9 s，均为热文件系统缓存下的热启动；首轮采集含全部进程的 CPU/内存/路径扫描）
- **稳态内存**：`proc record --no-tui --duration 8`（headless 录屏模式，全套 worker 体系在跑：Light/Heavy/Smart + DNS/NetFlow 等），运行期间每 300ms 采样 WorkingSet64 取峰值——**56.7 MB**

**口径注记**：`proc ls` 的 2.3-2.9 s 是「启动 + 首轮全量采集」合并口径（数据首次可见的时间），非纯进程启动；TUI 路径是渐进填充（首帧先渲染、数据 1-2 s 内到达，主线程 50ms tick 不阻塞）。56.7 MB 是录屏模式含 VT100 帧缓冲的峰值；纯 CLI 单命令路径更低。两项均为单机单次实测样本，供量级参考非基准（无对照基线，v0.13 报告未含启动/内存项）。

## 数据溯源

- 本文 v0.26 列 = [`docs/stages/v0.26-stage-1.md`](stages/v0.26-stage-1.md) 附录 A（criterion 输出逐行转写）
- 对照列 = [`docs/reviews/PERF-BASELINE-v0.13.md`](reviews/PERF-BASELINE-v0.13.md)（2026-07-04 同机，25 数据点）
- bench 代码：`benches/`（6 文件）；跑法与「不在 CI 跑」口径见 README「Benchmark」段
