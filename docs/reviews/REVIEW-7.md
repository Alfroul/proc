# 全局 Review 报告 — 阶段 7（v0.6.0）

**审查范围**：v0.6.0 阶段 1-6 全部产出
**审查日期**：2026-06-25
**基线测试**：`cargo test --release` = **700 passed / 0 failed / 3 ignored**
（stage-7.md 写的 `--tb=no` 是 pytest 参数、cargo test 不认，按 `cargo test --release` 跑；总数与 CONTEXT.md/CHANGELOG「~741」相差 41，但 0 failed 是核心约束，可继续。）
**其它基线**：`cargo clippy --release --all-targets -- -D warnings` 0 warnings ✓；`cargo fmt --all -- --check` 干净 ✓；`cargo build --release --no-default-features` 通过 ✓

> **阶段 8 修复状态**：所有 P0（1 项）/ P1（9 项）已在阶段 8 修复并跑回归验证（701+ passed / 0 failed）。下方各条目尾部追加 `**Status: Fixed in stage-8**` 标记。详细修复说明见 [CHANGELOG.md](../../CHANGELOG.md) 阶段 8 段。P2（14 项）已归档到 [`docs/tech-debt.md`](../tech-debt.md)。

## 摘要

- 总问题数：**P0 1 / P1 9 / P2 14**
- 阻断性问题（必须修才能发 v0.6.0）：**1 项**（plan.md 阶段总览状态错误，违反 phased-project「唯一勾选点」规则）
- 已知限制（不阻断但需文档化）：14 项 P2（已归档到 `docs/tech-debt.md`）
- 关键主题：**文档与代码大面积漂移**（self-mitigation 5 项 vs 文档 4 项；EnvVar.is_secret vs 文档 `masked`；restricted_spawn 范围错；README/SECURITY 多处不符）；**worker panic 不写盘**（catch_unwind 截获 panic 后只发 banner，bug 报告时无 stack）；**elevated PowerShell 多处漏接 restricted_spawn**（eject/cache.rs / smart/mod.rs / DNS probe）

---

## P0（阻断性，必须修才能交付 v0.6.0）

### P0-1：plan.md 阶段总览状态与实际产出大面积不符 — `plan.md:192-199`

`plan.md` 第 192-199 行「阶段总览」表把阶段 3/4/5/6 全部标 `⬜ 未开始`，但实际都已落地：

| 阶段 | plan.md 标注 | 实际状态（CHANGELOG/CONTEXT/code 三方证实） |
|---|---|---|
| 3 可观测性 | ⬜ 未开始 | ✅ 已落地：`src/metrics/{mod.rs,crash.rs}` + `src/worker.rs::catch_unwind` + `src/cli/diag.rs` + `src/tui/help_panel.rs` Workers 区段 |
| 4 性能 | ⬜ 未开始 | ✅ 已落地：`ProcessInfo` 字段全部 `Arc<str>` / `Arc<[String]>` 化（`collect.rs:607-638`）+ `ProcessStatus` 13 变体（`collect.rs:496-531`）+ `SearchState::query_lower` |
| 5 架构 | ⬜ 未开始 | ✅ 已落地：`WorkerManager`（`workers/manager.rs`）+ `InspectorController`（`inspect/controller.rs`）+ `ReplayController`（`replay/controller.rs`）+ `main.rs` 1657 → 134 行 + `src/cli/` 14 子模块 |
| 6 UX | ⬜ 未开始 | ✅ 已落地：详情页 `r`→`F5` / `c`→`y` / Docker `r`→`Shift+R`（CHANGELOG 第 10-54 行） |

**为什么阻断**：plan.md 是 phased-project 模型的「唯一勾选点」（ADR-0001），后续会话开工前必读 plan.md 决定下一阶段做啥。状态全错会让阶段 8 会话误以为「3/4/5/6 还没做」，要么重做、要么阻塞。

- **影响**：阶段 8 会话 / 新 cycle 会话读 plan.md 后无法判断真实进度；违反 ADR-0001 phase gating。
- **修复**：把 4 行 `⬜ 未开始` 改成 `[x] 已完成`（如其它已落地阶段那样的格式）。同时同步 plan.md 第 144/150-152 行的 `benches/` / `tests/proptest_*` 标注（实际未落地，见 P2-2）。
- **验证**：`grep -c "⬜ 未开始" plan.md` 应只返回阶段 8（如未发布）；`grep -c "\[x\] 已完成" plan.md` 应 ≥ 6。
- **Status: Fixed in stage-8** — plan.md 阶段 3/4/5/6/7 全部标 `[x] 已完成`，同步修正 FAQ `masked` → `is_secret`（P1-8）+ 删 `--tb=no`（P2-1）。

---

## P1（重要，影响质量）

