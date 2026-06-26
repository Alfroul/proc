# 技术债归档 — v0.6.0 Review 产出

> 源：`docs/reviews/REVIEW-7.md` P2 段（14 项）。按 v0.7.0 / v0.8.0+ 分组。
> 与 P0/P1 区分：P0/P1 阻断 v0.6.0 发布，**必须**在阶段 8 修；P2 不阻断，归档到下个 cycle。

---

## v0.7.0 候选（11 项）

### TD-1（P2-1）：清理文档中错误的 `--tb=no` 测试参数

**位置**：`CONTRIBUTING.md:28` / `plan.md:209` / `docs/stages/stage-{2,3,4,5,6,7}.md` 多处
**现状**：文档把 `cargo test --release --tb=no -q` 当作命令。`--tb=no` 是 pytest 参数，cargo test 不认（实测报错 `unexpected argument '--tb'`）。
**影响**：贡献者按文档跑命令会失败；多个 stage-N.md 复制粘贴此错误。
**修复**：全文 grep `--tb=no` 删除（保留 `cargo test --release -q`）。
**验证**：`grep -rn "--tb=no" .` 应无结果。

### TD-2（P2-2）：stage-6.md 标注任务 3-5 推迟

**位置**：`docs/stages/stage-6.md:5`
**现状**：第 5 行「目标」仍写「引入 proptest + criterion + Linux stub 测试」，但 CHANGELOG 第 51-53 行已明确"任务 3-5 不在本 slice 范围"。
**影响**：未来会话读 stage-6.md 误以为任务 3-5 应在 v0.6.0 落地。
**修复**：stage-6.md 头部加"⚠ v0.6.0 实际只做了任务 1/2（键位修复），任务 3-5（proptest/criterion/Linux stub）推迟到 v0.7.0+ — 见 CHANGELOG"。
**验证**：stage-6.md 头部包含推迟声明。

### TD-3（P2-3）：stage-7.md 切片 E 假设错误

**位置**：`docs/stages/stage-7.md:59-60`
**现状**：第 59-60 行问"proptest 是否覆盖了真实的 panic 路径" / "criterion bench 的 mock_process 是否反映真实数据分布"。项目从未引入这两个工具。
**影响**：阶段 7 review 时按不存在的工具提问，浪费精力。
**修复**：stage-7.md 切片 E 改问"是否需要引入 proptest（v0.7.0+ 评估）" / "性能回归测试是否覆盖 hot path"。
**验证**：stage-7.md 切片 E 不再假设 proptest/criterion 存在。

### TD-4（P2-4）：CONTEXT.md 显眼标注 WorkerManager::restart 未实现

**位置**：`CONTEXT.md:55`
**现状**：第 55 行写"restart(name) 故障恢复方法尚未实现（无调用方，surgical 原则不预实现）"，但藏在表格里不够显眼。
**影响**：用户/开发者读 CONTEXT.md 后误以为 worker 崩溃可热恢复。
**修复**：CONTEXT.md 顶部加"已知限制"段或 ⚠️ 标注；README 平台支持表加"worker 崩溃后只能重启 proc"。
**验证**：CONTEXT.md 顶部"已知限制"段含 restart 条目。

### TD-5（P2-5）：WorkerManager 含 Docker worker metrics

**位置**：`src/workers/manager.rs:7-9` / `src/cli/diag.rs:25`
**现状**：Docker logs worker 由 `DockerPanel` 自管（生命周期与 panel 绑定），`metrics_snapshot()` 不含它。`proc diag` 输出缺 Docker 行。
**影响**：用户报 bug 时附上的 diag 缺 Docker worker 数据。
**修复**：让 `DockerPanel` 实现 `metrics()` 接口（或暴露 `Arc<WorkerMetrics>`），App::worker_metrics 追加 Docker 行。注意 DockerPanel 可能 spawn 多个 logs worker（每容器一个），需聚合。
**验证**：`proc diag --json` 输出含 `{"name": "docker_logs_*", ...}`。

### TD-6（P2-6）：App 进一步拆 5 个 panel controller

**位置**：`src/app.rs`（1707 行 / 40+ 字段）
**现状**：v0.6.0 阶段 5 已拆出 InspectorController / ReplayController / WorkerManager（共 15 字段）。剩余 5 个 panel 字段（process_panel / port_panel / usb_panel / monitor_panel / docker_panel）仍直接持在 App。
**影响**：App 仍是协调器 + 部分状态混合；新功能加在 panel 时 App 字段会再膨胀。
**修复**：v0.7.0+ 评估把 5 个 panel 拆出对应 controller（PortPanelController / UsbPanelController / MonitorPanelController / DockerPanelController / ProcessPanelController），App 只持 controller 引用 + 全局状态（mode/snapshot/should_quit/...）。
**验证**：App 字段数从 40+ 降到 < 20；App::handle_key 简化为 dispatch。

### TD-7（P2-7）：test_stage8_perf_regress.rs 改名

**位置**：`tests/test_stage8_perf_regress.rs`
**现状**：文件注释写"Stage-8 一次性性能回归基线"，但实际是 stage-4 落地时一起写的（ProcessInfo 字段对齐）。当前在 stage-7，stage-8 还没开始。
**影响**：未来 stage-8 会话看到这文件以为已开始 / 误判进度。
**修复**：改名 `test_perf_baseline.rs`（或 `test_stage4_perf_regress.rs`）；注释更新。
**验证**：`grep -rn "stage8_perf" .` 应只在 git history / 旧引用出现。

