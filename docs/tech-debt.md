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

### TD-4（P2-4）：CONTEXT.md 显眼标注 WorkerManager::restart 未实现 ✅ Fixed in v0.7.0 阶段 1

**位置**：`CONTEXT.md:55`
**现状**：第 55 行写"restart(name) 故障恢复方法尚未实现（无调用方，surgical 原则不预实现）"，但藏在表格里不够显眼。
**影响**：用户/开发者读 CONTEXT.md 后误以为 worker 崩溃可热恢复。
**修复**：CONTEXT.md 顶部加"已知限制"段或 ⚠️ 标注；README 平台支持表加"worker 崩溃后只能重启 proc"。
**验证**：CONTEXT.md 顶部"已知限制"段含 restart 条目。

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

### TD-17：eBPF TLS SNI / JA4 指纹采集 — v0.7.0 阶段 8 遗留

**位置**：`src/ebpf/` 整个模块
**现状**：v0.7 阶段 8 MVP 只关联 DNS + connect（[`ProcessFlow`] 的 `dns_name` 通过 [`FlowAggregator`] 的 5s 窗口向前查 DnsQuery 填入）。TLS SNI 需要在 `SSL_write` / `SSL_read` 上挂 uprobe（OpenSSL / BoringSSL / LibreSSL 多个版本分支 + offset 不同），第一版未做。`bytes_out` / `bytes_in` 也留 0（要 hook `tcp_sendmsg` / `tcp_recvmsg`）。
**影响**：
- 命中 DNS cache 的查询（系统直接走 /etc/hosts 或缓存）关联不到 → `dns_name = None`，但 R15 仍能通过条件 2（端口扫描）兜底。
- 无字节计数 → UI Flow 子视图不能回答"实际发了多少字节"，仅能回答"谁连到哪里"。
**修复**（v0.8 候选）：
1. 用 `aya-rs` uprobe 在 `SSL_write` 入口抓 ClientHello 明文 SNI 字段（OpenSSL 优先，BoringSSL / LibreSSL 后续）。
2. JA4 指纹：从 ClientHello 抓 cipher suites + extensions + ALPN，按 [RFC 9503](https://www.ietf.org/archive/id/draft-ietf-tls-wg-tls-essentials-01.html) JA4 algorithm hash。
3. `tcp_sendmsg` / `tcp_recvmsg` kprobe 累计 bytes_out / bytes_in（按 `(pid, saddr, daddr)` 聚合）。
**验证**：curl https://example.com 后，端口面板 Flow 子视图该 flow 的 `dns_name == "example.com"` + `bytes_out > 0` + JA4 hash 字段。

### TD-18：Windows ETW Schannel 抓 SNI（同名功能 Win 版本）— v0.7.0 阶段 8 遗留

**位置**：`src/ebpf/` cfg-gate 失效的 Windows 平台
**现状**：v0.7 阶段 8 eBPF flow graph 仅 Linux + `ebpf` feature 启用。Windows 用户没此功能（仅有 DNS 日志 + per-process 网络速率半关联）。Windows 等价物：ETW `Microsoft-Windows-Schannel` event 196（Operations 196 含 SNI 字段），schema 复杂未做。
**影响**：Windows 用户不能回答"哪个二进制和哪个域名说了多少字节"，定位挖矿 / C2 弱于 Linux。
**修复**（v0.8 候选）：在 `src/disk_io_etw/provider.rs` 同款手写 windows-rs ETW 框架基础上，新开 Schannel session 抓 event 196，关联 (pid, sni, ts) → 与 DNS 日志互补。
**验证**：Windows 上 curl https://example.com → 端口面板 / `proc flows` 显示 SNI。

### TD-19：eBPF Linux 真实编译验证缺失 — v0.7.0 阶段 8 遗留

**位置**：`src/ebpf/{worker.rs,elf_loader.rs}` + `src/ebpf/ebpf-ebpf/src/main.rs`
**现状**：Part A + Part B 都在 Windows 会话落地，未在真实 Linux + root + 内核 5.10+ 环境验证：aya `TracePoint::attach` 真实签名、`RingBuf::try_from` API、tracepoint arg offset（`sys_enter_connect` 偏移 16 / `sched_process_exit` 偏移 24 在不同内核可能不同）、`include_bytes!` ELF 路径硬编码、内核态 `bpf_current_task_start_time` 占位 0（需 aya-tool BTF binding 补完）。
**影响**：Linux 用户首次 `cargo build --features ebpf` 可能失败；attach 失败时 App::flows 为空，UI 显示降级提示（不崩，但功能不可用）。
**修复**（v0.7 收尾或 v0.8）：Linux 会话跑 `cargo +nightly build --target bpfel-unknown-none -p proc-ebpf` + `cargo build --release --features ebpf` + `sudo cargo test --release --features ebpf --test test_ebpf_flow -- --ignored`，按报错修。
**验证**：Linux 真实环境 `proc flows` 显示活跃 flow；端口面板按 F 切换 Flow 子视图有数据。

---

## 历史回顾

- v0.6.0 Review（本文件来源）：`docs/reviews/REVIEW-7.md` 产出 1 P0 + 9 P1 + 14 P2。
- v0.6.0 阶段 8 应修：1 P0 + 9 P1（详见 REVIEW-7.md）。
- v0.7.0 候选：本文件 v0.7.0 段 11 项。
- v0.8.0+ 候选：本文件 v0.8.0+ 段 6 项（含 v0.7.0 阶段 8 遗留的 TD-17 / TD-18 / TD-19 eBPF 相关）。