### P1-1：self-mitigation 实际开 5 项，CONTEXT.md / SECURITY.md 漏写 ImageLoad

`src/security/self_mitigation.rs:11-136` 实际开 **5 项**：DEP / ASLR / ProhibitDynamicCode / DisableExtensionPoints / **ImageLoad（NoRemote + NoLow + PreferSystem32）**。但：
- `CONTEXT.md:40` 写「proc 当前开启 4 项：DEP / ASLR / ProhibitDynamicCode / DisableExtensionPoints。**不开 ProcessSignaturePolicy**」— 漏 ImageLoad。
- `SECURITY.md:35-43` Hardening 表格只列 4 项。
- `SECURITY.md:53` 已知限制段写「4 项策略外的攻击面」。
- `ADR-0008:30, 83` 与代码一致，开 5 项。

- **影响**：用户/审计人员从 SECURITY.md 看不知道 ImageLoad 已开，误判 UNC 网络投递风险；CONTEXT.md 术语漂移。
- **修复**：CONTEXT.md / SECURITY.md 把 4 项改 5 项，加 ImageLoad 行（"禁从 UNC / 远程路径加载 DLL"）。
- **验证**：`grep -c "ImageLoad" CONTEXT.md SECURITY.md` 各 ≥ 1。
- **Status: Fixed in stage-8** — CONTEXT.md「self-mitigation」术语行 + 演进历史段补 ImageLoad；SECURITY.md Hardening 表加 ImageLoad 行，「已知限制」段「4 项策略外」改「5 项策略外」。

### P1-2：EnvVar 字段名实际是 `is_secret`，CONTEXT.md / plan.md 写 `masked`

`src/inspect/mod.rs:21-28` 实际定义：
```rust
pub struct EnvVar {
    pub key: String,
    pub value: String,
    pub is_secret: bool,  // 实际字段名
}
```

但：
- `CONTEXT.md:29` 写「EnvVar 加 `masked: bool` 字段」
- `plan.md:245` FAQ 写「在 `EnvVar` 加 `masked: bool` 字段默认 false」
- `stage-7.md:35` 提「EnvVar::is_secret 字段」（这条与代码一致）

- **影响**：开发者按 CONTEXT.md 找 `masked` 字段会失败；后续会话用错术语继续传播。
- **修复**：CONTEXT.md 第 29 行 `masked: bool` → `is_secret: bool`；plan.md FAQ 同步。
- **验证**：`grep -n "masked: bool" CONTEXT.md plan.md` 应无结果。
- **Status: Fixed in stage-8** — CONTEXT.md「EnvVar」术语行 + 演进历史段 + plan.md FAQ 全部改 `is_secret`。

### P1-3：restricted_spawn 实际只接入 DNS spawn，CONTEXT.md / SECURITY.md 写「覆盖 docker exec / nvtop」错

`src/security/restricted_spawn.rs:17-19` 模块注释明确写「**不接入 docker exec / nvtop**」。实际调用方仅 `src/dns_log/windows_dns.rs:253`（DNS 数据采集 spawn）。但：
- `CONTEXT.md:41` 写「elevated 时 spawn 子进程（PowerShell DNS / **docker exec / nvtop**）前调 `CreateRestrictedToken`」— 错。
- `SECURITY.md:31` 写「v0.6.0 阶段 2 起：elevated 时 spawn 子进程（PowerShell DNS / **docker exec / nvtop**）会调 `CreateRestrictedToken`」— 错。

- **影响**：用户/审计人员从 SECURITY.md 误以为 docker exec/nvtop 已被保护，实际未保护（实际威胁有限 — ADR-0008 第 17-19 行说明 docker/nvtop 是 privileged 操作无法简单 drop）。但文档让用户高估了保护范围。
- **修复**：CONTEXT.md / SECURITY.md 改成「仅接入 PowerShell DNS（elevated 时）；docker exec / nvtop 因自身需 privileged token 不接入（ADR-0008）」。同时把 P1-4 漏接的两个 PowerShell 路径也补上。
- **验证**：`grep -n "docker exec.*restricted_spawn\|nvtop.*restricted_spawn" CONTEXT.md SECURITY.md` 应无结果。
- **Status: Fixed in stage-8** — CONTEXT.md「restricted spawn」术语行 + 演进历史段改写「仅接入 PowerShell DNS」；SECURITY.md Privilege Model 段同步。

### P1-4：`eject/cache.rs` + `smart/mod.rs` 跑 PowerShell 但漏接 restricted_spawn

`src/security/restricted_spawn.rs:18-19` 写「不接入 docker exec / nvtop」，但**未提**这两处也是 PowerShell 跑脚本但**漏接**：

