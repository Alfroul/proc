# 技术债归档 — v0.6.0 Review 产出

> 源：`docs/reviews/REVIEW-7.md` P2 段（14 项）。按 v0.7.0 / v0.8.0+ 分组。
> 与 P0/P1 区分：P0/P1 阻断 v0.6.0 发布，**必须**在阶段 8 修；P2 不阻断，归档到下个 cycle。

---

## v0.7.0 候选（11 项）

### TD-1（P2-1）：清理文档中错误的 `--tb=no` 测试参数 ✅ Fixed in v0.7.0 阶段 1

**位置**：`CONTRIBUTING.md:28` / `plan.md:209` / `docs/stages/stage-{2,3,4,5,6,7}.md` 多处
**现状**：文档把 `cargo test --release --tb=no -q` 当作命令。`--tb=no` 是 pytest 参数，cargo test 不认（实测报错 `unexpected argument '--tb'`）。
**影响**：贡献者按文档跑命令会失败；多个 stage-N.md 复制粘贴此错误。
**修复**：全文 grep `--tb=no` 删除（保留 `cargo test --release -q`）。
**验证**：`grep -rn "--tb=no" .` 应无结果。

### TD-2（P2-2）：stage-6.md 标注任务 3-5 推迟 ✅ Fixed in v0.7.0 阶段 1

**位置**：`docs/stages/stage-6.md:5`
**现状**：第 5 行「目标」仍写「引入 proptest + criterion + Linux stub 测试」，但 CHANGELOG 第 51-53 行已明确"任务 3-5 不在本 slice 范围"。
**影响**：未来会话读 stage-6.md 误以为任务 3-5 应在 v0.6.0 落地。
**修复**：stage-6.md 头部加"⚠ v0.6.0 实际只做了任务 1/2（键位修复），任务 3-5（proptest/criterion/Linux stub）推迟到 v0.7.0+ — 见 CHANGELOG"。
**验证**：stage-6.md 头部包含推迟声明。

### TD-3（P2-3）：stage-7.md 切片 E 假设错误 ✅ Fixed in v0.7.0 阶段 1

**位置**：`docs/stages/stage-7.md:59-60`
**现状**：第 59-60 行问"proptest 是否覆盖了真实的 panic 路径" / "criterion bench 的 mock_process 是否反映真实数据分布"。项目从未引入这两个工具。
**影响**：阶段 7 review 时按不存在的工具提问，浪费精力。
**修复**：stage-7.md 切片 E 改问"是否需要引入 proptest（v0.7.0+ 评估）" / "性能回归测试是否覆盖 hot path"。
**验证**：stage-7.md 切片 E 不再假设 proptest/criterion 存在。

### TD-4（P2-4）：CONTEXT.md 显眼标注 WorkerManager::restart 未实现 ✅ Fixed in v0.11.0 阶段 1（真正实装）

**位置**：`CONTEXT.md:55`
**现状**：第 55 行写"restart(name) 故障恢复方法尚未实现（无调用方，surgical 原则不预实现）"，但藏在表格里不够显眼。
**影响**：用户/开发者读 CONTEXT.md 后误以为 worker 崩溃可热恢复。
**修复**：CONTEXT.md 顶部加"已知限制"段或 ⚠️ 标注；README 平台支持表加"worker 崩溃后只能重启 proc"。
**验证**：CONTEXT.md 顶部"已知限制"段含 restart 条目。

> **v0.7.0 阶段 1 标 Fixed**：仅完成「文档标注」部分（CONTEXT.md 顶部加 ⚠️ 已知限制 + README FAQ），实际 `restart()` 函数尚未实装。
>
> **v0.11.0 阶段 1 标 Fixed（真正实装）**：ADR-0019 落地完整的 Worker Restart Policy：
> - 指数退避（5s / 30s / 5min）+ 最大重试 3 + reset 计数（1h 无 panic 后归零）
> - `WorkerManager::restart(name, now, crash_tx)` 记录 crash + 触发 respawn 决策
> - `WorkerManager::restart_tick(now, crash_tx)` 每 1s 检查 backoff 到期
> - `WorkerManager::restart_status(name, now)` 给 banner 渲染用
> - `RestartState` / `RestartStatus` 状态机纯逻辑（src/workers/restart.rs，14 unit test）
> - `tests/test_worker_restart.rs` 12 个集成测试覆盖端到端 + 真实 spawn → panic 路径
> - CONTEXT.md 顶部「已知限制」段更新为「worker panic 后自动按指数退避重启（5s/30s/5min），3 次失败后永久死亡需重启 proc」

### TD-5（P2-5）：WorkerManager 含 Docker worker metrics ✅ Fixed in v0.7.0 阶段 1

**位置**：`src/workers/manager.rs:7-9` / `src/cli/diag.rs:25`
**现状**：Docker logs worker 由 `DockerPanel` 自管（生命周期与 panel 绑定），`metrics_snapshot()` 不含它。`proc diag` 输出缺 Docker 行。
**影响**：用户报 bug 时附上的 diag 缺 Docker worker 数据。
**修复**：让 `DockerPanel` 实现 `metrics()` 接口（或暴露 `Arc<WorkerMetrics>`），App::worker_metrics 追加 Docker 行。注意 DockerPanel 可能 spawn 多个 logs worker（每容器一个），需聚合。
**验证**：`proc diag --json` 输出含 `{"name": "docker_logs_*", ...}`。

### TD-6（P2-6）：App 进一步拆 5 个 panel controller ✅ Fixed in v0.7.0 阶段 5

**位置**：`src/app.rs`（1707 行 / 40+ 字段）
**现状（已修复）**：v0.6.0 阶段 5 已拆出 InspectorController / ReplayController / WorkerManager（共 15 字段）。v0.7.0 阶段 5 把剩余 5 个 panel 字段（process_panel / port_panel / usb_panel / monitor_panel / docker_panel）拆出对应 controller。App 字段类型 `XxxPanel` → `XxxPanelController`，字段名保留（外部访问路径 `app.xxx_panel.panel.<field>`）。
**影响**：App 仍是协调器 + 全局状态容器；新功能加 panel 时只动对应 controller，不膨胀 App。
**修复（已落地）**：5 个 controller（`src/view_models/{process,port,usb,monitor,docker}_panel_controller.rs`），每个包装 inner panel + 提供 `panel()` / `panel_mut()` 访问器 + `handle_key` forward。`PanelAction` 枚举统一副作用，与 InspectorAction / ReplayAction 共存（v0.8 评估合并）。详见 ADR-0012。
**验证**：`tests/test_panel_controllers.rs` 6 case + 全量 100 passed；App::handle_key dispatch 改用 PanelAction；`wc -l src/app.rs` 受 ADR-0012 metrics 部分达成（handle_key 切到 PanelAction dispatch；app.rs 主体受其他阶段影响未压到 ≤ 1000，留 v0.7 阶段 6+ 评估）。

### TD-7（P2-7）：test_stage8_perf_regress.rs 改名 ✅ Fixed in v0.7.0 阶段 1

**位置**：`tests/test_stage8_perf_regress.rs`
**现状**：文件注释写"Stage-8 一次性性能回归基线"，但实际是 stage-4 落地时一起写的（ProcessInfo 字段对齐）。当前在 stage-7，stage-8 还没开始。
**影响**：未来 stage-8 会话看到这文件以为已开始 / 误判进度。
**修复**：改名 `test_perf_baseline.rs`（或 `test_stage4_perf_regress.rs`）；注释更新。
**验证**：`grep -rn "stage8_perf" .` 应只在 git history / 旧引用出现。

### TD-8（P2-8）：help_panel Workers 区段自适应列宽 ✅ Fixed in v0.7.0 阶段 1

**位置**：`src/tui/help_panel.rs:156-178`
**现状**：硬编码列宽 `name<10` `avg>5μs` `max>5μs` `polls>7` `drops>5`。worker 名 > 10 字符（如 `dns_log_worker` = 14）破坏对齐；终端窄时 `Paragraph::wrap(Wrap{trim:false})` 让整行软换行打乱表格。
**影响**：worker 名长时表格视觉错乱。
**修复**：worker 名 truncate 到 10 字符 + ellipsis；或改用 `Table` widget 替代 `Paragraph` + 手工对齐。
**验证**：worker 名 14 字符时表格列对齐无错位。

### TD-9（P2-9）：SearchState 增量 lowercase append ✅ Fixed in v0.7.0 阶段 1

**位置**：`src/search.rs:50-56`
**现状**：每次按键 `self.query_lower = self.query.to_lowercase()` 整体重算。注释承认"保持简单"未做增量。
**影响**：query < 64 字符时差异可忽略（μs 级）；stage-4 性能优化目标未彻底达成。
**修复**：`Char(c)` 时 `query_lower.push(c.to_ascii_lowercase())`（c 已经是 char，O(1)）；`Backspace` 时 `query_lower.pop()`。Unicode 大小写映射复杂的字符（如 `İ` → `i̇`）走整体重算 fallback。
**验证**：criterion bench（如 v0.7.0 引入）显示 query=64 时 lowercase 耗时降一个数量级。

### TD-10（P2-10）：DNS PowerShell probe 也走 restricted_spawn ✅ Fixed in v0.7.0 阶段 1

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

> **决策（v0.7.0 阶段 1）**：**不修**。watchdog spawn 是用户主动配置的命令
> （`alerts.toml` 自定义），威胁模型与 DNS PowerShell probe 本质不同：
> 1. PowerShell 走 `-Command` 接受任意脚本，是 RCE 经典跳板；
> 2. watchdog 命令是用户自己写的 binary / shell pipeline，用户最清楚是否需要
>    elevated token（如某些运维命令依赖 SeDebugPrivilege）。
>
> 强制走 restricted_spawn 会破坏依赖 elevated token 的合法用例，引入
> `inherit_privileges` config 选项又会让用户困惑。v0.8.0+ 若有真实需求反馈
> 再加 config 开关；现在不预实现。

---

## v0.8.0+ 候选（6 项）

### TD-12（P2-12）：Linux stub 测试覆盖增强 ✅ Fixed in v0.8.0 阶段 2

**位置**：`tests/` 下仅 `test_inspector.rs` / `test_platform_compat.rs` 2 个文件用 cfg-gate
**现状（已修复）**：新增 `tests/test_linux_stubs.rs`（Linux-only，6 case：env/dlls/handles/memory 对 bogus pid 返回 Err + self pid 走 Ok 路径），并在 `tests/test_platform_compat.rs` 加 4 个跨平台 inspect 契约 case（Windows/Linux/macOS 都跑，bogus pid 一律 Err）。
**影响**：Linux 平台支持的退化（如 env/dlls/handles/memory stub）有早期告警。
**修复（已落地）**：从外部 public API 视角验证 stub 行为的「失败契约」+「成功契约」双向。源码内 `#[cfg(test)] mod tests` 已有同类单元测试，集成测试层再加一层防 lib API 演化时 silently 破损。
**验证**：Linux 上 `cargo test --release --test test_linux_stubs` 跑 6 case；Windows 上文件被 `#![cfg(target_os = "linux")]` 整体 cfg-gate 掉，`running 0 tests`。

### TD-13（P2-13）：CI Linux job 验证 cfg-gate 测试实际运行 ✅ Fixed in v0.8.0 阶段 2

**位置**：`.github/workflows/ci.yml` `check-linux` job
**现状（已修复）**：v0.7 之前 Linux job 只跑 6 个手挑的 `--test xxx`（test_tree / test_throttle / test_record_color / test_platform_compat / test_alert / test_search），cfg(target_os="linux") 写错会静默 skip。v0.8.0 改成跑全量 `cargo test --release` + 测试 bin 数 ≥ 30 校验。
**影响**：cfg-gate 错误静默通过 CI 的风险消除；test_linux_stubs.rs / test_inspector.rs 的 Linux 部分 / 所有 inspect::* Linux 路径在 CI 上真正执行。
**修复（已落地）**：bash step 跑 `cargo test --release`，统计输出中 `test result:` 行数（每测试 bin 一行），< 30 直接 `exit 1` 并打 `::error::` annotation。阈值 30 是 v0.7.0 实际 ~50 个测试 bin 留余地的下限。
**验证**：下次 CI log 的 check-linux job 会显示 `test result: ok. N passed; ...` 多行（≥ 30 行），不再只是 6 行。

### TD-14（P2-14）：panic hook chain 时序验证 ✅ Fixed in v0.7.0 阶段 1

**位置**：`src/main.rs:60-61` + `src/tui/mod.rs::setup_terminal`（未读）
**现状**：`main.rs` 先 `init_tracing` 再 `install_panic_hook`；`run_tui` 内 `setup_terminal` 会用 `take_hook` chain 我们的 hook。需验证 chain 顺序：terminal restore → crash report → 默认 hook。
**影响**：若顺序错，TUI 模式 panic 时终端可能不被 restore（用户看到乱码），或 crash report 不写盘。
**修复**：阶段 8 在 TUI 模式手动触发 panic（如临时加 `panic!("test")` 到 tick），验证：终端正常恢复 + `crashes/` 下出现文件 + stderr 有崩溃提示。
**验证**：手动触发 panic 后终端正常 + crash report 生成。

### TD-15：FilterExpr 仅接入 List view（Tree / AppGroup 视图保留 substring）— v0.7.0 阶段 4 遗留 ✅ Fixed in v0.8.0 阶段 3

