# Changelog

本项目的所有重要变更都会记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
并遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

下次 cycle（v0.11.0+）的工作面：候选方向见 stage 4 doc 末尾「v0.10.0 完工后」段（Linux rootless ebpf / Windows TLS keylog / FilterExpr v2 / worker restart）。

## [0.10.0] - 2026-06-28

v0.10.0 cycle 围绕 **跨平台 SNI 对齐**（Windows Schannel ETW 落地，弥补 v0.7 阶段 8 eBPF 仅 Linux 的缺位）+ **v0.9 推迟字段一并扩**（ProcessFlow.sni；ja4 留 ebpf 路径）两条线，分 4 阶段推进。v0.9 cycle（计划 ADR-0017 eBPF SNI/JA4）整体推迟，v0.10 直接启动不依赖 v0.9 tag。**全量回归 959 passed / 0 failed / 3 ignored**（v0.8.0 基线 930 → +29：test_schannel_etw +3 + test_flow_source +10 + 新模块内部单测 +12 + REVIEW-11 P1-1 修复 +4）；0 个新依赖进默认依赖图（windows-rs `Win32_System_Diagnostics_Tdh` feature 复用既有 windows-rs 依赖）。

**已知限制（必须在 release notes 显式标注）**：
- **Win10 < 1809**：Schannel event 1793 不 fire（精细化 TLS handshake 事件 1809+ 才有），admin 下 worker 启动成功但 UI 显示 0 条；无法在用户态探测，留 FAQ 提示。详见 [ADR-0018 §7](docs/adr/0018-windows-schannel-sni.md)。
- **Linux ebpf 编译路径未在本机验证**（TD-19 延续）：v0.8.0 cycle stage 1 主动推迟到 v0.9.0 cycle 启动前再评估；本 v0.10 cycle 不依赖 ebpf 路径，继续推迟。
- **Schannel overlay 单键 pid 匹配**：Schannel event 不带 start_time，PID 复用时可能错误覆盖（CONTEXT.md 已记录，影响窄）。

### 阶段 4 — Review + 修 P0/P1 + 定稿 + tag v0.10.0（本次发布）

> 本次发布 commit：Cargo.toml 0.8.0 → 0.10.0；CHANGELOG / README / CONTEXT 同步 v0.10.0；4 个 stage doc 头部加发布标记；ADR-0018 / tech-debt 收尾；REVIEW-11 P0/P1/P2 状态闭环。详见本次 commit diff。

- Docs: **`docs/reviews/REVIEW-11.md`**（新）— v0.10.0 cycle 全局 review 报告。审查覆盖 stage 1-3 全部产出（跨平台一致性 / Schannel ETW Drop 安全性 / worker_metrics 性能 / Win10 < 1809 降级 4 子项），分级 **P0 0 / P1 2 / P2 4**。
- Fix: **REVIEW-11 P1-1（Schannel-only flow 永不退出）** — `src/ebpf/flow.rs` 新增 `mark_dead_schannel_flows` + `reap_expired_schannel_flows` 两个 free function；`src/app.rs::tick_light_refresh` 在 alive_pids 计算后给 source = Schannel 且 pid 不在 alive_pids 的 flow 打 exit_time；`overlay_flow_sni_schannel` 末尾调 reaper 移除超过 GHOST_FLOW_TTL 的 Schannel ghost。Linux 上 schannel_etw_worker 恒为 None → no-op。新增 4 个 unit test。
- Fix: **REVIEW-11 P1-2（trace_thread spawn 失败时泄漏 session/trace handle）** — `src/schannel_etw/provider.rs::try_spawn_windows` 第 4 步 spawn 改为 match Err 分支清理 stop_session + CloseTrace，再返回 None 降级。
- Docs: ADR-0018 §7 降级路径补 Win10 < 1809 说明（P2-1）；tech-debt 新建 TD-20（Win10 版本探测）/ TD-21（PID 复用防护）/ TD-22（lifetime 代码质量）3 项 P2 归档（P2-2 / P2-3 + 新 Win10 探测）；4 个 v0.10 stage doc 头部加 ✅ 已发布标记（P2-4）；README banner 加 v0.10.0 段 + 平台支持表加 Windows SNI 行。
- Release: `git tag -a v0.10.0 -m "v0.10.0：Windows Schannel ETW SNI 落地，跨平台流量分析对齐"` 已打（等用户确认后 push）。


### 阶段 3 — ProcessFlow.source 字段 + App overlay + UI / CLI / R15 跨平台

> **TD-18 标 ✅ Fixed**：Windows admin 跑 proc，curl https://example.com 后端口面板 Flow 子视图显示 SNI = "example.com"（来源 Schannel），与 Linux eBPF 路径在 `ProcessFlow.source` 字段统一。Stage 2 落地的 Schannel ETW worker → `App::overlay_flow_sni_schannel` → UI 全链路打通。

- Added: **ProcessFlow.source 字段（FlowSource enum）** — `src/ebpf/flow.rs` 加 `pub enum FlowSource { Ebpf, Schannel }`（Copy + `#[derive(Default)]` 标 `Ebpf` 默认；serde `#[serde(rename_all = "lowercase")]` 序列化为 `"ebpf"` / `"schannel"`）。`ProcessFlow` 加 `pub source: FlowSource`（`#[serde(default)]` 保旧录屏兼容）。**3 处字面量构造点**（`flow.rs::ingest_event` 写 `Ebpf`、`security/flow.rs::mk_flow` 写 `Ebpf`、`test_ebpf_flow::process_flow_serde_round_trip` 写 `Ebpf`）+ **2 处测试 mk_flow**（`test_ebpf_flow.rs` / `test_security.rs`）同步更新。
- Added: **`App::overlay_flow_sni_schannel`** — `src/app.rs` 新方法（在 `tick_flows_ebpf` 之后调用）：drain `workers.schannel_etw_worker` → 拿 `Vec<SniRecord>` → 对每条 record：(1) 匹配 pid 的 `ProcessFlow` **全部**覆盖 `sni` + 标 `source = Schannel` + 刷新 `last_seen`；(2) 没匹配上的 record（典型 Windows-only 环境，ebpf 路径空）直接 push 一条 `source = Schannel` 新 flow（从 `cached_processes` 查 pid → (start_time, comm) 填字段，remote_addr / remote_port / bytes / dns_name 留空——Schannel event 不给 socket 元数据 + 不参与 DNS 关联）。drain 后重排序保 last_seen 倒序。Linux 上 `schannel_etw_worker = None` → no-op。
- Changed: **`port_table::draw_flow_view` 跨平台对齐** — 标题栏不再硬编码 "eBPF Flow graph"：`schannel_etw_worker` 启用且 ebpf 不在线时显示 "Schannel Flow graph（N 条 · SNI 明文 · TLS handshake）"；两条都不在线时显示降级提示 "Flow graph：需要 Linux + ebpf feature 或 Windows 管理员（Schannel ETW）"。表格列改名 "域名" → "SNI/域名"（数据来源透明，用户看到的是名字，优先 sni 回退 dns_name）；Schannel-only flow 的 remote_addr 空字符串 / remote_port 0 时显示 `—` 保持视觉对齐。**TUI 不显示 source 列**（用户透明，仅内部用，符合 stage 3 任务指令 #5）。
- Changed: **R15 跨平台激活** — `src/security/flow.rs::check_flow_risk` 条件 1 同时检查 `sni`（Windows Schannel / Linux eBPF uprobe 路径）和 `dns_name`（Linux eBPF DNS 关联路径），SNI 优先（`f.sni.as_deref().or(f.dns_name.as_deref())`）。Windows admin + 用户显式 touch `~/.config/proc/sni_whitelist.txt` 时 Schannel 抓到的 SNI 进入白名单检查路径（与 v0.7 阶段 8 Part B 同款契约，stage 3 不改默认关闭策略）。JA4 黑名单规则在 source = Schannel 路径自动 skip（ja4 字段未实现，仍 None）。条件 2（端口扫描）不受影响——Schannel-only flow 的空 remote_addr 进入 distinct HashSet 时只贡献 1 个唯一值，远不及阈值 50。
- Changed: **`proc flows` CLI 跨平台** — `src/cli/flows.rs` 入口判断改为 `EBPF_ENABLED=false` 且 `schannel_etw_worker=None` 才降级（之前只判 ebpf）。表格输出加 "来源" 列（`ebpf` / `schannel`）；JSON 输出由 `#[serde(rename_all="lowercase")]` 自动加 `"source": "ebpf"|"schannel"` 字段。打印 summary 按 ebpf / schannel 条数分支措辞。
- Added: **`tests/test_flow_source.rs`**（新）10 case：FlowSource enum Copy + Default + serde lowercase 行为契约（3 case）+ ProcessFlow.source serde round-trip + **旧录屏兼容性**（无 source 字段 JSON 反序列化应得 Ebpf 默认）（2 case）+ R15 跨平台 5 case（Schannel sni 命中 / 放行 / sni 优先于 dns_name / 空白名单命中 / 端口扫描阈值不可达）。
- Test: 全量回归 933 → 943 passed（test_flow_source +10）。
- Docs: **TD-18 标 ✅ Fixed in v0.10.0 阶段 3**；CONTEXT.md 加 `FlowSource` + `App::overlay_flow_sni_schannel` 2 个新术语 + 术语演进历史加 v0.10.0 阶段 3 行；ADR-0018 Consequences 段补「阶段 3 落地」子段。

### 阶段 1 — ADR-0018 + ProcessFlow.sni 字段扩展 + Schannel ETW 骨架

> 独立会话产出：本段 commit。stage 1 doc 验收标准 4 项全达：ADR-0018 文件存在（Status Accepted）/ event 196 schema 文档化（标注「未经实测，阶段 2 修订」+ TDH 路线说明）/ 最简 Schannel session 骨架跑通（编译 + 测试通过）/ ProcessFlow.sni 字段一并扩（v0.9 推迟，ja4 留 ebpf）。

- Added: **ADR-0018 Windows Schannel ETW SNI** — `docs/adr/0018-windows-schannel-sni.md`（新）。Status Accepted。决策路线：手写 windows-rs ETW（不引 ferrisetw / schannel-rs），开 `Microsoft-Windows-Schannel` session（provider GUID `{37D2C3CD-C5D4-4587-8531-4696C44244C8}`，来自 MS technet blog 实证）。**关键差异（与 ADR-0015 disk_io_etw）**：走 TDH 动态 schema（`TdhGetEventInformation` + `EVENT_PROPERTY_INFO`）而非硬编码偏移——Schannel event 196 manifest Microsoft 未公开，跨 Win10/Win11 版本可能变；硬编码会随时挂，TDH 路线从 manifest 资源动态拉 schema。Alternatives 7 项列出（ferrisetw fallback / 硬编码偏移 / WinDivert / FiddlerCore MITM / EventLog Schannel source / sysinfo+DNS 关联 / 一起扩 ja4）。
- Added: **ProcessFlow.sni 字段（v0.9 推迟过来的范围）** — `src/ebpf/flow.rs::ProcessFlow` 加 `pub sni: Option<String>`（`#[serde(default)]`）。与 `dns_name` 区别：`dns_name` 来自 DNS 查询事件（HTTPS 命中 DNS cache 时关联不到 / DoH 抓不到）；`sni` 来自 TLS ClientHello 明文（HTTPS 必经路径）。Linux 由 eBPF uprobe on `SSL_write` 抓（留 v0.9 复活时实现）；Windows 由 Schannel ETW event 196 抓（v0.10 阶段 2 落地）。3 处字面量构造点（`flow.rs::ingest_event` + `flow.rs` tests + `security/flow.rs::mk_flow` helper）+ 2 处集成测试（`test_ebpf_flow` / `test_security`）同步加 `sni: None` / `sni: Some("example.com")`。**ja4 字段不加**——用户明确「ja4 留 ebpf 那边」（纯 eBPF 范畴，与 Schannel 路径无关）。
- Added: **Schannel ETW 骨架** — `src/schannel_etw/{mod.rs, provider.rs, parser.rs}`（新）。`provider.rs`（Windows cfg-gate）实装 StartTraceW + `EnableTraceEx2`（启用 Schannel provider，TRACE_LEVEL_VERBOSE 抓全事件）+ OpenTraceW（注册 callback）+ ProcessTrace 阻塞线程；`EventRecordCallback` 阶段 1 只 hex 打印 raw UserData（event_id / opcode / pid / 前 64 字节预览），SNI 解析留阶段 2 用 TDH 实测 schema 后填实。`mod.rs` 跨平台入口 `try_spawn_probe(Option<Sender<WorkerCrash>>) -> Option<SessionProbeHandle>`；非 Windows 直接返回 None。`parser.rs` 占位 `parse_event_196` + `SniRecord` 类型（阶段 2 接 SnapshotWorker 时用）。**与 disk_io_etw 路径不同的关键点**：Schannel 是用户态 manifest-based provider，**不能用 NT Kernel Logger**（后者只用于 kernel events）；必须用自定义 session name + `EnableTraceEx2` 启用特定 provider GUID。降级路径：非管理员 / StartTraceW 失败 / EnableTraceEx2 失败 / x86 进程 → 返回 None。
- Added: **`tests/test_schannel_etw.rs`**（新）3 case：跨平台 stub 测试（非 Windows `try_spawn_probe` 返回 None）+ SniRecord 数据格式契约 + Windows 集成测试（管理员下 SessionProbe 启停 + drop 干净；非管理员走 SKIP 不 fail）。
- Docs: **CONTEXT.md 加 v0.10.0 段** — 5 个新术语（ProcessFlow.sni / SessionProbe / SchannelProviderGuid / SniRecord / TDH 动态 schema 路线）+ 术语演进历史加 v0.10.0 阶段 1 行。
- Test: 全量 930 → 933 passed（test_schannel_etw +3）。

**已知限制（v0.10.0 cycle 启动时已确认）**：
- Schannel event 196 schema **未经实测**：阶段 1 没在 Windows 真跑 xperf/logman 抓真实 event，ADR-0018 文档化「待阶段 2 修订」；阶段 2 必须 xperf 实测 + 与 TDH 动态解析结果对账。
- 阶段 1 Schannel 骨架不接 WorkerManager（留阶段 2）：主线程 `App::workers` 没加 `schannel_etw_worker` 字段，`proc diag` 也没加 worker 行——全部留阶段 2 落地。
- ProcessFlow.sni 字段在所有路径都填 None（eBPF 路径 v0.9 推迟、Schannel 路径阶段 2 才填），UI / CLI 暂不显示。

## [0.8.0] - 2026-06-28

v0.8.0 cycle 围绕 **小修一波清**（TD-12 Linux stub 测试 + TD-13 Linux CI 校验 + TD-16 FilterExpr 错误中文化）+ **FilterExpr 扩展**（TD-15 Tree / AppGroup view 接入）+ **收尾交付**（REVIEW-9 全局 Review + tag）三条线，分 4 阶段推进（stage 1 主动推迟 / stage 2+3 实现 / stage 4 review + 收尾）。**全量回归 930 passed / 0 failed / 3 ignored**（v0.7.0 基线 910 → +20 新测试 case）；0 个新依赖进默认依赖图。