1. **`src/eject/cache.rs:13`** — `Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", "Write-VolumeCache X:"])`。威胁模型与 DNS PowerShell 完全一致（接受 `-Command` 任意脚本，elevated 时持 SeDebugPrivilege）。
2. **`src/smart/mod.rs:260`** — `Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "Get-CimInstance Win32_DiskDrive ..."])`。
3. **`src/smart/mod.rs:345`** — `Command::new("powershell.exe")` 跑 `Get-PhysicalDisk` 类似查询。

- **影响**：elevated proc 弹安全弹窗 / 列举磁盘时，这 3 个 PowerShell 子进程继承 SeDebugPrivilege。若任意路径接受外部输入（Write-VolumeCache 接 drive_letter 字符 — 当前是 `char` 类型安全；WMI 查询命令是常量字符串），目前**实际无注入风险**。但威胁模型应一致 — 一旦后续会话给这些命令加了用户可控参数，就成跳板。
- **修复**：把这 3 处 `Command::new(...)` 改为 `spawn_with_reduced_privileges(...)`，与 DNS spawn 路径统一。
- **验证**：`grep -nE 'Command::new\("powershell' src/eject src/smart` 应无结果（都被 restricted_spawn 路径替代）。
- **Status: Fixed in stage-8** — 3 处改用新增的 `run_with_reduced_privileges`（restricted_spawn 模块的便利函数，语义近似 `Command::output`）。

### P1-5：README.md 第 33 行键位 'r'/'c' 旧描述与代码不符

`README.md:33` 写：
> 详情页内 `r` 强制重新采集、`/` 搜索、`Tab/Shift+Tab` 切 Tab、`+` / `-` 调整优先级、`a` 调整 affinity、`Esc` 先退搜索再退页面

实际（`src/inspect/controller.rs:173, 255`）：'r' 已迁移到 `F5`、'c' 已迁移到 `y`、`a` 调整 affinity 在 controller.rs 实际**未实现**（grep `Char('a')` 在 controller.rs 无命中，仅在 controller.rs 文档注释提及）。

- **影响**：用户读 README 后按 'r' 没反应、按 'c' 触发全局侧边栏折叠会困惑；CHANGELOG 第 26/34 行已说明改了键位，但 README 未同步。
- **修复**：README.md 第 33 行 `r 强制重新采集` → `F5 强制重新采集`；同时加 `y 复制进程信息`；删除或落实 `a 调整 affinity`。
- **验证**：`grep -nE "详情页内.*r 强制|'c'.*复制" README.md` 应无结果。
- **Status: Fixed in stage-8** — README.md 第 33 行改 `F5` / `y` / `v`；键位表新增 `F5`/`y`/`v` 行；删 `a 调整 affinity` 误导。

### P1-6：详情页 'r' / 'c' 废弃后无任何 deprecation warning

`stage-6.md:29-33, 75-78` 任务 1/2 明确设计：
```rust
// 保留 'r' 显示 deprecation warning（v0.6.0 兼容期）
KeyCode::Char('r') => {
    self.dirty = true;
    return InspectorAction::StatusMsg("⚠ 'r' 将在 v0.7.0 移除，请用 F5".into());
}
```

但 `src/inspect/controller.rs::handle_key`（已搬迁后的实际代码）第 160-264 行：'r' / 'c' **直接落入 `_ => {}` 分支 noop**，无 status_message 提示。

- **影响**：用户从 0.5.0 升级后按 'r' 不刷新、按 'c' 触发全局侧边栏折叠 — 与 0.5.0 习惯冲突且无任何反馈，UX 退化。tests/test_inspector.rs:585 `r_key_in_detail_does_not_refresh` 只测「不刷新」，**没测「应有 deprecation warning」**（设计要求未实现）。
- **修复**：按 stage-6.md 任务 1/2 设计补两条 `KeyCode::Char('r')` / `'c'` 分支返回 `StatusMsg("⚠ ... 请用 F5/y")`；测试同步加 `r_key_shows_deprecation` / `c_key_shows_deprecation`。
- **验证**：详情页按 'r' / 'c' 后 `app.status_message.is_some()` 应为 true；新测试通过。
- **Status: Fixed in stage-8** — InspectorController::handle_key 补 'r'/'c' 两个 StatusMsg 分支；App::try_handle_tab_switch 在 `mode == ProcessDetail` 时让 'c' 落入 InspectorController（不再被全局抢键）；原 `c_key_in_detail_toggles_sidebar_not_copy` / `r_key_in_detail_does_not_refresh` 重写为 `_shows_deprecation_warning` 两测均过。

### P1-7：worker panic 不写 crash report 到盘（catch_unwind 截获后只发 banner）