**位置**：`src/view_models/process_panel.rs::handle_tree_key` / `handle_app_group_key` / `app_group_filtered_visual_items` / `get_filtered_tree_visible`
**现状（已修复）**：v0.7.0 阶段 4 把 FilterExpr 接入了 List view（`:` 激活 + cached_sorted 缓存按 mode 分支）。v0.8.0 阶段 3 把 Tree / AppGroup 也接通：
- `get_filtered_tree_visible(&self, cached_processes: &[ProcessInfo])` 签名加 cached_processes；FilterExpr 分支建 `pid → &ProcessInfo` HashMap，按 visible TreeNode 的 pid 取原 ProcessInfo 再 `FilterExpr::apply`。Substring 分支保留 v0.6 行为。
- `app_group_filtered_visual_items(&self, cached_processes: &[ProcessInfo])` 同款扩参。FilterExpr 分支：
  - **Header 项（聚合）**：用 group 的 `total_cpu` / `total_memory` + `display_name` 构造合成 ProcessInfo，apply 时 `cpu > 50` 表示「该 .exe 总 cpu > 50」。命中 → 整组保留。
  - **Child 项（单进程）**：Header 不命中时按 pid 查 cached_processes 取原 ProcessInfo，命中的 child 保留并自动展开该组。
- `handle_tree_key` / `handle_app_group_key` 在 `'/'` 旁边加 `':'` 激活 FilterExpr 模式，复用 `SearchState::activate_filter_expr()`。
- 内部 helper（`tree_move_cursor` / `tree_toggle_select` / `tree_initiate_kill` / `tree_select_orphans` / `tree_select_stale` / `app_group_move_cursor` / `app_group_toggle_expand` / `app_group_toggle_select` / `app_group_initiate_kill`）签名加 `cached_processes: &[ProcessInfo]`。
- 外部调用点（`src/app.rs::handle_scroll` / `src/tui/process_tree.rs::draw` / `src/tui/app_group_view.rs::draw`）传 `&app.cached_processes[..]`。
**影响**：Tree / AppGroup 视图按 `:` 进 FilterExpr 模式，`cpu > 5` / `mem > 100mb` / `name =~ /chrome/` 都能正确过滤；parse 失败保留上一次成功 AST（与 List 同款契约）。
**修复（已落地）**：`tests/test_filter_expr.rs` 加 10 个新 case（Tree × 5 + AppGroup × 5）：cpu_gt / pid_equality / keeps_prev_ast_on_bad_input / substring_mode_unchanged / empty_query_returns_all / aggregate_cpu_header_match / child_partial_match / memory_aggregate / app_group_keeps_prev_ast / app_group_substring_mode_unchanged。930 passed / 0 failed（基线 920 + 10 新）。
**验证**：`cargo test --release -q` ≥ 925 passed；clippy + fmt 通过；Tree / AppGroup 视图按 `:` 进 FilterExpr 模式 + 表达式正确过滤。

### TD-16：FilterExpr 错误信息用 nom 内部 ErrorKind 直出 — v0.7.0 阶段 4 遗留 ✅ Fixed in v0.8.0 阶段 2

**位置**：`src/filter/parser.rs::to_parse_error` / `error_kind_to_chinese` / `char_to_chinese`
**现状（已修复）**：parse 失败时 `msg` 字段不再 `format!("expected {:?}", ErrorKind::TakeWhile1)` 直出 nom 内部枚举名。新增 `error_kind_to_chinese(&ErrorKind) -> &'static str`（TakeWhile1 → 「缺少字段名/值」、Tag → 「缺少关键字/操作符」、AlphaNumeric → 「未知字段名」、Verify → 「正则编译失败」、Digit/Float → 「数字格式错误」等）+ `char_to_chinese(char) -> &'static str`（括号 → 「缺少括号」、引号 → 「缺少引号」、斜杠 → 「缺少斜杠」）。括号闭合用 `cut(char(')'))` 让 alt 不回退，`(cpu > 5` 缺 `)` 时最内层错误真正指向 Char(')') 而非被 leaf 的 TakeWhile1 覆盖。
**影响**：TUI 标题栏 / CLI stderr 显示「filter parse error at offset 5: 缺少字段名/值」之类，用户一眼能定位问题。
**修复（已落地）**：`tests/test_filter_expr.rs` 加 3 个中文契约 case（`cpu >` → 含「缺少」、`name =` → 含「缺少」、`(cpu > 5` → 含「括号」），并强化 `err_missing_value` / `err_unbalanced_open_paren` 断言排除 nom 内部枚举名泄漏。
**验证**：`cargo run --release -- ls --filter 'cpu >'` 输出「filter 语法错误: filter parse error at offset 5: 缺少字段名/值」。

### TD-17：eBPF TLS SNI / JA4 指纹采集 — v0.7.0 阶段 8 遗留 ✅ Fixed in v0.12.0 阶段 1（平台决策不再追 Linux eBPF 路径）

