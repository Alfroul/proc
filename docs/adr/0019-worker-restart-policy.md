# ADR-0019: Worker Restart Policy — 指数退避 + 最大重试 + reset 计数

- **Status**: Accepted
- **Date**: 2026-06-29
- **Phase**: v0.11.0 阶段 1

## 背景

自 v0.6.0 阶段 3 起，所有 `SnapshotWorker<T>` 主循环外包 `std::panic::catch_unwind`，panic 时通过 `crash_tx` 把 `WorkerCrash` 推给主线程渲染红色 banner。但 **TD-4 长期挂账**：catch_unwind 截获 panic 后 worker 线程退出，主线程只是显示 banner，**没有任何热恢复路径**——用户必须手动重启 proc 才能让该 worker 重新工作（如 port_worker 死亡 → 端口面板永远不刷新；dns_log_worker 死亡 → DNS 视图停止滚动）。

`WorkerManager::restart(name)` 在 v0.6.0 阶段 5 落地时被显式留空（注释「无调用方，surgical 原则不预实现」），v0.7-v0.10 一直推迟。v0.11 cycle 把这条债清掉。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| **A. 立即重启（无退避）** | 最简实现，~30 行代码 | panic loop 风险：worker 持续 panic 时 spawn→crash→spawn 死循环，CPU 100% + crash report 文件爆炸 |
| **B. 永久死亡（v0.10 行为）** | 当前实现，零改动 | 用户必须手动重启 proc；单 worker 故障影响整个工具可用性 |
| **C. 用户手动重启按钮** | 用户可控 | 侵入：用户得在场；管理员场景 proc 跑在后台没人看，banner 显示 1 周也没人按按钮 |
| **D. 指数退避（5s / 30s / 5min）+ 最大重试 3 + reset 计数** | 自动恢复常见瞬时 panic（sysinfo 短暂失败 / 网络瞬时问题）；持续 panic 时及时止损（5min × 3 次 ≈ 15min 后永久失败）；长时间稳定运行后给 worker 重新机会 | 实现复杂度比 A 高 ~150 行；reset 窗口（1h）的语义需在 banner 透明展示 |

## 决策

**选 D**。具体策略：

1. **指数退避**：
   - `retry_count == 0` → 等待 **5s** 后第一次 respawn
   - `retry_count == 1` → 等待 **30s** 后第二次 respawn
   - `retry_count == 2` → 等待 **5min（300s）** 后第三次 respawn
   - `retry_count >= 3` → **永久失败**（不再 respawn）

   backoff 函数 `backoff_for(retry_count)` 是纯函数，便于单元测试。

2. **最大重试 3 次**：3 次 respawn 后 worker 仍 panic 视为「该 worker 在当前会话不可恢复」，避免无意义 spawn 循环。用户重启 proc 后 retry_count 从 0 开始。

3. **reset 计数（1h 无 panic 后归零）**：上次成功 spawn 距今 ≥ 1h 且 retry_count > 0 时，重置 `retry_count = 0`。设计意图：worker 偶发 panic 后恢复稳定运行，下一次 panic 不应受历史累计惩罚。

4. **状态机**：
   ```rust
   pub struct RestartState {
       pub retry_count: u32,           // 已成功 spawn 的次数（达到 MAX_RETRIES=3 永久失败）
       pub last_crash: Option<SystemTime>,   // 最近一次 panic 时刻（backoff 起算点）
       pub last_restart: Option<SystemTime>, // 最近一次 respawn 时刻（reset 起算点）
       pub last_reset: SystemTime,           // 最近一次 retry_count 归零时刻
   }
   ```

5. **WorkerManager API**：
   - `restart(&mut self, name: &str, now: SystemTime, crash_tx) -> bool`：crash 发生时调，记录 `last_crash`，backoff 窗口未到返回 false，到期则 respawn + retry_count+=1 + 返回 true。
   - `restart_tick(&mut self, now: SystemTime, crash_tx) -> Vec<&'static str>`：每 1s 调一次，遍历 `restart_history` 中 `last_crash.is_some()` 的项尝试 respawn。
   - `restart_status(&self, name: &str, now: SystemTime) -> RestartStatus`：banner 渲染查此方法得到 `Healthy / Restarting { retry_count, remaining_secs } / Restarted { retry_count, elapsed_secs } / PermanentFailure { retry_count }`。

6. **worker 名字映射**：`WorkerCrash.worker` 字段是 `SnapshotWorker::spawn(thread_name, ...)` 传入的 thread_name（如 `"port-snapshot-worker"` / `"dns-log-worker"`），与 `WorkerManager` 字段名（`port_worker` / `dns_log_worker`）不同。`spawn_one(name)` 内部 match thread_name 字面量 → 调对应 `crate::xxx::spawn(crash_tx)` 入口。