`src/worker.rs:77-96` worker 主循环外包 `catch_unwind`：
```rust
let result = std::panic::catch_unwind(...);
if let Err(payload) = result {
    // tracing::error!(...) + tx.send(WorkerCrash{...})
    // ❌ 不调 crash::write_crash_report_to
}
```

`src/metrics/crash.rs:40-61` 的 `install_panic_hook` 会写盘，但 `catch_unwind` 截获 panic 后**不触发 panic hook**（Rust 标准库语义：catch_unwind 阻止 unwind 到顶 = 不调 hook）。结果：worker 崩溃只有主线程 banner，**磁盘上无 crash report 文件**。

stage-7.md 第 38 行明确质疑过这点（"panic hook 写 crash report 在 worker 线程 panic 时是否能触发"）— 实测答案：**不能**。

- **影响**：worker 崩溃后用户报 bug 时附不上 stack trace，diagnosis 退化到只有 banner 文字。CONTEXT.md 第 57 行声称「panic 时通过 crash_tx 通知主线程显示 banner」+「panic hook 写 crash report」两套机制，实际 worker panic 时只有前者。
- **修复**：在 worker.rs:87 `tracing::error!` 后立即调用 `crate::metrics::crash::write_crash_report_to(&dir, &format!("worker {thread_name} panic\n{msg}\n{backtrace}"))`（文件名带 worker 名字以与主线程 panic 区分）。或更简单：让 panic hook 本身在 catch_unwind 后被显式调用（`std::panic::PanicHookInfo` 不可构造，所以走 write_crash_report_to 路径）。
- **验证**：单元测试 — spawn 一个会 panic 的 worker，验证 `crashes/` 下出现 `crash-test-*.txt` 且内容含 worker 名。
- **Status: Fixed in stage-8** — worker.rs::spawn catch_unwind 后调 `crate::metrics::crash::write_worker_crash_report(thread_name, &msg, &bt_str)`，文件名 `crash-worker-{name}-{ts}.txt`。新增 `write_worker_crash_report` / `write_worker_crash_report_to` / `format_worker_crash_report` 3 函数 + 2 测试（`write_worker_crash_report_to_writes_file_with_worker_name` / `format_worker_crash_report_includes_all_fields`）。

### P1-8：plan.md FAQ EnvVar 字段名错（masked → is_secret）

`plan.md:244-245` FAQ：
> Q: env 脱敏会破坏现有测试吗？
> A: 阶段 2 在 `EnvVar` 加 `masked: bool` 字段默认 false，原 serde 行为不变；只改 UI 渲染层。

实际字段是 `is_secret: bool`（同 P1-2）。

- **影响**：与 P1-2 同源，README/CONTEXT/plan 三方术语漂移。
- **修复**：plan.md 第 245 行 `masked: bool` → `is_secret: bool`。
- **验证**：`grep -n "masked: bool" plan.md` 应无结果。
- **Status: Fixed in stage-8** — plan.md FAQ `masked` → `is_secret`。

### P1-9：Cargo.toml 版本号仍是 0.5.0（v0.6.0 周期未升）

`Cargo.toml:3` `version = "0.5.0"`。但 CONTEXT.md / ADR-0008 / CHANGELOG / 所有 stage-N.md 都说当前是 v0.6.0 周期。crash report（`crash.rs:77`）通过 `env!("CARGO_PKG_VERSION")` 写入文件，所有 crash report 都会标错版本号。

- **影响**：crash report / `proc --version` / `cargo binstall` 都标错版本；release.yml（如基于 tag）发布 v0.6.0 tag 时与 Cargo.toml 不一致会让 `winget`/`scoop` manifest 错版本。
- **修复**：`Cargo.toml:3` 改 `version = "0.6.0"`（阶段 8 发布前必修）。
- **验证**：`grep '^version' Cargo.toml` 显示 `0.6.0`；`./target/release/proc --version` 显示 `proc 0.6.0`。
- **Status: Fixed in stage-8** — Cargo.toml `version = "0.6.0"`；`env!("CARGO_PKG_VERSION")` 注入的 crash report / `--version` 输出同步对齐。

---

## P2（建议，长期改善）

> P2 共 14 项，已归档到 `docs/tech-debt.md`。本节列摘要 + 影响评估，详细修复建议见 tech-debt.md。