**位置**：`src/ebpf/` 整个模块
**现状**：v0.7 阶段 8 MVP 只关联 DNS + connect（[`ProcessFlow`] 的 `dns_name` 通过 [`FlowAggregator`] 的 5s 窗口向前查 DnsQuery 填入）。TLS SNI 需要在 `SSL_write` / `SSL_read` 上挂 uprobe（OpenSSL / BoringSSL / LibreSSL 多个版本分支 + offset 不同），第一版未做。`bytes_out` / `bytes_in` 也留 0（要 hook `tcp_sendmsg` / `tcp_recvmsg`）。
**影响**：
- 命中 DNS cache 的查询（系统直接走 /etc/hosts 或缓存）关联不到 → `dns_name = None`，但 R15 仍能通过条件 2（端口扫描）兜底。
- 无字节计数 → UI Flow 子视图不能回答"实际发了多少字节"，仅能回答"谁连到哪里"。
**修复**（v0.11+ 候选，原计划 v0.8 / v0.9 整体推迟）：
1. 用 `aya-rs` uprobe 在 `SSL_write` 入口抓 ClientHello 明文 SNI 字段（OpenSSL 优先，BoringSSL / LibreSSL 后续）。
2. JA4 指纹：从 ClientHello 抓 cipher suites + extensions + ALPN，按 [RFC 9503](https://www.ietf.org/archive/id/draft-ietf-tls-wg-tls-essentials-01.html) JA4 algorithm hash。
3. `tcp_sendmsg` / `tcp_recvmsg` kprobe 累计 bytes_out / bytes_in（按 `(pid, saddr, daddr)` 聚合）。
**验证**：curl https://example.com 后，端口面板 Flow 子视图该 flow 的 `dns_name == "example.com"` + `bytes_out > 0` + JA4 hash 字段。
**v0.10 cycle 推进**：ProcessFlow.sni 字段已在 v0.10 阶段 1 扩上（v0.9 推迟范围一并完成），但 eBPF 路径仍填 `None`——uprobe 实装需要 Linux 真机环境（与 TD-19 同款推迟到 v0.11+）。Windows 路径已通过 Schannel ETW（ADR-0018）覆盖 SNI 字段（`source = Schannel`）。ja4 / bytes 字段未扩（用户明确「ja4 留 ebpf 那边」）。

> **v0.12.0 阶段 1 标 ✅ Fixed**：[ADR-0022](adr/0022-windows-only-platform.md) 决策 proc 转为 Windows-only 应用，整个 `src/ebpf/` 模块（含 uprobe / kprobe / TLS SNI / JA4 / bytes 计数 path）在 stage 2 整体删除。
>
> - SNI 字段（`ProcessFlow.sni`）已由 v0.10 Schannel ETW（ADR-0018）覆盖 Windows 路径，v0.12 仅保留 `FlowSource::Schannel` 变体，删除 `FlowSource::Ebpf`。
> - JA4 指纹 / bytes_out / bytes_in 计数不再实装——属于纯 Linux eBPF 范畴，平台决策不追这个方向 = TD 自动清零。
> - Linux 用户迁移路径：`git checkout v0.11.0` 仍可用旧 eBPF 路径；如需新功能欢迎 fork（v0.11.0 是最后含 Linux 代码的 release）。

### TD-18：Windows ETW Schannel 抓 SNI（同名功能 Win 版本）— v0.7.0 阶段 8 遗留 ✅ Fixed in v0.10.0 阶段 3

**位置**：`src/schannel_etw/`（v0.10 阶段 1-3 新增，Windows cfg-gate）
**现状**：**已修复**。v0.10 cycle 落地完整路径：阶段 1 ADR-0018 + 骨架 → 阶段 2 实测修订 provider GUID `{91CC1150-71AA-47E2-AE18-C96E61736B6F}`（原 `{37D2C3CD-...}` 不 fire）+ event ID 1793（原推测 196 实测不出现）+ 字段名 `TargetName`（原推测 `ServerName`）+ TDH 动态 schema 解析 + `SnapshotWorker<Vec<SniRecord>>` + WorkerManager 集成 → **阶段 3** `ProcessFlow.source` 字段（`FlowSource` enum）+ `App::overlay_flow_sni_schannel` 把 worker drain 的 SniRecord 关联到 ProcessFlow（pid 匹配覆盖 / 新建 Schannel flow）+ UI 跨平台对齐（`port_table::draw_flow_view` 标题动态切换 ebpf / schannel）+ R15 白名单跨平台（同时检查 sni + dns_name）+ `proc flows` CLI 跨平台（表格加「来源」列 + JSON 加 source 字段）。
**验证**（用户 admin 下自测）：Windows 上 `proc` 后 curl https://example.com → 端口面板按 F 切到 Flow 子视图显示 SNI = "example.com"（来源 Schannel）+ `proc flows` 表格显示「来源 = schannel」列。stage 3 阶段 2 集成测试 `spawn_collects_self_sni_when_admin` 已落地（admin 下验证），用户没在 admin 下跑过（UAC 反复取消），但 stage 3 落地时若用户 admin 跑 proc 看到 SNI 显示正常即间接验证 stage 2 fix 正确。

### TD-19：eBPF Linux 真实编译验证缺失 — v0.7.0 阶段 8 遗留 ✅ Fixed in v0.12.0 阶段 1（平台决策不再追 Linux eBPF 路径）

**位置**：`src/ebpf/{worker.rs,elf_loader.rs}` + `src/ebpf/ebpf-ebpf/src/main.rs`
**现状**：Part A + Part B 都在 Windows 会话落地，未在真实 Linux + root + 内核 5.10+ 环境验证：aya `TracePoint::attach` 真实签名、`RingBuf::try_from` API、tracepoint arg offset（`sys_enter_connect` 偏移 16 / `sched_process_exit` 偏移 24 在不同内核可能不同）、`include_bytes!` ELF 路径硬编码、内核态 `bpf_current_task_start_time` 占位 0（需 aya-tool BTF binding 补完）。
**影响**：Linux 用户首次 `cargo build --features ebpf` 可能失败；attach 失败时 App::flows 为空，UI 显示降级提示（不崩，但功能不可用）。
**修复**（v0.7 收尾或 v0.8）：Linux 会话跑 `cargo +nightly build --target bpfel-unknown-none -p proc-ebpf` + `cargo build --release --features ebpf` + `sudo cargo test --release --features ebpf --test test_ebpf_flow -- --ignored`，按报错修。
**验证**：Linux 真实环境 `proc flows` 显示活跃 flow；端口面板按 F 切换 Flow 子视图有数据。
**v0.8.0 / v0.10.0 cycle 推进**：用户主要用 Windows 开发，stage 1（WSL2 / Linux 真机验证）主动推迟到 **v0.11.0+ cycle 启动前再评估**（v0.8.0 / v0.10.0 都不依赖 ebpf 路径，推迟无成本）。stage 4 review（REVIEW-9 / REVIEW-11）已确认此推迟不影响 cycle 收尾；Linux 验收标准（`cargo +nightly build -p proc-ebpf --target bpfel-unknown-none --release` / `cargo build --release --features ebpf`）跟随 stage 1 跳过；release CI `proc_ebpf` 后缀二进制构建步骤的 `continue-on-error=true` 设计让 Linux 编译失败不阻断主 release（5 target 主二进制优先发货）。README banner + CHANGELOG 显式标注此 known limitation。

> **v0.12.0 阶段 1 标 ✅ Fixed**：[ADR-0022](adr/0022-windows-only-platform.md) 决策 proc 转为 Windows-only 应用，整个 Linux eBPF 路径（src/ebpf/ + ebpf feature flag + workspace ebpf-ebpf sub-project）在 stage 2 整体删除。「Linux 真实编译验证」不再有意义——平台决策直接放弃 Linux 支持，TD 自动清零。
>
> - v0.12 起 `cargo build` 不再尝试编译 ebpf 路径（feature flag 已在 stage 1 改为空 stub，stage 2 删代码时一并清）。
> - release CI 简化：5 target → 1 target（x86_64-pc-windows-msvc），不再有 `proc_ebpf` 后缀二进制 build step。
> - Linux 用户迁移路径：`git checkout v0.11.0` 仍可用旧 eBPF 路径，但 v0.11.0 是最后含 Linux 代码的 release；如需 v0.12+ 新功能欢迎 fork。

---

## v0.11.0+ 候选（v0.10.0 stage 4 review 产出）

### TD-20：Win10 < 1809 版本探测（P2-1 归档）

**位置**：`src/schannel_etw/provider.rs::try_spawn_windows`（候选改造点）
**现状**：Schannel event 1793 是 Win10 1809+ 才有的精细化 TLS handshake 事件（build 17763+）。Win10 < 1809 admin 用户：`try_spawn_windows` 成功（StartTraceW + EnableTraceEx2 + OpenTraceW 全过），但 event 1793 永远不 fire。accum 永远空，UI 显示「Schannel Flow graph（0 条）」误导用户。
**影响**：Win10 早期版本（1709 / 1703）admin 用户以为「没流量」，实际是 OS 不支持 event 1793。
**修复**：评估用 `RtlGetVersion` 在 `try_spawn_windows` 启动时探测 build number < 17763 → 直接返回 None（让 UI 显示「需要 Win10 1809+」更明确的提示）。
**验证**：Win10 1709 admin 下 `try_spawn` 返 None + UI 标题显示版本提示（非「0 条」）。
**v0.10.0 stage 4 决策**：不修。理由：(1) Win10 1809 是 2018-11 发布（7+ 年前），绝大多数用户已升级；(2) `RtlGetVersion` API 在不同 Windows 版本行为不同（manifest-guided 行为可能撒谎），需充分测试；(3) 当前 UI 显示「0 条」不挂 / 不崩，只是 UX 不够友好。归档为 v0.11+ 候选。

### TD-21：Schannel overlay PID 复用防护（P2-2 归档）

**位置**：`src/app.rs::overlay_flow_sni_schannel:1525-1534`
**现状**：overlay 用 `flow.pid == rec.pid` 单键匹配。Schannel event 没给 `start_time`（只有 `EVENT_HEADER.ProcessId`）。进程 A（pid=1000）退出后 pid=1000 被 sysinfo 重用给新进程 B，accum 内 A 的 Schannel event（仍在 1s drain 窗口内）会被 overlay 到 B 的 flow 上。
**影响**：误标一个 flow 的 sni（影响 R15 评分一次），不会崩溃 / 数据破坏。CONTEXT.md 已记录此限制。
**修复**：用 `cached_processes` 查 pid 的当前 start_time，与 flow.start_time 比对，不一致则视为 PID 复用、跳过覆盖（让 record 走「未匹配」分支新建一条 source = Schannel flow）。
**验证**：mock pid=1000 flow（start_time=T1）+ alive_pids 含 pid=1000 start_time=T2 → overlay 跳过；flow.start_time 与 alive 一致 → overlay 命中。
**v0.10.0 stage 4 决策**：不修。理由：(1) 时间窗口窄（accum 1s drain + sysinfo PID 复用罕见）；(2) CONTEXT.md 已记录，用户透明；(3) 影响一次评分不持续，优先级低于 TD-19 ebpf 真实验证。归档为 v0.11+ 候选。

### TD-22：`property_at_index` 生命周期标注代码质量（P2-3 归档） ✅ Fixed（v0.25 stage 1 现状核查确认已修）

**位置**：`src/schannel_etw/provider.rs:456-475`
**现状**：函数签名 `Option<&'static EVENT_PROPERTY_INFO>` 的 `'static` 标注技术上错误——返回的引用生命周期实际绑定到 `info_ptr` 指向的 buffer（来自 `tdh_get_event_info_buffer` 返回的 `Vec<u8>`）。Rust 借用检查无法表达「生命周期绑到 raw pointer 来源」，用 `'static` 绕过。
**影响**：实际用法安全（调用方立即读 `NameOffset` 不跨 await / 不存进长生命周期字段）。Clippy / fmt 不报。
**修复**：改成 `Option<&'a EVENT_PROPERTY_INFO>` + 加 lifetime parameter；或 inline 到调用点直接读字段。
**验证**：`cargo clippy --release --all-targets -- -D warnings` 仍 0 warnings。
**v0.10.0 stage 4 决策**：不修。理由：(1) 不引发 UB（实际用法安全）；(2) 修复增加 ~5 行代码但语义不变；(3) 优先级低于功能 / 测试改进。归档为 v0.11+ 代码质量候选。

> **v0.25 stage 1 回填 ✅ Fixed**：现状核查发现签名已是 `fn property_at_index(info_buf: &[u8], idx: usize) -> Option<&EVENT_PROPERTY_INFO>`（`provider.rs:484`，lifetime elision 自动传播，函数注释明确「不再撒谎说 `&'static`」）——与 TD-35（dns_log/etw.rs 同款问题，v0.12.0 阶段 5 修复）对应，schannel 版在后续重构中已修，仅 TD 状态未回填。本条关闭无代码改动。

---

## v0.12.0+ 候选（v0.11.0 stage 7 REVIEW-13 P2 归档）

> 源：`docs/reviews/REVIEW-13.md` P2 段（15 项）。与 P0/P1 区分：P0/P1 阻断 v0.11.0 发布，已在阶段 8 修；P2 不阻断，归档到下个 cycle。

### TD-23（REVIEW-13 P2-1）：DNS ETW diag JSON 输出不含 dns_collector 字段 ✅ Fixed in v0.12.0 阶段 5

**位置**：`src/cli/diag.rs:54`（human-readable 模式有 dns_collector 行，JSON 模式无）
**现状**：用户用 `proc diag --json` 报 bug 时附上的 JSON 缺 collector 类型信息。JSON 是 bug report 的主要格式，工程化场景几乎都用 JSON。
**影响**：用户报「DNS 日志缺数据」类 bug 附 JSON 时无法看出走的是 ETW 还是 PowerShell fallback。
**修复**：在 JSON 模式输出 object 中加 `"dns_collector": "etw" | "powershell" | "none"` 字段（与 human-readable 的 `dns_collector: <kind>` 行对齐）。
**验证**：`proc diag --json | jq '.dns_collector'` 输出 `"etw"` / `"powershell"` / `"none"`。
**v0.11.0 stage 8 决策**：不修。理由：(1) human-readable 模式已含此信息，用户可临时用 human-readable；(2) JSON schema 变更需在 rmcp MCP tools 同步——优先级低于 P1 修复。归档为 v0.12+ 候选。

### TD-24（REVIEW-13 P2-2）：worker restart spawn_one 失败时 retry_count 不增加，无法到达 permanent_failure ✅ Fixed in v0.25 stage 2

**位置**：`src/workers/manager.rs:215-220`（`try_respawn` 在 spawn_one 返 false 时仅不调 on_respawned）
**现状**：panic 后 `spawn_one` 失败（如 `detect_collector()` 返 None 因环境变化），`on_respawned` 不调用，retry_count 不增加。`state.last_crash` 仍在，下次 `restart_tick`（1s 后）会再次尝试 spawn_one——按 backoff 间隔（5s/30s/5min）重试。这意味着环境持续不支持该 worker 时，**永远无法到达 permanent_failure 状态**——worker 看似在重试，实际每次都失败。
**影响**：少见。典型发生在管理员 → 非管理员权限切换后 ETW worker panic。banner 持续显示 restarting 状态，但实际 respawn 永远失败。
**修复**：在 `spawn_one` 失败时调用 `state.on_respawn_failed(now)` 让 retry_count += 1，达到 MAX_RETRIES 后进入 permanent_failure 止损。
**验证**：mock spawn_one 永远返 false → restart_tick 调 MAX_RETRIES 次后 banner 显示 permanent_failure。
**v0.11.0 stage 8 决策**：不修。理由：(1) 实际触发场景极少（权限切换）；(2) banner 已显示 restarting，用户能感知；(3) 修复需新增 RestartState::on_respawn_failed 方法 + 状态机扩展测试，改动中等。归档为 v0.12+ 候选。

> **v0.25 stage 2 标 ✅ Fixed**（commit `43300d9`，ADR-0035 D3）：`RestartState::on_respawn_failed(now)`（retry_count `saturating_add(1)` + `last_crash` 重定位到失败尝试点——backoff 从该点重新起算）+ `try_respawn` 在 `spawn_one` 返 false 时调用。测试按 TD 原文 mock 口径：非 canonical thread_name（`"mock-failing-worker"`）直插 `restart_history` 走 `spawn_one` `_ => false` 确定性失败（规避 disk-io-etw 需管理员的环境不确定性），三次 tick（5s/30s/300s）后 retry_count == 3 + `PermanentFailure` banner + 止损不再尝试。

### TD-25（REVIEW-13 P2-3）：docker worker 不在 canonical_worker_thread_name 列表 ✅ Fixed in v0.25 stage 1（ADR-0019 追加决策 8）

**位置**：`src/workers/manager.rs:278-288`（`canonical_worker_thread_name` 列表）+ ADR-0019 未明确文档化此例外
**现状**：`canonical_worker_thread_name` 列出 6 个 worker（port / usb / net-flow / dns-log / disk-io-etw / schannel-etw），不含 docker-snapshot-worker / docker-logs-worker-{name}。docker worker panic 时 `WorkerManager::restart` 因 canonical 返回 None 直接返 false，docker worker 不会自动 respawn。
**影响**：CONTEXT.md line 9 of manager.rs 注释明确「Docker worker 仍由 DockerPanel 自管」，但 ADR-0019 文档未明确这一例外，未来维护者会困惑。
**修复**：在 ADR-0019 §决策 7「不实装 ebpf_worker restart」之后追加「不实装 docker worker restart：DockerPanel 自管 worker 生命周期，独立 spawn/drop 逻辑」。或者把 docker worker 也接入 restart（需重构 DockerPanel 把 worker handle 暴露给 WorkerManager）。
**验证**：ADR-0019 文档明确 docker 例外；或 docker worker panic 后也走 restart 路径。
**v0.11.0 stage 8 决策**：不修。理由：(1) ADR 文档补充；(2) docker worker 接入 restart 需重构 DockerPanel（影响大）。归档为 v0.12+ 文档候选。

> **v0.25 stage 1 标 ✅ Fixed**：ADR-0019 追加决策 8「不实装 docker worker restart：DockerPanel 自管」——设计例外显式文档化（纯 doc，不重构 DockerPanel）。

### TD-26（REVIEW-13 P2-4）：HRESULT 映射不完整，CERT_E_EXPIRED / CERT_E_UNTRUSTEDROOT 都归 Unknown ✅ Fixed in v0.12.0 阶段 3

**位置**：`src/security/signature.rs:83-93`（`from_wintrust_result`）
**现状**：仅映射 3 个 HRESULT：0 → Signed / TRUST_E_SUBJECT_NOT_SIGNED → Unsigned / CRYPT_E_REVOKED → Revoked。其他都归 Unknown 扣 5 分（仅 Windows）。
未映射的关键 HRESULT：
- `CERT_E_EXPIRED` (0x800B0101) — 证书过期（应类似 Unsigned 严重）
- `CERT_E_UNTRUSTEDROOT` (0x800B0109) — 不受信根（应类似 Unsigned 严重）
- `CERT_E_WRONG_NAME` (0x800B0113) — 名称不匹配
- `TRUST_E_CERT_SIGNATURE` (0x80096010) — 签名无效
- `CERT_E_CHAINING` (0x800B010A) — 链断裂
**影响**：证书过期 / 不受信根的进程扣分偏宽松（Unknown 5 vs 应 15-20）。
**修复**：扩 from_wintrust_result 映射 + 加 SignatureStatus 变体（如 `Expired` / `UntrustedRoot`），或在 Unknown 桶内细分 weight。
**验证**：mock 各 HRESULT → 验证 SignatureStatus 映射 + 扣分 weight。
**v0.11.0 stage 8 决策**：不修。理由：(1) 当前 6 状态机已能区分主要场景；(2) 扩状态机需改 badge / Display / risk_factor 多处连锁；(3) 影响窄（CERT_E_EXPIRED 等不常见）。归档为 v0.12+ 候选。

> **v0.12.0 阶段 3 标 ✅ Fixed**：SignatureStatus 扩到 9 变体（加 Expired / UntrustedRoot / ChainError），from_wintrust_result 扩 5 HRESULT 映射（CERT_E_EXPIRED / CERT_E_UNTRUSTEDROOT / CERT_E_CHAINING / CERT_E_WRONG_NAME / TRUST_E_CERT_SIGNATURE），badge / Display / risk_factor 全部连锁更新。tests/test_signature.rs 扩到 38 case 覆盖新变体。详见 CONTEXT.md 术语演进历史 v0.12.0 阶段 3 行。

### TD-27（REVIEW-13 P2-5）：TRUSTED_SIGNERS 列表较短，缺常见 vendor ✅ Fixed in v0.12.0 阶段 3

**位置**：`src/security/signature.rs:50-59`
**现状**：仅 8 个 vendor：Microsoft / Google / Mozilla / Apple / Intel / NVIDIA。
**缺**：Adobe / Cisco / Oracle / VMWare / Docker / Red Hat / Apache Software Foundation / Python Software Foundation / Electron.js / GitHub 等。
**影响**：常见软件（如 Adobe Reader / Cisco VPN / Docker Desktop / Oracle JDK）的进程被标为 `Signed`（扣 10 分）而非 `Trusted`（不扣分），用户视角误报。
**修复**：扩列表 + 走 `path_rules.toml` 类似的用户配置入口（`trusted_signers.toml`），让用户标记自家应用。
**验证**：常见 vendor 进程显示 🔒 而非空 badge。
**v0.11.0 stage 8 决策**：不修。理由：(1) 列表扩充争议（哪些 vendor 算「trusted」主观）；(2) 用户配置入口 `trusted_signers.toml` 需新 schema + UI 反馈机制，优先级低于 P1 修复。归档为 v0.12+ 候选（与 path_rules / lineage_rules 一同设计统一用户规则系统）。

> **v0.12.0 阶段 3 标 ✅ Fixed**：TRUSTED_SIGNERS 内置列表扩到 24 vendor（加 Adobe / Cisco / Oracle / VMWare / Docker / Red Hat / Apache / Python / GitHub / Electron / AMD 等）；新建 `src/security/trusted_signers.rs`（~190 行）实装 `TrustedSignersRule` + `load_trusted_signers()` 读 `~/.config/proc/trusted_signers.toml`；`verify_signature_with_policy` 加 `trusted_rules` 参数集成；SecurityScorer 加 `trusted_signers_rules` 字段构造时一次性加载。用户零配置即可正确评分 24 个常见 vendor。

### TD-28（REVIEW-13 P2-6）：regex 中不能 escape `/`，影响 CIDR / URL pattern ✅ Fixed in v0.12.0 阶段 4

**位置**：`src/filter/parser.rs:425-431`（`parse_regex_lit` 用 `take_till1(|c| c == '/')`）
**现状**：用户写 `remote_addr =~ /127\.0\.0\.1\/8/` 想匹配 CIDR `127.0.0.1/8`，但 parser 在第一个 `/` 停止，pattern 变成 `127\.0\.0\.1\`，剩余 `/8/` 被当成 trailing input 报错。
**影响**：CIDR / URL pattern 不能直接表达，用户需用 `[\/]` character class（regex crate 支持）绕。
**修复**：要么支持 `\/` escape（修改 parser），要么文档建议用户用 `[\/]`。
**验证**：`remote_addr =~ /127\.0\.0\.1\/8/` 正确解析为 pattern `127\.0\.0\.1/8`。
**v0.11.0 stage 8 决策**：不修。理由：(1) 影响 narrow（CIDR 用得少）；(2) `[\/]` workaround 可用；(3) parser 改动需考虑转义序列连锁（`\d` / `\w` 等是否也支持）。归档为 v0.12+ 候选。

> **v0.12.0 阶段 4 标 ✅ Fixed**：`parse_regex_lit` 改用状态机扫描——遇 `\` + `/` → pattern 追加单 `/`（drop 反斜杠）；`\` + 其他字符 → `\X` 原样保留让 regex crate 解释；非转义 `/` → pattern 结束。兼容性：旧表达式（无 `\/`）行为不变。tests/test_filter_expr.rs 加 6 case 覆盖 CIDR / URL / 路径场景。

### TD-29（REVIEW-13 P2-7）：NetworkIn 用 Vec 线性查找 ✅ Fixed in v0.12.0 阶段 5

**位置**：`src/filter/mod.rs:270-280`（`FilterExpr::apply_network` 的 NetworkIn 分支用 `values.iter().any(...)`）
**现状**：N 个值的 in 列表，每个 flow 检查 O(N)。N 通常 < 10，但极端用户写 100 个 IP 黑名单 + 1000 个 flow → 100K 操作每 tick。
**修复**：在 FilterExpr::NetworkIn 构造时把 Vec 转为 HashSet，apply 时 O(1) 查找。改动小（~10 行）。
**验证**：benchmark NetworkIn 100 values × 1000 flows 不超过 1ms。
**v0.11.0 stage 8 决策**：不修。理由：(1) N 通常 < 10，性能差异微秒级；(2) 当前 50ms tick 预算充足。归档为 v0.12+ 性能候选。

### TD-30（REVIEW-13 P2-8）：`%` 单位与 cpu / mem 字段交互语义不清 ✅ Fixed in v0.12.0 阶段 4

**位置**：`src/filter/parser.rs:406-414`（`parse_number_value` 的 `%` 分支）
**现状**：`mem > 5%` 解析为 `Value::Percent(5)`，与 `mem > 5`（字节）在 `apply_num` 下等价（5 == 5）。用户期望 `mem > 5%` 是「内存占用 > 5%」（基于总内存），实际是「内存字节数 > 5 字节」。
**影响**：用户写 `mem > 50%` 期望过滤占用 50%+ 内存的进程，实际过滤内存 > 50 字节的进程（几乎全部命中）。
**修复**：在 `Field::Mem::extract` 中把字节转 % 总内存（需 `System::total_memory()`），或者在 parser 阶段拒绝 `mem%` 组合（更严格）。
**验证**：`mem > 50%` 在 16GB 系统上等价于 `mem > 8GB`。
**v0.11.0 stage 8 决策**：不修。理由：(1) cpu 字段本身就是 %，与 mem 字段单位不一致是历史问题；(2) 修复需 EvalCtx 加 total_memory 字段或 parser 严格化（破坏向后兼容）。归档为 v0.12+ UX 候选（与 FilterExpr v3 字段单位语义重构一同设计）。

> **v0.12.0 阶段 4 标 ✅ Fixed**：EvalCtx 加 `total_memory: u64` 字段；apply 在 `(Num, Percent)` 分支检测 `field == Mem && total_memory > 0` 走换算路径 `mem / total_memory * 100.0` 与百分号字面量比较；total_memory == 0（测试 / 未知容量）退回 legacy 避免 div by zero。ProcessPanel 加 `total_memory` 字段 init_tree / refresh_tree 同步刷新。cpu 字段本身就是 0-100 标度不变；disk_read/write / net_sent/recv 字段没有自然除数保留 legacy。tests/test_filter_expr.rs 加 6 case 覆盖 mem% 换算 + 边界条件。

### TD-31（REVIEW-13 P2-9）：跨 ctx 表达式不支持（如 `cpu > 5 AND sni =~ /evil/`）

**位置**：`src/filter/mod.rs:204-288`（`apply` 与 `apply_network` 完全分离）
**现状**：Flow 视图调 `apply_network` 时，process 字段变体（FieldCmp / Regex）返 false。`cpu > 5 AND sni =~ /evil/` 在 Flow 视图下：`cpu > 5`（false） AND `sni =~ /evil/`（true） → false。
**影响**：用户视角：「我想看 chrome 进程的 evil.com flow」无法直接表达。需先在 Process 视图找 chrome pid，再在 Flow 视图按 pid 过滤。
**修复**：在 NetworkEvalCtx 加 `process: Option<&ProcessInfo>` 字段，apply_network 对 process 变体在 process 存在时走 apply 逻辑。Flow 视图构造 ctx 时通过 pid 关联 process。改动中等（~50 行），与 surgical 原则冲突（ctx 类型不再纯净）。
**验证**：`cpu > 5 AND sni =~ /evil/` 在 Flow 视图下命中 chrome 高 CPU + evil.com SNI 的 flow。
**v0.11.0 stage 8 决策**：不修。理由：(1) 类型系统分离是 ADR-0011 v0.11 阶段 3 设计选择（保证字段不跨 ctx 误用）；(2) 修复破坏 surgical 原则，与 stage-3 doc 任务指令「类型系统保证字段不跨 ctx 误用」冲突。归档为 v0.12+ 设计候选（需重新评估 FilterExpr 整体架构）。

### TD-32（REVIEW-13 P2-10）：R17 ScriptInterpreter 不分场景扣分（系统登录脚本也命中） ✅ Fixed in v0.12.0 阶段 5

**位置**：`src/security/lineage.rs:179-182`（`detect_suspicious_chain` 的 ScriptInterpreter 优先级）
**现状**：当前进程是 wscript/cscript/mshta 即扣 15 分，不看祖先。系统登录脚本 / IT 部门部署脚本都命中。
**影响**：企业环境常见 wscript.exe 启动脚本，被标可疑。15 分扣分较低，但用户视角误报。
**修复**：增加「直接父是 services.exe / wininit.exe（系统启动）→ 不扣分」白名单。或降低 weight 到 5。
**验证**：mock chain [services.exe → wscript.exe] → 不命中 ScriptInterpreter。
**v0.11.0 stage 8 决策**：不修。理由：(1) 15 分扣分较轻，不影响用户使用；(2) 修复需扩 lineage_rules.toml schema 支持白名单（与 TD-27 trusted_signers.toml 一同设计）。归档为 v0.12+ UX 候选。

### TD-33（REVIEW-13 P2-11）：R18 + path_check 叠加扣分导致 Downloads 等合法路径扣 30 分 ✅ Fixed in v0.12.0 阶段 5

**位置**：`src/security/score.rs` 第 3 步（path_check）+ 第 18 步（R18）
**现状**：用户从 Downloads 运行合法安装包（如 VS Code installer），同时命中：
- v0.6 path_check downloads_dir (15)
- R18 UserProfileDownloads (15)
- 总扣分 30

CONTEXT.md 明确「surgical 原则——安全评分偏向严格」。这是设计选择，但**用户视角是误报**。
**修复**：在 path_check 内部「命中 downloads_dir 时跳过 R18 检查」（去重），或者 R18 UserProfileDownloads weight 从 15 降到 5。
**验证**：Downloads 路径签名进程扣分从 30 降到 15（或 20）。
**v0.11.0 stage 8 决策**：不修。理由：(1) surgical 原则明确「叠加扣分」是设计行为；(2) 用户配置 path_rules.toml 可以补充注释解释；(3) 修复需评估 R18 内部 weight 调整对其他场景的影响。归档为 v0.12+ UX 候选。

### TD-34（REVIEW-13 P2-12）：plan.md 不用 [x] checkbox 风格，stage-7.md 任务清单描述与实际不匹配 ⬜ Obsolete（v0.25 stage 1 判废）

**位置**：`plan.md`（表格风格）+ `docs/stages/v0.11-stage-7.md:55`（假设 checkbox 风格）
**现状**：stage-7.md 任务清单第 7 项「plan.md 中所有功能阶段已 [x]」假设 plan.md 用 checkbox 标记阶段完成情况，但 plan.md 实际是表格风格（「阶段 N 实装：...」描述）。
**影响**：v0.11 阶段 7 review 时按不存在的格式提问，浪费精力。
**修复**：要么改 plan.md 用 checkbox（破坏现有风格），要么改 stage-7.md 任务清单第 7 项描述（更准确：「plan.md 阶段表 + CONTEXT.md 演进历史段全部更新到 v0.11」）。后者更 surgical。
**验证**：stage-7.md 任务清单第 7 项描述与 plan.md 实际风格匹配。
**v0.11.0 stage 8 决策**：不修。理由：(1) stage-7.md 已落 ✅，本次 cycle 不再触发；(2) 文档风格调整优先级低。归档为 v0.12+ 文档候选。

> **v0.25 stage 1 判废（Obsolete）**：v0.13+ cycle 起 plan.md 已不在流程中（ADR-0001 phased-project skill adoption——brainstorm.md 替代 plan.md 作 cycle-level 项目宪法，进度追踪在 brainstorm stage 总览表唯一勾选点）。TD 描述的「风格不匹配」对象已不存在，关闭。

### TD-35（REVIEW-13 P2-13）：`property_at_index` 的 `'static` lifetime 不正确（DNS ETW 版本） ✅ Fixed in v0.12.0 阶段 5

**位置**：`src/dns_log/etw.rs:510-526`
**现状**：返回 `&'static` 但实际生命周期与 `info_ptr` 指向的 buffer 绑定（调用方 info_buf 保活）。严格说应改为 `Option<&'a EVENT_PROPERTY_INFO>` + 加生命周期参数。实际不会触发 use-after-free（info_buf 在调用栈保活），但是 API 契约不准确。**与 TD-22 同款问题（schannel_etw 版本）**。
**影响**：实际用法安全（调用方立即读字段，不跨 await / 不存进长生命周期字段）。
**修复**：加 lifetime parameter。与 TD-22 一同修复。
**验证**：`cargo clippy --release --all-targets -- -D warnings` 仍 0 warnings。
**v0.11.0 stage 8 决策**：不修。理由：与 TD-22 同款（不引发 UB / 修复增加代码但语义不变）。归档为 v0.12+ 代码质量候选。

### TD-36（REVIEW-13 P2-14）：MCP DNS tool 拿不到历史（每次调用重启 ETW session） ✅ Fixed in v0.12.0 阶段 5

**位置**：`src/mcp/handler.rs:891-910`（`make_dns_json`）
**现状**：MCP 每次调用 `proc_dns` 都创建一个临时 `EtwDnsCollector`（启动 ETW session + spawn ProcessTrace 线程），drain 一次拿现有数据，然后 collector drop（关闭 session）。**启动前发生的 DNS 查询无法被捕获**——session 启动后到 drain 之间的查询（短暂窗口）才能拿到。
**影响**：MCP 用户调用 `proc_dns` 通常拿到空结果或少量结果（取决于 drain 间隔）。与 v0.6 PowerShell 路径行为一致，不是 v0.11 引入。
**修复**：让 MCP handler 持有长生命的 EtwDnsCollector（与 App::workers.dns_log_worker 类似的生命周期）。改动中等（MCP handler 需 state 化）。
**验证**：MCP `proc_dns` 调用前触发 DNS 查询，调用时能拿到结果。
**v0.11.0 stage 8 决策**：不修。理由：(1) 与 v0.6 行为一致，不引入回归；(2) MCP handler state 化改动大（涉 rmcp SDK state 传递机制）。归档为 v0.12+ 候选。

### TD-37（REVIEW-13 P2-15）：`signature_risk_factor` 中 `_ => None` 通配符可能掩盖新加变体 ✅ Fixed in v0.11.0 阶段 8

**位置**：`src/security/signature.rs:264`
**现状**：`signature_risk_factor` 用 `_ => None` 通配符兜底 Trusted 变体（不扣分），但未来加新 SignatureStatus 变体也会静默落入此桶。
**影响**：未来加新 SignatureStatus 变体时编译器不会强制更新 match，新变体可能静默不扣分。
**修复**：把 `_ => None` 改为 `SignatureStatus::Trusted => None`，让编译器在新加变体时强制更新 match。
**验证**：`cargo build` 在加新 SignatureStatus 变体时报「non-exhaustive match」错误。
**v0.11.0 stage 8 决策**：✅ **已修复**（合并到 P1-3 修复）。`signature_risk_factor` 的 `_ => None` 已改为显式 `SignatureStatus::Trusted => None`，未来加新变体时编译器会强制穷尽 match。本 TD 标 ✅ Fixed in v0.11.0 阶段 8。

---

## v0.13.0+ 候选（v0.12.0 stage 6 REVIEW-14 P2 归档）

> v0.12.0 cycle stage 6 全局 Review（详见 [`docs/reviews/REVIEW-14.md`](reviews/REVIEW-14.md)）产出 7 个 P2，归档为 TD-38 ~ TD-43。覆盖跨平台残留 cfg gate 清理 / regex DoS 防护 / SYSTEM_BOOT_ENTRIES 严格化 / CI workflow 更新等方向。**v0.12.2 闭环 4 项**（TD-38 / 39 / 42 / 43），剩余 TD-40 / TD-41 留 v0.13+。

### TD-38（REVIEW-14 P2-1）：`signature.rs` mock policy 路径 cfg gate 残留 ✅ Fixed in v0.12.2

**位置**：`src/security/signature.rs:232`（`#[cfg(not(target_os = "windows"))]` 块）
**现状**：v0.11 stage 4 ADR-0021 设计的 mock policy 测试入口——`policy_override` 路径让非 Windows CI 也能跑 mock HRESULT。v0.12 Windows-only 后此块永不编译（mock 路径在 Windows 分支 `if let Some(result) = policy_override` 已短路）。
**影响**：代码冗余（约 10 行 dead branch），不影响功能；保留更安全（让代码能在非 Windows cargo check 通过）。
**修复**：surgical 清理——删除 `#[cfg(not(target_os = "windows"))]` 块；mock policy 路径合并到 Windows 分支前的 `if let Some(result) = policy_override` 短路逻辑。
**验证**：`cargo build --release` + 7 个 mock_policy_* unit test 全过。
**v0.12.2 决策**：✅ **已修复**。删 `verify_signature_with_policy` 内的 `#[cfg(not(target_os = "windows"))]` 块 + 同步移除 Windows 分支上的 `#[cfg(target_os = "windows")]` 属性（proc 转为 Windows-only 后 attr 冗余）。7 个 `mock_policy_*` unit test 全过，行为完全一致（mock 路径由函数顶部的 `if let Some(result) = policy_override` 短路）。原 stage 6 决策「保留让代码跨平台 cargo check 友好」在 v0.12 release 稳定后翻盘——proc 已正式 Windows-only，不再追求跨平台 cargo check。

### TD-39（REVIEW-14 P2-2）：`tests/test_inspect.rs` macOS stub 测试 mod cfg gate 残留 ✅ Fixed in v0.12.2

**位置**：`tests/test_inspect.rs:97`（`#[cfg(not(any(target_os = "windows", target_os = "linux")))]`）
**现状**：macOS / 其他非 Win/Linux 平台的 stub 测试 mod（env / dlls / handles / memory 在不支持平台返 PermissionDenied）。v0.12 Windows-only 后此 mod 永不编译。
**影响**：约 30 行测试代码冗余，不影响功能；保留让代码能在 macOS cargo check 通过（贡献者用 macOS 开发时 cargo test 不报错）。
**修复**：删除整个 `non_target_stubs` mod。
**验证**：`cargo test --release` 全过（无 macOS 测试需要保留）。
**v0.12.2 决策**：✅ **已修复**。删整个 `non_target_stubs` mod；文件顶部 doc comment 同步把「三类用例」改「两类用例」（删 macOS 跨平台条目）。原 stage 6 决策「macOS 贡献者友好」在 v0.12 release 稳定后翻盘——贡献者实际只在 Windows 开发，保留 dead mod 反而误导。

### TD-40（REVIEW-14 P2-3）：`trusted_signers.toml` regex 无复杂度限制 ✅ Fixed in v0.25 stage 2

**位置**：`src/security/trusted_signers.rs:74`（`regex::Regex::new(&raw.vendor_pattern)`）
**现状**：用户在 `~/.config/proc/trusted_signers.toml` 配置的 `vendor_pattern` 直接传给 `regex::Regex::new`，无 size / 复杂度限制。
**影响**：理论 ReDoS 风险（如 `(?i)^.*a.*b.*c.*$` 在长字符串上慢）。实际匹配对象是 `CompanyName` 字段（一般 < 100 字符），风险低。
**修复**：加 regex 复杂度 lint 或 size limit（如 `regex::RegexBuilder::size_limit(64 * 1024)`）。
**验证**：构造极端 regex 验证 RegexBuilder 拒绝。
**v0.12.0 stage 6 决策**：不修。理由：(1) regex crate 自身有 NFA simulation 防回溯爆炸；(2) 实际匹配对象长度短，风险低；(3) 修复需评估 size_limit 阈值（过严误报 / 过松无效）。归档为 v0.13+ 安全候选。

> **v0.25 stage 2 标 ✅ Fixed**（commit `43300d9`，ADR-0035 D3）：`RegexBuilder::new(pattern).size_limit(64 * 1024).build()` 替换裸 `Regex::new`——嵌套量词展开超编译预算的 rule 被 filter_map 拒绝（测试用 `(a{300}){300}` 展开 ~90K 指令验证拒绝 + 普通 `^Adobe` 存活）。regex crate 自带 API，deps +0。

### TD-41（REVIEW-14 P2-4）：SYSTEM_BOOT_ENTRIES 白名单按 process name 不严格

**位置**：`src/security/lineage.rs:134`（`SYSTEM_BOOT_ENTRIES = ["services.exe", "wininit.exe", "svchost.exe"]`）
**现状**：TD-32 实装的白名单按 process name 匹配（lowercase），不按 image path。攻击者需先 privilege escalation 才能 spawn 这些名字的进程，PID 复用风险存在但极低。
**影响**：理论攻击场景（attacker 已有 privilege escalation → spawn `services.exe` 在非系统目录 → 绕过 R17 系统启动白名单）。实际触发需先攻破其他防线。
**修复**：加 image path 校验（调 `QueryFullProcessImageName` 拿全路径，要求 `C:\Windows\System32\services.exe`）。
**验证**：构造非系统目录的 `services.exe` → 验证不被白名单命中。
**v0.12.0 stage 6 决策**：不修。理由：(1) 攻击场景需先 privilege escalation，安全评分已是事后防线；(2) 修复需引入新的 windows-rs API 调用 + 缓存机制（性能影响）；(3) surgical 原则下当前实现可接受。归档为 v0.13+ 安全候选。

### TD-42（REVIEW-14 P2-6）：`.github/workflows/ci.yml` 仍有 `check-linux` job ✅ Fixed in v0.12.2

**位置**：`.github/workflows/ci.yml:36-42`（`check-linux` job 在 ubuntu-latest 上跑全量 cargo test）
**现状**：v0.12 Windows-only 后 Linux CI 永远失败（src/ Linux 路径已删，ubuntu-latest 上 cargo build 不通过）。stage 2 doc 任务 7 应删但未删。
**影响**：GitHub Actions PR check 永远红叉，掩盖真正的 CI 问题；contributor 困惑。
**修复**：删除整个 `check-linux` job。
**验证**：GitHub Actions PR check 不再有 Linux job。
**v0.12.2 决策**：✅ **已修复**。删整个 `check-linux` job。`check-macos` / `msrv` / `audit` 三个 job 保留（`msrv` 验证 `rust-version = "1.85"` + `audit` 扫描 Cargo.lock 漏洞，与平台无关；`check-macos` 即使 cargo check 失败也只是黄叉不阻断 master push，留着无害）。原 stage 6 决策「归档为 v0.13+ CI 整理候选」在 v0.12.0 release 稳定后翻盘——TD-43 同本批次处理。

### TD-43（REVIEW-14 P2-7）：`.github/workflows/release.yml` Linux / macOS target 应删 ✅ Fixed in v0.12.2

**位置**：`.github/workflows/release.yml`（如有 Linux / macOS build matrix target）
**现状**：v0.12 Windows-only 后 Linux / macOS build target 永远失败（src/ Linux 路径已删）。stage 2 doc 任务 7 应删但未删。
**影响**：release workflow 触发时 Linux / macOS build 步骤失败，整个 release 卡住。
**修复**：删 Linux / macOS build target，仅保留 `x86_64-pc-windows-msvc`。
**验证**：触发 release workflow 全过。
**v0.12.2 决策**：✅ **已修复**。build matrix 从 5 target 裁到 1 target（仅 `x86_64-pc-windows-msvc`），同步删 Linux musl tools 安装步 / Linux ebpf kernel + userspace 二进制构建步 / 对应的 Package / Upload artifact / Upload to Release 条件分支。`update-winget` job 不动（原本只引用 Windows artifact）。

---

## v0.14.0+ 候选（v0.13.0 stage 2 PERF-BASELINE 归档）

> v0.13.0 cycle stage 2 Slice 产出 [`docs/reviews/PERF-BASELINE-v0.13.md`](reviews/PERF-BASELINE-v0.13.md)，分析 6 个 criterion benchmark × 多档 fixture 共 25 个数据点。**用户选方案 c**：cycle 缩到 4 stage（baseline + 报告 + Review + 收尾），不动业务代码。stage 2 评估的 4 个候选项（含 1 个中 ROI + 2 个低 ROI + 1 个侦察报告误读）全部归档为 TD-44 ~ TD-47 留 v0.14+ cycle 评估。**核心结论**：proc 当前架构在 1000 进程规模下无用户感知瓶颈；唯一 mean > 5 ms 的 hot path（parent_chain 16.5 ms）在 worker 独立线程不阻塞 UI。

### TD-44（PERF-BASELINE 候选 2）：tui_draw format! 风暴优化（低 ROI）

**位置**：`src/tui/process_table.rs:71-159` + `src/format.rs:3-46`
**现状**：criterion `bench_tui_draw` 实测 5.6 ms @ 1000 进程，但**数字高估**——bench 对全部 1000 行跑 `format!`，生产代码 `src/tui/process_table.rs:71-75` 用 `.skip(scroll_offset).take(rows_visible)` 只格式化可见行（典型 30-50 行）。真实生产单帧 < 1 ms。
**影响**：用户感知不到差（bench 5.6 ms 是 fixture 全量，真实路径 ~500 µs/frame）。优化后 bench 可降到 ~2 ms，真实路径降到 ~200 µs，**用户无感**。
**修复**：用 `itoa` / 直接 `Cell<Cow<_>>` 替换 `format_bytes` / `format_speed` / `format!("{:.1}", cpu)` 等热路径 format!。
**验证**：`cargo bench --bench bench_tui_draw` 数字下降；全量回归 ≥ 1115 passed 不变。
**v0.13 stage 2 决策**：不修。理由：(1) bench 高估，真实生产 < 1 ms 已在用户无感区；(2) 用户感知不到差；(3) 修复需重写 5+ 个 format! 调用点 + 单元测试，工作量 ~150 行。归档为 v0.14+ 候选。

### TD-45（PERF-BASELINE 候选 3）：record deserialize 加速（低 ROI）

**位置**：`src/record/reader.rs`（bincode deserialize 路径）
**现状**：criterion `bench_record_serialize` 实测 deserialize 165 µs @ 1000 进程，比 serialize（14 µs）慢 **12×**。
**影响**：replay 路径偶发触发（用户按方向键 seek 一帧），165 µs 单帧 seek 完全无感。但 30 min session × 30 FPS = 54000 frames，如做「快进/倒放」连续 seek 多帧，体验可能下降。
**修复**：换 bincode option（如 `bincode::config::standard().with_little_endian().with_fixint_encoding()` vs `varint`），或考虑零拷贝方案。但改 bincode 配置影响向后兼容（旧录屏文件格式），需迁移层。
**验证**：构造 1000 进程 × 1800 frames 的录屏文件，跑连续 seek；旧文件向后兼容。
**v0.13 stage 2 决策**：不修。理由：(1) replay 偶发触发，用户无感；(2) 改 bincode 配置影响向后兼容，风险高；(3) 工作量 ~200 行 + 兼容层。归档为 v0.14+ 候选。

### TD-46（PERF-BASELINE 附录）：command_palette fuzzy 优化（侦察报告误读）

**位置**：`src/tui/command_palette.rs:225-237`（`recompute_matches` nucleo fuzzy）
**现状**：v0.13 brainstorm 侦察报告说「`command_palette.rs:813, 912` 的 `to_lowercase().contains()` 每帧」。**实际查证**：line 813 / 912 在 `#[test]` 模块（`empty_query_matches_all_items` / `theme_items_in_matches` 等单元测试的 assert），**不是生产路径**。
**影响**：侦察报告这条疑点不成立；生产 fuzzy 用 nucleo（line 109-150 matcher 已复用避免每次按键重建），性能无问题。
**修复**：N/A（侦察报告纠错）。
**验证**：N/A。
**v0.13 stage 2 决策**：N/A — 侦察报告误读，无需修复。归档以记录纠错历史（避免 v0.14+ cycle 重新调查）。

### TD-47（PERF-BASELINE 候选 1）：parent_chain Arc 重构（中 ROI）

**位置**：
- 字段定义：`src/collect.rs:588` — `pub parent_chain: Vec<(u32, String)>`
- 写入热路径：`src/collect.rs:953-966`（`pid_to_chain` 构建 + `chain.clone()` 写回）
- `build_parent_chain` 实函数：`src/security/lineage.rs:149-176`（line 172 `parent_proc.name.to_string()` 每祖先 1 次 String alloc）
- UI 消费者：`src/tui/detail_view.rs:371-376, 406-413`（`.as_str()`）
- 评分消费者：`src/security/lineage.rs:200, 209, 222-231, 333-338`（`.as_str()`）
- 其他构造点：`src/record/conversions.rs:49` / `src/eject/locks.rs:92` / 测试 `src/security/lineage.rs:574`

**现状**：criterion `bench_refresh_heavy` 实测 1000 进程 mean **16.5 ms**（worker 独立线程 `proc-heavy-refresh`，2s 周期，0.83% 持续 CPU）。每周期 ~10000 次 heap alloc（1000 进程 × 5 平均链深 × 2 次 clone —— `build_parent_chain` 内 push + `chain.clone()` 写回）。
**影响**：
- **不阻塞 UI 帧预算**（worker 独立线程）
- **0.83% CPU 持续负载**（笔记本电池场景可忽略）
- **~10000 allocs/s 的 GC 压力**（现代 allocator 上无感，但长期 cache miss 可累积）

**修复方案**：
1. `ProcessInfo::parent_chain: Vec<(u32, String)>` → `Vec<(u32, Arc<str>)>`（保留 serde 兼容性 — `Arc<str>` 不 impl Serialize，需要在 `FrameProcess` 中转层做 `String` 转换，FrameProcess 已是 String）
2. `build_parent_chain` 把 `parent_proc.name.to_string()` 改 `Arc::clone(&parent_proc.name)` — 零 heap alloc
3. 把 `chain.clone()` 改成 `Arc::clone` —— 需要把 `Vec<(u32, Arc<str>)>` 升一级为 `Arc<[(u32, Arc<str>)]>`，processes 写入时 Arc 整链一次
4. UI/评分消费者：`.map(|(_, n)| n.as_str())` → `.map(|(_, n)| n.as_ref())` 或 `&**n`

**验证**：`cargo bench --bench bench_refresh_heavy` 数字从 16.5 ms 降到 ~3-5 ms（kill 90% 堆分配后剩 HashMap 构建 + Arc atomic increment）；全量回归 ≥ 1115 passed 不变。

**v0.13 stage 2 决策**：不修。理由：(1) worker 独立线程不阻塞 UI；(2) 0.83% CPU + 10000 allocs/s 在现代 allocator 上无感；(3) 修复涉及 ~10 处消费点签名变更 + serde 兼容层，~300-400 行 + 测试 + bench 更新；(4) 用户拍板方案 c 跳过 stage 3+。**留 v0.14+ cycle 重新评估**：如 v0.14 选「性能优化 cycle」，本 TD 是首选优化点（中 ROI，唯一有量化 before/after 数字的候选）。

---


---

## v0.14.0+ 候选补遗（v0.13.0 stage 3 REVIEW-v0.13 归档）

> v0.13.0 cycle stage 3 Review 产出 [`docs/reviews/REVIEW-v0.13.md`](reviews/REVIEW-v0.13.md)，P2 = 1 项归档到此段。

### TD-48（REVIEW-v0.13 P2-1）：未覆盖 hot path 的 criterion benchmark 补充

**位置**：v0.13 stage 1 选了 6 个核心 hot path bench（搜索 / 排序 / heavy refresh / TUI 渲染 / 录屏序列化 / FilterExpr apply），未单独 bench 的 7 类路径：
- `src/tui/port_table.rs`（port / flow 渲染）
- `src/tui/sidebar.rs`（monitor_sidebar hardware 指标面板）
- `src/view_models/docker_panel.rs`（containers / events / logs 渲染）
- `src/tui/detail_view.rs:69-72`（handles 每帧 `to_lowercase()` + `format!("{:x}", raw_handle)`，侦察报告疑点 3）
- `src/security/signature.rs` BackgroundScorer（admin 场景验签）
- `src/filter/mod.rs::FilterExpr::apply_network`（Flow 视图 FilterExpr）
- `src/security/flow.rs::check_flow_risk`（大规模 flows 评分）

**现状**：上述路径要么在 worker 独立线程（signature verify / docker logs），要么低频触发（detail_view 仅在选中进程时）。stage 2 PERF-BASELINE 用侧面数据 / 线程归属 / 触发频率论证非瓶颈，但**缺直接 criterion 数字**——v0.14+ cycle 如要做「performance guard」（每次 PR 跑 bench 比对），这些路径无 baseline。

**影响**：
- 上述路径要么在 worker 独立线程，要么是低频触发——stage 2 已用「不在 UI 主线程 / 偶发触发 / 复用同款 AST」等侧面论证，但**没有直接 bench 数字**
- 用户报「卡」时如涉及上述路径（如 docker_panel 渲染卡），无 baseline 对照定位

**修复方案**：v0.14+ cycle 评估时，按优先级补 bench：
1. **优先**：`signature verify async`（admin 场景每进程验签，BackgroundScorer 独立线程但影响评分延迟）+ `detail_view_handles`（侦察报告疑点 3，每帧渲染开销未实测）
2. **中**：`port_table / docker_panel` 渲染（用户报「卡」时定位成本）
3. **低**：`monitor_sidebar / Flow view filter / check_flow_risk`（间接路径或低频触发）

**验证**：每个新 bench 跑 100 / 500 / 1000 进程 × 3 档 fixture，加入 PERF-BASELINE-v0.14 报告。

**v0.13 stage 3 决策**：归档 v0.14+ cycle 评估。理由：(1) stage 2 拍板清单问题 4 用户已确认「不加」——v0.13 cycle 范围已锁定；(2) 上述路径在 stage 2 已用侧面数据论证非瓶颈，不阻断 v0.13.0 发布；(3) v0.14+ cycle 重新评估时，可基于 v0.13 baseline + 用户反馈重新选优先级。

---

## v0.15.0+ 候选补遗（v0.14.0 stage 5 REVIEW-v0.14 归档）

> v0.14.0 cycle stage 5 Review 产出 [`docs/reviews/REVIEW-v0.14.md`](reviews/REVIEW-v0.14.md)，P2 = 1 项归档到此段。

### TD-49（REVIEW-v0.14 P2-1）：VT100 replay 路径无倒放 / 搜索 + 长录屏搜索遍历优化

**位置**：
- VT100 倒放：`src/tui/mod.rs::run_vt_replay`（VT100 字节流反向解释器需 ~1000+ 行实装）
- VT100 搜索：同上（VT100 文件无结构化数据，无法 apply FilterExpr）
- 长录屏搜索遍历优化：`src/record/frame.rs::RecordingFooter`（footer 加索引段让阈值搜索 O(1)）+ `src/replay/search.rs::recompute_matches`（按 footer 索引段短路）

**现状**：
- VT100 replay 路径不加倒放 / 搜索是 stage 3 / stage 4 doc「不在本 stage 范围」段明确 surgical 跳过的——VT100 文件是字节流（无结构化帧索引），倒放需要把整个字节流倒序解析（每个 VT500 序列需反向应用：clear / cursor move / SGR 等），实现成本远超 stage 4 的 ~250 行预算；VT100 文件无结构化数据（无 UiFrame），无法 apply FilterExpr。
- 长录屏搜索遍历延迟是 stage 3 doc §「风险 2」明确的已知限制：30 min × 30 FPS × 1000 进程 = 54000 frames × 165 µs = ~9 秒用户可感。当前 `ReplaySearch::recompute_matches` 走同步遍历（input 变化时调一次），未引入异步搜索（保持 surgical 简单）。

**影响**：
- VT100 replay 用户感知不到差（VT100 replay 是独立 CLI 子命令 `proc replay-vt`，与 UiFrame replay `proc replay` 入口不同，用户使用时已知类型）
- 长录屏搜索遍历 ~9 秒用户可感但可接受（30 min 录屏是极端场景，常用 5-10 min 录屏 < 1 秒）；遍历期间 TUI 阻塞（同步路径），无异步提示

**修复方案**（v0.15+ cycle 评估）：
1. **VT100 倒放**（高成本 ~1000+ 行）：实装 VT500 反向解释器（每个 VT500 序列需反向应用：clear / cursor move / SGR 等），或转码 VT100 字节流到 UiFrame 结构（让 VT100 replay 享受 UiFrame replay 全部能力，但转码本身也是 ~1000+ 行）
2. **VT100 搜索**（同上）：VT100 文件转码到 UiFrame 后自动获得搜索能力
3. **长录屏搜索遍历优化**（中成本 ~200 行）：`RecordingFooter` 加索引段（如 `max_cpu_frame_idx` / `first_critical_anomaly_idx` / `cpu_threshold_frames: Vec<usize>`），让阈值搜索 O(1)；substring / regex 搜索仍走遍历路径（无自然索引）

**验证**：
- VT100 倒放：`run_vt_replay` 加 `r` 键分支 + 反向迭代字节流 + VT500 反向解释器单元测试
- 长录屏搜索优化：`recompute_matches` 按 footer 索引段短路（如 `cpu > 80` 走 `footer.max_cpu_frame_idx` 直接定位）；criterion bench 对比 before/after

**v0.14 stage 5 决策**：归档 v0.15+ cycle 评估。理由：(1) VT100 replay 倒放 / 搜索实现成本高（~1000+ 行），用户痛点弱于书签 / 搜索 / 倒放（VT100 replay 是独立子命令，用户已知类型，UiFrame replay 已有完整能力）；(2) 长录屏搜索遍历是极端场景（30 min × 1000 进程），常用 5-10 min 录屏 < 1 秒用户无感，footer 加索引段需评估 schema 演进（FOOTER_MAGIC 末字节 bump）；(3) v0.14 cycle 已交付完整 UiFrame replay v2 能力（按需加载 + 书签 + 搜索 + 倒放），VT100 replay 是次要路径，留 v0.15+ cycle 评估时基于用户反馈重新决定优先级。

---

## v0.16.0+ 候选补遗（v0.15.0 stage 4 REVIEW-v0.15 归档）

> v0.15.0 cycle stage 4 Review 产出 [`docs/reviews/REVIEW-v0.15.md`](reviews/REVIEW-v0.15.md)，P2 = 5 项归档到此段。

### TD-50（REVIEW-v0.15 P2-1）：`proc_metrics_smart` vs `proc_smart` 入口重叠 ✅ Fixed in v0.25 stage 3

**位置**：
- `src/mcp/handler/metrics.rs::make_metrics_smart_json`（device=None 走聚合 vs device=Some 走详细 attributes）
- `src/mcp/handler/mod.rs::proc_smart`（v0.7 既有 17 tool 之一，单设备详细 attributes）
- `src/mcp/handler/mod.rs::proc_metrics_smart`（v0.15 cat 4 新 tool）

**现状**：v0.15 stage 3 决策 2 选 (b) 方案落地 —— `proc_metrics_smart(device=None)` 返系统级聚合（all disks 摘要），`proc_metrics_smart(device=Some)` 与 `proc_smart(device=Some)` 同款返详细 attributes。device=Some 时两 tool 100% 重叠。

**影响**：agent 调用 confusion（两 tool 都能查单设备 SMART，schema 略不同但内容相同）。无功能阻断。

**修复方案**（v0.16+ cycle 评估）：
1. **(a) 废弃 `proc_smart`**（推荐）：标 Status Deprecated，schema 加 `x-deprecated: true` hint，agent 优先调 `proc_metrics_smart`。理由：`proc_metrics_smart` 双路径设计更通用（聚合 + 单设备），`proc_smart` 是 v0.7 历史遗留
2. **(b) 合并入口**：`proc_smart` alias 到 `proc_metrics_smart`，统一 helper
3. **(c) 保持现状**：documented 作为互补，agent 二选一

**v0.15 stage 4 决策**：归档 v0.16+ cycle 评估。理由：(1) `proc_smart` 是 v0.7 既有 17 tool 之一，外部 client（Claude Desktop / Cursor）可能已集成，废弃需评估破坏性；(2) `proc_metrics_smart` 双路径设计是 stage 3 决策 2 落地，stage 1 §4c 待定项已闭环；(3) 保持现状 (c) 是 surgical 默认，agent 二选一不阻断。

> **v0.25 stage 3 标 ✅ Fixed**（ADR-0035 D2）：按修复方案 (a) 落地——schema 层 `_meta: {"x-deprecated": true}`（rmcp `#[tool]` 宏 `meta` 属性 → `Tool.meta`，MCP 规范官方扩展键）+ README 注记。与 v0.17 的 description `[Deprecated]` 文本层 hint 双轨（v0.17 落地文本层时 TD 仍记 open——ADR-0035 stage 1 核查确认 schema 层无痕迹）。tool 本体不删（外部 client 兼容），MCP tool 46 不变锚保持。运行时断言：`ProcMcpHandler::proc_smart_tool_attr().meta` 含 `x-deprecated: true` + `proc_metrics_smart_tool_attr().meta` 为 None（阴性对照）。

### TD-51（REVIEW-v0.15 P2-2）：`MonitorManager` 无持久化

**位置**：
- `src/mcp/handler/cli.rs::make_monitor_add_json` / `make_monitor_remove_json`（每次 `ProcMcpHandler::new()` 都新建 MonitorManager）
- `src/monitor/manager.rs::MonitorManager::new()`（in-memory 空表）

**现状**：`MonitorManager` 是 in-memory 的（无磁盘持久化），每次 `new()` 都是空表。v0.15 stage 2 的 `monitor_add` / `monitor_remove` 仅在 process 内有效，跨 tool call 丢失。与既有 `proc_monitor_list` v0.7 行为一致（都空表起步）。

**影响**：agent 跨 tool call 配置监控规则无效（add 后 list 看不到）。无错误，但 agent 视角 confusion。

**修复方案**（v0.16+ cycle 评估）：
1. **加配置文件持久化**（推荐）：`~/.config/proc/monitors.toml`（与 `trusted_signers.toml` 同款路径），`MonitorManager::new()` 时 load，add/remove 时 write
2. **加 MCP handler 持久 MonitorManager 字段**：与 v0.12 TD-36 持久 dns_collector 同款模式，`ProcMcpHandler` 加 `monitor_manager: Arc<Mutex<MonitorManager>>` 字段，跨 tool call 共享

**v0.15 stage 4 决策**：归档 v0.16+ cycle 评估。理由：(1) v0.7 `proc_monitor_list` 既有契约是「空表起步」（list 在 production TUI 路径有持久化，但 MCP 路径未集成）；(2) agent 视角的监控配置应持久化是合理需求，但需评估配置 schema 与 TUI 路径一致性；(3) v0.16 cycle 主题 D2（操作 + 录屏类）会涉及更多写操作 MCP tool，统一评估持久化策略。

### TD-52（REVIEW-v0.15 P2-3）：`metrics_system` sparkline 30s 历史不暴露 ✅ Fixed in v0.17 stage 4（v0.25 stage 3 回填）

**位置**：
- `src/mcp/handler/metrics.rs::make_metrics_system_json`（仅返当前快照，无 sparkline 历史）
- brainstorm §类别 4 提「30 秒火花线图历史」

**现状**：v0.15 stage 3 决策 3 + 风险 5 明确「sparkline 30s 历史暂不做」—— MCP 一次性 request-response 模型不适合 worker 累积，需要持久化 + worker 1s tick 推送（与 LightWorker 同款）。

**影响**：agent 看不到 CPU/内存 30s 趋势，只能看当前快照。无功能阻断（与 `proc_diag` 同款一次性快照语义）。

**修复方案**（v0.16+ cycle 评估）：
1. **加 MCP handler 持久 SystemSnapshot 历史**：`ProcMcpHandler` 加 `system_history: Arc<Mutex<Vec<SystemSnapshot>>>` 字段，1s tick push 一次，30s cap
2. **加 Resource subscribe**：rmcp 0.11 `Resource subscribe` 模式，client 订阅 system metrics 更新事件（与 brainstorm 主题 B 可观测性 cycle 同款方向）

**v0.15 stage 4 决策**：归档 v0.16+ cycle 评估。理由：(1) MCP 一次性 request-response 模型与 sparkline 持久化语义不直接兼容，需评估 rmcp 0.11 Resource subscribe 能力（与 brainstorm 主题 B 可观测性 cycle 同款方向）；(2) `proc_diag` 是 v0.7 既有一次性快照 tool，`metrics_system` 同款语义是 surgical 默认；(3) agent 当前能用 `metrics_system` 拿当前快照 + 多次调用对比，趋势需求可在 client 侧累积。

> **v0.25 stage 3 标 ✅ Fixed（v0.17 stage 4 实装，状态回填滞后——ADR-0035 D2 现状核查发现）**：v0.17 stage 4 在 feature `mcp-persistent-state`（默认启用）下落地修复方案 1 变体——`ProcMcpHandler::system_history: Arc<Mutex<VecDeque<MetricsSample>>>`（30s cap，snapshot worker 1s tick 兼任 push，不 spawn 第二个 worker）+ `proc_metrics_history` tool（`metric: "cpu"|"memory"|"swap"`, `seconds` 默认 30 上限 30）。原 ADR-0026/0027 设计；tech-debt 状态未随 v0.17 回填，本行补记。

### TD-53（REVIEW-v0.15 P2-4）：`metrics_disk_io` per-process 不暴露 ✅ Fixed in v0.25 stage 3（改道 sysinfo delta）

**位置**：
- `src/mcp/handler/metrics.rs::make_metrics_disk_io_json`（仅返 total + per_disk + disks 三段，无 per-process）
- `src/disk_io_etw/{mod.rs, provider.rs, thread_map.rs}`（v0.7 落地的 per-process disk_io ETW worker）

**现状**：v0.15 stage 3 决策 5 明确「per-process disk_io 暂不暴露」—— 需要 ETW + thread_map（disk_io_etw worker 模式），MCP 一次性调用启动 ETW session 不实用（NT Kernel Logger 单实例限制 + 启动延迟 ~1s）。

**影响**：agent 看不到 per-process disk_io BPS（`proc_ls --sort disk_read` 是另一种视角，列表 + 排序）。无功能阻断。

**修复方案**（v0.16+ cycle 评估）：
1. **加 MCP handler 持久 disk_io_etw_worker**：与 `dns_collector` 同款模式（v0.12 TD-36），`ProcMcpHandler` 加 `disk_io_etw: Arc<Mutex<Option<DiskIoEtwHandle>>>` 字段，handler spawn 时启动 worker，metrics_disk_io tool drain 一次
2. **加 proc_inspect(disk_io) tab**：详情页视角看单进程 disk_io 历史

**v0.15 stage 4 决策**：归档 v0.16+ cycle 评估。理由：(1) disk_io_etw worker 启动延迟（NT Kernel Logger单实例）+ 非管理员 / x86 fallback 复杂度高，MCP 一次性调用不适合；(2) `proc_ls --sort disk_read` 已覆盖列表视角，详情页视角 v0.16 cycle 评估；(3) v0.16 cycle 主题 D2（操作 + 录屏类）会涉及更多 worker 路径，统一评估 MCP handler 持久 worker 字段策略。

> **v0.25 stage 3 标 ✅ Fixed（改道实装，ADR-0035 D2 终判）**：原修复方案 1（handler 持久 disk_io_etw worker，dns_collector 先例模式）**否决**——① NT Kernel Logger 全局单实例：MCP server 与 TUI 同机并存互抢 session，后启动者恒失败；② MCP server 常态非提权运行，ETW 恒 None 等于死代码；③ 启动延迟 ~1s + x86 cfg-gate。**改道方案**（TD-54 持久 snapshot 落地后解锁）：`run_snapshot_worker` 在 `refresh_heavy_incremental` 返 `Ok(true)` 时做 sysinfo delta（`compute_process_disk_speeds`——TUI `update_disk_speeds` 同款 `(pid, start_time)` 键控 `disk_usage` 差分 / elapsed，worker 局部基线无锁竞争）填 `ProcessInfo.disk_read_speed/write_speed`（`proc_ls` 的 `disk_read_bps/disk_write_bps` 同步受益）+ `metrics_disk_io` 响应加 `per_process` 段（read+write 降序 top-N 默认 10，`source: "sysinfo-delta"` 口径声明——Windows IO counters 含非磁盘 IO 如命名管道，与 TUI 非管理员档同口径）。规模 ~200-300 → ~80 业务行。

### TD-54（REVIEW-v0.15 P2-5）：`proc_flows` / `metrics_*` 多次调用 SystemSnapshot::new + App::new 累积开销 ✅ Fixed in v0.17 stage 3（v0.25 stage 3 回填）

**位置**：
- `src/mcp/handler/cli.rs::make_flows_json`（`App::new() + 2s warm-up` 每次 ~2s）
- `src/mcp/handler/metrics.rs::make_metrics_*_json` 5 helper（`SystemSnapshot::new() + refresh()` 每次 ~500ms）
- `src/mcp/handler/cli.rs::make_export_json`（同款 SystemSnapshot 路径）

**现状**：v0.15 stage 2 风险 1 + stage 3 风险 4 文档化 —— 每次 tool call 都新建 App / SystemSnapshot，agent 多次调用累积开销大。

**影响**：agent 多次调 metrics_* / proc_flows / proc_export 累积 ~500ms-2s/次。可接受（agent 典型 task 调 1-2 次）。

**修复方案**（v0.16+ cycle 评估）：
1. **加 MCP handler 持久 SystemSnapshot / App**：与 `dns_collector` 同款模式，`ProcMcpHandler` 加 `snapshot: Arc<Mutex<SystemSnapshot>>` 字段，1s tick refresh
2. **加 TTL 缓存**：handler 内 `HashMap<ToolName, (timestamp, result)>` 缓存，TTL 1s（与 worker 1s tick 对齐）

**v0.15 stage 4 决策**：归档 v0.16+ cycle 评估。理由：(1) `App::new()` 不是 Send + Sync（包含多个 worker handle + UI 状态），跨 tool call 共享需评估线程安全；(2) SystemSnapshot 共享较简单但需评估 freshness（worker 路径 vs MCP 路径同步）；(3) agent 实际不会高频调（典型 task 调 1-2 次），优化收益边际；(4) v0.16 cycle 主题 D2 涉及更多 worker 路径，统一评估。

> **v0.25 stage 3 标 ✅ Fixed（v0.17 stage 3 实装，状态回填滞后——ADR-0035 D2 现状核查发现）**：v0.17 stage 3 在 feature `mcp-persistent-state`（默认启用）下落地修复方案 1——`ProcMcpHandler::snapshot: Arc<Mutex<Option<SystemSnapshot>>>` 持久字段 + `run_snapshot_worker`（`mcp-snapshot-worker` 线程）1s tick refresh + `refresh_heavy_incremental`，`metrics_*` / `proc_ls` / `proc_tree` / `proc_export` 生产路径复用 snapshot 跳过 `SystemSnapshot::new + refresh` 累积开销（原 App::new Send+Sync 顾虑以「共享 SystemSnapshot 数据结构非 App」化解，ADR-0026）。`App::new` 路径的 `proc_flows`（~2s warm-up）不在 v0.17 落地范围（flows 数据源是 Schannel worker 非 SystemSnapshot），本 TD 主诉的 SystemSnapshot 复用已闭环。

### TD-55（REVIEW-v0.20 P2-1）：Sonnet 50 query 真实对照验收 deferred

**位置**：`tests/test_agent_v0_20_stage_4.rs::test_agent_stage4_anthropic_acceptance`（`#[ignore]`，已就位）

**现状**：v0.20 stage 4 用户拍板（2026-08-17）无 `ANTHROPIC_API_KEY`，brainstorm 风险 1 mitigate 5 的 Sonnet 对照（≥ 48/50）未实测。降级验证已做：24 CI 纯逻辑测试（消息转换 / tool_choice 映射 / 响应解析 / stream 聚合）+ 无效 key 真实 API 403 错误映射（HTTP 全链路）。anthropic 对照 fixture 录制一并 deferred——MockProvider hash 只含 query 文本，同目录混放两 provider fixture 会覆盖索引，如需录落 `tests/fixtures/agent-anthropic/`。

**影响**：「multi-provider 抽象可移植」的云端实证缺口；本地（E2B）+ 测试（Mock）两路已闭环。

**修复方案**：有 key 后一条命令：`ANTHROPIC_API_KEY=... cargo test --release --features anthropic -- --ignored test_agent_stage4_anthropic_acceptance`（FULL 50 query 预期 <10 分钟）。

**v0.20 stage 4 决策**：归档 v0.21+（TD-56 / TD-57 同批验证）。

**v0.23 cycle 追踪**：无 key 第 4 个 cycle open（v0.20/0.21/0.22/0.23，v0.23 brainstorm 决策 3 拍板不跑）；闭环路径不变——有 key 后 `ANTHROPIC_API_KEY=... proc agent eval --provider anthropic --output ...` 一条命令三连闭环（终态确认见 REVIEW-v0.23 观察 4）。

**v0.24 cycle 追踪**：无 key 第 5 个 cycle open（v0.24 brainstorm 决策 4 拍板不跑）；闭环路径不变。v0.25+ 若启动「模型升级 × RAG-on 复测」（REVIEW-v0.24 候选 1），Sonnet 云端档并入同一对比矩阵（E2B / E4B / Sonnet 三档光谱）自然到期。

### TD-56（REVIEW-v0.20 P2-2）：Anthropic model ID 未对真实 API 验证

**位置**：`src/agent/anthropic_provider.rs::DEFAULT_MODEL`（`claude-sonnet-4-6`，brainstorm 决策 9 写入）

**现状**：403 冒烟（无效 key）在 auth 层被拒，model 校验未到达。Anthropic API 常要求 dated ID（如 `claude-sonnet-4-6-YYYYMMDD`），若默认值不被接受首个真实请求 404 model-not-found。

**影响**：低——用户可在 agent.toml `[anthropic].model` 覆盖；代码默认值可能需按 API 错误信息调整一次。

**修复方案**：有 key 后首个真实请求即验证；404 时按 API error body 调整 `DEFAULT_MODEL`。

**v0.20 stage 4 决策**：归档 v0.21+（与 TD-55 同批）。

**v0.23 cycle 追踪**：无 key 继续 open（随 TD-55 同批，见其追踪行）。

**v0.24 cycle 追踪**：无 key 继续 open 第 5 个 cycle（随 TD-55 同批）。

### TD-57（REVIEW-v0.20 P2-3）：Anthropic nudge 路径连续 user 消息实测缺失

**位置**：`src/agent/anthropic_provider.rs::messages_to_anthropic`（空 assistant 消息跳过逻辑，stage 4 决策 B）

**现状**：runner 空响应 nudge 重试路径（push 空 assistant 占位 + nudge user 消息）在 Anthropic 侧转换后可能出现相邻两条 user 消息（空 assistant 被跳过）——Anthropic 同角色消息 merge 语义未实测。Sonnet 空响应罕见，路径低频。

**影响**：极低——仅 Sonnet 空响应重试场景；若 API 拒绝相邻同角色消息，会在该路径返 400（可观测不致命）。

**修复方案**：TD-55 验收跑完后确认（50 query 若无空响应则路径未触发，保留观察）；如需修复在 `messages_to_anthropic` 合并相邻 user 文本消息。

**v0.20 stage 4 决策**：归档 v0.21+（随 TD-55 顺带覆盖）。

**v0.23 cycle 追踪**：无 key 继续 open（随 TD-55 顺带覆盖，见其追踪行）。

**v0.24 cycle 追踪**：无 key 继续 open 第 5 个 cycle（随 TD-55 顺带覆盖）。

### TD-58（REVIEW-v0.21 P2）：`test_alert::test_metric_extract_process_cpu` 并发 flaky（观察项）

**位置**：`tests/test_alert.rs::test_metric_extract_process_cpu`

**现状**：v0.21 stage 1 开工基线验证首跑 1 failed（全量并发场景），重跑 / 单独跑 / anthropic 档均过。根因是真实 `SystemSnapshot` 采集在 CI 并发下偶尔慢（CPU 利用率断言依赖采集窗口），环境时序敏感非回归。v0.21 stage 2 注记 A2 同款根因——该决策已把 A2 测试改用 `proc_help(meta)` 零 IO 查询规避（CI 并发下稳定）。

**影响**：偶发（整个 v0.21 cycle 4 stage 基线验证仅再现 1 次）。失败时重跑即过，不阻断开发；但「首跑红」会消耗排查时间。

**修复方案**（再现时实施）：改零 IO 断言（与 stage 2 A2 决策同款——mock 采集路径或放宽断言窗口），或测试加 retry 语义。

**v0.21 stage 4 决策**：观察项，不修。理由：(1) 单次再现 + 重跑即过，修复优先级低于任何功能项；(2) 修复方案明确（A2 同款），再现时半天内闭环。连续 cycle 再现才升级为必修。

**v0.22 cycle 追踪**：整个 cycle 4 次开工基线验证（stage 1/2/3/4 双档）均未再现。继续观察，原判不动。

**v0.23 cycle 追踪**：各 stage 开工基线验证均未再现。继续观察，原判不动（stage 2 一次 `test_session_drop_during_confirm_does_not_hang` 编译+满载 5s 窗口 flaky 系另一测试，复跑绿，不并入本条）。

**v0.24 cycle 追踪**：各 stage 开工全量回归双档 0 failed 均未再现。继续观察，原判不动。

### TD-59（REVIEW-v0.22 P2）：session log 累积无轮转 / 清理（观察项）

**位置**：`src/agent/session_log.rs::SessionRecorder::start`（`dirs_config_dir()/sessions/<utc>-<provider>.jsonl`）

**现状**：v0.22 stage 3 落地的 session JSONL 留档默认开启（`agent.toml [session].log = true`），每次 TUI AgentPanel 会话产生一个文件（单会话几百 KB 量级——TextDelta 已按 ≥64 chars 聚合）。目录单调增长，无轮转 / 无清理。

**影响**：日常使用增长缓慢（一次会话一个文件，非 daemon 持续写）；几个月量级可达百 MB 级。非功能缺陷，是 brainstorm 决策 3 明确的延后决策（「不做轮转/清理（不预实现）；累积成问题再治理」）。

**修复方案**（触发时实施，半天内）：最简清理——proc 启动时删 N 天前 sessions 文件（TUI 启动路径一次性 glob + retain），或 sessions 目录体积上限 LRU。不做按大小轮转（会话粒度文件天然是轮转单元）。

**v0.22 stage 4 决策**：归档观察项。触发条件 = 用户磁盘占用可感知 / sessions 目录上万文件。归档目的：不让「不预实现」变成「永不处理」。

**v0.24 cycle 追踪**：维持观察，未触发。**新增联动注**：v0.24 起 session JSONL 兼任 RAG 主语料（`src/agent/rag/` 成功段状态机索引）——未来若实施轮转/清理，需与 RAG 语料口径联动（清理只影响索引新鲜度不影响正确性——`RagIndex::build` 全量重建按现存文件）；语料密度现状见 REVIEW-v0.24 Findings「RAG 语料密度」观察项。

### TD-60（REVIEW-v0.23 P2）：prompt v3 候选——修订 2（写操作发现链）单独实验（v0.24 实验关闭）

**位置**：`src/agent/prompts/system.md`（终态 v1；v2 措辞稿留 [ADR-0033 附录 A](../adr/0033-eval-experiments-and-record-tools.md) 备查，修订 2 段独立可用）

**现状**：v0.23 stage 3 prompt v2 双列实验（[归档](../eval/promptv2-70q-v0.23.md)）整体拍板「无明确增益 revert 回 v1」——但修订 2（写操作发现链：先 proc_help 发现并正常调用，blocked 后再解释 + 等价命令行）机制单独生效有 3 处证据（#21 列 ② 完整执行演示链 proc_help→proc_eject→proc_eject_status / idx63 两轮一致通过 / idx18 blocked 后行为符合措辞预期）；与修订 1（缺参引导）捆绑实验时整体增益落 E2B 方差带内（±3/±6）无法分离归因，且修订 1 有 L2 回退嫌疑（chain_incomplete +3/+4 两轮同向）。

**影响**：写操作类 query 的发现链引导仍是 v1 措辞（直接解释为主）——REVIEW-v0.21 观察 3 / v0.22 #21 的原始痛点未被本轮实验修复。

**修复方案**（v0.24+ 实施时）：单独 diff 修订 2（不含修订 1）→ QUICK 冒烟 → FULL 三列（基线 / v3 / 复跑方差列），按方差带标尺解读（单列差异 < ±3 通过数不可单独归因）；可与 v0.24 模型底座对比列同矩阵跑摊薄挂机成本。

**v0.23 stage 4 决策**：归档 v0.24+（与 RAG cycle 一并评估）。理由：单变量复验成本一次 FULL ~47m + 措辞稿已备；v3 落地与否由数据说话（与 v2 同款 D4 保守标准）。

**v0.24 终态（已关闭）**：v0.24 stage 3 按修复方案原文路径执行（单独 diff 修订 2 → 独立 commit `e166e27` 先于挂机 → FULL 单变量列，E2B × RAG off × v3，binary `v0.23.0-8-ge166e27`）——**负结果**：净通过 27/70（**-5 vs 基线 / -8 vs 方差列，落带外向下**）+ L0 13/23（掉 4-6）+ output_degraded 24（高于基线带 19-21）双向一致恶化 → D4 保守标准 **revert 回 v1**（commit `7959030`，system.md 终态 = v1）。机制层面单 query 冒烟发现链三段完整复现（proc_help 找 tool → proc_kill 带参调用 → blocked 后解释+给命令行，v0.23 三处证据延续）但 70q 规模发现链措辞让简单 query 绕路（L0 受伤最重），且 v3 列无 RAG 经验缓冲。**结论：prompt 措辞杠杆在 E2B 上用数据关闭**（叠加 v0.23 v2 捆绑负结果，措辞维度两轮穷尽）；更强底座上措辞敏感度可能不同，但已非独立候选——随「模型升级 × RAG-on 复测」组合才可能重开（REVIEW-v0.24 候选 1）。归档 [docs/eval/rag-v3-70q-v0.24.md](../eval/rag-v3-70q-v0.24.md)。

### TD-61（REVIEW-v0.23 P2）：GBNF grammar × tools 复测（观察项，挂 llama-server 升级节点）

**位置**：`agent.toml [llama-cpp] grammar_file = "tool_call"`（注释态）→ llama-server 请求体 grammar 字段（`src/agent/builder.rs` 接线）

**现状**：ADR-0033 附录 B 判定性结论——llama-server `b8685` 对 grammar + tools 同传的请求显式 400 拒绝（`"Cannot use custom grammar constraints with tools."`，冒烟 12 请求零生成），**tools 协议模式下结构性不可用**；结论绑定该版本（升级 llama.cpp 后互斥校验若放开可重开）。

**影响**：output_degraded 21 次（E2B 失败大头，proc_finish 泄漏型为主）的零代码修复路径暂时关闭——进一步改善杠杆只剩模型升级（v0.24 RAG cycle 一并决策）。

**修复方案**（触发时实施）：升级 llama.cpp 后按附录 B 判定表重跑冒烟（smoke1 L0 3×2 即可判定——请求校验层错误与场景无关）；若互斥放开，GBNF 单变量列复入对比矩阵（预期消灭 proc_finish 泄漏型退化）。

**v0.23 stage 4 决策**：观察项，挂 llama-server 升级节点，不主动排期。归档目的：不让「结构性不可用」的版本绑定结论变成永久结论。

**v0.24 cycle 追踪**：维持观察——v0.24 未升级 llama-server（四列实验均 b8685），节点未触发。若 v0.25+ 启动「模型升级 × RAG-on 复测」需升级 llama.cpp，则 GBNF 复测顺手并入（附录 B 判定表 smoke1 即可判定）。

---

## v0.25 cycle 清仓盘点（2026-08-30，stage 3 收尾状态总检）

> v0.25 是「TD 清仓 + session 语料卫生」维护型轻 cycle（ADR-0035）。本段是 cycle 收尾时点的 tech-debt 全量状态总检——open 存量从 v0.24 末的十余项收敛到 8 项（全部为既定归档 / 观察项，无新增债）。

| 状态 | 条目 | 说明 |
|---|---|---|
| **本 cycle 关闭（实装）** | TD-24 / TD-40 | stage 2（commit `43300d9`）：on_respawn_failed 止损状态机 / regex size_limit(64KB) |
| **本 cycle 关闭（实装）** | TD-50 / TD-53 | stage 3：`_meta.x-deprecated` schema hint / sysinfo delta 改道 per-process 段 |
| **本 cycle 回填（v0.17 已落地）** | TD-52 / TD-54 | sparkline 30s 历史 + `proc_metrics_history` tool / 持久 snapshot + 1s tick worker——代码先落地状态未回填，本 cycle 对齐 |
| **本 cycle 判废** | TD-34 | plan.md 已不在 v0.13+ 流程（brainstorm 替代） |
| **本 cycle 回填（已修未记）** | TD-22 | `property_at_index` lifetime 修复在后续重构中完成，stage 1 核查发现补记 |
| **维持归档（理由仍立）** | TD-11 / TD-20 / TD-21 / TD-41 / TD-51 | watchdog spawn 威胁模型 / Win10 1809 探测 / overlay 单键 pid / SYSTEM_BOOT_ENTRIES image path / MonitorManager 持久化（feature 非 debt，v0.26+ 候选） |
| **单特性候选（v0.26+ 主题评估）** | TD-31 / TD-44~49 | FilterExpr 跨 ctx / 性能优化 / replay 增强——与维护型定位不符留档 |
| **观察项（触发条件未到）** | TD-55~57 / TD-58 / TD-59 / TD-61 | 无 key（第 6 个 cycle）/ flaky 未再现 / 轮转未触发 / llama-server 未升级 |
| **已关闭（前 cycle）** | TD-60 | prompt v3 负结果（v0.24 数据关闭） |

---

## v0.26 cycle 追踪（2026-08-31 起，展示冲刺 cycle）

| 状态 | 条目 | 说明 |
|---|---|---|
| **stage 2 修复即关闭（未立 TD）** | R1 llama e2e flaky 竞态 | brainstorm「基线验证异常记录」R1 段首触发（2026-08-31，v0.20 引入以来首次）：`test_llama_cpp_provider` 内两个真实 server 测试默认并行，end_to_end 清理断言用**全局** `tasklist` 扫 llama-server.exe，被 grammar 测试自己仍存活的 server 误报「drop 后未退出」。stage 2 根治——`LlamaServerHandle::pid()` + `LlamaCppProvider::server_pid()` getter + 断言改查自身子进程 PID（`tasklist /FI "PID eq N"`），`--test test_llama_cpp_provider` 连跑 3 轮稳定绿（27/27 × 3）。按 brainstorm 决策 4 处置**不留 TD-62 观察项**。注意：非 TD-58 本体（TD-58 是 `test_alert::test_metric_extract_process_cpu` CPU 采集窗口 flaky，另一处，继续 open 观察） |

---

## 历史回顾

- v0.6.0 Review（本文件来源）：`docs/reviews/REVIEW-7.md` 产出 1 P0 + 9 P1 + 14 P2。
- v0.6.0 阶段 8 应修：1 P0 + 9 P1（详见 REVIEW-7.md）。
- v0.7.0 候选：本文件 v0.7.0 段 11 项。
- v0.8.0+ 候选：本文件 v0.8.0+ 段 6 项（含 v0.7.0 阶段 8 遗留的 TD-17 / TD-18 / TD-19 eBPF 相关）。
- v0.25 cycle 清仓（2026-08-30）：关闭 TD-24 / TD-40 / TD-50 / TD-53（实装）+ TD-52 / TD-54（v0.17 落地回填）+ TD-34（判废）+ TD-22（已修回填）——打包清单三组全清（ADR-0035 D3 终判表只砍不加兑现，无新增债）。
- v0.26 stage 2（2026-08-31）：R1 llama e2e flaky 竞态修复即关闭（未立 TD-62，brainstorm 决策 4 处置）——PID 断言替代全局 tasklist 扫描。