### TD-8（P2-8）：help_panel Workers 区段自适应列宽

**位置**：`src/tui/help_panel.rs:156-178`
**现状**：硬编码列宽 `name<10` `avg>5μs` `max>5μs` `polls>7` `drops>5`。worker 名 > 10 字符（如 `dns_log_worker` = 14）破坏对齐；终端窄时 `Paragraph::wrap(Wrap{trim:false})` 让整行软换行打乱表格。
**影响**：worker 名长时表格视觉错乱。
**修复**：worker 名 truncate 到 10 字符 + ellipsis；或改用 `Table` widget 替代 `Paragraph` + 手工对齐。
**验证**：worker 名 14 字符时表格列对齐无错位。

### TD-9（P2-9）：SearchState 增量 lowercase append

**位置**：`src/search.rs:50-56`
**现状**：每次按键 `self.query_lower = self.query.to_lowercase()` 整体重算。注释承认"保持简单"未做增量。
**影响**：query < 64 字符时差异可忽略（μs 级）；stage-4 性能优化目标未彻底达成。
**修复**：`Char(c)` 时 `query_lower.push(c.to_ascii_lowercase())`（c 已经是 char，O(1)）；`Backspace` 时 `query_lower.pop()`。Unicode 大小写映射复杂的字符（如 `İ` → `i̇`）走整体重算 fallback。
**验证**：criterion bench（如 v0.7.0 引入）显示 query=64 时 lowercase 耗时降一个数量级。

### TD-10（P2-10）：DNS PowerShell probe 也走 restricted_spawn

**位置**：`src/dns_log/windows_dns.rs:238`
**现状**：probe `Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"])` 未走 restricted_spawn。
**影响**：probe 不接受外部数据，注入风险低；但与 spawn 路径不一致 — elevated 时仍持 SeDebugPrivilege。
**修复**：probe 改用 `spawn_with_reduced_privileges` + `.wait()` 替代 `.status()`。
**验证**：`grep -n 'Command::new("powershell' src/dns_log/` 应只在 restricted_spawn fallback 路径出现。

### TD-11（P2-11）：评估 monitor/watchdog spawn 是否走 restricted_spawn

**位置**：`src/monitor/watchdog.rs:87`
**现状**：`Command::new(&cmd)` 用户配置命令直接 spawn，未走 restricted_spawn。
**影响**：用户主动配置的 watchdog 命令，威胁模型不同于 DNS PowerShell（无 -Command 注入风险）；但 elevated 时仍持 SeDebugPrivilege。
**修复**：v0.7.0+ 评估是否对所有 watchdog spawn 走 restricted_spawn（可能破坏用户依赖 elevated token 的自定义命令 — 需 config 选项 "inherit_privileges"）。
**验证**：决策落地后 — 文档说明 watchdog spawn 的权限模型。

---

## v0.8.0+ 候选（3 项）

### TD-12（P2-12）：Linux stub 测试覆盖增强

**位置**：`tests/` 下仅 `test_inspector.rs` / `test_platform_compat.rs` 2 个文件用 cfg-gate
**现状**：Linux 平台支持的退化（如 env/dlls/handles/memory stub）只 2 个测试文件覆盖。
**影响**：Linux 平台支持退化无早期告警。
**修复**：v0.7.0+ 加 Linux 等价 stub 测试：env stub（返回空 Vec）/ dlls stub / handles stub / memory stub 各加 cfg-gate 测试。
**验证**：`cargo test --release` 在 Linux CI runner 上能跑通所有 cfg(target_os = "linux") 测试。

### TD-13（P2-13）：CI Linux job 验证 cfg-gate 测试实际运行

**位置**：`.github/workflows/ci.yml`（未读）
**现状**：未知 CI 是否在 Linux job 上跑 `cargo test`，cfg-gate 写错（cfg 笔误）会让测试静默跳过。
**影响**：cfg-gate 错误静默通过 CI。
**修复**：阶段 8 验证 GitHub Actions Linux job 实际跑了 `test_inspector.rs` / `test_platform_compat.rs` 的 Linux 部分；若没跑，加 `cargo test --release` 到 Linux CI step。
**验证**：CI Linux job log 显示测试数 > 0（不是"0 tests run"）。

### TD-14（P2-14）：panic hook chain 时序验证

**位置**：`src/main.rs:60-61` + `src/tui/mod.rs::setup_terminal`（未读）
**现状**：`main.rs` 先 `init_tracing` 再 `install_panic_hook`；`run_tui` 内 `setup_terminal` 会用 `take_hook` chain 我们的 hook。需验证 chain 顺序：terminal restore → crash report → 默认 hook。
**影响**：若顺序错，TUI 模式 panic 时终端可能不被 restore（用户看到乱码），或 crash report 不写盘。
**修复**：阶段 8 在 TUI 模式手动触发 panic（如临时加 `panic!("test")` 到 tick），验证：终端正常恢复 + `crashes/` 下出现文件 + stderr 有崩溃提示。
**验证**：手动触发 panic 后终端正常 + crash report 生成。

---

## 历史回顾

- v0.6.0 Review（本文件来源）：`docs/reviews/REVIEW-7.md` 产出 1 P0 + 9 P1 + 14 P2。
- v0.6.0 阶段 8 应修：1 P0 + 9 P1（详见 REVIEW-7.md）。
- v0.7.0 候选：本文件 v0.7.0 段 11 项。
- v0.8.0+ 候选：本文件 v0.8.0+ 段 3 项。