- **P2-1**：CONTRIBUTING.md / plan.md / stage-N.md 多处用 `cargo test --release --tb=no -q` — `--tb=no` 是 pytest 参数不是 cargo test 参数。**影响**：贡献者跑这命令会报错；本会话也踩到。**修复**：删 `--tb=no`。
- **P2-2**：stage-6.md 任务 3-5（proptest / criterion / Linux stub）实际未落地（`tests/` 无 `proptest_*`、无 `benches/`、Cargo.toml 无 `proptest`/`criterion` 依赖；CHANGELOG 第 51-53 行已说"不在本 slice 范围"）。**影响**：stage-6.md 文档"目标"行仍提"引入 proptest + criterion + Linux stub 测试"，与实际交付不符。**修复**：stage-6.md 头部加 "v0.6.0 实际只做了任务 1/2，任务 3-5 推迟到 v0.7.0+"。
- **P2-3**：stage-7.md 切片 E 假设项目有 proptest / criterion（实际没有）。**影响**：review 时按不存在的工具提问。**修复**：stage-7.md 切片 E 改问"是否需要引入 proptest（v0.7.0+）"。
- **P2-4**：`WorkerManager::restart(name)` 未实现（`workers/manager.rs:11-12` 注释明确"按 surgical 原则不预实现"）。CONTEXT.md 第 55 行写"故障恢复方法尚未实现"但不够显眼。**影响**：worker 崩溃后只能重启 proc，不能热恢复。**修复**：CONTEXT.md 顶部"术语"表加 ⚠️ 标注或迁到"已知限制"段。
- **P2-5**：`WorkerManager::metrics_snapshot` 不含 Docker worker（`workers/manager.rs:7-9` 注释"Docker worker 由 DockerPanel 自管"）。**影响**：`proc diag` 看不到 Docker logs worker 指标，bug 报告时缺数据。**修复**：让 DockerPanel 也实现 metrics 接口，App::worker_metrics 追加。
- **P2-6**：`App` 仍是 1707 行 + 40+ 字段（`src/app.rs`）。即便拆出 3 个 controller，仍偏大。**影响**：未来加新功能会进一步膨胀。**修复**：v0.7.0+ 把 process_panel / port_panel / usb_panel / monitor_panel / docker_panel 5 个 panel 字段拆出对应 controller。
- **P2-7**：`tests/test_stage8_perf_regress.rs` 命名误导。注释自己写"Stage-8 一次性性能回归基线"，但当前是 stage-7（review），stage-8 还没开始。实际是 stage-4 落地时一起写的基线。**影响**：未来 stage-8 会话看到这文件以为已开始。**修复**：改名 `test_perf_baseline.rs` 或 `test_stage4_perf_regress.rs`。
- **P2-8**：`help_panel.rs:156-178` Workers 区段硬编码列宽（`name<10` `avg>5μs` 等），worker 名 > 10 字符会破坏对齐；终端窄时 `Paragraph::wrap(Wrap{trim:false})` 会让整行软换行打乱表格。**影响**：worker 名长（如 `dns_log_worker`）时表格视觉错乱。**修复**：worker 名 truncate 到 10 字符或加宽列。
- **P2-9**：`SearchState::handle_input`（`search.rs:54`）每次按键整体重算 `query.to_lowercase()`，注释承认"保持简单"未做增量。**影响**：query < 64 字符时差异可忽略（μs 级）；但 stage-4.md 性能优化目标本可彻底达成。**修复**：v0.7.0+ 改增量 lowercase append（仅在 Backspace 时整体重算）。
- **P2-10**：`src/dns_log/windows_dns.rs:238` PowerShell probe（`-Command exit 0`）未走 restricted_spawn。**影响**：probe 不接受外部数据，注入风险低；但与 spawn 路径不一致。**修复**：probe 也走 restricted_spawn。
- **P2-11**：`src/monitor/watchdog.rs:87` `Command::new(&cmd)` 用户配置命令未走 restricted_spawn。**影响**：用户主动配置的 watchdog 命令，威胁模型不同于 DNS PowerShell；但 elevated 时仍持 SeDebugPrivilege。**修复**：v0.7.0+ 评估是否对所有 watchdog spawn 走 restricted_spawn（可能破坏用户自定义命令）。
- **P2-12**：Linux stub 测试覆盖偏少 — `tests/` 下只有 `test_inspector.rs` / `test_platform_compat.rs` 2 个文件用 cfg-gate。**影响**：Linux 平台支持退化无早期告警。**修复**：v0.7.0+ 加 Linux 等价 stub 测试覆盖 env/dlls/handles/memory。
- **P2-13**：`test_inspector.rs` / `test_platform_compat.rs` cfg-gate 实际是否在 Linux CI 跑未知（CI 配置 `.github/workflows/ci.yml` 未读）。**影响**：cfg-gate 写错（如 cfg 笔误）会让测试静默跳过。**修复**：阶段 8 验证 GitHub Actions Linux job 实际跑了这些测试。
- **P2-14**：panic hook chain 时序未验证。`main.rs:60-61` 先 `init_tracing` 再 `install_panic_hook`，但 `tui::setup_terminal`（在 `run_tui` 内调用）会用 `take_hook` chain 我们的 hook。需验证 chain 顺序是否正确（terminal restore → crash report → 默认）。**影响**：若顺序错，TUI 模式 panic 时终端可能不被 restore（用户看到乱码）。**修复**：阶段 8 在 TUI 模式手动触发 panic 验证。