7. **不实装 ebpf_worker restart**：Linux-only 路径（feature flag `ebpf`），v0.11 cycle 不动 ebpf（TD-19 推迟范围）。ebpf_worker panic 后 banner 显示但状态保持 Healthy（无 restart_history 条目）。

8. **不实装 docker worker restart：DockerPanel 自管**（v0.25 stage 1 TD-25 追加）：`canonical_worker_thread_name` 列表（port / usb / net-flow / dns-log / disk-io-etw / schannel-etw）**不含** docker-snapshot-worker / docker-logs-worker-{name}。docker worker 由 DockerPanel 自管生命周期（面板进入时独立 spawn、退出时 drop），worker handle 不在 `WorkerManager` 字段内——docker worker panic 时 `WorkerManager::restart` 因 canonical 返回 None 直接返 false，不自动 respawn。这是设计选择非遗漏：接入 restart 需重构 DockerPanel 把 worker handle 暴露给 WorkerManager（影响面大、收益窄——docker 子系统可用性由面板自身的 spawn/drop 逻辑兜底，重进面板即重建）。

## 后果

### 正面

- **TD-4 清账**：v0.6.0 起长期挂账的「worker panic 后无热恢复」彻底解决
- **常见瞬时 panic 自愈**：sysinfo 短暂失败 / DNS PowerShell 偶发 timeout / ETW session 瞬时占用 等场景 worker 自动恢复，用户无感知
- **持续 panic 止损**：5min × 3 = 15min 内 respawn 3 次都失败则永久死亡，避免 panic loop 拖垮系统
- **长跑友好**：reset 窗口让稳定运行的 worker 不被历史累计惩罚
- **banner 透明**：用户能看到「restarting in 30s」/「restarted (retry #2)」/「permanent failure」三种状态，知道该不该手动重启 proc

### 负面

- **重启期间 worker 缺数据**：port_worker 重启 5s 期间端口面板不刷新；dns_log_worker 重启 30s 期间 DNS 视图停止滚动；schannel_etw_worker 重启期间 Schannel flow 不更新。banner 已显式提示。
- **3 次失败后永久死亡 + banner 持续红色**：用户必须看到 banner 才知道要重启 proc；持续 panic 的 worker 触发条件由用户决定是否值得手动介入。
- **额外字段**：`WorkerManager.restart_history: HashMap<&'static str, RestartState>` 增加少量内存（每个 worker ~40 bytes，最多 6 个 worker）。
- **测试覆盖成本**：单元测试需要 mock SystemTime（用 SystemTime::now + 比较），集成测试需要管理员权限 + 真实 panic 注入。

### 缓解

- 单元测试覆盖 RestartState 状态机（pure logic，无 IO）：backoff_for / reset 窗口 / MAX_RETRIES 边界。
- 集成测试 admin-only：spawn proc → 注入 worker panic → 验证 banner 显示 restarting → 5s 后 worker 恢复（try_recv_latest 有新数据）。
- banner 三态渲染让用户透明感知重启进度，避免「不知道发生了什么」。

## 实现 Notes

- `src/workers/manager.rs`：加 `restart_history` 字段 + `RestartState` + `RestartStatus` enum + 4 个方法
- `src/app.rs`：
  - `tick_poll_crashes` 升级：drain `crash_rx` 时除了 push 到 `active_crashes`，同时调 `workers.restart(crash.worker, now, crash_tx)`
  - `tick_light_refresh` 或 `tick()` 1s 节奏调用 `workers.restart_tick(now, crash_tx)`
  - **关键**：crash_tx 不能从 WorkerManager 自己拿（它没存），App::new 创建的 `crash_tx` 需要在 App 字段保留一份（或从 channel 再 clone）—— 实际上 App::new 创建 `let (crash_tx, crash_rx) = channel()`，crash_tx 之前只 clone 给 WorkerManager::new 后就 drop；改为 App 持有 `crash_tx: Sender<WorkerCrash>` 用于 restart 路径
- `src/tui/layout.rs::draw_crash_banner`：每条 crash 多查一次 `workers.restart_status(name, now)`，根据状态选择文案 + 颜色
- `tests/test_worker_restart.rs`：单元测试 RestartState 状态机（pure）+ 退避窗口 + MAX_RETRIES + reset

## 参考

- [TD-4](../tech-debt.md) — CONTEXT.md 显眼标注 WorkerManager::restart 未实现（v0.7.0 阶段 1 标 Fixed 表示文档标注落地，本 ADR 才是真正实装）
- [CONTEXT.md 「后台 worker」段](../../CONTEXT.md) — WorkerManager 术语
- `src/metrics/crash.rs::WorkerCrash` — panic 通知结构
- `src/worker.rs::SnapshotWorker::spawn(thread_name, ...)` — thread_name 与 WorkerManager 字段名映射来源
