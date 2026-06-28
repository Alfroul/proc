# ADR-0013: PSI monitoring (Linux 4.20+) via hand-rolled parser

## Status

**Accepted** — v0.7.0 阶段 6 引入

## Context

Linux 4.20+ 暴露 `/proc/pressure/{cpu,mem,io}`（Pressure Stall Information），是判断"系统真卡了"的金标准：

```
some avg10=2.30 avg60=1.10 avg300=0.50 total=681245
full avg10=0.50 avg60=0.10 avg300=0.00 total=150000
```

- `some`：至少一些任务被 stall 的时间占比
- `full`：所有非 idle 任务都被 stall 的时间占比（CPU 真的在等）
- `avg10/60/300`：10 秒 / 60 秒 / 300 秒滑动平均（百分比）
- `total`：累计 stall 微秒数

比 load average 准：load average 是 1/5/15 分钟指数移动平均，无法捕捉短期 stall；PSI 直接给当前压力。

v0.6 proc 没有读 PSI，监控面板缺少关键的"系统是否在 thrashing"指标。bottom / btop / glances 都有 PSI 显示。

## Decision

**手写 parser 读 `/proc/pressure/{cpu,mem,io}`（不引 `psi` crate / 不引 `procfs-core`），复用 v0.6 `LightWorker` 1s tick 采集，Linux only cfg-gate。**

具体决策：

1. **手写 parser**（~50 行）：
   ```rust
   fn read_psi_file(path: &str) -> Option<(PsiRecord, Option<PsiRecord>)> {
       let content = std::fs::read_to_string(path).ok()?;
       let mut lines = content.lines();
       let some = parse_record(lines.next()?, "some")?;
       let full = lines.next().and_then(|l| parse_record(l, "full").ok());
       Some((some, full))
   }
   ```
   - 理由：parser 简单（4 个 key=value），依赖 crate 也得 ~50 行
   - 不引依赖 = 编译时间不增加 / Cargo.lock 不膨胀

2. **复用 LightWorker（不新建 worker）**：
   - LightWorker 是 1s tick（v0.6 已有）
   - PSI 1s 周期足够（avg10 字段就是 10s 平均）
   - 不像 SMART（30s）需要独立 worker

3. **Linux cfg-gate**：
   ```rust
   #[cfg(target_os = "linux")]
   pub fn read_psi() -> Option<PsiStats> { /* 实装 */ }

   #[cfg(not(target_os = "linux"))]
   pub fn read_psi() -> Option<PsiStats> { None }
   ```
   - Windows / macOS 编译 stub，监控面板降级显示 "PSI: Linux 4.20+ only"

4. **5 个新 alert 规则**：
   ```rust
   pub enum Metric {
       // ...
       CpuPressureSome,
       MemPressureSome,
       MemPressureFull,
       IoPressureSome,
       IoPressureFull,
   }
   ```
   - 默认规则（无需配置即生效）：`MemPressureFull > 20% → Warning`
   - 用户可在 `alerts.toml` 自定义阈值

5. **CPU 只有 some 没 full**：
   - 内核设计：CPU `full` 没有意义（"所有任务都在等待 CPU" 等于 "CPU 空闲"，矛盾）
   - `PsiStats::cpu_full` 字段保留为 None（不是 0），让 UI 区分

6. **不读 cgroup v2 PSI**（第一版）：
   - `/sys/fs/cgroup/cpu.pressure` 等是 cgroup 级 PSI，对单机进程管理器意义不大
   - v0.8.0+ 可加（如果要监控容器内压力）

## Alternatives Considered

### A. 引 `psi` crate

**否决理由**：
- psi crate 包含 trigger monitor（PSI trigger API），proc 用不上
- 编译时间增加（虽然小，但累积）
- 手写 50 行更可控

### B. 引 `procfs-core`

**否决理由**：
- procfs-core 是通用 /proc parser，拖大量额外功能（cpuinfo / meminfo / mounts ...）
- proc 已经依赖 sysinfo（功能重叠）
- PSI 解析只是 procfs-core 的 `pressure.rs` 一个文件，单独引整个 crate 过重

### C. 用 aya-rs 挂 PSI trigger（kernel event-driven）

**否决理由**：
- PSI trigger 是 poll-based（fd 可读时触发），不是 stream
- proc 的 1s tick 已经足够，trigger 模式增加复杂度
- 与 v0.7 阶段 8 eBPF feature flag 解耦

### D. 不做 PSI（沿用 v0.6 load average）

**否决理由**：
- bottom / btop / glances 都有 PSI
- PSI 是"判断真卡"的金标准，load average 不准
- Linux 4.20+ 普及率高（2019 年内核），降级路径足够

### E. 读 cgroup v2 PSI（containers）

**否决理由**：
- 单机进程管理器场景下，全局 PSI 已足够
- 容器 PSI 留 v0.8.0+（如要监控 docker 容器内压力）

## Consequences

### 正面

- **判断"真卡了"的金标准**：比 load average 准
- **零新依赖**：手写 parser，不增 Cargo.lock
- **Linux 跨版本兼容**：4.20+ 全支持（2019 年以后）
- **alert 体系扩展**：5 个新 Metric 类型，与 v0.6 alerts.toml 兼容

### 负面

- **Linux only**：Windows / macOS 用户没 PSI（降级提示）
- **kernel 编译选项**：极少数发行版禁用 PSI（CONFIG_PSI=n），read_psi 返回 None
- **手写 parser 维护**：50 行 + 5 行测试，可控

### 缓解

- Linux CI 跑测试验证 parser 正确性
- 降级路径明确：`read_psi() == None` 时 UI 显示 "Linux 4.20+ only"
- kernel 禁用 PSI 的情况靠 stub fallback 自然降级

## Implementation Notes

- 入口：`src/psi.rs::read_psi() -> Option<PsiStats>`
- Worker 集成：`src/collect.rs::LightWorker` 加 PSI 采集（1s tick）
- App 集成：`src/app.rs::App::psi_stats: Option<PsiStats>`
- UI：`src/tui/sidebar.rs` 或 `right_panel.rs` 加 PSI 段
- Alert：`src/alert/rule.rs::Metric` 加 5 个变体 + 默认规则
- 测试：`tests/test_psi.rs`（Linux cfg-gate 测试 + alert 规则触发）

## Example Output

监控面板：

```
Pressure:
  CPU  some avg10=2.3% avg60=1.1% avg300=0.5%
  MEM  some avg10=0.1% avg60=0.0% avg300=0.0%
       full avg10=0.0% avg60=0.0% avg300=0.0%
  IO   some avg10=0.5% avg60=0.2% avg300=0.1%
       full avg10=0.2% avg60=0.1% avg300=0.0%
```

颜色分级：
- avg10 < 5% → 绿
- 5-20% → 黄
- 20-50% → 橙
- > 50% → 红

## References

- [PSI - Linux Kernel docs](https://docs.kernel.org/accounting/psi.html)
- [psi Rust crate](https://docs.rs/psi)（参考否定）
- [procfs-core pressure.rs](https://docs.rs/procfs-core/latest/src/procfs_core/pressure.rs.html)（参考否定）
- proc v0.6.0 `src/collect.rs::LightWorker`（复用 worker）
- proc v0.6.0 `src/alert/rule.rs::Metric`（alert 扩展点）