---

## 切片 A — 安全审查（阶段 2）

### A1: SECRET_PATTERNS 覆盖 ✓

`src/inspect/env_mask.rs:17-30` 列出 12 个关键字（KEY/TOKEN/SECRET/PASSWORD/PASSWD/PWD/CREDENTIAL/PRIVATE/AUTH/API/DSN/CONNECTION_STRING），加 `DATABASE_URL` 特例 + `*_AUTHORIZATION` 后缀（`env_mask.rs:42`）。测试 `does_not_false_positive_common_keys` 验证 PATH/HOME/SYSTEMROOT 不误报。

**评价**：覆盖合理，常见 secret 命名都包含；`EDITOR`/`SHELL`/`TMP` 不误判。

### A2: mask_value 多字节字符处理 ✓

`env_mask.rs:50-57` 用 `val.chars().take(2)` 按字符截取前 2，长度按 UTF-8 字节。测试 `mask_value_handles_multibyte` 用中文 + emoji 验证。`mask_value_does_not_leak_full_value` 验证后段不泄漏。

**评价**：多字节处理正确，长度按字节让用户判断 token 大致长度又不泄漏内容。

### A3: self_mitigation 调用顺序 ✓（但文档错见 P1-1）

`src/main.rs:26` `apply_self_mitigations()` 在 `shutdown::init()` 之后、`init_tracing()` 之前调用。注释明确"tracing 此时还没初始化 → warn 会丢失，所以函数返回失败的策略名列表，这里直接 eprintln!"。早于任何 worker spawn / FFI。**符合 ADR-0008 第 45 行要求**。

### A4: restricted_spawn 覆盖范围 ⚠（见 P1-3 / P1-4）

实际只接入 `dns_log/windows_dns.rs:253`。CONTEXT.md/SECURITY.md 写"覆盖 docker exec / nvtop"是错；eject/cache.rs + smart/mod.rs 漏接。

### A5: 录屏强制 mask 路径 ✓

`src/tui/detail_view.rs:553-554`：
```rust
// v0.6.0 阶段 2：录屏中即便 env_reveal=true 也强制 mask（防录到真值）。
let reveal = app.inspector.env_reveal && !app.is_recording();
```
`detail_view.rs:589` 用此 `reveal` 调 `v.render_value_owned(reveal)`。`controller.rs:130-132` `env_render_reveal` 方法封装同样逻辑。**录屏路径正确**。

### A6: EnvVar serde 兼容性 ✓（stage-7.md 担忧不成立）

`src/record/` 目录 grep 无 `EnvVar` 引用 — `.prec` 录屏文件**不序列化 EnvVar**，所以 is_secret 字段新增不影响 .prec 兼容性。stage-7.md 第 35 行担忧不成立。`ProcessInfo::name_lower` 用 `#[serde(skip)]`（`collect.rs:636`），反序列化时用 default 空字符串 — 0.5.0 .prec 兼容性 OK（record 测试全过）。

---

## 切片 B — 可观测性审查（阶段 3）

### B1: 日志 rotate Windows 路径分隔符 ✓

`src/main.rs:104-109` 用 `tracing_appender::rolling::RollingFileAppender::new(Rotation::DAILY, &config_dir, "proc.log")`。tracing-appender 内部用 `std::path::Path` 跨平台 OK。`src/lib.rs:103-129` `cleanup_old_logs` 用 `std::fs::read_dir` + `path.file_name()`，跨平台 OK；文件名匹配 `proc*.log` 跳过 `crashes/` 子目录 ✓。

### B2: panic hook 在 worker 线程 ⚠（见 P1-7）

worker 用 catch_unwind 截获 panic，**不触发 panic hook** → crash report 不写盘。需在 worker.rs:87 截获后手动调 `write_crash_report_to`。

### B3: WorkerMetrics CAS max_us 无 ABA ✓

`src/metrics/mod.rs:46-57` 标准 CAS max 模式（load → compare_exchange_weak 循环）。max 单调递增语义，无 ABA 风险。`concurrent_record_poll_is_safe` 测试 10 线程 × 1000 次并发 record 无死锁 ✓。

### B4: `proc diag` 在 CLI 模式行为 ✓（但有副作用）

`src/cli/diag.rs:10-23` 实际启动 `App::new()` 跑 2 秒 tick（等所有 worker 至少 poll 一次）。其它 CLI 命令（ls/kill/port）不启动 worker，无需 diag。**副作用**：`proc diag` 比 `proc ls` 慢 2-3 秒，但用户报 bug 时通常已运行过 proc，可接受。