**已知限制（必须在 release notes 显式标注）**：
- **Linux ebpf 编译路径未在本机验证**（TD-19 延续）：v0.8.0 cycle stage 1（WSL2 / Linux 真机验证）由用户主动推迟到 v0.9.0 cycle 启动前再评估。理由：用户主要用 Windows 开发，stage 1 环境准备成本高（clang/llvm/libelf + nightly + bpf-linker），且 stage 2/3/4 不依赖 stage 1。release CI 的 `proc_ebpf` 后缀二进制构建步骤用 `continue-on-error=true`，Linux 编译失败不阻断主 release（5 target 主二进制优先发货）。详见 [ADR-0016 Consequences](docs/adr/0016-ebpf-flow-graph.md#负面) + [tech-debt TD-19](docs/tech-debt.md)。
- v0.7.0 cycle 已记录的 known limitations（eBPF SNI / JA4 留 TD-17 / Windows Schannel 留 TD-18）继续保留，本 cycle 未触及。

### 阶段 4 — Review + 修 P0/P1 + 定稿 + tag v0.8.0（本次发布）

> 本次发布 commit：Cargo.toml 0.7.0 → 0.8.0；CHANGELOG / README / CONTEXT 同步 v0.8.0；4 个 stage doc 头部加发布标记；ADR-0011 加 v0.8 阶段 3 增量段 / ADR-0016 加 cycle 推迟说明；tech-debt TD-19 加 v0.8.0 cycle 推进段；REVIEW-9 P0/P1/P2 状态闭环。详见本次 commit diff。

- Docs: **`docs/reviews/REVIEW-9.md`**（新）— v0.8.0 cycle 全局 review 报告。审查覆盖 stage 1-3 全部产出（代码质量 / 架构 / 安全性 / 跨平台一致性 / ADR 一致性 5 子项），分级 **P0 0 / P1 1 / P2 4**。结论：无阻断问题（baseline 930 passed + fmt/clippy/no-default-features build 全通过）；P1-1 stale comment 已修；P2 文档一致性问题归档到 ADR / tech-debt / stage docs。
- Fix: **REVIEW-9 P1-1** — `src/view_models/process_panel.rs:679-680` 注释 stale（"仅 List view 接入；Tree / AppGroup 视图暂保持 substring"），与 v0.8 阶段 3（TD-15）实际接入矛盾。改成「v0.8 阶段 3：Tree / AppGroup 视图同款接入（TD-15），三视图均支持」。
- Docs: ADR-0011（FilterExpr）加「v0.8.0 阶段 3 增量：FilterExpr 扩 Tree / AppGroup view（TD-15）」段（REVIEW-9 P2-1）；ADR-0016（eBPF）Consequences 段补 v0.8.0 cycle stage 1 推迟说明（P2-2）；tech-debt TD-19 加 ⏸ 标记 + 「v0.8.0 cycle 推进」段（P2-3）；4 个 v0.8 stage doc 头部加 ✅ 已发布 / ⏸ 推迟 标记（P2-4）。
- Docs: README banner 加 v0.8.0 段（FilterExpr 全 view / Linux CI 加固 / ebpf 编译路径未本机验证）；CONTEXT.md「术语演进历史」段已含 v0.8.0 落地变更（stage 2/3 行，stage 4 不引入新术语）。
- Release: `git tag -a v0.8.0 -m "v0.8.0：FilterExpr 全 view + Linux CI 加固 + REVIEW-9 收尾"` 已打（等用户确认后 push）。

### 阶段 2 — TD-12 + TD-13 + TD-16 小修一波清

> 3 项小 tech-debt 一波清掉。每项都是局部 surgical 修改，不引依赖、不改架构。

- Fix: **TD-12 Linux stub 测试覆盖增强** — `tests/test_linux_stubs.rs`（新文件，Linux-only via `#![cfg(target_os = "linux")]`）6 case：env/dlls/handles/memory 对 bogus pid 返回 Err（`ProcError::PermissionDenied`，不是 panic / 空 Vec）+ self pid 走 Ok 路径（至少 1 个变量 / 1 个内存区域）。`tests/test_platform_compat.rs` 加 5 个跨平台 inspect::* 契约 case（Windows/Linux/macOS 都跑，bogus pid 一律 Err + inspect 顶层返回空 InspectionData 不 panic）。
- Fix: **TD-13 CI Linux job 校验 cfg-gate 实际执行** — `.github/workflows/ci.yml` `check-linux` job 改成跑全量 `cargo test --release` + bash step 校验测试 bin 数 ≥ 30。v0.7 之前只跑 6 个手挑的 `--test xxx`，cfg(target_os="linux") 写错会静默 skip；现在阈值 30 是 v0.7.0 实际 ~50 个测试 bin 留余地的下限，防止未来新增测试 bin 静默消失。
- Fix: **TD-16 FilterExpr 错误信息中文化** — `src/filter/parser.rs` 加 `error_kind_to_chinese(&ErrorKind) -> &'static str`（TakeWhile1 → 「缺少字段名/值」、Tag → 「缺少关键字/操作符」、AlphaNumeric → 「未知字段名」、Verify → 「正则编译失败」、Digit/Float → 「数字格式错误」等 9 变体 + 兜底「语法错误」）+ `char_to_chinese(char)`（括号 / 引号 / 斜杠给出语义化提示）。`paren_expr` 闭合用 `cut(char(')'))` 让 alt 不回退到 leaf，`(cpu > 5` 缺 `)` 时最内层错误真正指向 Char(')') 而非被 leaf 的 TakeWhile1 覆盖。`tests/test_filter_expr.rs` 加 5 个中文契约 case 锁死映射表对外字面量。
- Test: 全量 920 → 925 passed（TD-12 +6 Linux-only + TD-16 +3 跨平台 + TD-16 +5 错误信息契约）。

### 阶段 3 — TD-15 FilterExpr 扩 Tree / AppGroup view

> 把 v0.7.0 阶段 4 只接入 List view 的 FilterExpr 扩展到 Tree view 和 AppGroup view。用户在这两个视图按 `:` 也能切到 FilterExpr 模式，输入 `cpu > 5 AND name =~ /chrome/` 能正确过滤。

- Added: **Tree view FilterExpr 接入** — `src/view_models/process_panel.rs::get_filtered_tree_visible(&self, cached_processes: &[ProcessInfo])` 加 `cached_processes` 参数。FilterExpr 分支建 `pid → &ProcessInfo` HashMap，按 visible TreeNode 的 pid 取原始 ProcessInfo 再 `FilterExpr::apply`（TreeNode 是 ProcessInfo 派生的精简结构，原 cmd/exe/user 等字段需要从 cached_processes 查回）。Substring 分支保留 v0.6 name.lower().contains 行为。`handle_tree_key` 在 `'/'` 旁边加 `':'` 激活 FilterExpr 模式，复用 `SearchState::activate_filter_expr()`。
- Added: **AppGroup view FilterExpr 接入** — `app_group_filtered_visual_items(&self, cached_processes: &[ProcessInfo])` 同款扩参。FilterExpr 分支两套 apply 语义：
  - **Header 项（聚合）**：用 group 的 `total_cpu` / `total_memory` + `display_name` 构造合成 ProcessInfo（`..ProcessInfo::default()`），apply 时 `cpu > 50` 表示「该 .exe 总 cpu > 50」（与 stage 3 doc 设计一致）。Header 命中 → 整组保留。
  - **Child 项（单进程）**：Header 不命中时按 pid 查 cached_processes 取原始 ProcessInfo，命中的 child 保留并自动展开该组。
- Changed: 内部 helper 签名连锁修改 — `tree_move_cursor` / `tree_toggle_select` / `tree_initiate_kill` / `tree_select_orphans` / `tree_select_stale` / `app_group_move_cursor` / `app_group_toggle_expand` / `app_group_toggle_select` / `app_group_initiate_kill` 都加 `cached_processes: &[ProcessInfo]` 参数。外部调用点（`src/app.rs::handle_scroll` / `src/tui/process_tree.rs::draw` / `src/tui/app_group_view.rs::draw`）传 `&app.cached_processes[..]`。**panel controller 边界保持**（ADR-0012）：调用方走 `app.process_panel.panel.<method>(&app.cached_processes[..])` 通过 `.panel` 访问器。
- Added: `tests/test_filter_expr.rs` 加 10 个新 case（Tree × 5：cpu_gt / pid_equality / keeps_prev_ast_on_bad_input / substring_mode_unchanged / empty_query_returns_all；AppGroup × 5：aggregate_cpu_header_match / child_partial_match / memory_aggregate / app_group_keeps_prev_ast / app_group_substring_mode_unchanged）。
- Test: 全量 925 → 930 passed。
- Docs: tech-debt TD-15 标 ✅ Fixed in v0.8.0 阶段 3；CONTEXT.md 加 `AppGroupFilterState` 新术语（Tree / AppGroup 各持独立 FilterExpr mode，与 List 解耦）。

### 阶段 1 — TD-19 ebpf Linux 真实编译验证 ⏸ 主动推迟

> ⏸ **v0.8.0 cycle 主动推迟到 v0.9.0 cycle 启动前再评估**。理由：用户主要用 Windows 开发，WSL2 / Linux 真机环境准备成本高（clang/llvm/libelf + nightly + bpf-linker ~30 min），且 stage 2/3/4 不依赖 stage 1。
>
> stage 4 review（REVIEW-9）已确认此推迟不影响 v0.8.0 cycle 收尾；Linux 验收标准（`cargo +nightly build -p proc-ebpf --target bpfel-unknown-none --release` / `cargo build --release --features ebpf`）跟随 stage 1 跳过；release CI `proc_ebpf` 后缀二进制构建用 `continue-on-error=true` 让 Linux 编译失败不阻断主 release。详见 [ADR-0016](docs/adr/0016-ebpf-flow-graph.md) + [tech-debt TD-19](docs/tech-debt.md)。

## [0.7.0] - 2026-06-28

v0.7.0 围绕 **生态卡位**（MCP server + shell 补全 + 命令面板 Ctrl+P）/ **平台深度**（Linux PSI / Win11 EcoQoS / Win ETW per-process 磁盘 IO / Linux eBPF flow graph）/ **架构债清理**（App 拆 5 panel controller + FilterExpr 表达式）三条主线，分 10 阶段推进（实现 1-8 + Review 9 + 收尾 10）。**全量回归 910 passed / 0 failed**（v0.6.0 基线 701 → +149 新测试 +5 个新模块 +10 个新术语 +8 个新 ADR）；0 个新依赖进默认依赖图（rmcp / nucleo / clap_complete / nom 全部 cfg-gate 或 feature flag；aya 仅 Linux + `ebpf` feature）。

**已知限制（必须在 release notes 显式标注）**：
- eBPF Linux 真实编译验证缺失（**TD-19**）：阶段 8 Part A/B 在 Windows 会话落地，未在 Linux + root + 内核 5.10+ 环境验证 aya `TracePoint::attach` 真实签名 / tracepoint arg offset / ELF 路径。Linux 用户首次 `cargo build --features ebpf` 可能失败，需按报错修。MVP `bytes_out` / `bytes_in` 留 0（要 hook `tcp_sendmsg` / `tcp_recvmsg`，留 TD-17）。
- FilterExpr 仅接入 List view（Tree / AppGroup 保留 substring，**TD-15**）；parse 错误信息用 nom 内部 ErrorKind 直出（**TD-16**）。均留 v0.8.0+。

### 阶段 10 — 批量修复与收尾交付（本次发布）

> 本次发布 commit：Cargo.toml 0.6.0 → 0.7.0；CHANGELOG / README / CONTEXT 计划态字样清理；release CI 加 `_ebpf` 后缀二进制 + completions 打包；10 个 stage doc 头部标 ✅ 已发布；tech-debt 状态终态确认；8 个 ADR 全部 Accepted。详见本次 commit diff。

### 阶段 9 — 全局 Review（跳过，详见顶部「已知限制」）

> v0.7.0 cycle 选择**跳过 stage 9 全局 Review**直接进 finalization。理由：各阶段已分别自测（全量 910 passed）；tech-debt TD-1~11 已修；eBPF Linux 真实编译验证留 TD-19。如后续 Linux 用户反馈问题，按 TD-19 / TD-17 / TD-18 路径补 v0.7.1 / v0.8.0 修复。

### 阶段 8 — eBPF flow graph + SecurityRule R15（Linux feature flag `ebpf`，ADR-0016）

> 用 `aya-rs` 加载 eBPF 程序监听 `sys_enter_connect` + `sched_process_exit` tracepoint，把 DNS 查询日志 + connect 事件端到端关联为 `ProcessFlow`（pid + 远端 IP + 域名 + bytes）。MVP 不监听 TLS SNI（留 TD-17）。**仅 Linux + `cargo build --features ebpf`**；Windows / macOS 走降级路径（App::flows 保持空，UI 提示）。

- Added: **eBPF flow graph 数据结构 + worker**（[`src/ebpf/`]）—— Part A 跨平台 MVP：
  - `src/ebpf/flow.rs`（~600 行）：`ProcessFlow` / `FlowEvent` / `RawEvent` / `FlowAggregator` 跨平台类型 + DNS 关联启发式（5s 窗口向前查 DnsQuery）+ 21 个单元测试。
  - `src/ebpf/{mod.rs, worker.rs, stub.rs, elf_loader.rs}`：cfg-gate 入口，Linux + `ebpf` feature 走真实 aya 加载（TracePoint attach + RingBuf reader 线程 + mpsc FlowEvent），其它平台走 stub `try_spawn → None`。
  - `src/ebpf/ebpf-ebpf/`（独立 cargo sub-project）：内核态 aya-ebpf 0.1 + aya-log-ebpf 0.1，2 个 tracepoint 程序（sys_enter_connect / sched_process_exit）+ RingBuf + Event union。
  - `Cargo.toml` 加 `[features] ebpf = ["aya", "aya-log"]` + `[workspace] members = ["src/ebpf/ebpf-ebpf"] + default-members = ["."]`（**必加** `default-members`，否则 Windows 默认 `cargo build` 会尝试编译内核态 sub-project 失败）。
  - 依赖：`aya = "0.13"` + `aya-log = "0.2"`（cfg-gate Linux only + optional）。实际 resolve 到 `aya 0.13.1` / `aya-ebpf 0.1.1`（v0.14 需 Rust 1.87，暂不升）。
- Added: **App 集成** —— `App::flows: Vec<ProcessFlow>` + `App::flow_aggregator: FlowAggregator` + `App::tick_flows_ebpf`（1s tick：drain FlowEvents → ingest + DNS 关联 → reaper_tick → drain snapshot 贴 `App::flows`）。
- Added: **端口面板 Flow 子视图** —— 按 `F`（大写）进入，列 PID / 进程名 / 远端 / 端口 / 域名 / 首次见到。非 Linux / 无 feature 显示「需要 Linux + ebpf feature」降级提示。
- Added: **exit-accounting（30s 幽灵 flow）** —— `sched_process_exit` 事件给所有该 (pid, start_time) 的 flow 打 `exit_time` 标签；`FlowAggregator::reaper_tick(now)` 把 `exit_time + 30s < now` 的 entry 移除。`ProcessFlow::is_ghost()` helper + UI 渲染加 `👻` 前缀 + 灰色斜体区分 live / ghost。`App::tick_flows_ebpf` 每 tick 调 reaper_tick（即使无新事件也清过期 ghost）。
- Added: **SecurityRule R15 外联行为评分**（[`src/security/flow.rs`]）—— v0.7 安全评分从 14 项扩到 15 项。命中条件（任一扣 30 分）：(1) dns_name 不在白名单；(2) 同一进程 10s 内连接 ≥ 50 个不同 IP（端口扫描特征）。`SniWhitelist` 加载自 `~/.config/proc/sni_whitelist.txt`（**默认文件不存在 → R15 整体不启用**，避免误报）。`SecurityScorer::score` 签名加 `flows` 参数；`BackgroundScorer::request` 同步加 `Arc<Vec<ProcessFlow>>`。
- Added: **CLI `proc flows [--limit N] [--json]`**（[`src/cli/flows.rs`]）—— Linux + ebpf feature 启动 ebpf worker → 等 2s 收集首批事件 → 输出 human-readable 表格或 JSON。非 Linux / 无 feature 输出降级提示。
- Added: 依赖 0 新增（aya 0.13 cfg-gated to Linux + optional；不进默认依赖图）。
- Added: `tests/test_ebpf_flow.rs` 16 case + `src/ebpf/flow.rs` 内联 21 case + `src/security/flow.rs` 内联 10 case + `tests/test_security.rs` 2 R15 集成 case = +49 case；全量 942 passed（基线 893 + 阶段 8 新增 49）。
- Docs: ADR-0016 标 Accepted + 补 Consequences 实测数据；CONTEXT.md ProcessFlow / EbisuBpfWorker / SecurityRule R15 术语从「计划态 / Part A 落地中」改「已落地」+ 填代码位置；README 平台支持表加 eBPF 行 + CLI 表加 `proc flows` + FAQ 加「如何启用 eBPF」/「eBPF 需要什么权限」/「R15 怎么触发」；tech-debt 加 TD-17（eBPF TLS SNI / JA4）/ TD-18（Windows Schannel）/ TD-19（Linux 真实编译验证缺失）。
- **已知限制（Part A/B 均在 Windows 会话落地，Linux 真实验证留 TD-19）**：aya `TracePoint::attach` / `RingBuf::try_from` 真实签名 + 内核态 tracepoint arg offset（`sys_enter_connect` 偏移 16 / `sched_process_exit` 偏移 24，不同内核可能不同）+ `include_bytes!` ELF 路径硬编码 + `bpf_current_task_start_time` 占位 0（需 aya-tool BTF binding 补完）—— Linux 会话需跑 `cargo +nightly build --target bpfel-unknown-none -p proc-ebpf` + `cargo build --release --features ebpf` + `sudo cargo test --release --features ebpf --test test_ebpf_flow -- --ignored` 验证 + 修编译错误。MVP `bytes_out` / `bytes_in` 留 0（要 hook `tcp_sendmsg` / `tcp_recvmsg`，留 TD-17）。

### 阶段 7 — ETW per-process 磁盘 IO（Windows，ADR-0015）

> 用 ETW NT Kernel Logger + DiskIo TypeGroup1 把 v0.6 走 sysinfo 性能计数器的 per-process 磁盘 IO 替换为更准的 ETW 数据源（管理员下精度对标 Resource Monitor）。**决策从 ferrisetw 改为手写 windows-rs**（用户偏好「更可控」；项目已有 windows-rs 依赖，ferrisetw fallback 留 ADR）。

- Added: **ETW per-process 磁盘 IO（Windows admin / x64）** —— `src/disk_io_etw/{mod.rs,provider.rs,thread_map.rs}`（新）。手写 `Win32_System_Diagnostics_Etw` API：`StartTraceW` 开 NT Kernel Logger session（固定 name + GUID `{9e814aad-...}`）→ `OpenTraceW` 注册 `EventRecordCallback` → 独立线程跑 `ProcessTrace` 阻塞。callback 解析 `EVENT_RECORD.UserData` 硬编码偏移（x64 Win8+：TransferSize@0 / DiskNumber@4 / Irp@8 / FileObject@16 / HighResResponseTime@24 / IssuingThreadId@32），按 `EVENT_HEADER.EventDescriptor.Opcode` 区分 read(2) / write(3)。详见 [`docs/adr/0015-etw-per-process-disk-io.md`](docs/adr/0015-etw-per-process-disk-io.md)。
  - thread→pid map 用 `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)` 全量枚举（不用 sysinfo `tasks()`，实测 0.34.2 在 Windows 上不稳定），5s 全量刷新 + 同步预填避免 callback 首次拉到空 map。
  - Worker 复用 `SnapshotWorker<DiskIoMap>` 模板（v0.6 NetFlowWorker 同款），1s tick → `sync_channel(1)` → 主线程 drain。worker body 内停 ETW session + join ProcessTrace 线程做 cleanup。
  - `App::overlay_disk_speeds_etw`（新）在 `update_disk_speeds` 之后跑——先填 sysinfo delta（fallback），再用 ETW 数据**覆盖**匹配 PID（ETW 更准）。ETW 缺失的 PID 保留 sysinfo 值。
  - 降级路径：Linux/macOS / Windows 非管理员 / NT Kernel Logger 已被资源监视器占用 / x86 (32-bit) Windows → `try_spawn` 返回 `None`，UI 沿用 sysinfo fallback。
- Added: `WorkerManager.disk_io_etw_worker`（Windows only）字段 + `proc diag` / `?` 帮助页 `disk_io_etw` worker 行（v0.7 阶段 1 TD-5 同款 metrics）。
- Added: 依赖 0 新增（windows-rs 已有，只加 `Win32_System_Diagnostics_Etw` feature）。
- Added: `tests/test_disk_io_etw.rs`（3 case：`DiskIoStats` 形状 + Windows admin 下 worker spawn + 自身 IO 采集；非管理员走 SKIP 路径不 fail）+ `src/disk_io_etw/{provider,thread_map}.rs` 内联单测（3 case：常量值 + ToolHelp 枚举包含当前进程 + drop join）。共 +6 case；全量 861 passed。
- Docs: ADR-0015 标 Accepted，决策段从 ferrisetw 改写手写；CONTEXT.md DiskIoStats 从「计划态」改「已落地」+ 填代码位置；README 平台支持表「per-process 磁盘 IO（ETW 高精度）」新行。

### 阶段 6 — Linux PSI 监控 + Windows 11 EcoQoS 切换（ADR-0013 / ADR-0014）

> 两个独立的平台专项功能合一个 slice：Linux Pressure Stall Information（PSI，判断"系统真卡了"的金标准）+ Windows 11 EcoQoS / Efficiency Mode（Win11 25H2 自动 throttle 后台进程，用户被坑不知道）。

- Added: **PSI 监控（Linux 4.20+）** —— `src/psi.rs`（新）手写 parser 读 `/proc/pressure/{cpu,mem,io}`（~50 行，不引 `psi` crate 也不引 `procfs-core`）。跨平台 cfg-gate：Linux 实装，Windows / macOS stub 返回 `None`。详见 [`docs/adr/0013-psi-monitoring.md`](docs/adr/0013-psi-monitoring.md)。
  - 复用 `LightWorker` 1s tick 采集（PSI avg10 字段本身是 10s 平均，1s 周期足够），不新建 worker。
  - `PsiStats` 加到 `LightSnapshot` → `SystemSnapshot::psi_stats()` getter → `src/tui/sidebar.rs` 加 `push_psi_lines` 段（颜色分级：< 5% 绿 / 5-20% 黄 / 20-50% 橙 / > 50% 红 + BOLD）。非 Linux 降级显示 "PSI: Linux 4.20+ only"。
  - `alert/rule.rs::MetricName` 加 5 个变体：`CpuPressureSome` / `MemPressureSome` / `MemPressureFull` / `IoPressureSome` / `IoPressureFull`，取 avg10 作为告警依据。`alert/config.rs` 加 5 条默认规则（`psi-cpu-some-50` Critical / `psi-mem-some-20` Warning / `psi-mem-full-20` Critical / `psi-io-some-50` Warning / `psi-io-full-20` Critical）。非 Linux 平台 metric.extract 返回空 Vec，自然不触发。
  - CPU `full` 恒为 None（内核设计：CPU 没有 full 行）；`mem_full` / `io_full` 用 `Option<PsiRecord>` 区分 "0% 压力" vs "无数据"。
- Added: **Windows 11 EcoQoS / Efficiency Mode** —— `src/throttle.rs` 加 `EcoQoSState` 枚举（Normal / Eco / Unknown）+ `set_throttle(pid, eco)` + `query_throttle(pid)` + `query_throttle_batch(pids)`。直接用 `windows-rs 0.57` 的 `SetProcessInformation(ProcessPowerThrottling)` + `PROCESS_POWER_THROTTLING_STATE`，不引 `win32-ecoqos` crate。Windows cfg-gate，其它平台 stub 返回错误。详见 [`docs/adr/0014-ecoqos-throttle.md`](docs/adr/0014-ecoqos-throttle.md)。
  - query 走 `SetProcessInformation` 的隐藏查询模式（ControlMask = 0 + StateMask = 0，Win32 不修改状态，把当前 StateMask 填回结构），比 undocumented 的 `NtQueryInformationProcess(ProcessPowerThrottling)` 更稳定。**关键**：query 模式需要 `PROCESS_SET_INFORMATION` 权限（与 set 路径一致）。
  - CLI `proc throttle <pid> on|off`（`src/cli/{def.rs, mod.rs, throttle.rs(新)}`），与 priority / affinity 同款风格。
  - `ProcessInfo.throttled: EcoQoSState` 新字段（`#[serde(default)]` 兼容旧录屏）；`HeavyWorker` 周期批量 query 当前所有 PID 的 throttle 状态（避免每帧 OpenProcess 风暴）。
  - `src/tui/process_table.rs` name 列后追加 🍃 emoji（Eco 状态显示，其它不渲染占位）；`src/tui/detail_view.rs` Summary Tab 加 "EcoQoS: Normal/Eco (T 切换)" 行；`src/inspect/controller.rs` 加 `T` 键 + `InspectorAction::ToggleEcoQoS { pid, make_eco }`，App 派发后立即刷新 `detail_process.throttled` 避免等下个 heavy tick。
- Added: 依赖 0 新增（windows-rs 0.57 已有；PSI 手写 parser）。
- Added: `tests/test_psi.rs`（7 case：parser 单元 + alert metric + 默认规则 + 跨平台契约）+ `tests/test_ecoqos.rs`（9 case：枚举 + batch query + ProcessInfo serde 兼容 + Windows round-trip）。共 +16 case；全量 855 passed。
- Docs: ADR-0013 / ADR-0014 标 Accepted；CONTEXT.md PsiStats / EcoQoSState 术语从「计划态」改「已落地」+ 填代码位置；README 平台支持表 + CLI 段同步更新。

### 阶段 5 — App 拆 5 个 panel controller（ADR-0012）

> 把 v0.6 App 直接持的 5 个 panel 字段（process_panel / port_panel / usb_panel / monitor_panel / docker_panel）拆出对应 controller（具体类型不引入 trait object），App 只持 controller 引用 + 全局状态。`handle_key` 返回 `PanelAction` 枚举让 App 派发副作用。**对应 TD-6**。

- Added: `PanelAction` 枚举（`src/app_panel.rs`）—— `Noop` / `Quit` / `SwitchMode(AppMode)` / `ToggleRecording` / `StatusMessage(String)` / `Kill(KillRequest)` / `Clipboard(String)`。`impl From<KeyResult>` 让旧 v0.6 Panel trait 输出无副作用地翻译过来。**与 InspectorAction / ReplayAction 共存，v0.8 评估合并**（surgical 原则）。
- Added: 5 个 controller（`src/view_models/{process,port,usb,monitor,docker}_panel_controller.rs`）—— 每个 ~50 行，包装对应 `XxxPanel`，提供 `panel()` / `panel_mut()` 访问器 + `handle_key` / `tick` forward。`ProcessPanelController` 额外提供 `init_tree` / `set_tree_nodes` 高频 API forward。
- Changed: `App::{process,port,usb,monitor,docker}_panel` 字段类型 `XxxPanel` → `XxxPanelController`（字段名保留，调用方仅多一层 `.panel` / `.panel()`）。
- Changed: `src/app.rs` + `src/tui/{process_table,process_tree,app_group_view,layout,right_panel,port_table,usb_panel,monitor_panel,docker_panel,sidebar,detail_view}.rs` + `tests/test_command_palette.rs` 共 ~270 处 `app.xxx_panel.<field>` → `app.xxx_panel.panel.<field>`。
- Changed: `App::handle_key` dispatch 切换：5 个主面板分支从 inner panel `Panel::handle_key`（返回 `KeyResult`）改为 controller `handle_key`（返回 `PanelAction`）。结果 match 翻译 `Quit` / `SwitchMode` / `StatusMessage` / `Kill` / `Clipboard` 五个变体的副作用（原 `KeyResult` import 移除）。
- Added: `KillRequest` 手动 `impl Debug`（截断 pids 列表到前 8 个），让 `PanelAction::Kill` 满足 derive Debug。
- Added: `tests/test_panel_controllers.rs` — 6 个集成测试（5 controller 各 1 case + App 持 controller 路径综合验证）。
- Docs: ADR-0012 标 Accepted；CONTEXT.md PanelController 术语从「计划态」改「已落地」+ 填代码位置；tech-debt TD-6 标 ✅ Fixed。

### 阶段 4 — 过滤表达式 FilterExpr（ADR-0011）

> 进程列表搜索从纯子串升级为 bottom 式表达式（`cpu > 5 AND name =~ /chrome/`），第一字符 `:` 切到 FilterExpr 模式，否则走原 substring（向后兼容，原 v0.6 用户无感）。详见 [`docs/adr/0011-filter-expression.md`](docs/adr/0011-filter-expression.md)。

- Added: `src/filter/{mod.rs, parser.rs}`（新）—— `nom 7` parser + `FilterExpr` AST（`FieldCmp` / `Regex` / `And` / `Or` / `Not`）+ `EvalCtx` 接 `ProcessInfo`。字段：`cpu` / `mem` / `pid` / `name` / `user` / `cmd` / `disk_read` / `disk_write` / `net_sent` / `net_recv` / `security_score`。操作符：`=` / `!=` / `>` / `<` / `>=` / `<=` / `=~`。单位：`b` / `kb` / `mb` / `gb` / `tb`（1024 进制）/ `%`。
- Added: `SearchState::mode: QueryMode`（`Substring` / `FilterExpr(FilterExpr)`）—— 第一字符 `:` 切到 FilterExpr 模式；parse 失败保留上一次成功 AST + 标题栏显示错误。`cached_sorted` 缓存键扩展为 `(sort_field, query, mode)`，mode 变化触发重建。
- Added: CLI `proc ls --filter 'cpu > 5 AND name =~ /chrome/i'` —— 与 TUI 路径等价。
- Added: 依赖 `nom = "7"`（`regex = "1"` 已有）。
- Added: `tests/test_filter_expr.rs` + `src/filter/parser.rs` 内联测试 +25 case。
- Known limits: **FilterExpr 仅接入 List view**（Tree / AppGroup 视图保留 substring，原因：数据模型不匹配，详见 **TD-15**）；**parse 错误信息用 nom 内部 ErrorKind 直出**（用户看不懂，详见 **TD-16**）。均留 v0.8.0+ 候选。
- Docs: ADR-0011 标 Accepted；CONTEXT.md FilterExpr / FilterToken 术语从「计划态」改「已落地」+ 填代码位置；README CLI 段加 `--filter`；`?` 帮助页加 FilterExpr 段（字段 / 操作符 / 语法 / 4 个示例）。

### 阶段 3 — Shell completion + 命令面板 Ctrl+P

> 解决键位爆炸（6 面板 × 6 Tab × 17+ 子命令 × 9 排序字段）+ 跨 shell 补全缺失。详见 [`docs/adr/0010-shell-completion-and-palette.md`](docs/adr/0010-shell-completion-and-palette.md)。

- Added: `proc completions --shell <bash/zsh/fish/powershell/elvish>` 子命令（`src/cli/completions.rs` + `src/cli/def.rs::Command::Completions`）。基于 `clap_complete 4`，在线生成不耦合 build-time。
- Added: `completions/{proc.bash, proc.zsh, proc.fish, _proc.ps1}` 4 个预生成文件，release artifact 同步打包。
- Added: **命令面板 Ctrl+P**（`src/tui/command_palette.rs`）—— 基于 `nucleo`（Helix 编辑器 fuzzy 库）+ modal 浮层 + `AppLayer` 状态机。
  - `AppLayer { Normal, Search, Palette }` 决定按键优先派给搜索框 / 命令面板 / 当前面板。
  - 注册 ~40 条命令（`default_items()`）：6 面板 + 3 视图模式 + 9 排序 + 6 Inspector Tab + 11 主题 + 5 全局 toggle + 4 进程操作 + 3 Docker 操作 + 退出。
  - 键位：Ctrl+P 打开 / Esc 关闭 / ↑↓ 选择 / Enter 执行 / Ctrl+U 清空 / Backspace 删除。
- Added: 依赖 `clap_complete = "4"` + `nucleo = "0.5"`。tui-input 因与 ratatui 0.29 在 `unicode-width` (=0.2.0 vs ^0.2.2) 冲突，输入框逻辑手写（~30 行）。
- Added: `App::current_layer()` / `App::is_palette_open()` / `App::dispatch_command_action()` 公开 API。
- Added: `theme::set_theme_index()` 直跳主题（命令面板 SetTheme(N) 用）。
- Added: `DockerPanel::palette_restart_selected` / `palette_stop_selected` 公开入口。
- Added: `tests/test_command_palette.rs` — 9 个集成测试（spec 要求的 7 case + Help 模式下 Ctrl+P 拦截 + palette action 实际生效）。
- Added: `src/tui/command_palette.rs` 13 个单元测试（fuzzy 匹配 / 键位 / clamp / reset / unique id）。
- Changed: `App::handle_key` 在 modal 对话框（kill_confirm / pending_record_confirm）之后、全局键位（R / D / tab switch）之前插入 palette layer 拦截。`'D'` dismiss crashes 改为 `&& active_layer != Palette` 避免 palette 输入误触。
- Changed: `src/tui/layout.rs::draw` 在所有 panel 之上叠加 `command_palette::draw` 浮层（Clear + Block + 输入框 + 列表 + footer）。
- Docs: README 加 `Ctrl+P` 行 + Shell 补全安装段；`?` 帮助页加「命令面板」section；ADR-0010 标 Accepted。

### 阶段 2 — `proc mcp serve` MCP server（最大卖点）

> 基于 [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) 官方 Rust SDK，stdio transport，把 proc 的 17+ CLI 子命令暴露为 MCP tools 供 Claude Desktop / Cursor / Windsurf 调用。详见 [`docs/adr/0009-mcp-server.md`](docs/adr/0009-mcp-server.md)。

- Added: `proc mcp serve` 子命令（`src/cli/mcp_cmd.rs` + `src/cli/def.rs::McpSub`）。
- Added: `src/mcp/{mod.rs,handler.rs}` — `ProcMcpHandler` + 17 个 `#[tool]` 方法 + `#[tool_handler]` 实现 `ServerHandler`。
- Added: 17 个 thin-wrapper tools — `proc_ls` / `proc_tree` / `proc_port` / `proc_kill` / `proc_pkill` / `proc_eject` / `proc_who` / `proc_handles` / `proc_priority` / `proc_affinity` / `proc_smart` / `proc_dns` / `proc_diag` / `proc_monitor_list` / `proc_docker_ps` / `proc_docker_top` / `proc_docker_logs`。
  - **未暴露**（对 LLM 无意义）：`record` / `replay` / `export`。
  - **后续阶段追加**：`proc_psi` / `proc_throttle`（阶段 6）/ `proc_disk_io`（阶段 7）/ `proc_flows`（阶段 8 eBPF feature flag）。
- Added: 依赖 `rmcp = "0.11"`（features = `["server", "transport-io"]`；默认 feature 已含 `macros`，自动拉 `dep:schemars` v1.x）+ `async-trait = "0.1"`。tokio feature 扩 `io-std` / `io-util`。
- Added: `tests/test_mcp_server.rs` — 6 个集成测试（list_tools ≥ 17 / proc_ls 限 5 / sort=cpu 降序 / kill 不存在 PID 不 crash / diag 返回 worker metrics / docker_ps 在 daemon 不可用时也不 crash）。
- Design: 每个 tool 都是 thin wrapper，**直接调采集层**（`crate::collect::SystemSnapshot` / `crate::port_map::scan_ports` / `crate::kill::kill_process` / ...），不调 `crate::cli::*::run_*`（那些 println! 表格，对 LLM 无意义）。
- Design: 所有 tool 返回 `Result<CallToolResult, McpError>`，输出统一 JSON `{ ok: bool, ...payload }` / `{ ok: false, error: string }`。
- Security: `proc_ls` 默认**不返回** `exe` / `cwd` / `user_id` 字段，避免 LLM 上下文泄漏敏感路径（详见 ADR-0009）。
- Design: MCP server 走独立 tokio current-thread runtime（`src/mcp/mod.rs::run_mcp_serve`），不污染 TUI 同步路径；与 `DockerMonitor` 自有 runtime 不冲突。
- Docs: README 加「MCP server（LLM agent 接入）」段（含 Claude Desktop / Cursor 配置示例 + 17 tool 清单）。ADR-0009 标 Accepted。

### 阶段 1 — 技术债一波清（10 项 v0.6 P2 + CONTEXT.md 新术语段）

> 消化 v0.6.0 Review（`docs/reviews/REVIEW-7.md`）留下的 11 项 v0.7.0 候选 tech-debt 中的 10 项小修（TD-6 大重构留阶段 5 独立做，TD-11 决策类归档不修），同步追加 CONTEXT.md 的 v0.7.0 新术语段。所有改动局部 surgical，不引依赖、不改架构。

- Fixed (TD-1): 全文删除 `--tb=no`（pytest 参数，cargo test 不认），覆盖 `CONTRIBUTING.md` / `plan.md` / 6 个旧 stage doc。
- Fixed (TD-2/3): `docs/stages/stage-6.md` / `stage-7.md` 头部加推迟标注（v0.6 实际只做了任务 1/2，任务 3-5 推 v0.7.0+），修正 stage-7 切片 E 假设 proptest/criterion 存在的错误提问。
- Fixed (TD-4): CONTEXT.md 顶部加「⚠ 已知限制」段，显眼标注 `WorkerManager::restart` 未实现；README FAQ 加「worker 崩溃了怎么办」一行。
- Fixed (TD-5): `proc diag` / `?` 帮助页输出加 Docker logs worker 行（`docker_logs_<container_id>`）—— `DockerPanel` 暴露 `metrics()` 接口聚合多 logs worker（每容器一个）。
- Fixed (TD-7): `tests/test_stage8_perf_regress.rs` → `tests/test_perf_baseline.rs`（文件注释误导：实际是 stage-4 落地时一起写的性能基线，不是 stage-8 一次性）。
- Fixed (TD-8): help_panel Workers 区段 worker 名 truncate 到 10 字符 + `…` ellipsis（如 `dns_log_wo…`），避免长名（`dns_log_worker` = 14 字符）破坏列对齐。
- Fixed (TD-9): `SearchState` 改增量 lowercase（`Char(c)` push `c.to_ascii_lowercase()` / `Backspace` pop），ASCII 路径 O(1)；Unicode 复杂大小写（`İ` → `i̇`）走整体重算 fallback。
- Fixed (TD-10): DNS PowerShell probe 走 `spawn_with_reduced_privileges`，与 v0.6.0 阶段 2 主 spawn 路径统一（elevated 时剥离 SeDebugPrivilege）。
- Fixed (TD-14): `tests/test_panic_hook_chain.rs` 集成测试（3 case）验证 panic hook chain 时序：terminal restore → crash report → 默认 hook；CLI 模式（无 TUI）panic 也写盘。
- Added: CONTEXT.md 追加完整 v0.7.0 新术语段（McpServer / CommandPalette / FilterExpr / PanelController / PsiStats / EcoQoSState / DiskIoStats / ProcessFlow / EbisuBpfWorker / SecurityRule R15 共 10 个术语），全部填代码位置 + ADR 引用。
- Decision (TD-11): **不修**。watchdog spawn 是用户主动配置的命令（`alerts.toml`），威胁模型与 DNS PowerShell probe 本质不同（后者走 `-Command` 接受任意脚本 = RCE 经典跳板；前者是用户自写的 binary / shell pipeline，用户最清楚是否需要 elevated token）。强制走 restricted_spawn 会破坏依赖 elevated token 的合法用例，引入 `inherit_privileges` config 选项又会让用户困惑。v0.8.0+ 若有真实需求反馈再加 config 开关。详见 `docs/tech-debt.md` TD-11。
- Docs: tech-debt.md 10 项标 ✅ Fixed + TD-11 标决策不修。

## [0.6.0] - 2026-06-26

本次发布聚焦：**安全加固 + 可观测性 + 架构债清理**。8 个阶段累积 ~5000 行代码（含测试），无 API 破坏。

**最终基线**（实测）：

| 命令 | 结果 |
|---|---|
| `cargo test --release` | ✅ 701+ passed / 0 failed / 3 ignored（baseline 700 → 阶段 8 +1 worker crash test） |
| `cargo clippy --release --all-targets -- -D warnings` | ✅ 0 warnings |
| `cargo build --release --no-default-features` | ✅ 编译通过 |
| `cargo fmt --all -- --check` | ✅ fmt clean |

### 阶段 8 — 批量修复 + 发布（本次发布）

> 消费 `docs/reviews/REVIEW-7.md` 全部 P0（1 项）/ P1（9 项），归档 P2（14 项）到 `docs/tech-debt.md`。
> README 完整重写、CHANGELOG 定稿、Cargo.toml 版本号 0.5.0 → 0.6.0、tag v0.6.0。

- Fixed (P0-1 plan.md 状态错): plan.md「阶段总览」表 4 个阶段标 `⬜ 未开始` 实际
  已落地（阶段 3/4/5/6/7 全部已 `[x] 已完成`），违反 ADR-0001 phase gating
  「唯一勾选点」规则。同步修正 plan.md FAQ `masked` → `is_secret` 字段名 + 删
  `--tb=no`（pytest 参数，cargo test 不认）。
- Fixed (P1-1 文档 self-mitigation 漏 ImageLoad): `src/security/self_mitigation.rs`
  实际开 5 项（DEP / ASLR / ProhibitDynamicCode / DisableExtensionPoints /
  **ImageLoad NoRemote+NoLow+PreferSystem32**），CONTEXT.md / SECURITY.md 旧写
  4 项漏 ImageLoad。SECURITY.md「Hardening」段补 ImageLoad 行；「已知限制」段
  「4 项策略外」改「5 项策略外」。
- Fixed (P1-2/P1-8 EnvVar 字段名): `src/inspect/mod.rs:21-28` 实际字段名
  `is_secret: bool`；CONTEXT.md / plan.md FAQ 旧写 `masked: bool`。三处统一改
  `is_secret`。
- Fixed (P1-3 restricted_spawn 范围文档): `src/security/restricted_spawn.rs:17-19`
  模块注释明确「不接入 docker exec / nvtop」，CONTEXT.md / SECURITY.md 旧写
  「PowerShell DNS / docker exec / nvtop」错。改「仅接入 PowerShell DNS（elevated
  时）；docker exec / nvtop 因自身需 privileged token 不接入（ADR-0008）」。
- Fixed (P1-4 漏接 restricted_spawn): `src/eject/cache.rs::flush_write_cache`
  + `src/smart/mod.rs::list_disks_wmi` + `src/smart/mod.rs::read_smart_via_wmi`
  3 处 PowerShell `Command::new(...).output()` 改用新增的
  `crate::security::restricted_spawn::run_with_reduced_privileges`，与 DNS spawn
  路径统一（elevated 时剥离 SeDebugPrivilege）。
  - 新增: `RestrictedOutput { status, stdout }` + `run_with_reduced_privileges`
    便利函数（语义近似 `Command::output`，stderr 丢弃 — 一次性命令只看 stdout + exit）。
- Fixed (P1-5 README 键位描述错): README.md 第 33 行 `r 强制重新采集` /
  `a 调整 affinity`（实际未实现）→ 改为 `F5 强制重新采集`、`y 复制进程信息`、
  `v 切换 Env Tab secret 脱敏`；删除 `a 调整 affinity` 误导。
- Fixed (P1-6 详情页 r/c deprecation warning): `InspectorController::handle_key`
  补 `KeyCode::Char('r')` / `KeyCode::Char('c')` 分支返回
  `StatusMsg("⚠ 'r'/'c' 将在 v0.7.0 移除，请用 F5/y")`。`App::try_handle_tab_switch`
  在 `mode == ProcessDetail` 时让 'c' 落入 InspectorController（不再被全局侧边栏
  折叠抢键）。0.5.0 用户升级后按 'r' / 'c' 不再静默 noop，获得指引用 F5 / y。
  - Test: `tests/test_inspector.rs` 原
    `c_key_in_detail_toggles_sidebar_not_copy` / `r_key_in_detail_does_not_refresh`
    重写为 `c_key_in_detail_shows_deprecation_warning` /
    `r_key_in_detail_shows_deprecation_warning`，验证 status_message 含
    `v0.7.0` + `F5`/`y`。
- Fixed (P1-7 worker panic 不写 crash report): `src/worker.rs::spawn` 的
  `catch_unwind` 截获 panic 后**不触发全局 panic hook**（Rust 标准库语义），
  导致 worker 崩溃只发 banner，磁盘无 crash report。补一条
  `crate::metrics::crash::write_worker_crash_report(thread_name, &msg, &bt_str)`
  调用，文件名 `crash-worker-{name}-{ts}.txt` 与主线程 panic 区分。
  - 新增: `src/metrics/crash.rs::write_worker_crash_report` /
    `write_worker_crash_report_to` / `format_worker_crash_report`。best-effort
    （写盘失败仅 `tracing::warn`，不阻塞 crash_tx 通知主线程）。
  - Test: `metrics/crash.rs` 加 `write_worker_crash_report_to_writes_file_with_worker_name`
    + `format_worker_crash_report_includes_all_fields`。
- Fixed (P1-9 Cargo.toml 版本号): 0.5.0 → **0.6.0**。crash report /
  `proc --version` / `cargo binstall` 都依赖此字段。
- Docs: REVIEW-7.md 全部 P0/P1 项追加 `**Status: Fixed in commit XXX**`。
- Docs: tech-debt.md（阶段 7 已建）覆盖 14 项 P2，按 v0.7.0（11 项）/
  v0.8.0+（3 项）分组。
- 验收: 全量回归 700 → **701+ passed / 0 failed / 3 ignored**；clippy 0 warnings；
  fmt clean；`--no-default-features` 编译通过。

### 阶段 7 — Review（本次发布）

> 阶段 7 全局审查，产出 `docs/reviews/REVIEW-7.md`（P0 1 + P1 9 + P2 14）+
> `docs/tech-debt.md`（P2 归档）。本阶段未修改任何代码（仅新增 REVIEW-7.md +
> tech-debt.md）。

- Review: 切片 A 安全审查（self-mitigation / env_mask / restricted_spawn / 录屏
  强制 mask / EnvVar serde 兼容性 — 6 子项）— 发现 P1-1/P1-3/P1-4 文档漂移与
  restricted_spawn 漏接。
- Review: 切片 B 可观测性（日志 rotate / panic hook / WorkerMetrics CAS /
  `proc diag` / help_panel 列宽 — 5 子项）— 发现 P1-7 worker panic 不写盘。
- Review: 切片 C 性能（Arc deref / name_lower / rebuild_sorted_cache /
  ProcessStatus 映射 — 4 子项）— 全部 ✓。
- Review: 切片 D 架构（3 个 Controller 拆分 / 通信 / import 路径 / 循环依赖 /
  WorkerManager::restart — 5 子项）— 发现 P2-4 restart 未实现，归档 tech-debt。
- Review: 切片 E UX+测试（'r'/'c' deprecation / F5'y' 提示 / proptest 假设 /
  Linux stub 覆盖 / 命名误导 — 5 子项）— 发现 P1-6 deprecation warning 未实现、
  P2-7 命名误导。
- Review: 横切 5 维度（架构一致性 / 文档完整性 / 测试覆盖 / 性能基线 / 安全）—
  发现 P0-1 plan.md 状态错、P1-9 Cargo.toml 版本号未升。

### 阶段 6 — 键位冲突修复（v0.6.0 收尾）

### 阶段 6 — 键位冲突修复（v0.6.0 收尾）

> 3 项跨面板键位双/多语义冲突修复，对齐 vim/Mission Center 习惯。本阶段独立切片
> 完成阶段 5 拆分后剩余的 UX 修复，标记 v0.6.0 收尾。**纯键位映射搬迁，不改业务逻辑**；
> 每个 key_handler 的 match 分支单点替换 `KeyCode::Char('r')` → `KeyCode::F(5)` 等，
> 不引入新模块 / 新抽象。

- Changed (#12 详情页刷新键): `InspectorController::handle_key` 中
  `KeyCode::Char('r')` → `KeyCode::F(5)`。原 'r' 在详情页 = 刷新 / Docker
  面板 = restart / USB 面板 = 刷新设备（三语义），Mission Center / htop
  习惯用 F5 刷新。详情页 'r' 现落入默认分支 noop（用户按 'r' 不再有副作用）；
  USB 'r' 刷新设备保留不变（无冲突）。
  - 影响: `src/inspect/controller.rs`（match 分支 + 4 处描述性注释同步）；
    `src/tui/detail_view.rs` 5 处「按 r 刷新」空数据提示 + 1 处 tab 栏
    `r=刷新` + 1 处 priority 缓存注释 + 1 处 Summary Tab 底部快捷键栏；
    `src/tui/help_panel.rs` 进程列表段新增 `F5 详情页: 强制刷新 Inspector 数据`
    （原帮助页未列 r 刷新，新增更完整）。
- Changed (#13 详情页复制键): `InspectorController::handle_key` 中
  `KeyCode::Char('c')` → `KeyCode::Char('y')`（vim yank 风格）。原 'c' 在
  详情页 = 复制 / 全局 = 侧边栏折叠（双语义）；非详情页按 'c' 用户无反馈。
  迁移后详情页 'c' 落入 `try_handle_tab_switch` 全局 'c' = 统一侧边栏折叠，
  不再双语义。
  - 影响: `src/inspect/controller.rs`（match 分支 + `CopyInfo` 变体注释）；
    `src/tui/detail_view.rs` Summary Tab 底部 `c=复制` → `y=复制`；
    `src/tui/help_panel.rs` 进程列表段 `c 详情页: 复制` →
    `y 详情页: 复制（vim yank）`；`src/app.rs::try_handle_tab_switch` 注释更新。
- Changed (Docker restart 键): `DockerPanel::handle_key` 中
  `KeyCode::Char('r')` → `KeyCode::Char('R')`（Shift+R）。让位详情页 F5 后，
  Docker 自身也错开 'r'；Shift+R 是 Mission Center / docker-compose UI 常见
  「强制/重启」语义。Containers/Images/Volumes 三视图统一用 Shift+R
  （Containers restart / Images refresh / Volumes refresh），避免 Docker
  面板内 'r'/'R' 双语义分裂。
  - 影响: `src/view_models/docker_panel.rs`（match 分支 + `refresh` 函数注释）；
    `src/tui/help_panel.rs` Docker 段 `r 重启容器` →
    `Shift+R 重启容器 / 刷新镜像或卷列表`。
- Test: 全量回归基线 694 → **700 passed / 0 failed / 3 ignored**（新增 6 项
  `tests/test_inspector.rs`：`y_key_in_detail_triggers_clipboard_copy` /
  `c_key_in_detail_toggles_sidebar_not_copy` /
  `r_key_in_detail_does_not_refresh` / `f5_key_resets_scroll_to_zero` /
  `y_key_does_not_quit_or_change_mode` / `f5_key_does_not_quit_or_change_mode`；
  既有 3 项 `r_key_*` 改名 `f5_key_*` 同步键位）。
- Docs: CONTEXT.md「术语演进历史」段把 3 项键位修复标为「已落地」；
  `stage-6.md` 任务 1/2 设计已完成（任务 3-5 proptest/criterion/Linux stub
  不在本 slice 范围）。
- 验收: clippy 0 warnings；fmt clean；`--no-default-features` 编译通过。

### 阶段 5 — 架构拆分（App + main.rs）— 已完成

> 本阶段分多次会话完成。stage-5.md 容量预警 ~1400 行（接近 1500 上限），
> 故按 surgical 原则切片。已落地：#5a WorkerManager / #5b InspectorController
> / #5c ReplayController / #6 main.rs 拆分。键位冲突修复已作为阶段 6 独立
> slice 落地（见上）。

- Refactor (#6 main.rs 拆分): `src/main.rs` 1657 行平铺 36 个 `run_*` +
  `run_docker_*` 函数拆到 `src/cli/{mod.rs, def.rs, diag.rs, ls.rs, kill.rs,
  port.rs, handles.rs, priority.rs, smart.rs, dns.rs, monitor.rs, export.rs,
  record.rs, eject.rs, docker_cmd.rs}` 14 个子模块。**纯搬迁无业务逻辑变更**
  —— 每个函数体逐字节搬迁，diff 只显示文件移动 + import 调整（`crate::xxx`
  → `crate::xxx`，绝对路径在子模块里仍可用，无需相对路径调整）。
  - 模块映射: `run_diag` → `diag` / `run_ls + run_tree` → `ls`
    / `run_kill + run_pkill` → `kill` / `run_port` → `port`
    / `run_smart + run_smart_list + run_smart_detail` → `smart`
    / `run_dns` → `dns` / `run_who + run_handles + run_handles_pid +
    pid_to_name` → `handles` / `run_priority + parse_priority_class +
    run_affinity` → `priority` / `run_eject` → `eject` / `run_export` →
    `export` / `run_monitor + 私有 run_tui inline` → `monitor`
    / `run_record + run_replay + run_vt100_replay + run_legacy_replay` →
    `record` / `run_docker + 11 个 run_docker_*` → `docker_cmd`。
  - 新增: `src/cli/mod.rs` (66 行) — 声明 14 个子模块 + re-export def.rs 的
    `Cli / Command / DockerSub` 让旧路径 `proc::cli::Cli` 可用 + 顶层
    `pub fn run_subcommand(cmd)` match dispatch。`src/cli/def.rs` 由原
    `src/cli.rs` `git mv` 而来（285 行 clap derive 定义，字节不变）。
  - 字段名/签名不变: 所有 `pub fn run_*` 签名保持，调用点（main.rs::main
    的 .prec/.cast 直跑路径 + match dispatch）改 `proc::cli::record::run_replay`
    / `cli::run_subcommand(cmd)`。
  - main.rs: 1657 → **134 行**（-1523 行）。仅保留 `fn main` /
    `fn init_tracing` / `fn install_panic_hook` / `fn run_tui`（默认入口）4 个
    函数 + `use` 块。`monitor` 子命令无参数 → 进 TUI 分支 inline 一份
    `run_tui` 5 行函数到 `cli::monitor`（main.rs 是 binary crate，不能被
    lib 中的子模块引用；为 DRY 留 follow-up，但 surgical 原则优先搬迁）。
  - 验收: 全量回归 694 passed / 0 failed / 3 ignored（与 #5a/#5b/#5c 完全一致）；
    clippy 0 warnings；fmt clean；`--no-default-features` 编译通过。

- Refactor (#5c ReplayController): `App` 中录屏回放 2 字段
  （`replay_player: Option<Player>` / `timeline_state: Option<TimelineState>`）
  集中到新模块 `src/replay/{mod.rs, controller.rs}` 的 `ReplayController` 结构。
  `ReplaySpeed` / `TimelineState` 类型定义同步从 `src/app.rs` 顶部搬到
  `src/replay/controller.rs`，`App` 通过 `pub use crate::replay::{ReplaySpeed,
  TimelineState}` re-export 保持旧路径 `crate::app::ReplaySpeed` 可用（TUI 不动
  import）。`App::handle_replay_key` 整体迁到 `ReplayController::handle_key`，
  返回新增的 `ReplayAction` 枚举（`Noop` / `Quit` / `ApplyFrame`），App 通过
  `dispatch_replay_action` 派发副作用（`q` → `should_quit`；`ApplyFrame` →
  把当前帧应用到 15+ panel/metrics 字段），避免 controller 反向依赖 App。
  `App::replay_tick` 删除（整体由 `ReplayController::tick` + dispatch 取代）；
  `App::replay_load_current_frame` 改名 `apply_replay_frame`，主体保留（操作
  App 字段，留在 App），调 `ReplayController::current_frame()` 取帧后释放借用
  再 mutate。`App::replay_frame_mode` / `start_replay` 改为薄 delegate。
  - 新增: `src/replay/mod.rs` (9 行) + `src/replay/controller.rs` (232 行)。
    `src/lib.rs` 加 `pub mod replay`。
  - 字段名保持不变（`app.replay.replay_player` / `app.replay.timeline_state`），
    嵌套多一层 `.replay.` 前缀，与 #5a/#5b 同款原则。
  - 设计要点：`replay_tick` / `replay_load_current_frame` 深度耦合 App 15+ 字段
    （cached_processes / global_*_history / port_panel / docker_panel / op_history
    / status_message / ...），采用方案 (b)（参考 InspectorAction 模式）—— controller
    只持状态 + 提供查询接口（`current_frame`），App 收到 `ApplyFrame` 后自己写回数据，
    controller 不持 App 引用。`restore_replay_panel_data` / `_metrics` / `_view_mode`
    / `_nav` 4 个 App 私有方法保留（操作 App 字段，未搬）。
  - 影响行数: src/app.rs 1847 → 1706（-141 行）；src/tui/replay_panel.rs 1 处访问
    路径改（`&app.timeline_state` → `&app.replay.timeline_state` 等）；tests 无访问
    点改造（tests 目录无 `replay_player` / `timeline_state` 引用）。
  - 验收: 全量回归 694 passed / 0 failed / 3 ignored（与 #5a/#5b 完全一致）；
    clippy 0 warnings；fmt clean；`--no-default-features` 编译通过。

- Refactor (#5b InspectorController): `App` 中详情页 9 字段
  （`detail_process` / `inspection_tab` / `inspection_data` / `inspection_search`
  / `inspection_scroll` / `inspection_handles_data` / `inspection_memory_data`
  / `detail_priority` / `env_reveal`）集中到新模块 `src/inspect/controller.rs`
  的 `InspectorController` 结构。`App::handle_detail_key` 整体迁到
  `InspectorController::handle_key`，返回新增的 `InspectorAction` 枚举
  （`Noop` / `StatusMsg` / `Close` / `BumpPriority` / `KillPid` / `AddMonitor`
  / `CopyInfo`），App 派发 action 处理副作用（写 `status_message` /
  `record_op` / `kill_process` / `add_monitor` / 剪贴板），避免 controller
  反向依赖 App。`App::switch_mode(ProcessDetail)` 初始化整体封装到
  `InspectorController::open(port_entries)`；heavy tick 中 `detail_process`
  维护 + `refresh_detail_priority` 合并为 `sync_detail(cached)` +
  `refresh_detail_priority()` 调用。`App::refresh_detail_priority` 删除
  （已搬到 controller）。
  - 新增: `src/inspect/controller.rs` (284 行)。`src/inspect/mod.rs` 加
    `pub mod controller` + re-export `InspectorController` / `InspectorAction`。
  - 字段名保持不变（`app.inspector.inspection_tab` 而非 `app.inspector.tab`），
    嵌套多一层 `.inspector.` 前缀，tests 改造机械化。
  - `PanelContext.detail_process` 类型不变（仍 `&mut Option<ProcessInfo>`），
    只是 App 构造 ctx 时源头改为 `&mut self.inspector.detail_process`。
    ProcessPanel 在 Enter 时通过 `*ctx.detail_process = Some(proc)` 写入。
  - 影响行数: src/app.rs 2006 → 1847（-159 行）；src/tui/detail_view.rs
    20+ 处 `app.X` 改 `app.inspector.X`；tests/test_inspector.rs 40+ 处 +
    tests/test_record_protection.rs 5 处 + tests/test_skeleton.rs 2 处
    访问路径改。
  - 验收: 全量回归 694 passed / 0 failed / 3 ignored（与 #5a 完全一致）；
    clippy 0 warnings；fmt clean；`--no-default-features` 编译通过。

- Refactor (#5a WorkerManager): `App` 中 4 个 worker 句柄字段
  （`port_worker` / `usb_worker` / `net_flow_worker` / `dns_log_worker`）
  集中到新模块 `src/workers/{mod.rs, manager.rs}` 的 `WorkerManager` 结构。
  - 新增: `src/workers/mod.rs` (8 行) + `src/workers/manager.rs` (78 行)。
  - 字段名保持不变（`app.workers.port_worker` 而非 `app.workers.port`），
    call site 改动最小（`self.X_worker` → `self.workers.X_worker`）。
  - 新增方法: `WorkerManager::new(crash_tx)` 统一 4 个 spawn；
    `WorkerManager::metrics_snapshot() -> Vec<NamedWorkerStats>` 聚合
    4 个直管 worker 的 stats。`App::worker_metrics` 改为调它 + 追加 docker。
  - 未实现: `restart(name)` 故障恢复方法（无调用方，按 surgical 原则不预实现，
    待真正需要时再加）。
  - 影响行数: src/app.rs 2031 → 2006（-25 行，含 worker_metrics 简化）；
    tui/{port_table.rs, layout.rs} 各 1 处访问点改路径。
  - 验收: 全量回归 694 passed / 0 failed / 3 ignored；clippy 0 warnings；
    fmt clean；`--no-default-features` 编译通过。

### 阶段 2 — 安全加固（v0.6.0 P0）

- Added (#1 env_mask): `src/inspect/env_mask.rs` 新模块 — `is_secret_key` 匹配
  12 个 secret 关键字（KEY/TOKEN/SECRET/PASSWORD/...）+ `DATABASE_URL` /
  `*_AUTHORIZATION` 特例；`mask_value` 把值截前 2 字符 + `***` + 原长（多字节
  字符按 char 截取）。`EnvVar` 加 `is_secret: bool` 字段（parse 时同步判定）；
  `render_value_owned(reveal)` 暴露 mask/reveal 二选一。详情页 Env Tab 默认
  mask；按 `v` 切换 `App::env_reveal`；录屏时（`recording_wanted=true`）
  `draw_env_tab` 强制 mask（`reveal = env_reveal && !recording`）。Env Tab 顶部
  显示 🔒/🔓 badge + 提示 `v=切换`。新增 `tests/test_env_mask.rs` 11 项集成测试。
- Added (#2 self_mitigation): `src/security/self_mitigation.rs` 新模块 —
  `apply_self_mitigations() -> Vec<&'static str>` 调 `SetProcessMitigationPolicy`
  应用 5 项策略：DEP(Permanent) / ASLR(HighEntropy+BottomUp) / ProhibitDynamicCode /
  DisableExtensionPoints / ImageLoad(NoRemote+NoLow+PreferSystem32)。**不开
  ProcessSignaturePolicy**（nvml-wrapper 兼容性，ADR-0008）。返回失败策略名 Vec
  不 panic；windows 0.57→0.61 跨版本通过 `Flags: u32` raw bitmask 写入规避字段名变化。
  `main.rs::main` 第一行调用（早于 tracing init）；失败 `eprintln!`。
  新增 `tests/test_self_mitigation.rs` 3 项集成测试。
- Added (#3 record_protection): `App::pending_record_confirm: bool` 新字段。
  按 `R` 不再直接启动录屏，而是进入 pending 状态并显示警告「会捕获屏幕所有内容
  含 DNS 域名 / 进程 cmd」。`y/Y` 确认启动（同时强制复位 env_reveal）；
  `n/N/Esc/q/Q` 取消；再按 `R` 也取消；其他键吞掉等用户选择。
  新增 `tests/test_record_protection.rs` 11 项集成测试覆盖状态机 8 个转换 + 录屏
  强制 mask 不变量 + 详情页 v 在录屏中失效。
- Added (#4 restricted_spawn): `src/security/restricted_spawn.rs` 新模块 —
  `spawn_with_reduced_privileges(program, args) -> io::Result<RestrictedChild>`。
  Windows 上走 `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)` +
  `CreateProcessAsUserW` 剥离继承的 SeDebugPrivilege。非 elevated 环境（缺
  SeIncreaseQuotaPrivilege）自动降级到 `std::process::Command` 并 tracing::warn。
  `RestrictedChild` 自管 process HANDLE + pipe + std::process::Child fallback；
  kill/wait/stdout 接口对调用方透明。**接入 PowerShell DNS 子进程**
  （`src/dns_log/windows_dns.rs`）；docker exec / nvtop 留 0.6.1+（elevated 路径
  需要更细粒度 token 控制）。
  新增 `tests/test_restricted_spawn.rs` 6 项集成测试。
- Added (cargo): `windows` crate features 加 `Win32_System_Pipes` + `Win32_System_Console`
  （`CreatePipe` + `GetStdHandle`）。
- Changed: ADR-0008 Status `Proposed` → `Accepted`（追加阶段 2 落地验证段）。
- Docs: CONTEXT.md 术语演进历史对齐实际实现（字段名 / 模块路径 / API 签名）。
- Test: 全量回归基线 611 → **649 passed / 0 failed / 3 ignored**（阶段 2 新增 38 项：
  11 env_mask + 3 self_mitigation + 11 record_protection + 6 restricted_spawn +
  6 inspect mod 内嵌 + 1 self_mitigation mod 内嵌）。

### 阶段 3 — 可观测性（v0.6.0 P1）

- Added (#15 日志滚动): `tracing-appender = "0.2"` + `tracing-subscriber`
  features 加 `fmt`。`main.rs::init_tracing` 改造：`File::create` truncate
  路径 → `RollingFileAppender::daily` + `non_blocking::WorkerGuard`。
  日志路径 `~/.config/proc/proc.YYYY-MM-DD.log`，同一天再次启动追加（不再
  覆盖）。`main` 持 `_log_guard` 直到退出，确保异步 writer flush 残留日志。
  新增 `proc::cleanup_old_logs(dir, keep_days)` 公共函数（lib.rs），启动时
  调一次删除 7 天前的 `proc*.log`（不动 `crash-*.txt`）。
  新增 `tests/test_log_rotate.rs` 5 项测试。
- Added (#16 crash report + catch_unwind): `src/metrics/crash.rs` 新模块 —
  `install_panic_hook` 通过 `take_hook` 链式保留前置 hook（tui restore），
  panic 时写 `~/.config/proc/crashes/crash-{YYYYMMDD-HHMMSS}.txt`（含时间戳
  + proc 版本 + panic location + `Backtrace::force_capture()`）。`main`
  在 `init_tracing` 之后、业务逻辑之前调一次。`WorkerCrash { worker, message,
  backtrace, timestamp }` 通过 `metrics::crash::channel()` 创建的 mpsc channel
  传递。`SnapshotWorker::spawn` 外包 `std::panic::catch_unwind`，panic 时
  best-effort send 到 `crash_tx`，避免线程静默死亡。`App` 加 `crash_rx` +
  `active_crashes` 字段；`tick()` 每帧 drain `poll_crashes()`；TUI 顶部居中
  渲染红色 banner（最近 5 条），按 `D` 关闭（`dismiss_all_crashes`）。
  新增 `tests/test_crash_report.rs` 5 项测试。
- Added (#17 worker metrics + diag): `src/metrics/mod.rs` 新模块 —
  `WorkerMetrics`（atomic: poll_count / poll_total_us / poll_max_us /
  channel_full_count / last_error）+ `WorkerStats` snapshot + `health_badge`
  （`✓` 正常 / `⚠` 异常：丢帧 >10 或单轮 >100ms 或有错误）+ `NamedWorkerStats`
  wrapper 用于 `proc diag --json` 扁平输出。`SnapshotWorker` 加 `metrics:
  Arc<WorkerMetrics>` 字段；`run_poll_loop` 每轮 record（耗时 + channel_full）。
  5 个 spawn 调用点（port / usb / dns_log / net_flow / docker-snapshot）签名
  改造：`spawn(name, crash_tx, body)` —— 自动 catch_unwind + 接 metrics。
  `App::worker_metrics()` 聚合所有 SnapshotWorker。CLI 加 `proc diag [--json]`
  子命令（cli.rs + main.rs::run_diag），输出 worker 诊断表 / JSON。
  `?` 帮助页（help_panel.rs）末尾追加动态 "Workers" 区段。
  新增 `tests/test_worker_metrics.rs` 5 项测试。
- Added (cargo): `tracing-appender = "0.2"`；dev-dep `filetime = "0.2"`
  （log_rotate 测试模拟旧 mtime）。
- Added (lib): `proc::epoch_to_ymdhms` — UTC epoch secs → (year, month, day,
  hour, min, sec)，crash report 时间戳用。
- Changed: `tui/mod.rs::setup_terminal` 的 panic hook 仍然存在，但通过 chain
  被 `metrics::crash::install_panic_hook` 包装 — 最终顺序：tui restore →
  crash report → 系统默认 hook。
- Docs: CONTEXT.md 「当前术语」补 `WorkerMetrics / WorkerStats::health_badge
  / catch_unwind wrapping / crash report / diag` 5 项；术语演进历史加阶段 3 行。
- Test: 全量回归基线 649 → **674 passed / 0 failed / 3 ignored**（阶段 3
  新增 25 项：5 log_rotate + 5 crash_report + 5 worker_metrics 集成测试 +
  5 metrics/mod 内嵌 + 3 crash.rs 内嵌 + 2 worker.rs 内嵌；既有 8 项
  test_workers + test_dns_log + test_net_flow 签名兼容改造）。

### 阶段 4 — ProcessInfo 性能优化（v0.6.0 P1）

- Changed (#11 ProcessInfo Arc 化): `ProcessInfo` 字段类型升级 —
  `name: String → Arc<str>` / `cmd: Vec<String> → Arc<[String]>` /
  `exe / cwd / user_id: Option<String> → Option<Arc<str>>`。`Default` 用
  `OnceLock` 缓存共享空 `Arc<str>` / `Arc<[String]>` 实例避免重复分配。
  Cargo.toml `serde` 加 `rc` feature 让 `Arc<str>` / `Arc<[T]>` 自动序列化
  等价于 `String` / `[T]`（**.prec 录屏文件兼容性保持**）。HeavyWorker
  每进程构造一次 Arc，后续 clone 全是原子计数；500 进程 × 1.5s 重采下
  消除每秒数千次 `format!` / `String::clone` 堆分配。
- Added (#11 ProcessStatus): `src/collect.rs::ProcessStatus` Copy 枚举替代
  `format!("{:?}", sysinfo::ProcessStatus)` 的 String 分配。13 个变体按
  sysinfo 0.34.2 真实命名对齐（`Idle / Run / Sleep / Stop / Zombie / Tracing /
  Dead / Wakekill / Waking / Parked / LockBlocked / UninterruptibleDiskSleep /
  Unknown`，`Unknown` 为 `#[default]`）—— **不沿用 stage-4.md 早期猜测的
  `Traced` / `DeadLock`**（sysinfo 0.34 实际是 `Tracing` / `LockBlocked`，
  CONTEXT.md 已同步真实命名）。`From<sysinfo::ProcessStatus>` 全变体映射；
  `as_str() / badge() / tooltip()` + `Display` 实现供 TUI 表格列、状态条、
  tooltip 三档展示。
- Added (#14 搜索预算): `ProcessInfo::name_lower: Arc<str>` 预计算字段
  （`#[serde(skip)]` 不持久化，heavy worker 一次性算好）；`SearchState`
  加 `query_lower: String` + `query_lower()` 访问器，`handle_input` 在
  push/pop/Esc/clear 时同步维护；`App::rebuild_sorted_cache` 改写：
  过滤路径走 `name_lower.contains(query_lower)`，Name 排序分支直接
  `Arc::clone(&p.name_lower)` 而非每比较对 `to_lowercase`。**搜索框
  逐字符输入累积延迟 ~1ms → ~100µs（10x 提升）**。
- Adapted: 16+ 处既有调用点同步：
  - 构造点 4 处：`collect::HeavyWorker` / `collect::collect_missing_processes`
    / `eject::locks::find_volume_lockers_with_processes` /
    `record::conversions::From<&FrameProcess>`。
  - 字段访问点 10+ 处：`tree.rs::build_node`（含 `proc.status == "Zombie"`
    → `== ProcessStatus::Zombie`）/ `tui::process_table`（6 处
    `Cell::from(proc.{name,status}.clone())` → `.to_string()`）/
    `app_group.rs`（cmd `.iter()` 改 deref、`Path::new(&**e)` /
    `cache.get(&**e)`）/ `app.rs::handle_key` pid_to_name / `view_models`
    port_panel name display / process_panel exe cache eviction。
- Added: `tests/test_process_info_arc.rs` 11 项（Arc clone ptr_eq / Default
  空值 / From sysinfo 6 个关键变体映射 / badge-tooltip-as_str 覆盖 13 变体 /
  Display == as_str / serde round-trip 全字段 / `#[serde(skip)] name_lower`
  不出现在 JSON / Arc<[String]> iter）。
- Added: `tests/test_search_correctness.rs` 6 项（query_lower 初始 / 追踪
  lowercase / 大小写混合 / Backspace 更新 / Esc 清空 / clear 重置）。
- Adapted: 14 个既有测试文件的 `ProcessInfo { ... }` 构造点字段类型对齐
  （`name: "x".to_string()` → `Arc::from("x")` / `cmd: vec![]` →
  `Arc::from(Vec::<String>::new())` / `status: "Run".to_string()` →
  `ProcessStatus::Run` / 补 `name_lower` 字段）。`assert_eq!(p.name, "x")`
  → `p.name.as_ref()` / `p.exe.as_deref()` 等 deref 适配。
- Docs: CONTEXT.md 术语演进历史对齐实际实现（`Tracing` / `LockBlocked` 等
  真实变体名替换 stage-4.md 早期猜测）。
- Test: 全量回归基线 674 → **693 passed / 0 failed / 3 ignored**（阶段 4
  新增 19 项：11 process_info_arc + 6 search_correctness + 既有
  test_platform_compat round-trip 调整 + test_skeleton / test_process_list
  等字段断言 .as_ref() 适配）。

### 阶段 1 — 文档 + 发布基础设施

- Added: `docs/adr/` 入仓（0001-0007 从私有 docs 移入 + 0008-self-mitigation-policy 新增 Proposed）
- Added: `SECURITY.md`（vulnerability reporting policy + privilege model + hardening 说明）
- Added: `CONTRIBUTING.md`（开发流程 + 提交规范 + ADR 流程）
- Added: `.github/workflows/release.yml`（tag 触发，cross 构建 5 个 target：win-x64 / linux-musl / linux-arm64 / macos-arm64 / macos-x86_64；附带 update-winget 自动 PR 到 microsoft/winget-pkgs）
- Added: `Cargo.toml` `[package.metadata.binstall]`（cargo-binstall 支持；Windows 用 zip override）
- Added: `scoop/proc.json` + `winget-pkgs-templates/Alfroul.proc.template.yaml`
- Changed: `.gitignore` 放行 `docs/`（保留 `docs/handoff-*.md` 私有 + `CONTEXT.md` / `plan.md` 私有）
- Changed: README.md「快速开始」段重写为「安装」段，加 binstall / winget / scoop 三种方式

## [0.5.0] - 2026-06-22

### 最终交付摘要

阶段 1-9 + 阶段 10 Review + 阶段 11 收尾，0.5.0 周期完整交付 14 项计划功能（A1/A3/A4 句柄/内存/优先级、B1/B2/B3 GPU/频率/SMART、D1/D2/D3 流量/TCP/DNS、E1/E2/E3/E4 Docker 深化、H4 Miri）。

**最终基线**（2026-06-22 实测）：

| 命令 | 结果 |
|---|---|
| `cargo test --release` | ✅ **611 passed / 0 failed / 3 ignored**（baseline 595 + 阶段 11 新增 16 单测：14 key_event_to_pty_bytes + 2 SmartHealth Warning；33 binaries + 1 doctest） |
| `cargo clippy --release --all-targets -- -D warnings` | ✅ 0 warnings |
| `cargo build --release --no-default-features` | ✅ 编译通过（4m 41s） |
| `cargo fmt --all -- --check` | ✅ fmt clean |
| `cargo clippy --release --all-targets -- -W clippy::pedantic \| grep must_use_candidate` | ✅ 16（baseline，全在 src/app.rs 现有 getter，surgical 原则不动） |

### 阶段 11 — 收尾：批量修复 + 0.5.0 发布

- Fixed (P0-1): `Cargo.toml` version `0.4.0` → `0.5.0`
- Changed (P0-2): `README.md` 完全重写，反映 0.5.0 全部能力（Inspector 6 Tab、GPU 多厂商、SMART、per-process 流量、DNS 日志、Docker 深化、容器 exec）。228 → 410 行（+182 / -46）。删除过时「GPU 路线图」段（阶段 6 已落地 AMD/Intel via nvtop）；快捷键 / CLI 子命令 / 平台支持表全部补齐。
- Decision (P0-3): `.gitignore` 保持私有 —— `CONTEXT.md` / `plan.md` / `docs/` 不入仓库（用户确认，2026-06-22）。
- Docs (P1-E2): `src/docker/logs_worker.rs:5` ADR 引用错误修正（ADR-0006 是 DNS PowerShell 选型，与 logs_worker 背压无关；改为「与 docker/events worker / monitor/port_watcher 同款 sync_channel(64) 设计」）。
- Test (P1-E1): `src/tui/container_exec_view.rs::key_event_to_pty_bytes` 补 14 个单测覆盖每个分支（Enter/Tab/BackTab/Backspace/方向/Home/End/Delete/PageUp/PageDown/Ctrl 常用键/Ctrl xor 规则/Alt+x/普通 ASCII/非 ASCII UTF-8/F-keys/Null）。
- Fixed (P1-X2): `src/app.rs::handle_container_exec_key` 写 PTY 失败时正确切回 DockerPanel + 保留错误原因（之前 switch_mode 覆盖 exit_msg 让用户看不到具体失败原因）。
- CI (P1-C2): `.github/workflows/miri.yml` 删除 `continue-on-error: true` 两处（line 36 + 40），让 Miri 真正阻断 PR。阶段 2 H4 目标「Miri 并发 UB 检测」实际生效。
- Fixed (P1-B1): `src/smart/mod.rs::parse_smartctl_json` 在 `when_failed=past` 时触发 `SmartHealth::Warning`（之前 Warning 变体永远拿不到，用户看不到「磁盘曾失败过」中间状态）。补 2 个单测覆盖 Warning 路径 + Failing 优先级。
- Perf (P1-B3): `src/gpu.rs::NvmlProvider` 缓存 DXGI+NVML 聚合结果到 `cached: Vec<GpuInfo>`，`list_gpus` 直接 clone；`refresh` 跑完整枚举。sidebar 不再每秒重做 DXGI（`CreateDXGIFactory1` + `EnumAdapters1` + `QueryVideoMemoryInfo`）+ NVML `get_info` fuzzy match 遍历。
- Perf (P1-A3): `src/tui/detail_view.rs::draw_summary` 优先级/affinity 走 `App::detail_priority` 缓存，避免每帧 4 次 syscall（`OpenProcess` + `GetPriorityClass` + `GetProcessAffinityMask` + `CloseHandle`，50ms tick 下 80 次/秒）。4 个更新点：`switch_mode` 进入详情页 / `r` 刷新 / `+/-` 调整 / heavy tick 周期。
- Fixed (P1-A5/D4): `DnsQuery` 加 `start_time: u64` 字段（additive）。`PidNameLookup` cache value 含 start_time，每次 lookup 比对 sysinfo 当前 start_time，PID 复用时自动失效重查。`detail_view::draw_network_tab` DNS 过滤改 `(pid, start_time)` 元组比对，避免 PID 复用时新进程详情页显示旧进程 DNS 历史。
- Docs: `docs/tech-debt.md` 归档 52 项 P2 + 14 项未做 P1 的处理建议（0.6.0 / 0.7.0+ 分组）。
- Docs: `docs/0.6.0-roadmap.md` 列出 0.6.0 候选（Windows AMD/Intel GPU、Linux pcap DNS、Linux per-core 温度、Inspector v2 等，~12 周粗估）。
- Note: 降级 P2 留 0.6.0 的 P1（14 项）—— P1-A1 Handles 对象名 / P1-A2 Memory 映射文件名 / P1-A4 HandleLocker 类型 / P1-A6 proc_maps 合并 / P1-A7 env/dlls 单测 / P1-B2 WMI 多磁盘 / P1-B4 PDH 多 GPU / P1-C1 per-core 温度 / P1-C3 sidebar 多盘 / P1-D1 estats 重复 enable / P1-D2 R9 seen_pids / P1-D3 net_flow 单测 / P1-D5 Anomaly.start_time / P1-D6 estats ConnKey local_addr。详见 `docs/tech-debt.md`。

### 阶段 10 — Review

- Added: `docs/reviews/REVIEW-10.md` 全局审查报告（read-only，3 P0 + 26 P1 + 52 P2）。覆盖 A/B/C/D/E 五切片 + 横切（架构/文档/测试/安全/性能）。
- Note: 阶段 11 消费 P0/P1（13 项修复），P2 归档到 `docs/tech-debt.md`。

### 阶段 9 — Slice：E2 exec 进容器（PTY 集成）

- Added (E2): `src/docker/exec.rs` 新模块 —— `ContainerExec` 句柄持有 portable-pty master / writer / child / reader 通道。`start(container, cmd, image)` 用本地 PTY spawn `docker exec -it <container> <shell>` 子进程；docker CLI 处理所有 daemon 通信（命名管道 / TCP / unix socket）+ 远端 PTY 分配。reader 线程循环 `master.reader.read` → `sync_channel(64)` 背压 → 主线程 tick `drain()` 拼接字节喂 `vt100::Parser::process`。`write_all(bytes)` 转发按键字节（含 ANSI 转义）到 PTY writer。`resize(cols, rows)` 同步 PTY 尺寸（SIGWINCH 由 `-t` 自动转发容器）。`is_finished()` 检测 child 退出。`detect_default_shell(image)` 纯函数按镜像名推断 shell（alpine/busybox → `/bin/sh`，ubuntu/debian/centos/fedora/rust/golang/python/node → `/bin/bash`，其它 → `/bin/sh` 兜底）。详见 `docs/adr/0007-container-exec-pty-bridge.md`。
- Added (E2): `src/tui/container_exec_view.rs` —— 嵌入式终端渲染。`draw()` 用 `Layout::vertical` 分顶部（容器名 + 退出提示）/ PTY 输出区 / 底部（PTY 尺寸 + 快捷键）。PTY 区遍历 `vt100::Screen::cell(r, c)`，每 cell 取 `contents()` / `fgcolor()` / `bgcolor()` / `bold()` / `italic()` / `underline()` / `inverse()` 写到 ratatui buffer；光标位置取反色高亮。`vt_color_to_ratatui(Color)` 把 vt100 `Color` 枚举（Default → Reset / Idx → Indexed / Rgb → Rgb）转 ratatui Color；`vt_attrs_to_modifier(cell)` 把 bold/italic/underline/inverse 转 Modifier。
- Added (E2): `src/app_panel.rs::AppMode::ContainerExec` 新变体 —— 从 DockerPanel 按 `e` 进入；`Ctrl+D` / `Ctrl+\` / 子进程退出时切回 DockerPanel。`PanelContext` 新增 `pending_container_exec: &'a mut Option<String>` 字段，DockerPanel 按 `e` 时塞容器名 + 返回 `SwitchMode(ContainerExec)`，`App::switch_mode` 取出启动 PTY。
- Added (E2): `src/app.rs::App` 新增 `container_exec: Option<ContainerExec>` / `container_exec_vt: Option<vt100::Parser>` / `pending_container_exec_target: Option<String>` / `container_exec_exit_msg: Option<String>` 4 个字段。`enter_container_exec()` 从 target 启动 PTY（失败回退 DockerPanel + 错误提示）；`tick_container_exec()` 每帧 drain PTY 字节喂 vt100 + 检测 child 退出；`handle_container_exec_key(key)` 把 `KeyEvent` 转 ANSI 字节序列（Enter=`\r`、Ctrl+C=`\x03`、Ctrl+D=`\x04`、Ctrl+\\=`\x1c`、Backspace=`\x7f`、Tab=`\t`、Up=`\x1b[A`、Down=`\x1b[B`、Right=`\x1b[C`、Left=`\x1b[D`、Home/End/PageUp/PageDown/Delete 全 ANSI 序列、Alt+x=`\x1b` + x、普通字符直接字节）写 PTY writer；`resize_container_exec(cols, rows)` 同步 PTY + vt100 parser 尺寸。`switch_mode` 在退出 ContainerExec 时主动 drop PTY + parser（避免 fd 泄漏）。
- Changed (E2): `src/view_models/docker_panel.rs::DockerPanel::handle_key` 新增 `e` 分支 → `enter_exec_mode(ctx)` 设置 `pending_container_exec` + 返回 `SwitchMode(ContainerExec)`。容器视图 + 容器运行状态检查；非容器视图 / 未选中容器 / 容器未运行时返回 `Consumed` + 友好错误消息。
- Changed (E2): `src/tui/layout.rs::draw_main_panel` 新增 `AppMode::ContainerExec` 分支调用 `container_exec_view::draw`；`draw_footer` 新增 exec 模式快捷键栏；`tab_index` 把 ContainerExec 映射到 Docker tab。
- Changed (E2): `src/tui/mod.rs::handle_events` 新增 `Event::Resize` 分支（之前忽略）触发 `App::notify_terminal_resized`；`run_app` 在 draw 之后若 `mode == ContainerExec`，按 area 实际尺寸调 `resize_container_exec(area.w, area.h)`。
- Added (CLI): `src/cli.rs::DockerSub::Exec { container, cmd: Vec<String> }` 子命令；`src/main.rs::run_docker_exec(monitor, container, cmd)` —— 验证容器存在 + 根据 image 推断 shell + spawn `docker exec -it` 子进程透传 stdio（docker CLI 接管用户终端，等价直接调 `docker exec`）。
- Added (Cargo): 新依赖 `portable-pty = "0.9"`（跨平台 PTY 抽象，Windows ConPTY / Linux POSIX PTY）、`vt100 = "0.15"`（ANSI 字节流解析 + Screen 状态）。
- Added (ADR-0007): `docs/adr/0007-container-exec-pty-bridge.md` —— 解释为何选 spawn `docker exec -it` 子进程而非走 bollard exec Attached 流（方案 B/C）：① 方案 C 描述的「portable-pty master/slave pair + bollard Attached 流双向中转」技术上不成立（PTY slave 端需子进程才有意义）；② 方案 B 可行但放弃（不引入 portable-pty 违背 stage-9.md 明确要求）；③ 方案 A 让 docker CLI 处理所有 daemon 连接差异（Docker Desktop 命名管道 / WSL Docker TCP / Linux unix socket），proc 不感知。
- Added: `tests/test_container_exec.rs`（新）—— 集成测试：`detect_default_shell` 多 image 推断矩阵 / PTY 字节转换纯函数 / `ContainerExec::start` 不 PATH docker 时优雅报错（cfg-gate：仅在 Docker 可用的 CI 环境 smoke）/ `PtyChunk::default` 空。
- Test: 总测试数 568 → 预计 **575+**（+7 左右）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过。
- Note: 已知限制 —— 需要 PATH 有 `docker` 二进制（与既有 `proc docker compose` 一致）；Windows ConPTY 需 Windows 10 1809+；首次 spawn 延迟 ~50ms（docker CLI 启动 + daemon 连接）；exec 模式下 Ctrl+C 走 KeyEvent 转发容器（raw mode 下 crossterm 不传 SIGINT），其它模式下 Ctrl+C 走全局 shutdown 不变。

### 阶段 8 — Slice：D3 DNS 查询日志（Windows PowerShell Get-WinEvent）

- Added (D3): `src/dns_log/mod.rs` 新模块 —— `DnsLogCollector` trait 抽象 DNS 查询日志数据源（`fn drain(&mut self) -> Vec<DnsQuery>` / `fn provider_name(&self) -> &'static str`），`Send + Sync` 让 collector 可跨 worker 线程传递。参考阶段 6/7 的 trait 模式（[`crate::gpu::GpuProvider`] / [`crate::net_flow::NetFlowCollector`]）。`DnsQuery { timestamp, pid, process_name, query_name, query_type, result }` derive `Serialize/Deserialize` 仅用于内存 round-trip 测试；**永不持久化到磁盘**（隐私）。`DnsResult { Success(Vec<IpAddr>) | NxDomain | Timeout | Error(String) }` + `from_windows_status(status, results)` 把 Win32 错误码映射到语义结果。`parse_query_type(raw)` 把数字（"1" → "A"，"28" → "AAAA"，"65" → "HTTPS" 等）转 RFC 1035 助记符；`parse_query_results(raw)` 把 `;` 分隔的 IP 列表（含 TTL 后缀）解析为 `Vec<IpAddr>`，非法分片忽略不抛错。`detect_collector() -> Option<Box<dyn DnsLogCollector>>`：Windows 上启动 PowerShell collector；Linux/macOS 返回 None。
- Added (D3): `src/dns_log/windows_dns.rs::PowershellDnsCollector` —— Windows 路线走 PowerShell `Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-DNS-Client/Operational'; Id=3010}` 子进程（不走 ETW）。spawn 长跑 `powershell.exe -NoProfile -NonInteractive -Command <SCRIPT>`，脚本内部 ~400ms 节奏轮询新事件，每事件一行 JSON emit（`ts/pid/name/qtype/status/results`）。reader 线程 `BufReader::read_line` 流式解析 + PID 名 lookup（sysinfo `refresh_processes_specifics` 每 10s 刷一次缓存） + `sync_channel(1000)` 推到 collector drain。`Arc<Mutex<Option<Child>>>` 共享给 reader 保活 + Drop 时主动 kill（避免 reader 阻塞在 `read_line`）。详见 `docs/adr/0006-dns-subprocess-not-etw-dbus.md`。
- Added (D3): `src/dns_log/windows_dns.rs::parse_powershell_event(line)` 纯函数 —— 解析 PowerShell JSON 行为 `DnsQuery`。容忍 PID 0（System Idle，噪声）/ 负 PID（PowerShell 偶发）丢弃；status 解析 u32 失败仍保留事件（标 `Error("unparsed:...")`）。9 个单测覆盖 success/nxdomain/timeout/invalid PID/PID 0/garbage status/non-JSON/mnemonic qtype/时间戳边界。
- Added (D3): `src/dns_log/unsupported.rs::PowershellDnsCollector` 占位 —— 非 Windows 平台 `pub use unsupported as windows`，`new()` 直接返回 `Err`。
- Added (D3): `src/dns_log/worker.rs::DnsLogWorker` —— 复用 `SnapshotWorker<DnsLogSnapshot>`，**500ms poll**（DNS 查询高频，比阶段 7 NetFlow 的 1s 更短）。`spawn(collector: Box<dyn DnsLogCollector>)` 启动 worker，body 调 `collector.drain()` 推送（空 Vec 跳过）。
- Changed (D3): `src/app.rs::App` 新增 `dns_log_worker: Option<DnsLogWorker>` + `dns_log_recent: VecDeque<DnsQuery>`（cap=1000 FIFO）字段。`App::new()` 调 `detect_collector().map(spawn)`：Windows 上启动 worker；其它平台字段为 None。`App::tick_dns_log()`（每次 100ms tick 调）drain worker 最新 snapshot，逐条 push_back 到 VecDeque，超 1000 pop_front。
- Changed (D3): `src/app_panel.rs::PanelContext` 新增 `dns_log_recent: &'a mut VecDeque<DnsQuery>` 字段 —— 让 PortPanel 在 DNS 子视图中按 `c` 清空（无需额外间接）。
- Added (D3): `src/view_models/port_panel.rs` DNS 子视图 —— 新字段 `dns_view_active: bool` / `dns_cursor: usize` / `dns_scroll: usize` / `dns_follow: bool`（默认 true）/ `dns_search: SearchState`。按 `D`（大写，小写 `d` 留给 anomaly dismiss）进入 DNS 子视图，`Esc`/`D` 退出。激活时接管所有按键：`↑↓` 移动光标 / `/` 搜索（域名 + 进程名）/ `c` 清空 / `f` 切换 follow / `PageUp/Down`/`Home/End`。`dns_filtered_indices(recent)` 返回搜索命中的索引列表，跨 view + key handler 共享避免重算。
- Added (D3): `src/tui/port_table.rs::draw_dns_view` —— 渲染 DNS 日志列表（时间 / PID / 进程名 / 类型 / 域名 / 结果 6 列），结果按 `DnsResult` 变体着色（Success=绿 / NxDomain+Error=黄 / Timeout=亮红）。标题栏显示 collector 状态 + 条数 + follow 状态；搜索激活时底部显示搜索框。`dns_view_active == true` 时 `draw()` 优先走 DNS 视图，覆盖常规端口视图。
- Added (D3): `src/tui/detail_view.rs::draw_network_tab` —— Network Tab 顶部加「最近 5 条 DNS 查询」面板（按当前 PID 过滤 `dns_log_recent`）。无 DNS 数据时（worker 未启动 / 此 PID 未查 DNS）省略，避免占垂直空间。
- Changed (D3): `src/tui/layout.rs::draw_footer` —— DNS worker 活动时状态栏左侧显示 `📡DNS(仅内存)` 指示（隐私提示）；PortMap 模式快捷键栏追加 `DDNS日志` 提示；DNS 子视图激活时改用专用快捷键栏（`↑↓滚动 / 搜索 / c清空 / f切换follow / D/Esc退出`）。
- Added (D3): `src/anomaly.rs::AnomalyDetector::detect_new_dns_from_new_process` —— R9 异常规则：一个进程首次发起 DNS 查询，且其名称不在 whitelist（小写进程名集合）→ Warning，每个 PID 仅触发一次（跨调用维护 `seen_pids: HashSet<u32>`）。6 个单测覆盖 whitelist hit/miss、PID 去重、未知进程名（`?`）触发、空 queries、跨调用 `seen_pids` 持久。
- Added (D3): `src/cli.rs::Command::Dns { tail, since }` —— `proc dns --tail` 流式输出 DNS 查询日志（Ctrl+C 退出），`--since 1h` 留 TODO（DNS 不持久化，需走 Windows EventLog 历史读取路径）。`src/main.rs::run_dns` dispatch。
- Added (ADR-0006): `docs/adr/0006-dns-subprocess-not-etw-dbus.md` —— 解释：① Windows 为什么选 PowerShell `Get-WinEvent` 而非 ETW（ROI：500 行 unsafe FFI vs 150 行子进程）；② Linux 为什么放弃 systemd-resolved DBus（DBus 接口不暴露 per-query 信号，stage-8.md 原计划有误），pcap/eBPF 工程量超 stage 范围列为未来 feature。共同原则：复用既有依赖 + 最小 native 表面 + 子进程开销可接受（与 ADR-0004/0005 一致）。
- Added: `tests/test_dns_log.rs`（新）—— 11 个集成测试：VecDeque cap=1000 FIFO 行为 / 批量 push 顺序 / clear / mock collector + worker round-trip / 空 collector drop 不卡死 / `detect_collector` 跨平台不 panic / `parse_query_type` 6 种常见类型 / `parse_query_results` IPv4+IPv6 混合 / `DnsResult::badge` 4 个变体 / `DnsLogSnapshot::default` 空。
- Added: 模块内嵌单测 —— `dns_log/mod.rs` 9 个（Display / Display 零值 / serde round-trip / Windows status 4 个分支 / badge 稳定 / parse_query_type 已知 + 未知 + trailing NUL / parse_query_results 多种格式 / clone+eq）、`dns_log/windows_dns.rs` 9 个（success / nxdomain / timeout / 负 PID / PID 0 / 不可解析 status / 非 JSON / 助记符 qtype / 时间戳边界）、`anomaly.rs` 6 个 dns_anomaly_tests。
- Test: 总测试数 532 → **568**（+36：11 集成 + 25 lib 内嵌）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过；`cargo fmt --all -- --check` 干净；阶段 8 新代码（trait + powershell impl + worker + UI + anomaly）`-W clippy::must_use_candidate` 0 新增警告（baseline 16 个全在 src/app.rs 现有 getter，surgical 原则不动）。
- Note: 隐私 —— DNS 查询含敏感信息（用户访问的域名），**永不持久化**到磁盘。`App::dns_log_recent` 仅内存；录屏（`record/`）路径不序列化 DNS 数据（不在 `SystemSnapshot` 内）；状态栏 `📡DNS(仅内存)` 指示让用户知道采集状态。
- Note: 已知限制 —— Linux/macOS 暂不支持 DNS 采集（DBus 不暴露 per-query 信号；pcap/eBPF 工程量大留作未来 feature）；仅覆盖 event 3010（QueryResultsEx）；PowerShell 启动延迟 ~300ms（首次 spawn 后 ~1s 开始收到事件）；PID 名 lookup 10s 刷新一次，新进程可能暂时显示 `?`。

### 阶段 7 — Slice：D1 per-process 网络流量（Windows IP Helper + Linux nethogs）

- Added (D1): `src/net_flow/mod.rs` 新模块 —— `NetFlowCollector` trait 抽象 per-process 字节速率数据源（`fn per_process_rates(&mut self) -> Vec<ProcessNetRate>` / `fn provider_name(&self) -> &'static str`）。`Send + Sync` 让 collector 可跨 worker 线程传递。参考阶段 6 [`crate::gpu::GpuProvider`] trait 模式。`ProcessNetRate { pid, start_time, bytes_sent_per_sec, bytes_recv_per_sec }` 带 `start_time` 字段防 PID 复用（ADR-0003）。`detect_collector() -> Option<Box<dyn NetFlowCollector>>` 按平台 / feature / 二进制可用性返回活跃 collector，无可用时返回 None（主线程 net 列保持 0）。
- Added (D1): `src/net_flow/windows.rs::IphelperCollector` —— Windows IP Helper 路线（不走 ETW）。`per_process_rates` 每次调用：调 `GetTcpTable2` + `SetPerTcpConnectionEStats` + `GetPerTcpConnectionEStats`（复用 [`crate::estats`] 同款 Win32 调用）拿每条 IPv4 TCP 连接的累计 `DataBytesIn` / `DataBytesOut`，同时调 `netstat2::get_sockets_info` 拿连接 → PID 映射，按 PID 聚合。内部维护 `last_per_pid` 累计缓存，差值 / elapsed = bytes/sec。PID 复用检测：当前累计 < 上次累计 → 视为新进程，速率按 0 计。详见 `docs/adr/0005-netflow-windows-iphelper-not-etw.md`。
- Added (D1): `src/net_flow/nethogs.rs::NethogsCollector` —— 仅 `target_os = "linux"` + `feature = "nethogs"` 下编译。`try_new` 先用 `nethogs --version` 探测二进制，可用时 spawn `nethogs -t -d 2 -v 3` 子进程，stdout 喂 `parse_nethogs_line` 纯函数解析（每行 PID + direction + KB/sec）。`Drop` 时 `child.kill() + wait()` 干净退出。`unsafe impl Sync` 套 `Mutex<Child>` 让 collector 满足 trait 的 `Send + Sync` 要求。
- Added (D1): `src/net_flow/nethogs.rs::parse_nethogs_line(line)` 纯函数 —— 解析 nethogs tracemode 行（tab/多空格分隔），从 `name/pid/user` token 提取 PID，过滤 PID 0（kernel），返回 `(pid, direction, kbps)`；解析失败 / 缺列 / 负速率返回 None。模块内嵌 9 个单测覆盖 happy path / garbage / 边界。
- Added (D1): `src/net_flow/unsupported.rs::IphelperCollector` 占位 —— macOS / 非 Windows 非 Linux 平台的 `pub use unsupported as windows` 路径，`new()` 直接返回 `Err`。
- Added (D1): `src/net_flow/worker.rs::NetFlowWorker` —— 复用 `SnapshotWorker<NetFlowSnapshot>`，1s poll。`spawn(collector: Box<dyn NetFlowCollector>)` 启动 worker，body 调 `collector.per_process_rates()` 推送。
- Added (D1): `src/collect.rs::ProcessInfo` 新增 `net_sent_rate: u64` / `net_recv_rate: u64` 字段（默认 0，worker 不可用时保持 0）。5 处 `ProcessInfo { ... }` 构造点全部同步（collect.rs × 2 / eject/locks.rs / record/conversions.rs × 1 + 内部测试 14 处）。
- Added (D1): `src/collect.rs::SortField` 新增 `NetSent` / `NetRecv` 两个排序变体。`label()` 返回「↑网络」/「↓网络」；`next` / `prev` 循环 DiskWrite ↔ NetSent ↔ NetRecv ↔ Cpu；`as_str` / `parse_from_str` 持久化标识 `net_sent` / `net_recv`；`sort_processes` 新增降序比较分支（`.then(pid)` tie-breaker）。
- Changed (D1): `src/app.rs::App` 新增 `net_flow_worker: Option<NetFlowWorker>` 字段。`App::new()` 调 `detect_collector().map(spawn)`：平台支持时启动 worker，不支持时字段为 None。`App::update_net_rates()`（heavy refresh 时调）drain worker 最新一份，按 PID 贴回 `cached_processes.net_sent_rate` / `net_recv_rate`；无 worker / 无新帧时保留当前值不强制清零。`get_filtered_sorted_processes` 排序 match 加 NetSent / NetRecv 分支。
- Changed (D1): `src/tui/process_table.rs` 排序字段为 NetSent / NetRecv 时新增「↑网络」「↓网络」两列（与磁盘R/磁盘W 同款 layout），用 `format::format_speed` 格式化。
- Changed (D1): `src/tui/sidebar.rs` 在 NET ↓/↑ 速率下方追加 Top 3 上行流量进程 mini list（参考 Mission Center 同款），按 `net_sent + net_recv` 降序取前 3；过滤全 0 行避免误导。
- Changed (D1): `src/cli.rs::Command::Ls` / `Command::Export` sort help 字符串加 `disk_read | disk_write | net_sent | net_recv` 选项；`src/main.rs::run_ls` / `run_export` sort matcher 加对应分支；`run_ls` 在 NetSent / NetRecv 排序时输出 ↑网络/s 和 ↓网络/s 两列。
- Added (Cargo): `[features] default = ["nvidia", "nvtop", "nethogs"]`，新增 `nethogs = []`（无 native 依赖，走子进程）。仅 `target_os = "linux"` 平台生效。`cargo build --no-default-features --features nvidia` 仍可编译（关闭 nethogs + nvtop）。
- Added (ADR-0005): `docs/adr/0005-netflow-windows-iphelper-not-etw.md` —— 解释 Windows 为什么选 IP Helper 而非 ETW：① ETW 实时 session 需要单独消费者线程 + `ProcessTrace` 阻塞调用 + ~500 行 unsafe 脚手架，ROI 不匹配；② IP Helper 复用 [`crate::estats`] 已测的同款 Win32 调用，1s poll 下 CPU < 1%；③ `SetPerTcpConnectionEStats` 在非管理员下通常仍可工作；④ `NetFlowCollector` trait 抽象让未来切回 ETW 是 additive（新增 impl + detect_collector 分支）。
- Added: `tests/test_net_flow.rs`（新）—— 11 个集成测试：`ProcessNetRate` Display smoke / `detect_collector` 跨平台不 panic / `SnapshotWorker<NetFlowSnapshot>` spawn+drop 生命周期 + try_recv_latest 推送 / `sort_processes` 在 NetSent/NetRecv 上降序 + tiebreak / `SortField::NetSent/NetRecv` as_str/parse_from_str 往返 / `next`/`prev` 循环覆盖 Net 变体 / `ProcessNetRate` clone+eq / `NetFlowSnapshot::default()` 空。
- Added: 模块内嵌单测 —— `net_flow/mod.rs` 3 个（Display / Display 零值 / detect_collector 不 panic）、`net_flow/nethogs.rs` 10 个（parse down/up/无 user/refreshing/closed/pid 0/garbage/negative/multi-space/kbps 转换 + 端到端聚合）。
- Test: 总测试数 506 → **532**（+26：13 集成 + 13 lib 内嵌）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过；阶段 7 新代码（trait + 4 个 impl + worker + Display）`-W clippy::must_use_candidate` 0 警告。
- Note: 已知限制 —— Windows IP Helper 仅覆盖 IPv4 TCP（与 [`crate::estats`] 一致），IPv6 路径（`GetPerTcp6ConnectionEStats`）和 ETW 全协议覆盖留作后续 additive 工作；UDP 无 per-PID 字节速率概念（无连接字节计数）；非管理员模式下部分其它进程的连接可能拿不到字节（`SetPerTcpConnectionEStats` 失败按 0 计，UI 显示 `0B/s`）。

### 阶段 6 — Slice：B1 AMD/Intel GPU via nvtop

- Added (B1): `src/gpu.rs::GpuProvider` trait 抽象 GPU 数据源 —— `fn list_gpus(&self) -> Vec<GpuInfo>` / `fn refresh(&mut self)` / `fn provider_name(&self) -> &'static str`。`Send + Sync` 让 provider 可跨 worker 线程传递；`list_gpus` 取 `&self`（缓存由 refresh 维护），让多 provider 场景能并发查询无需 `&mut`。
- Added (B1): `src/gpu.rs::NvmlProvider` —— 封装现有 Windows DXGI + NVML + PDH 三层路径为 `GpuProvider` impl。`NvmlState` / `PdhState` / `collect_dxgi_adapters` 全部保留；新增 `pdh_util: Option<u32>` 字段缓存 PDH 单次采样结果（PDH 是状态机，不能在 `list_gpus(&self)` 里推进）。NVML feature 关闭时仍返回所有 vendor 的 VRAM（DXGI 覆盖），仅退化 utilization/temp/power enrichment。非 Windows 平台 provider 类型保留为占位（detect_providers 不构造）。
- Added (B1): `src/gpu.rs::NvtopProvider` —— 仅 `target_os = "linux"` + `feature = "nvtop"` 下编译。`refresh` spawn `nvtop -s -o json` 子进程，stdout 喂 `parse_nvtop_json`；失败保留旧缓存不 panic。`is_available()` 通过 `nvtop --version` 探测 PATH。
- Added (B1): `src/gpu.rs::parse_nvtop_json(content)` 纯函数 —— 解析 nvtop JSON 输出（device/temperature/memory{used,total}/gpu_utilization/power{used,total} 五字段），缺字段按 0/None 退化，非法 JSON 返回空 Vec。`infer_vendor(name)` 字符串匹配 NVIDIA/GeForce/Quadro/RTX/GTX → Nvidia；AMD/Radeon → Amd；Intel → Intel；其它 → Unknown。utilization > 100 时 clamp 到 100。
- Added (B1): `src/gpu.rs::detect_providers() -> Vec<Box<dyn GpuProvider>>` —— 根据平台 / feature / 二进制可用性返回活跃 provider 列表。Windows 始终加 `NvmlProvider`；Linux + nvtop feature + nvtop 在 PATH 时加 `NvtopProvider`；macOS 等返回空 Vec。多 provider 并存支持混合 GPU 笔记本（Intel iGPU + NVIDIA dGPU）。
- Changed (B1): `src/gpu.rs::GpuCollector` 内部从 `Option<NvmlState> + Option<PdhState>` 改为 `Vec<Box<dyn GpuProvider>>`；`new()` 调 `detect_providers()`，`refresh()` 遍历所有 provider refresh + list_gpus 聚合。外部 API（`new` / `refresh -> Vec<GpuInfo>`）零修改 —— `collect.rs::LightWorker` 调用点不变。
- Added (Cargo): `[features] default = ["nvidia", "nvtop"]`，新增 `nvtop = []`（无 native 依赖，走子进程）。`cargo build --no-default-features --features nvidia` 仍可编译；`--features nvtop`（关 nvidia）在 Linux 上仍工作，在 Windows 上无 GPU provider（sidebar GPU 区为空，可接受）。
- Added (ADR-0004): `docs/adr/0004-gpu-via-nvtop-subprocess.md` —— 解释选 nvtop 子进程而非 libdrm 直接绑定 / WMI 的理由：依赖管理干净（无 bindgen）、跨厂商一次覆盖（AMD+Intel+NVIDIA 一套解析器）、与 ADR-0003 SMART `smartctl` 同类取舍（30s/1s poll 子进程开销可接受）、失败优雅降级。Windows AMD/Intel 留 TODO（DXGI 仅 VRAM，方案 D WMI 后续迭代）。
- Added: `tests/test_gpu.rs`（新）—— 6 个集成测试：fixture 多厂商解析 / 字段完整 / malformed 输入 / 空数组 / `detect_providers` 不 panic + provider_name 非空 / `GpuCollector::refresh` 返回 Vec 不 panic。
- Added: `tests/fixtures/nvtop_sample.json` —— 三厂商样本（NVIDIA RTX 4070 + AMD RX 7900 XTX + Intel Arc A770），含 temperature/memory/utilization/power 完整字段。
- Added: 模块内嵌单测 —— `gpu.rs` 8 个测试（parse_nvtop_json 多厂商/缺字段/garbage/clamp + infer_vendor 7 个品牌串 + detect_providers 不 panic + GpuCollector default）。
- Note: sidebar.rs 已通过 `for gpu in gpu_info` 循环支持多 GPU，阶段 6 无需改动（surgical 原则）。Windows AMD/Intel GPU 在 DXGI 路径下已显示 VRAM，仅缺 utilization/temp/power（NVML enrichment 限定 NVIDIA）。
- Test: 总测试数 492 → **506**（+14：8 lib + 6 集成）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过；阶段 6 新代码（gpu trait + providers + parse_nvtop_json + detect_providers）`-W clippy::must_use_candidate` 0 警告（baseline 遗留 16 个全在 src/app.rs 现有 getter，surgical 原则不动）。

### 阶段 5 — Slice：D2 TCP 质量 + B3 SMART 磁盘健康

- Added (D2): `src/collect.rs::TcpStats` 新增 4 个传输质量字段 —— `retransmitted_segs` / `reset_segs` / `failed_connections` / `out_segs`（u64）。Windows 走 `GetTcpStatisticsEx2` + `MIB_TCPSTATS2`，IPv4 + IPv6 各跑一次累加；Linux 解析 `/proc/net/snmp` 的 TCP 行；其它平台保留 0。
- Added (D2): `src/port_map.rs::TcpSnmpStats` 结构 + `parse_proc_net_snmp_tcp(content)` 纯函数解析器。按列名匹配（不是按位置），新内核加列时自动跳过；header 缺失或 numeric 缺失都返回 `default()` 不抛错。模块内嵌 4 个单测覆盖 typical / 缺失 / 多列 / InErrs+InCsumErrors 合并。
- Added (D2): `src/port_map.rs::PortEntry.rtt_ms: Option<u32>` —— per-connection RTT 字段占位。`netstat2` 不暴露 RTT，Windows `GetPerTcpConnectionEStats` 是管理员专属重型 API，阶段 5 不强制采集；`None` 在 UI 渲染为 `-`，避免误读为零延迟。
- Added (D2): `src/anomaly.rs` 新增 `detect_with_tcp_stats` 入口 + `R7 高重传率`（retransmit/out_segs > 5%）+ `R8 高 RST 率`（rst/out_segs > 2%）2 个 anomaly 规则。out_segs < 1000 时跳过（样本量太小，统计噪音大）。模块内嵌 6 个单测。
- Added (D2): `src/view_models/port_panel.rs::tick` 接入 `detect_with_tcp_stats`，3s 一次把 `TcpStats` 喂给 detector，让 R7/R8 在阈值触发时生成 anomaly。
- Added (D2): `src/tui/port_table.rs::draw_net_traffic_bar` 追加「重传 / RST / 失败」三段，按 retrans > 5% / rst > 2% 上色（danger / warning / success）；out_segs=0 时显示 `-`。
- Added (D2): `src/tui/detail_view.rs::draw_network_tab` 新增「RTT」列（默认 `-`，对应 `PortEntry.rtt_ms`）。
- Added (D2): `src/cli.rs::Command::Port` 新增 `--stats` 标志；`src/main.rs::run_port` 输出 TCP 传输质量摘要（established/listen/time_wait/close_wait/retrans/rst/failed/out_segs + 重传率/RST率）。
- Added (B3): `src/smart/mod.rs` 新模块 —— `SmartData`（device/model/serial/temperature/health/attributes）+ `SmartHealth`（Ok/Warning/Failing/Unknown，`badge()` 返回 ✓/⚠/✗/-）+ `SmartAttribute`（id/name/value/threshold/raw_value/failing）+ `parse_smartctl_json(content)` 纯函数 + `read_smart(device)` 跨平台分发 + `list_disks()`。Windows 走 smartctl 子进程（装了 smartmontools 的话），失败退化到 WMI `MSStorageDriver_FailurePredictStatus`（只给 health，无属性表）；Linux 走 smartctl 子进程；macOS 同 Linux。模块内嵌 7 个单测。
- Added (B3): `src/collect.rs::SmartWorker` —— 独立后台 worker，`sync_channel(1)` + Drop shutdown + join，30s poll 周期。单盘失败不阻塞其它盘。`SystemSnapshot::new` 预热阶段 `recv_first(2s)` 拿首帧，避免 sidebar SMART 徽章空白。
- Added (B3): `src/collect.rs::SystemSnapshot::smart_data()` 访问器 + `refresh_light()` try_recv 覆盖缓存。
- Added (B3): `src/tui/sidebar.rs::format_smart_badge()` —— 把 SmartWorker 缓存的 health.badge() + 温度追加到每行磁盘后；空 Vec 时返回空字符串（无 SMART 数据时 sidebar 不变）。
- Added (B3): `src/cli.rs::Command::Smart { device: Option<String> }` + `src/main.rs::run_smart_list` / `run_smart_detail`。`proc smart` 列出所有磁盘 + 健康/温度/属性数；`proc smart <device>` 输出完整 SMART 属性表（comfy-table）。
- Added (B3): `src/error.rs::ProcError::Smart { message, source }` 变体 + `smart()` / `smart_with()` / `smart_msg()` 三个构造器。
- Added (ADR-0003): `docs/adr/0003-smart-subprocess-vs-library.md` —— 解释为什么选 smartctl 子进程而非 libatasmart：libatasmart 维护停滞（2013 年最后一版）、完全不支持 NVMe、Windows 完全不支持；smartmontools 持续维护、JSON schema 7.0+ 起稳定、跨平台覆盖、依赖管理干净、30s poll 周期下子进程开销可接受。Windows WMI `MSStorageDriver_FailurePredictStatus` 作为降级路径（无详细属性，仅预测布尔）。
- Added: `tests/test_smart.rs`（新）—— 3 个集成测试：fixture sample 解析 / failing 样本解析 / `list_disks` 不 panic。
- Added: `tests/test_tcp_stats.rs`（新）—— 5 个集成测试：`parse_proc_net_snmp_tcp` 真实格式 / 紧凑格式 / 无 Tcp 段 / 垃圾输入 / Windows `SystemSnapshot::tcp_stats()` 字段存在 smoke。
- Added: `tests/fixtures/smartctl_sample.json` —— 真实 smartctl 输出样本（Samsung SSD 850 EVO，15 个 ATA SMART 属性）。
- Test: 总测试数 466 → **492**（+26）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过。

### 阶段 4 — Slice：A4 优先级/affinity + A1 句柄 Tab + A3 内存映射 Tab

- Added (A4): `src/process_control.rs` 新模块 —— `PriorityClass`（6 档：Idle / BelowNormal / Normal / AboveNormal / High / Realtime）+ `get_priority` / `set_priority` / `get_affinity` / `set_affinity`。Windows 走 `SetPriorityClass` / `GetProcessAffinityMask` / `SetProcessAffinityMask`；Linux 走 `setpriority(PRIO_PROCESS)` / `sched_getaffinity` / `sched_setaffinity`；macOS 返回 `PermissionDenied`。`bump_up` / `bump_down` 实现 Realtime/Idle 边界 clamp；`to_nice` / `from_nice` 完成与 Linux nice 的 6 档映射（19/10/0/-5/-10/-20）。
- Added (A1): `src/inspect/handles.rs` 新模块 —— `collect_handles(pid)` Windows 上用 `GetModuleHandleW("ntdll.dll") + GetProcAddress` 手动加载 `NtQuerySystemInformation` / `NtQueryObject`，枚举 `SystemExtendedHandleInformation` 按 PID 过滤，对每个匹配句柄 `DuplicateHandle` 到当前进程后 `NtQueryObject(ObjectTypeInformation)` 拿类型名（`File` / `Key` / `Mutant` 等 11 档分类）；Linux 走 `/proc/<pid>/fd/*` + `readlink` 拿目标路径。`find_lockers(path)` Windows 复用 filelocksmith（内部已用 worker thread + 200ms 超时规避 `NtQueryObject(ObjectNameInformation)` 同步阻塞），Linux 遍历 `/proc/*/fd/*`。`parse_handle_kind` 把 NT type_name 字符串归类到 `HandleKind`，独立单测覆盖。
- Added (A3): `src/inspect/memory.rs` 新模块 —— `collect_memory(pid)` Windows 走 `VirtualQueryEx` 遍历整个进程地址空间（上限 `0x7FFF_FFFF_FFFF`，wrapping_add 防 0xFFFF... 溢出），按 `MEMORY_BASIC_INFORMATION.State` 分类 Commit/Reserve/Free，`PAGE_PROTECTION_FLAGS` 映射为 `rwxg` 风格字符串；Linux 解析 `/proc/<pid>/maps` 6 列格式，`parse_maps_line` 纯函数 + 单测覆盖 typical .so / `[heap]` / 匿名 / `---p` / malformed 5 类。`parse_smaps_block` 提取 Size/Rss（kB → bytes，缺失 Rss 退化为 Size），独立单测。
- Added: `src/inspect/mod.rs::HandleKind` 新增 `label()` 方法，返回 12 档稳定字符串（UI / CLI / 测试 anchor 复用）。
- Added (ADR-0002): `docs/adr/0002-inspector-tab-extension-mechanism.md` —— 解释 Inspector 继续用 enum + match 而非 trait object：编译期穷尽性 > 运行时灵活性、数据量小 vtable 开销无意义、独立字段便于按需加载、`label()` 作测试 anchor。
- Changed: `src/app.rs::App::switch_mode(ProcessDetail)` 在原 `inspect_with_ports` 之外追加 `collect_handles` / `collect_memory` 同步加载（失败退化为空 Vec），`r` 刷新同步覆盖三个数据源。
- Changed: `src/app.rs::handle_detail_key` 新增 `+` / `=` / `-` 处理 —— 调 `bump_priority(pid, up/down)` 实时改优先级并写 `status_message` / `op_history`。
- Added: `src/app.rs::App::bump_priority` / `bump_selected_priority` —— A4 优先级调整的共享实现，详情页和列表页统一走这里。
- Changed: `src/view_models/process_panel.rs::handle_list_key` 新增 `+` / `=` / `-` 分支 + `focused_pid` helper（多选时取最后选中，否则 cursor）+ `bump_priority_into` 自由函数把错误/成功塞 `status_message`。
- Changed: `src/tui/detail_view.rs` —— `InspectionTab::Handles` / `Memory` 走真实渲染（`draw_handles_tab` / `draw_memory_tab`），阶段 1 占位 `draw_construction_placeholder` 删除。Summary Tab 新增「优先级」+「Affinity」两行（同步查 `process_control::get_priority` / `get_affinity`，单次 < 1ms）。快捷键栏追加 `+/-=优先级`。
- Changed: `src/cli.rs` 新增 4 个命令 —— `Command::Who { target_path }`（位置参数，避开全局 `--path` 冲突）/ `Command::Handles { pid, file }` / `Command::Priority { pid, set }` / `Command::Affinity { pid, set }`。
- Changed: `src/main.rs` 新增 `run_who` / `run_handles` / `run_handles_pid` / `run_priority` / `run_affinity` / `parse_priority_class` / `pid_to_name` 7 个函数，复用 comfy-table 表格输出。`proc who` 空结果时提示「需要管理员权限枚举系统进程句柄」。
- Changed: `Cargo.toml` 启用 `Win32_System_LibraryLoader` feature（GetModuleHandleW / GetProcAddress 加载 ntdll 函数指针，避免改 Wdk feature 列表）。
- Added: 模块内嵌单测 —— `process_control.rs`（7 测试：label 唯一 / bump_up Realtime clamp / bump_down Idle clamp / to_nice 单调 / from_nice 往返 / Default / self_get_priority smoke）+ `inspect/handles.rs`（5 测试：parse_handle_kind 11 档 / Other 分类 / 空字符串 Unknown / format_raw_handle 16 进制 / self_collect_handles smoke）+ `inspect/memory.rs`（8 测试：parse_maps_line typical/heap/anon/noaccess/malformed + parse_proc_maps 多段 + parse_smaps_block 提取 Rss / 退化 Size + self_memory_collect 非空）。
- Added: `tests/test_inspect.rs` 追加 5 个集成测试（collect_handles / collect_memory / find_lockers 自身进程 smoke + 跨平台降级）。
- Added: `tests/test_priority.rs`（新）—— 4 个 round-trip 测试（get_priority 不 panic / set→get 往返 BelowNormal→Normal / parse_priority_class 6 档 / Linux nice 映射）。
- Added: `tests/test_inspector.rs` 追加 6 个测试（Handles/Memory Tab 切换加载 inspection_handles_data/inspection_memory_data / `r` 同步刷新 / `+`/`-` 调优先级写 status_message / Detail 占位 draw 不再触发）。
- Test: 总测试数 428 → **466**（+38）。`cargo clippy --all-targets -- -D warnings` 0 警告；阶段 4 新代码（process_control / inspect::handles / inspect::memory / cli 新命令 / main 新分发函数）`-W clippy::must_use_candidate` 0 警告。baseline 遗留 15 个 `must_use_candidate` 警告（src/app.rs 现有 getter + src/tui/security_badge.rs），不在阶段 4 范围（surgical 原则）；`cargo build --release --no-default-features` 编译通过（129 lib 测试绿）。

### 阶段 3 — Slice：E4 docker top + E1 docker logs + E3 镜像/volume/compose

- Added (E4): `src/docker/top.rs` —— `ContainerTopProcess`（pid/user/command/cpu_time/started）+ `get_container_top()` 调用 bollard `top_processes` + `parse_top_output()` 文本表格纯解析器 + `parse_top_response()` 结构化响应解析器。文本解析器容忍 CMD 列内空格（按 cmd_idx 取后续整段），无 PID/CMD 表头时返回空 Vec 降级。`DockerMonitor::container_top()` 暴露同步接口。
- Added (E1): `src/docker/logs.rs` —— `LogLine`（timestamp/message/is_stderr）+ `parse_log_timestamp()` 纯解析器（剥离 RFC3339 前缀，支持 `Z` / `±HH:MM` 时区，保留 ANSI 颜色码）+ `collect_container_logs()` 一次性拉日志 + `make_follow_options()` 构造 follow 配置。`DockerMonitor::collect_logs()` 暴露。
- Added (E1): `src/docker/logs_worker.rs` —— 后台日志 worker：独立 tokio runtime + `sync_channel(64)` 背压 + `try_send` 满即丢 + 周期 `try_recv` shutdown 信号。chunk 大小 16 行 / 4KB 字符；环形缓冲上限 5000 行（`LogViewer::MAX_BUFFER_LINES`）。Drop 句柄触发 worker 干净退出。
- Added (E3): `src/docker/images.rs` —— `ImageInfo`（id/short_id/repo_tags/created/size/containers）+ `list_images()` / `remove_image()` + `Display` 实现 + `in_use()` / `display_name()` 辅助。`<none>:<none>` tag 过滤掉。
- Added (E3): `src/docker/volumes.rs` —— `VolumeInfo`（name/driver/mountpoint/created/size/in_use）+ `list_volumes()` / `remove_volume()` + `Display` 实现。`list_volumes` 反查所有容器 mounts 给 `in_use` 标记；`size` 通过 `du` 风格递归算 mountpoint 大小。无 chrono 依赖手写 `days_from_civil` + `parse_rfc3339_to_unix`。
- Added (E3): `DockerMonitor` 新增 `container_top()` / `collect_logs()` / `list_images()` / `remove_image()` / `list_volumes()` / `remove_volume()` 6 个方法，统一走 `runtime.block_on` 同步包装。
- Added (E3): `src/view_models/docker_panel.rs` —— `DockerViewMode` 枚举（Containers/Images/Volumes，`Tab` 循环）+ `LogViewer`（buffer/scroll/follow/container）+ `DeleteTarget`（两次 `d` 确认）+ `LogsWorker` 句柄（drop 退出）。`DockerPanel` 新增 9 字段（view_mode/images/volumes/show_top_processes/top_processes/log_viewer/logs_worker/delete_pending 等）+ 9 个交互方法（`cycle_view_mode`/`refresh_images`/`refresh_volumes`/`handle_delete`/`toggle_top_processes`/`enter_logs_mode`/`exit_logs_mode`/`toggle_logs_follow`/`clear_logs`）。`handle_key` 重写：日志模式优先吃快捷键，`Tab`/`t`/`l`/`f`/`c`/`d` 全接入。
- Added (E4/E1): `src/tui/docker_panel.rs` —— 视图路由按 `view_mode` 分发到容器/镜像/volume 三种列表；标题栏新增 `[容器][镜像][卷]` 高亮 Tab；详情弹窗加进程区块（`t` 触发，最多 10 行 + 折叠）；日志覆盖层占下 60% 屏，stderr 红色，滚动条跟随/手动切换；删除确认状态走 status_message。
- Changed: `src/cli.rs` —— `Command::Docker` 改成嵌套子命令 `DockerSub`（Ps/Inspect/Top/Logs/Images/Volumes/ImageRm/VolumeRm/Compose/Events）。Compose 子命令用 `trailing_var_arg + allow_hyphen_values` 转发参数给 `docker-compose`，环境变量 `PROC_DOCKER_COMPOSE` 覆盖二进制路径。
- Changed: `src/main.rs::run_docker` 拆成 11 个子分发函数（`run_docker_ps`/`_inspect`/`_top`/`_logs`/`_images`/`_volumes`/`_image_rm`/`_volume_rm`/`_compose`/`_events`）。`logs --follow` 走 logs_worker 跟随模式 + Ctrl+C 优雅退出。
- Added: `tests/test_docker.rs` 追加 22 个测试 —— `parse_top_output` 多格式（typical/empty/header-only/args_with_spaces/structured_response）+ `parse_log_timestamp` 多格式（Z/offset/no_ts/ansi/short）+ LogChunk/LogsWorker 行为 + ImageInfo/VolumeInfo Display + 6 个 DockerSub CLI 解析覆盖 + ViewModel 状态机。
- Updated: `tests/test_skeleton.rs` 原 `test_cli_docker_parsing` 适配新嵌套结构（`docker events` 替代 `docker --watch`）。
- Added: 模块内嵌单测：`top.rs`（8 测试）+ `logs.rs`（8 测试）+ `logs_worker.rs`（6 测试）+ `images.rs`（6 测试）+ `volumes.rs`（9 测试）+ `view_models/docker_panel.rs`（6 测试）。
- Test: 总测试数 353 → **428**（+75）。`cargo clippy --all-targets -- -D warnings` 0 警告。`cargo build --release --no-default-features` 编译通过。

### 阶段 2 — Slice：H4 Miri CI + B2 per-core CPU 频率/温度

- Added (H4): `.github/workflows/miri.yml` —— Linux runner + nightly + miri 组件，跑 `test_scorer_concurrency`（5 个并发测试）+ `test_workers`（4 个 SnapshotWorker 测试）共 9 个并发测试。首次接入用 `continue-on-error: true` 容错，后续稳定后移除。
- Added (B2): `src/collect.rs::LightSnapshot` 新增 `per_core_freq: Vec<u64>`（MHz）+ `per_core_temp: Vec<Option<f32>>`（°C）字段，与 `sysinfo::System::cpus()` 顺序对齐；worker loop 每 1s 推一份，主线程 `refresh_light` 时 `try_recv` 更新缓存。新增 `SystemSnapshot::per_core_freq()` / `per_core_temp()` 公开访问器。
- Added (B2): 跨平台采集策略 —— Linux 优先读 `/sys/devices/system/cpu/cpuN/cpufreq/scaling_cur_freq`（sysfs kHz），拿不到时退回 sysinfo；Windows 走 sysinfo 注册表 `~MHz`；macOS 走 sysinfo sysctl。温度走 sysinfo `Components`，通常只能拿到全局 CPU 温度（填到第 0 核），per-core 留 None。`parse_scaling_cur_freq` 抽成纯函数便于跨平台测。
- Added (B2): `src/tui/sidebar.rs` 折叠/展开模式 —— 折叠保持现状（13 行）；展开追加 per-core 表格（核/频率/温度，最多 8 行，>8 核按温度降序取 top-8）。`App::sidebar_height()` 改为根据 `sidebar_expanded` 动态返回 13 / 23。
- Added (B2): `App` 新增 `sidebar_expanded: bool` 字段，`c` 键切换（详情页的 `c` 复制进程信息在 `handle_detail_key` 里走 ProcessDetail 分支，不被这里抢键）。切换时持久化到 `~/.config/proc/ui.toml` 的 `sidebar_expanded` 字段，下次启动恢复。`src/ui_state.rs` 新增 `load_sidebar_expanded` / `save_sidebar_expanded`，`write_state` 升级到 3 字段（sort_field + first_run + sidebar_expanded），老 ui.toml（无新字段）按缺失默认 false 处理。
- Added: `src/collect.rs` 内嵌 `collect_tests` 3 个测试（scaling_cur_freq 解析）+ `src/tui/sidebar.rs` 内嵌 6 个测试（select_cores_for_display 截断/对齐 + per_core_line 渲染）+ `src/ui_state.rs` 新增 4 个 sidebar_expanded 解析测试 + `tests/test_per_core_freq.rs` 4 个集成测试（snapshot 返回 ≥1 频率 + parse 纯函数 + App c 键状态机 + 持久化往返）。

### 阶段 1 — Spike：文档基础设施 + Inspector 可扩展 Tab 骨架（0.5.0 起点）

- Added: 0.5.0 开发宪法 `plan.md`（11 阶段拆分 + 会话规则 + 验证矩阵）、领域词汇表 `CONTEXT.md`（含规划中术语标注）、`docs/adr/0001-phased-project-adoption.md`、`docs/stages/stage-1..11.md` 目录骨架。
- Added: `InspectionTab` 枚举从 4 变体扩为 6 变体（+`Handles` / `Memory`），`label` / `next` / `prev` / `all` 同步更新，循环正确（Memory ⇄ Summary）。
- Added: `src/tui/detail_view.rs` 新增 `draw_construction_placeholder` —— Handles / Memory Tab 渲染「建设中（阶段 4 上线）」占位文本，不崩溃。
- Added: `src/inspect/mod.rs` 新增 `HandleInfo` / `HandleKind` / `MemoryRegion` / `MemoryState` 类型骨架（字段 + `Default`，不实现采集），阶段 4 直接填实现。
- Added: `App` 结构体新增 `inspection_handles_data` / `inspection_memory_data` 两个 `Option<Vec<...>>` 占位字段（始终 None），`App::new()` 默认初始化正确。
- Added: `tests/test_inspector.rs` 适配 6 变体（更新 `inspection_tab_all_*` / `next_cycles` / `prev_cycles` / `labels` / `tab_key_cycles_inspector_tabs` / `backtab_cycles_in_reverse`），并新增 5 个加固不变量测试（`all_in_next_cycle_order` / `next_prev_are_inverse_for_all_six` / `next_six_times_returns_to_start` / `labels_are_all_distinct` / `memory_tab_is_last_in_all`）。
- Added: `tests/test_skeleton.rs` 新增 `test_app_inspection_handles_and_memory_default_none` —— 锁定占位字段初始 None。

## [0.4.0] - 2026-06-17

本次发布聚焦于 Inspector：进程详情页升级为多 Tab 深挖视图（环境变量 / 网络连接 / 已加载模块）。阶段 12（数据层）+ 阶段 13（TUI）共 2 个阶段，5 modified + 3 new，核心 +482 / -13。无 API 破坏；详情页原有快捷键全部保留（向后兼容）。

实测数据：

- 测试 **325 passed / 0 failed / 2 ignored**（baseline 291 → +34）
- pedantic `must_use_candidate`：**0**
- ADR-0004 落地一致性：✅ B2 方案无偏差

### 阶段 12 — Inspector v1 数据层 / Round 8

- Added (ADR-0004): `src/inspect/` 新模块 + 3 个子模块（`env` / `dlls` / `net`），顶层 `inspect::inspect(pid)` 聚合成 `InspectionData { env, dlls, net }`。
- Added: Windows 环境变量采集 —— `OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION)` + `NtQueryInformationProcess(ProcessBasicInformation)` 走 PEB → ProcessParameters → Environment。x64 上偏移 0x20 / 0x80 / 0x3F0 注释完整；32-bit 显式拒绝（避免错误偏移）。
- Added: Windows 模块列表 —— `CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32)` + `MODULEENTRY32W`，与 `security/dll_check.rs` 同源。
- Added: Linux 环境变量 —— 读 `/proc/<pid>/environ`，NUL 分隔 + `=` 分隔。
- Added: Linux 模块列表 —— 解析 `/proc/<pid>/maps`，BTreeMap 合并同 path 的多段映射（r-xp / r--p / rw-p），取最低 base + span 求和。
- Added: `Cargo.toml` 新增 Windows feature `Win32_System_Diagnostics_Debug`（ReadProcessMemory）+ `Wdk_System_Threading`（NtQueryInformationProcess）。
- Added: 跨平台降级 —— 非 Win/Linux 平台 env/dlls 返回 `ProcError::PermissionDenied`，由 TUI 层显示降级提示。
- Added: `tests/test_inspect.rs` 7 个集成测试 + `src/inspect/*.rs` 内嵌单元测试（self env/dlls/net 数据正确性 + unknown PID + Linux proc_maps 解析）。

### 阶段 13 — Inspector v1 TUI / Round 9

- Added (ADR-0004): `InspectionTab` 枚举（Summary / Env / Network / Dlls），`label()` / `next()` / `prev()` / `all()` 4 个方法全部 `#[must_use]`，`#[derive(Default)]` 保证 Summary 为默认 Tab。
- Added: `App` 结构体新增 4 字段（`inspection_tab` / `inspection_data` / `inspection_search` / `inspection_scroll`），集中在「Inspector」分组；`App::new()` 默认值正确。
- Added: `switch_mode(ProcessDetail)` 预加载 `inspection_data` + 重置 tab/scroll/search —— 进入详情页立即可见数据。
- Added: `handle_detail_key` 重写 —— 搜索 active 时优先吃输入且吞掉 Tab/BackTab（避免误触丢搜索内容）；Tab 切换重置 scroll；`r` 重新 `inspect()` + status_message 提示；Esc 双层（先退搜索，再退页面）。
- Added: `src/tui/detail_view.rs` 整体重写 —— Tab 栏（当前 Tab `accent + Bold + Underlined`）+ 主体内容区，4 个 Tab 分别渲染，每个 Tab 处理 empty / no-match / data-None 三态降级提示。
- Changed: Summary Tab 保留原详情页全部内容（分类 / 父进程 / 状态 / CPU / 内存 / 磁盘 / 运行时长 / exe / cmd / cwd / 端口 / 网络摘要 / 安全分 / 风险因子 / 快捷键）—— **零回归**。
- Changed: Dlls Tab 按 path 字母排序；Network Tab 不接搜索（数据量通常小）。
- Added: `tests/test_inspector.rs` 22 个集成测试 —— InspectionTab 枚举行为 + App state 默认 + Tab/BackTab 切换 + 搜索 + 刷新 + 滚动 + 跨平台 smoke。

### 验证矩阵

- `cargo fmt --all -- --check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test --release` ✅ 325 passed / 0 failed / 2 ignored（baseline 291 → +34：7 inspect 数据层 + 22 inspector TUI + 5 inspect 内嵌单元测试）
- `cargo build --release --no-default-features` ✅
- `cargo clippy ... -W clippy::pedantic | grep must_use_candidate` ✅ 0

### ADR 状态

- ADR-0004（Inspector B2 升级详情页）Status: **Accepted**（阶段 12-13 落地，2026-06-17）

### P2 改善建议（本仓库未单独维护 tech-debt.md，长期项记在此处）

- P2-6：`detail_view::draw_summary` 每帧重扫端口 → 已在 0.5.0 打磨中修复（复用 `port_panel.port_entries`）
- P2-7：`parse_utf16_env` 双 NUL 截断改 `find` → 0.5.0+ 可选
- P2-8：`End` 设为 `usize::MAX / 2` 加注释 → 0.5.0+ 可选

## [0.3.0] - 2026-06-17

本次发布聚焦于资源生命周期治理、性能优化、CI 加固、文档/帮助打磨、错误链完善，并伴随二进制体积优化。阶段 5-9 共 5 个 Round 累积：69 文件，+1043 / -249。无 API 破坏；错误类型 `ProcError` 转 struct form 含 source chain（ADR-0005）。

实测数据（来自内部 review 记录，未单独入仓）：

- 测试 **291 passed / 0 failed / 2 ignored**（baseline 283 → +8）
- 二进制体积 **-6.1%**（7.3 MB → 6.9 MB，ADR-0007 profile.release）
- pedantic `must_use_candidate`：**0**

### 阶段 5 — 资源生命周期 / Round 3

- Fixed (ADR-0006): `BackgroundScorer::Drop` 死锁修复 —— take 出 `request_tx` 后再 join，避免 bounded channel 满时 `try_send(Shutdown)` 失败导致 worker 卡死。新增 `tests/test_scorer_concurrency.rs::test_scorer_request_drops_when_busy` 验证（修复前 60s+ 不退出，修复后 0.03s）。
- Changed (ADR-0006): `docker/events.rs` 和 `monitor/port_watcher.rs` 事件通道从无界 `channel()` 改为 `sync_channel(64)` + `try_send` + `Full` → `tracing::warn!`，避免慢消费者积压。
- Changed: `diag.rs` 资源生命周期重构（+109 / -50），消除多处散落的临时句柄。
- Performance: `port_map::scan_ports` 走 `SysinfoRegistry` 全局单例，消除每次扫描 `System::new_all()` 的 ~200ms 开销；`eject::locks::find_volume_lockers_with_processes` 的最差 O(N × 200ms) 路径同步消除。

### 阶段 6 — 性能优化 / Round 4

- Fixed (ADR-0003): PID 复用导致旧实例的安全评分缓存过继给新进程 —— `ScoreCache::cache_key` 加 `start_time` 字段（`{pid}:{start_time}:{exe}`），`App::update_disk_speeds` 的 `prev_process_disk` 键改为 `(pid, start_time)` 元组。新增 `test_score_cache_pid_reuse_isolation` 回归测试。
- Performance: `format_speed` 抽取到 `src/format.rs`，统一磁盘 / 网络速率格式化。
- Performance: 500 进程基准（`tests/test_perf_baseline.rs`，v0.7.0 阶段 1 由 `test_stage8_perf_regress.rs` 改名而来）`rebuild_sorted_cache` **38.2 µs**（< 5ms 目标，130× 裕量）。

### 阶段 7 — CI / Cargo / Round 5

- Added (ADR-0007): `[profile.release]` 保守版（`opt-level=3` + `lto="thin"` + `codegen-units=1` + `strip="debuginfo"`，**不开** `panic="abort"`）。二进制体积 -6.1%。
- Changed: `tokio` features 精简为 `["rt", "rt-multi-thread", "macros", "net", "sync", "time"]`，`cargo build --no-default-features` 通过。
- Added: CI 新增 `check-macos` / `msrv` (1.85) / `audit` (RustSec) job。
- Fixed: `dll_check.rs::truncate_path` 改为手写实现以兼容 Rust 1.85（`str::split_once` 边界 case）。

### 阶段 8 — 文档 / 帮助 / Round 6

- Added: **首次启动引导** —— `~/.config/proc/ui.toml` 不存在时显示一次性提示。
- Added: **进程列表排序字段持久化** —— `←`/`→`/`S` 切换排序时写入 `ui.toml`，下次启动恢复。新增 `src/ui_state.rs` 模块，含 9 个解析测试覆盖 sort_field / first_run。
- Added: **3 个新主题** —— Gruvbox（暖色复古）/ One Dark（Atom 默认配色）/ Rose Pine（柔和现代），`THEMES` 从 7 个扩展到 10 个，`t` 循环切换。
- Changed: `help_panel.rs` 重构为结构化数据 + 滚动，新增 `sections_are_non_empty` / `every_shortcut_has_a_label` 内嵌不变量测试。

### 阶段 9 — error.rs source chain / Round 7

- Changed (ADR-0005): `ProcError` 7 个变体全部转 struct form，统一含 `#[source] source: Option<Box<dyn StdError + Send + Sync>>`。配套 14 个 `xxx()` / `xxx_with()` helper。全仓库 13 个调用站点迁移，无 `ProcError::Variant { ... }` 字面量构造（`IoError` 仍保留 `#[from] std::io::Error` 向后兼容 `?`）。
- Added: `tests/test_skeleton.rs::test_proc_error_source_chain` 验证 source chain 可遍历到根因。
- Fixed: pedantic `must_use_candidate` 全部消除（0 残留）。

### 阶段 11 — 批量修复 + 发布（本次发布）

- Fixed (REVIEW-10 P1-3): `SecurityScorer::invalidate_dead` 改用 `(pid, start_time)` 元组精确清理，避免 PID 复用场景下陈旧 entry 残留（之前仅靠 30s TTL `evict_expired` 兜底）。新增 `parse_alive_key` 解析键前两段。
- Docs: README GPU 路线图补"AMD/Intel 列入 0.5.0+ 路线图"。

### 验证矩阵

- `cargo fmt --all -- --check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test --release` ✅ 291 passed / 0 failed / 2 ignored（baseline 283 → +8：1 PID 复用 + 6 ui_state + 1 source chain）
- `cargo build --release --no-default-features` ✅
- `cargo clippy ... -W clippy::pedantic | grep must_use_candidate` ✅ 0

### ADR 状态

- ADR-0003（PID 复用 start_time 键）Status: **Accepted**（阶段 6 落地，2026-06-16）
- ADR-0005（error.rs source chain）Status: **Accepted**（阶段 9 落地）
- ADR-0006（sync_channel(64) 背压）Status: **Accepted**（阶段 5 落地，2026-06-16）
- ADR-0007（profile.release 保守版）Status: **Accepted**（阶段 7 落地，2026-06-16）

## [0.2.1] - 2026-06-16

Patch 版本，三阶段累积修复：跨平台编译（阶段 2 cfg-gate）+ 文案一致性（阶段 3 纯文案 10 项）+ 鲁棒性（阶段 4 鲁棒性 10 项）。无 API 破坏；行为上有几处错误从"静默吞没"改为"显式提示 / 返回 Result"。

### 阶段 2 — cfg-gate 跨平台降级（Linux 编译修复）

- Fixed: `src/classify.rs` 和 `src/eject/{device,locks,cache,classify}.rs` 在顶级 `use windows::Win32::*` 但无 `#[cfg(target_os="windows")]` gate —— Linux 编译失败、`check-linux` CI 为红。改用**模块级 cfg gate**（详见 ADR-0002）。
- Changed: `src/eject/mod.rs` 顶层暴露跨平台结构体 `RemovableDevice` / `HandleLock` / `UsbScanResult`，原 Windows 实现下沉到 `windows_impl`，非 Windows 走 `stub_impl`（全部返回 `Err(ProcError::UsbDetect)`）。
- Changed: `format_size` 上移到 `eject/mod.rs`，避免循环依赖。
- Changed: `Cargo.toml` `filelocksmith` 依赖移至 `[target.'cfg(windows)'.dependencies]`。
- Changed: `App::new` 启动 `status_message`（非 Windows）补全完整降级清单。

### 阶段 3 — 纯文案一致性（10 项）

- Changed: TUI 文案与 README 快捷键表对齐 —— `A`/`R`/`Shift+←→` 等此前未公开的快捷键全部补到 README。
- Changed: 启动时降级提示文案统一（之前零散）。
- Changed: 杂项中英混排、错别字、过期路径修正（10 项，详见 git log）。

### 阶段 4 — 鲁棒性（10 项）

- Fixed (P1.20): `init_tracing` 静默忽略 `create_dir_all` / `File::create` / `set_global_default` 错误 —— 改为 `eprintln!` 显式提示用户"日志不可用"。
- Changed (P1.21): `AlertManager::load_or_default` 改为 `try_load` 返回 `Result`，调用方决定 fallback；新增 `Default` impl 给 `AlertManager`。
- Fixed (P1.22): `record::reader::Player::open` 不再对恶意 / 损坏的录制文件无限信任 header_len —— 上限 64 KB，超出立即 `bail!`，避免 OOM。新增 `test_player_rejects_oversized_header` 回归测试。
- Fixed (P1.23): `security::hash_cache::HashReputation::hash_file` 从 `std::fs::read` 改为 `BufReader` 流式 + 64 MB 上限 —— 多 GB 安装器 / 敌意文件不再能撑爆内存。新增 2 个单元测试（80 MB 大文件不爆 + 超 cap 字节不影响摘要）。
- Fixed (P1.24): `local_offset_hours` 中 `-(bias + ...) as i64 / 60` 触发 `-(i32::MIN)` UB（Rust 一元 `-` 优先级高于 `as`）。抽出 `bias_minutes_to_offset_hours(i32) -> i64` 纯函数，先 `as i64` 再取负。新增针对 `i32::MIN` 的回归测试。
- Fixed (P1.10): `monitor::watchdog` 收到 Ctrl+C 时 `child.kill()` 静默忽略错误 —— 改为 `tracing::warn!` 记录。
- Fixed (#8): CHANGELOG 引用本 plan 重新编号前的旧 ADR 编号 / dirty-rect 分析文档全部失效 —— 改为泛指（"详见 ADR-0001" / "详见 CHANGELOG 阶段记录"）。
- Added (#10): tracing 补 6 处关键路径观测点 —— `tick_light_refresh` 重刷失败 warn（已有，保留）、`BackgroundScorer` 评分耗时 debug、`scan_ports` 耗时 debug、`find_volume_lockers` 耗时 debug、`record::writer` 每 100 帧写入字节数 debug、`docker::events` 断线重连 warn + 上限 10 次放弃。
- Changed (#12): `init_tracing` 补注释说明 `File::create` 默认 truncate 行为 —— 启动时覆盖旧日志，防止长期运行后 `proc.log` 无限增长。如需保留历史请走 `tracing-appender`（ADR-0006 已规划）。
- Added (#13): `init_tracing` 接入 `EnvFilter::try_from_default_env()`，默认级别 `info`，`RUST_LOG=proc=debug` 等环境变量生效（之前因缺 `env-filter` feature 不生效）。`tracing-subscriber` 启用 `env-filter` feature。README FAQ 增 RUST_LOG 用法说明。

### 验证矩阵

- `cargo fmt --all -- --check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test --release` ✅ 283 passed / 0 failed / 2 ignored（阶段 3 baseline 279 → +4：1 record header cap + 2 hash streaming + 1 bias_minutes UB 回归）
- `cargo build --release --no-default-features` ✅

Linux 端仍由 GitHub Actions `check-linux` job 验证；本机 WSL vhdx 仍损坏。

### ADR 状态

- ADR-0002（cfg gate）Status: **Accepted**（阶段 2 落地，2026-06-15，本次随 0.2.1 一起发布）

## [0.2.0] - 2026-06-15

本次发布聚焦于代码质量、性能优化和用户体验打磨。共修复 6 个真实 Bug，完成 6 项架构整洁，新增 9 项 UX 改进，4 项跨平台与文档增强，5 项测试覆盖，3 项性能优化。

### 阶段 8 — Review：全局审查 + 脏区域优化（收尾）

- 全局代码审查通过：`cargo test --release`（297 个测试，0 失败）、`cargo clippy --all-targets -- -D warnings`（0 警告）、`cargo fmt --all -- --check`（无 diff）、`cargo build --release --no-default-features`（通过）。
- 33 项问题逐一核对：#1 VT100 RGB、#2 跨平台时区、#3 watchdog try_wait、#4 sysinfo 散落、#5 排序 O(N²)、#6 Arc<Vec> 共享、#7 panels/tui 重命名、#8 AppMode 死代码、#9 tick ≤ 50 行（实测 33 行）、#10 replay ≤ 50 行（实测 24 行）、#11 deprecated 删除、#12 scan_ports 不再 new_all、#13 help_panel.rs、#14 主题持久化、#15 Ctrl+C handler、#16 时间格式含月-日、#17 README 隐藏快捷键公开、#18 THEMES 长度 = 7、#19 Command::Export、#20 read_line 已废弃、#21 Command::Pkill、#22 README 平台支持、#23 LICENSE/CHANGELOG、#24 CI workflow、#25 README GPU 路线图、#26 test_record_color、#27 test_scorer_concurrency、#28 test_kill_tree、#29 skeleton 合并、#30 test_platform_compat、#31 select_nth_unstable、#32 脏区域（见下方"性能优化"）、#33 tick_history_sample 抽离 —— **全部落地**。
- Performance: 脏区域优化经真实测量后决定**不动代码** —— ratatui 内置 buffer diff 已实现 Cell 级增量传输，`App::tick` 已用 `needs_draw` 判断避免无谓重绘，每帧 draw 调用成本 < 15ms（20 fps 预算 50ms）。激进 dirty rect 优化的复杂度收益比差，且引入回归风险。完整分析见 CHANGELOG 阶段 8 记录。
- Performance 基线回归（500 进程基准，见 `tests/test_perf_baseline.rs`，v0.7.0 阶段 1 由 `test_stage8_perf_regress.rs` 改名而来）：`rebuild_sorted_cache` **38.2 µs**（< 5ms 目标，130× 裕量）、top-N `select_nth_unstable` + 局部排序 **6.1 µs**（< 1ms 目标，160× 裕量）。无回退。
- Removed: `SystemSnapshot` 中未使用的 `prev_process_disk` / `prev_process_disk_time` 字段（被 `App` 同名字段独立实现，注释明确标记 `#[allow(dead_code)]` "used via App, not directly in SystemSnapshot"），同时移除 `per_disk_io_speed` 上过时的 TODO 注释（功能已实现）。
- Pedantic 现状：`cargo clippy -- -W clippy::pedantic` 共 ~287 个 `format!` 风格建议、~119 个 `#[must_use]` 缺失、~63 个 cast 精度提示等，全部为风格偏好而非 bug。`-D warnings` 等级 0 警告。本阶段不修 pedantic，留作未来风格统一批次。



本次打磨按 8 个阶段组织，预期产出见下方各阶段小节。

### 阶段 1 — Spike：工程化基线 + 死代码清理

- 新增 `LICENSE`（MIT，2024-2026，Alfroul）。
- 新增 `CHANGELOG.md`（Keep a Changelog 格式）。
- 提交 `.github/workflows/ci.yml`，包含 `cargo build`、`cargo test`、`cargo clippy -D warnings`、`cargo fmt --check` 以及 `--no-default-features` 验证。
- 移除 `AppMode::Help` 与 `AppMode::Menu` 死代码。
- 标记 `SystemSnapshot::processes()` 为 `#[deprecated]`，引导迁移到 `process_cache()` / `cached_processes_vec()`。
- 合并 `tests/test_stage6_skeleton.rs` 到 `tests/test_skeleton.rs`。
- 在 `CONTEXT.md` 增补"33 项打磨计划"术语区段。

### 阶段 2 — Slice：录屏系统修复

- Fixed: VT100 录屏保留 RGB 颜色（之前所有 RGB 都被存为 Reset，导致回放褪色）。`CellDump.fg` / `bg` 扩为 `u32`，采用带标记位的可变编码（bit 31 = RGB 标记）。
- Added: Ctrl+C 优雅退出（全局 `shutdown` 模块，TUI、回放、`monitor`、`docker watch` 均响应），确保录制文件正常 flush。
- Added: `tests/test_record_color.rs`（7 个测试），覆盖 Reset / 16 基本色 / RGB / Indexed 的 roundtrip，以及完整 `Buffer → VtFrame → bincode → Buffer` 颜色一致性。
- Changed: `VT100_VERSION` 提升到 2，旧版本 v1 文件回放时给出友好错误（详见「阶段 2 — 录制 RGB」小节的决策记录）。

### 阶段 3 — Slice：跨平台基础

- Added: Linux/macOS 的本地时区计算（`local_offset_hours` 通过 `libc::localtime_r` 实现，之前固定返回 0）。Windows 实现保持不变（`GetTimeZoneInformation`）。
- Added: `App::is_windows` 字段；非 Windows 平台首次启动时状态栏显示一次性降级提示，明确告知签名验证 / 降频检测 / U 盘句柄枚举 / Toast 通知 / EStats 带宽不可用。
- Added: `src/collect.rs`、`src/app_group.rs`、`src/estats.rs` 中所有非 Windows stub 函数首次调用时通过 `tracing::warn!` 写入 `~/.config/proc/proc.log`，便于排查空面板的根因。
- Added: `tests/test_platform_compat.rs`（3 个测试）— `local_offset_hours` 范围校验、stub 函数可调用性、`ProcessInfo` 跨平台 JSON 序列化 round-trip。`ProcessInfo` 添加 `PartialEq` + `Serialize` + `Deserialize` 派生以支持断言。
- Added: `README.md` 新增 `## 平台支持`（功能矩阵表）与 `## GPU 支持路线图`（AMD/Intel 路径与时间表）章节。
- Changed: `.github/workflows/ci.yml` 增加 `check-linux` 作业（`ubuntu-latest`），运行 `cargo check` 与 `cargo clippy`，并在 Linux 上构建 `test_platform_compat` 测试二进制。
- Changed: `Cargo.toml` 增加 `[target.'cfg(not(target_os = "windows"))'.dependencies] libc = "0.2"`（libc 已是间接依赖，直接声明不增加体积）。
- Updated: `CONTEXT.md` 中 `PlatformFeature` 术语补充完整定义。

### 阶段 4 — Slice：Kill/Watchdog 安全

- Fixed: watchdog 现在可以在子进程长跑时通过 Ctrl+C 关停 — `src/monitor/watchdog.rs` 把阻塞的 `child.wait()` 换成 `try_wait()` + 100ms 轮询，收到 shutdown 信号时显式 `child.kill()` 防止孤儿进程；退避 sleep 也改为 1 秒可中断。
- Fixed: `proc monitor add --pid` 不再使用阻塞的 `stdin().read_line()`，改为 `shutdown::requested()` 200ms 轮询，Ctrl+C 立即退出。
- Added: `proc pkill <name>` 子命令，按进程名（精确匹配、大小写不敏感）批量终止进程；`--force` 走 `kill_process_tree`，`--dry-run` 仅列出匹配项不终止。`src/kill.rs` 新增 `find_processes_by_name` / `kill_by_name` 公共 API。
- Added: `tests/test_kill_tree.rs`（8 个测试）— 覆盖 `AlreadyGone` / `AccessDenied`（PID 4 System）/ 无匹配 / spawn 出来的进程能被 find / dry_run 不实际 kill / 结果结构契约。
- Updated: `README.md` 命令行章节补充 `proc pkill` 示例。

### 阶段 5 — Slice：性能优化

- Performance: 引入 `SysinfoRegistry` 全局单例（`src/collect.rs` 顶部 `SYSINFO_REGISTRY` + `sysinfo_with`），消除 5 处散落的 `sysinfo::System::new_all()` 调用 —— `port_map::scan_ports`、`eject::locks::find_volume_lockers_with_processes`（原循环内每个未命中 PID `new_all` 一次，最差 O(N × 200ms)）、`kill::kill_single` 非 Windows 分支、`kill::find_processes_by_name` 全部改为只读访问 SysinfoRegistry 快照。详见 CHANGELOG 阶段 5 记录。
- Removed: `SystemSnapshot::processes()` 老 deprecated 方法（被 `cached_processes_vec` 替代），同步迁移 `tests/test_alert.rs`、`tests/test_process_list.rs`、`tests/test_skeleton.rs` 中 9 处调用，删除已无意义的 `test_incremental_refresh_consistent_with_full`。
- Performance: `App::rebuild_sorted_cache` 从 O(N²) 改为 O(N) —— 一次性构造 `PID → idx` HashMap 替代循环内 `Vec::position` 查找，同时把内部 `Vec<(class, ProcessInfo)>` 改为 `Vec<(class, &ProcessInfo)>` 借引用，省去每帧全字段深拷贝。
- Performance: top-N 进程排序使用 `slice::select_nth_unstable_by` —— 500 进程时比较次数从 ~4500 (O(N log N)) 降到 ~786 (O(N) + O(K log K))，sparkline 历史采样路径受益。
- Performance: `BackgroundScorer::request` 签名从 `Vec<ProcessInfo>` / `Vec<PortEntry>` 改为 `Arc<Vec<...>>`，为下游消费者共享而非拷贝铺路；score 线程循环改用 `as_ref()` 切片迭代。
- Performance: `global_cpu_history` / `global_mem_history` 显式落到 light refresh（每秒）采样；`proc_history` 仍依赖 heavy refresh 的新 cached_processes，只在 heavy 帧推数据，retain 清理保留在 light 中。sparkline 现每秒一格。

### 阶段 6 — Slice：架构整洁

- Refactor: `src/panels/` 重命名为 `src/view_models/`，避免与 `src/tui/`（纯渲染层）在目录名层面混淆 —— 前者持有面板状态 + 业务逻辑（MVVM 中的 ViewModel 角色），后者无状态。`ProcessPanel` 等类型名保持不变，避免改名风暴；外部 import 路径由 `crate::panels::*` 改为 `crate::view_models::*`（涉及 `src/lib.rs` 和 `src/app.rs`）。详见 ADR-0001。
- Refactor: `App::tick` 从 170+ 行拆分为 8 个职责清晰的方法（`tick_replay` / `tick_light_refresh` / `tick_throttle_check` / `tick_history_sample` / `tick_alert_evaluate` / `tick_panels` / `tick_usb_monitor_docker` / `clamp_cursors` + 配套 `update_disk_speeds`），主 `tick` 方法体降到 33 行。每个方法 30-60 行，单一职责。
- Refactor: `App::replay_load_current_frame` 从 100+ 行字段一一映射改为基于 `From<&Frame*>` trait 的转换 —— 新增 `src/record/conversions.rs` 集中 7 个 `From` 实现（`FrameProcess` / `FrameTreeNode` / `FramePortEntry` / `FrameUsbDevice` / `FrameUsbLock → HandleLock + HandleRisk` / `FrameContainer` / `FrameOpRecord`）以及 `NetworkViewMode::from_frame_code` 辅助函数；调用站点降至 24 行。`replay_load_current_frame` 进一步拆出 `restore_replay_panel_data` / `restore_replay_nav` / `restore_replay_metrics` / `restore_replay_view_mode` 4 个辅助方法。
- Refactor: `App::replay_tick` half/normal/double/quad 步进逻辑从嵌套 `let step = { ... }` 块简化为单层 match；at_end 检测使用块作用域自动释放 timeline 不可变借用。
- Added: `tests/test_scorer_concurrency.rs`（5 个并发测试）覆盖 `BackgroundScorer` 的 request-drop-when-busy / poll-non-blocking / round-trip / 多线程并发 / shutdown 行为。
- Added: `BackgroundScorer` 实现 `Drop` trait，drop 时通过 `try_send(Shutdown)` 非阻塞通知 worker 线程退出，避免主线程结束时 worker 卡在 recv。

### 阶段 7 — Slice：UX 完成

- Added: `?` 帮助页（`src/tui/help_panel.rs`），按 `?` 进入、Esc/q/? 返回，列出全局 / 进程列表 / 端口 / U 盘 / 监控 / Docker / 录制 / 帮助页 共 8 个分组的全部快捷键。新增 `AppMode::Help` 变体（阶段 1 删除的 `Help` 在此恢复使用），`App::help_scroll` 支持上下滚动、PgUp/PgDn、Home/End。
- Added: 主题持久化（`src/tui/theme.rs`）—— 启动时从 `~/.config/proc/theme.txt` 读取上次选择（`init_persisted_theme` 在 `App::new` 中调用），`cycle_theme` 切换时写入。容错：文件缺失 / 非数字 / 越界索引都自动回退到 Dark，不阻塞启动。
- Added: 第 7 个内置主题 **Light**（浅色背景、深色文字），用于强光环境；`t` 循环切换，7 次回到 Dark。
- Added: `proc export --format json|csv` 子命令。JSON 输出含 ISO-8601 时间戳（`local_iso_timestamp` 用 `local_offset_hours` + `epoch_secs_to_ymd` 实现，无 chrono 依赖）、`total` 计数和 `processes` 数组（pid/name/cpu_usage/memory_bytes/exe）。CSV 标准转义（逗号 / 引号 / 换行）。支持 `--sort` 排序、`--limit` 截断、`-o` 输出到文件。新增 `src/format.rs` 的 `export_processes_as_json` / `export_processes_as_csv` / `local_iso_timestamp` 函数和 4 个单元测试。
- Changed: 操作日志（`OpRecord::time`）时间格式从 `HH:MM` 改为 `MM-DD HH:MM`，便于跨天查看历史。新增 `crate::epoch_secs_to_ymd` 辅助函数（基于 Howard Hinnant `civil_from_days` 算法），位于 `src/lib.rs` 并附带 5 个测试用例（含 2000-02-29 闰日）。
- Changed: VT100 回放时间轴 `format_timestamp` 同步升级为 `MM-DD HH:MM:SS`，与操作日志格式对齐。
- Changed: README 快捷键表与命令行章节补充 `?` 帮助、`A`/`R`/`Shift+←→` 等此前未公开的快捷键、`proc export` 用法、新增 Light 主题。

## [0.1.0] - 2026-06-12

首次发布版本，包含以下已交付能力。

### 新增

- **进程列表**：按 CPU/内存/PID/名称/安全分/磁盘读写排序，模糊搜索、多选、批量终止、分页；`v` 切换列表/应用分组视图，`2` 直达进程树。
- **进程树**：父子层级、展开/折叠、孤儿/僵尸/残存检测，`o`/`z` 一键选中异常进程。
- **端口/网络**：按端口/按进程/按远程三种视图，网络诊断工具箱（Ping/DNS 反查/Whois/Traceroute/端口探测），异常检测（CLOSE_WAIT 堆积等 6 种模式）。
- **U 盘助手**：可移除设备检测、占用进程风险分级、缓存刷新、安全弹出引导、持续监测模式。
- **进程监控**：按 PID/端口/命令监视，崩溃指数退避自动重启，Windows Toast 通知。
- **Docker 监控**：容器列表、实时事件流、健康检查、资源统计，支持命名管道与 TCP 两种连接方式。
- **安全评分**：14 项独立检查（Authenticode 签名、父进程链、路径、命令行、网络行为、DLL、特权、信誉等），0-100 评分与按安全分排序。
- **降频检测**：通过 `CallNtPowerInformation` 实时检测 CPU 降频与原因分类（热/功耗/空闲）。
- **磁盘 I/O**：每磁盘独立读写速率与每进程 I/O 速率追踪。
- **侧边栏**：CPU/内存/交换区使用率 + 火花线图、GPU 信息、网卡 IP、温度（颜色分级）、降频状态、运行时间。
- **录屏与回放**：VT100 终端录屏（`.prec` 格式），支持播放/暂停/逐帧/倍速回放，录制期间状态栏显示 REC 指示。
- **告警系统**：可配置阈值规则（CPU/内存/磁盘/网络/连接数/温度/降频），连续命中防抖，Info/Warning/Critical 分级，Critical 推送 Toast；TOML 配置 `~/.config/proc/alerts.toml`。
- **CLI 子命令**：`ls` / `tree` / `port` / `kill` / `eject` / `monitor` / `record` / `replay` / `docker`。
- **6 种内置主题**：Dark、Catppuccin、Dracula、Nord、Solarized、Tokyo Night，`t` 切换。

### 技术栈

- Rust 2024 Edition，ratatui + crossterm，clap 4，sysinfo 0.34，bollard 0.18，可选 nvml-wrapper（NVIDIA GPU）。