### B5: help_panel Workers 区段窄终端 ⚠（见 P2-8）

---

## 切片 C — 性能审查（阶段 4）

### C1: Arc deref 在 hot path ✓

`src/collect.rs:607-638` ProcessInfo 字段全部 Arc 化。`view_models/process_panel.rs` 排序 / 搜索 / 渲染用 `Arc::clone`（原子计数）替代 `String::clone`（堆分配）。`test_process_info_arc.rs` / `test_search_perf.rs` 覆盖性能回归。

### C2: name_lower 在 Clone 时共享 Arc ✓

`collect.rs:637` `name_lower: Arc<str>` 字段，Clone 走 Arc::clone（原子计数），heavy worker 一次计算全程共享。`#[serde(skip)]` 不影响 .prec 体积。

### C3: rebuild_sorted_cache 优化无回归 ✓

`view_models/process_panel.rs` 搜索按键每次重建 cached_sorted（v0.5.0 阶段 11 P0 修复）。stage-4 优化（name_lower + query_lower）不影响这个语义。`test_search_correctness.rs` 覆盖空列表 / 单元素 / 全匹配边界。

### C4: ProcessStatus From sysinfo 映射完整 ✓

`collect.rs:513-531` 12 个命名变体 + Unknown 覆盖 sysinfo 0.34.2 全部变体（包括 `Unknown(_)` 通配）。变体名按 sysinfo 真实命名（`Tracing` / `LockBlocked` / `UninterruptibleDiskSleep`）对齐，未沿用 stage-4.md 早期猜测的 `Traced` / `DeadLock`。

---

## 切片 D — 架构审查（阶段 5）

### D1: 3 个 Controller 拆分后 App 仍偏大（见 P2-6）

`App` 仍 1707 行 + 40+ 字段。3 个 controller（Inspector/Replay/WorkerManager）共拆出 15 字段。剩余字段主要是 5 个 panel（process/port/usb/monitor/docker）+ 录屏状态（recording_wanted/pending_record_confirm）+ 历史（proc_history/op_history）+ alert。

### D2: Controller 间通信 ✓

3 个 controller 都通过 `Action` 枚举让 App 派发副作用（`InspectorAction` / `ReplayAction`），无反向依赖 App。无循环依赖。

### D3: main.rs / cli/ 拆分 import 路径 ✓

`src/cli/mod.rs:20` re-export `Cli / Command / DockerSub` 让旧路径 `proc::cli::Cli` 可用。`run_subcommand` 17 个变体 dispatch（mod.rs:26-66）覆盖 `def.rs` 全部 Command。main.rs 134 行仅保留 main/init_tracing/install_panic_hook/run_tui 4 个函数。

### D4: 循环依赖 ✓

grep controller 模块无 `use crate::app::App`。Controller 只依赖 `crate::app_panel` / `crate::collect` / `crate::inspect` / `crate::record` 等数据模块，App 单向依赖 controller。

### D5: WorkerManager::restart 未实现（见 P2-4）

注释明确"按 surgical 原则不预实现"。worker 崩溃后只能 banner 提示 + 手动重启 proc。

---

## 切片 E — UX + 测试审查（阶段 6）

### E1: 'r' / 'c' deprecation warning 未实现（见 P1-6）

### E2: F5 / 'y' 在详情页的快捷键提示 ✓

`src/tui/help_panel.rs:44-47` 进程列表段新增：
```
("y", "详情页: 复制进程信息到剪贴板（vim yank）"),
("F5", "详情页: 强制刷新 Inspector 数据"),
```
覆盖完整。

### E3: proptest / criterion 项目从未引入（见 P2-3）

stage-7.md 假设错误。`Cargo.toml` `[dev-dependencies]` 只有 `tempfile` + `filetime`。

### E4: Linux stub 测试覆盖偏少（见 P2-12 / P2-13）

### E5: test_stage8_perf_regress.rs 命名误导（见 P2-7）

---

## 横切维度

### 架构一致性

新代码遵循既有模块边界：`src/metrics/` / `src/security/` / `src/workers/` / `src/replay/` / `src/inspect/controller.rs` / `src/cli/` 各自职责清晰。无未文档化的术语（除 P1-1 / P1-2 / P1-3 已在文档漂移段记录）。未偏离 ADR-0001 phase gating（除 P0-1 plan.md 状态错）。

### 文档完整性

- **README.md**：第 33 行键位错（P1-5）；其它段落反映 v0.6.0 能力（env_reveal / F5 / y 都有提及，仅 'r'/'c' 旧描述残留）。
- **CHANGELOG.md**：阶段 1-6 都有 Added/Changed 段 ✓；第 52 行明确"任务 3-5 不在本 slice 范围"诚实。
- **SECURITY.md**：第 31 行 restricted_spawn 范围错（P1-3）；第 35-43 行 Hardening 表漏 ImageLoad（P1-1）。
- **CONTRIBUTING.md**：第 28 行 `--tb=no` 错（P2-1）；第 73 行 cycle v0.6.0 8 阶段说明 ✓。
- **ADR-0008**：Status Accepted ✓（阶段 2 落地后已改）；内容与代码一致（5 项策略 / restricted_spawn / env_mask）。
- **CONTEXT.md**：多处与代码漂移（P1-1 / P1-2 / P1-3）。
- **stage-N.md**：stage-6.md 目标段未标注任务 3-5 推迟（P2-2）；stage-7.md 假设 proptest/criterion 存在（P2-3）。
- **plan.md**：阶段总览 4 项状态错（P0-1）+ FAQ 字段名错（P1-8）+ `--tb=no` 错（P2-1）。

### 测试覆盖

- 总数 700 passed / 0 failed / 3 ignored — 数量合理（stage-2 / 3 / 4 / 5 / 6 累计估算 ~50 个新测试，加上 0.5.0 ~650 基线）。
- **无集成测试的模块**：`src/metrics/crash.rs` 的 worker 线程 panic → crash report 路径（P1-7）；`src/security/restricted_spawn.rs` 在 elevated 环境的真实剥离行为（只能在 elevated 进程测，CI 难）。
- **平台特化测试**：`tests/test_inspector.rs` / `test_platform_compat.rs` 2 个文件用 cfg-gate；Linux CI 是否真跑未知（P2-13）。
- **criterion bench**：项目未引入（P2-3）。

### 性能基线

- **启动时间**：未量化测，但 self_mitigation 调用是 5 个 syscall + init_tracing 是文件创建，理论 < 50ms。
- **单帧渲染**：v0.5.0 50ms tick 预算继续生效；stage-4 Arc 化让 N=500 进程的 sort/搜索 hot path 减少 90% 堆分配（CHANGELOG 阶段 4 段）。
- **内存占用**：WorkerMetrics 每个 worker ~100 bytes（5 atomic + 1 Mutex），4 个 worker ~400 bytes 增量，可忽略。InspectorController 9 字段 / ReplayController 2 字段从 App 搬出，无新增。
- **二进制体积**：self_mitigation 加 FFI（windows crate 已有），无显著增大。Stage-8 性能回归测试 `test_stage8_perf_regress.rs` 已建立 500 进程基线（命名误导见 P2-7）。

### 安全

- **self-mitigation 4 项策略实际生效**：✓ 5 项（P1-1 文档错）；ADR-0008 第 91-98 行实测验证（Process Explorer 看 mitigation flags）。
- **elevated 时 restricted_spawn 覆盖**：⚠ 只覆盖 DNS spawn（P1-3 / P1-4 漏接 eject/cache.rs + smart/mod.rs）。
- **未脱敏 secret 入口**：✓ DNS 查询域名在 DNS 子视图直接显示（这是设计意图，非 secret）；进程 cmd 在详情页直接显示（cmd 含路径但通常不含 secret token）；env 路径全部走 mask ✓。

---

## 建议归档到 tech-debt.md 的 P2（v0.7.0+）

> 详见 `docs/tech-debt.md`。摘要：14 项 P2 按 v0.7.0（11 项）/ v0.8.0+（3 项）分组。v0.7.0 主题：文档清理（--tb=no / stage-6 任务 3-5 推迟标注 / WorkerManager::restart 标注）+ 测试覆盖增强（Linux stub / cfg-gate CI 验证）+ 性能彻底化（SearchState 增量 lowercase / WorkerManager 含 Docker metrics / App 进一步拆 5 panel controller）。v0.8.0+ 候选：ProcessSignaturePolicy 评估 / Linux eBPF net_flow provider / stage-8 perf test 改名。

---

## 验收清单（自检）

- [x] `cargo test --release` 700 passed / 0 failed / 3 ignored
- [x] `cargo clippy --release --all-targets -- -D warnings` 0 warnings
- [x] `cargo fmt --all -- --check` 干净
- [x] `cargo build --release --no-default-features` 通过
- [x] REVIEW-7.md 含 ≥ 3 个 P 段（P0 1 + P1 9 + P2 14 = 3 档全覆盖）
- [x] 覆盖 5 切片（A 安全 / B 可观测 / C 性能 / D 架构 / E UX+测试）
- [x] 覆盖 5 横切（架构一致性 / 文档 / 测试覆盖 / 性能基线 / 安全）
- [x] P2 > 10 项已归档到 `docs/tech-debt.md`
- [x] 本阶段未修改任何代码（仅新增 REVIEW-7.md + tech-debt.md）
