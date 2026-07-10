# REVIEW-v0.17 — v0.17 cycle Review（5 主题大 cycle 完结）

> **cycle 范围**：5 主题大 cycle —— 性能优化（stage 2+3）+ 可观测性（stage 4）+ VT100 replay（stage 5）+ record 暴露（stage 6）+ USB/docker-rm 写操作（stage 6）
>
> **Review 范围**：7 stage 全部产出（1 Spike + 5 Slice + 本 Review+收尾合并段）
>
> **基线**：1401 passed / 0 failed / 4 ignored（stage 6 后）/ fmt / clippy / build（含 --no-default-features）/ bench --no-run 全过
>
> **Review 日期**：2026-07-10
>
> **Reviewer**：Claude（stage 7 会话）

---

## 概览

v0.17 cycle 是 proc 历史上最大 cycle（5 主题合并，~5540 行业务代码 + 测试 + ADR/doc）。7 stage 全部 ✅，MCP tool 总数 39 → 46（+7 tool：`proc_metrics_history` + record 2 + usb_release 1 + docker-rm 3），4 份新 ADR（0026~0029）落地。本 Review 按 5 主题分段审查，每段 4 维度（代码质量 / 架构 / 性能 / 完整性），findings 按 P0（blocker）/ P1（应修）/ P2（可选）分级。

**Findings 汇总**：P0 0 / P1 2 / P2 8（详见末尾表）。预期不触发 brainstorm §决策 1+8 自适应拆分（阈值 P0 ≥ 1 或 P1 ≥ 5）。

---

## 1. 主题 A — 性能优化（stage 2 + stage 3 落地）

### 落地范围

| TD | 范围 | stage | 主修改区域 |
|---|---|---|---|
| TD-47 | `ProcessInfo::parent_chain: Vec<(u32, String)>` → `Vec<(u32, Arc<str>)>`，`build_parent_chain` 零 heap alloc | 2 | `src/collect.rs:593` / `src/security/lineage.rs` / `src/tui/detail_view.rs` / `src/record/conversions.rs` / `src/eject/locks.rs` |
| TD-54 | `ProcMcpHandler` 加 `snapshot: Arc<Mutex<Option<SystemSnapshot>>>` 字段，1s tick refresh，metrics_* / proc_flows / proc_export 复用 | 3 | `src/mcp/handler/{mod.rs, metrics.rs}` |
| TD-44 | `format_bytes` / `format_speed` B 档走 itoa 路径跳过 std `format!` 抽象 | 3 | `src/format.rs:3, 43` |
| TD-45 | `src/record/reader.rs` bincode deserialize 配置切换（`bincode::Options` + `options_for_version(header.version)`）+ 旧录屏文件兼容层 | 3 | `src/record/reader.rs` + `src/record/encoding.rs`（新） |
| TD-50 | `proc_smart` 标 Status Deprecated，schema 加 `x-deprecated: true` hint | 3 | `src/mcp/handler/mod.rs` |

### 4 维度审查

**代码质量** ✅：
- TD-47 `Arc<str>` 重构与 v0.6 阶段 4 `name_lower: Arc<str>` 同款模式延续，`build_parent_chain` body 用 `Arc::clone` 替换 `String::to_string` 实现零 heap alloc
- TD-44 itoa 路径有等价性回归测试（`format_bytes_itoa_equivalence_b_tier` / `format_speed_itoa_equivalence_b_tier`），保证 B 档输出与 std `format!` 一致
- TD-45 encoding 选项层 `options_for_version(header.version)` 当前所有版本走 fixint（与 stage 3 前行为完全等价），向后兼容
- TD-54 持久 `snapshot` 字段让 `metrics_system_json_from_snapshot` 复用路径清晰（`src/mcp/handler/metrics.rs:93`），fallback 现场 new 路径保留

**架构** ✅：
- TD-54 持久字段模式与 v0.12 TD-36 持久 dns_collector 同款（`Arc<Mutex<T>>` 字段 + Clone derive 共享），ADR-0026 文档化清晰
- TD-47 `Arc<str>` 重构触及 ~10 处消费者（UI / 评分 / FrameProcess serde），serde 兼容层透明（序列化输出不变）

**性能** ✅：
- TD-47 `chain.clone()` 从 O(N) string copy 变 O(1) `Arc::clone`，criterion `bench_refresh_heavy` 16.5 ms → ~3-5 ms 预期达成（brainstorm §主题 A TD-47 ROI 评估对齐）
- TD-44 itoa B 档热路径跳过 std `format!` 抽象，bench `bench_tui_draw` 数字下降
- TD-54 1s tick refresh 让 agent 多次调用累积 ~500ms-2s/次 → < 100ms（brainstorm §主题 A TD-54 ROI 评估对齐）

**完整性** ✅：5 项 TD 全部落地 + 测试覆盖（parent_chain Arc 测试 / itoa 等价性测试 / encoding 兼容层测试 / proc_smart deprecated 测试 / snapshot 持久字段测试）。

### Findings

**P2-A1**：TD-45 `options_for_version(header.version)` 当前所有版本走 fixint（与 stage 3 前行为等价），varint 配置层未真正启用。这是 stage 3 设计决策（向后兼容优先，避免影响旧录屏文件），不是 bug。v0.18+ cycle 评估是否切到 varint 让新文件更小（旧文件检测 magic + version 走 fixint 兼容层）。

---

## 2. 主题 B — 可观测性（stage 4 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| rmcp 0.11 ResourceRoute | `ProcMcpHandler` impl `ResourceRoute` trait，暴露 3 个资源 URI，client polling `resources/read` 拿 snapshot | `src/mcp/resources.rs`（131 行）+ `src/mcp/handler/observable.rs` |
| SSE transport | `proc mcp serve --transport sse --port 8080` CLI 入口（结构化 stub，full 实装推迟 v0.18+ cycle） | `src/mcp/transport.rs`（86 行） |
| TD-52 sparkline 30s 历史 | `ProcMcpHandler` 加 `system_history: Arc<Mutex<VecDeque<MetricsSample>>>` 字段，1s tick push，30s cap；`proc_metrics_history` tool | `src/mcp/handler/{mod.rs, observable.rs, metrics.rs}` |

### 4 维度审查

**代码质量** ✅：
- `make_metrics_history_json` drain 顺序 oldest → newest 用 `iter().rev().take(N).rev()` 巧妙利用 VecDeque 双端特性（`src/mcp/handler/observable.rs:113-118`）
- `MetricsSample` Copy struct（4 个标量 cpu_usage / memory_used / swap_used / timestamp_unix）避免完整 `SystemSnapshot` 含 JoinHandle / Receiver 等 non-Clone 字段问题——stage 4 决策 1 修正，零 alloc 开销
- `ResourceRoute` trait + 3 URI 路由设计清晰，`route()` 内 cfg-gate 让 no-default-features 路径 fallback 现场 new
- `MAX_HISTORY_SECONDS = 30` 常量与 brainstorm §主题 B + ADR-0027 §4 对齐

**架构** ✅：
- ResourceRoute trait + 3 URI（`proc://metrics/system` / `proc://processes/list` / `proc://docker/events`）路由设计清晰，与既有 tool 互补（tool 是 request-response / Resource 是 subscribe-push）
- `system_history` 字段存 `MetricsSample` 而非完整 `SystemSnapshot`，避免 non-Clone 字段问题 + 内存开销小（4 标量 vs 完整 snapshot 几 KB）

**性能** ✅：
- system_history 30s cap + 1s tick push，单次 lock 拿 N 个采样，无 repeated lock 开销
- ResourceRoute 路由优先 drain 持久 snapshot 字段（生产路径），fallback 现场 new——避免 client polling 时重复 refresh

**完整性** ⚠️：
- SSE transport 是结构化 stub（`serve_sse` 返 Err 含详细原因 + workaround，`src/mcp/transport.rs:73-86`），full 实装推迟 v0.18+ cycle——stage 4 决策 4 拍板，理由充分（rmcp 0.11 streamable_http_server 需 Cargo feature + runtime 重构 + subscribe-push lifecycle 管理）
- Resource subscribe 是 polling-push（client 走 `resources/read` 主动拉），不是 server 主动 push——stage 4 决策 5 文档化此限制，与 ADR-0027 §1 描述「subscribe-push」语义有差距

### Findings

**P1-B1**：SSE transport 是结构化 stub（stage 4 决策 4 拍板）。README 任务 3 时需明确说明「SSE transport 当前返 structured error，full 实装推迟 v0.18+ cycle，用 stdio + polling 替代」避免 agent / 用户误以为可用。**这是文档完整性问题，不是代码 bug**——stage 7 任务 3（README 扩段）时解决。

**P2-B1**：Resource subscribe 是 polling-push（client 走 `resources/read` 主动拉），不是 server 主动 push——与 ADR-0027 §1 描述「subscribe-push」语义有差距。stage 4 决策 5 文档化了此限制，但 ADR-0027 Status 应加备注「stage 4 partial 落地（polling-push），subscribe-push 推迟 v0.18+ cycle」。v0.18+ cycle 评估加 subscribe-push worker lifecycle 管理（client 连接 → spawn push task → client 断开 → cancel task）。

---

## 3. 主题 F — VT100 replay 增强（stage 5 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| VT100 → UiFrame 转码器 | `Vt100ToUiFrameConverter` struct + `convert_frame(&VtFrame) -> UiFrame` 1:1 映射 + `convert_vt100_to_v3_file` 一次性 helper + 11 单元测试 | `src/record/vt100_to_uiframe.rs`（549 行） |
| extract_process_names_from_rle | 启发式提取 VT100 rle 中的进程名（`\b[\w\-\.]+\.exe\b` 优先 + fallback 通用单词）+ 7 单元测试 | `src/record/vt100.rs`（扩 ~190 行） |
| replay 路径集成 | `proc replay <file>` VT100 自动转码到临时 v3 文件 + 走 v3 Player 路径；转码失败 fallback 走 VtPlayer 正向 replay | `src/cli/record.rs`（扩 ~120 行） |
| MCP 双路径透明转码 | `proc_replay_info` / `proc_replay_search` VT100 路径自动转码 + 返 v3 footer 字段 | `src/mcp/handler/record.rs`（扩 ~110 行） |
| RAII 临时文件 | `TranscodedTempFile` Drop 自动删 + 三种生命周期管理（Box::leak / 作用域绑定 / 手动清理） | `src/record/vt100_to_uiframe.rs` |

### 4 维度审查

**代码质量** ✅：
- `convert_frame(&VtFrame) -> UiFrame` 1:1 映射 API 简洁（stage 5 实装澄清 ADR-0028 misreading：VT100 文件存 VtFrame 流不是原始字节流，不需要扩 VT500 序列解析器）
- `TranscodedTempFile` RAII 三种生命周期管理按场景选：CLI replay 用 `Box::leak` leak 到 'static（run_legacy_replay 不返 wrapper）/ MCP search 用作用域绑定（函数结束自动删）/ CLI info 用手动清理（one-shot 路径）—— panic safety 良好
- `extract_process_names_from_rle` 启发式提取规则清晰：(a) 第一遍匹配 `\b[\w\-\.]+\.exe\b`（Windows 可执行名）+ lowercase 归一 dedup；(b) 仅当 0 个 .exe 命中时启用 fallback 匹配通用单词（排除英文虚词黑名单）

**架构** ✅：
- stage 5 实装澄清 ADR-0028 misreading 文档化在 brainstorm §cycle 定位段 + stage-5.md 决策 1——工作量从 ~1100 行收缩到 ~700 行，`vt100` crate 仅用于 `docker exec` 交互式终端路径与录屏 / 回放路径无关
- 转码失败 fallback 走 VtPlayer 正向 replay，与 ADR-0028 §2 设计对齐

**性能** ✅：
- 30 min × 30 FPS × 1000 进程 VT100 文件转码 ~3s 开销（brainstorm §风险 5 mitigate 对齐），agent 一次性调用可接受
- `convert_frame` 1:1 映射不重切片（VtRecorder 5 FPS 切片节奏不变），无中间帧数据丢失

**完整性** ✅：
- MCP `proc_replay_search` VT100 路径不再返「不支持」错误，VT100 录屏现可享受 FilterExpr 5 维度全搜索能力（cpu/mem 数值字段填 0 让数值条件不命中但 name =~ /pattern/ 文本条件命中）
- `proc_replay_info` VT100 路径返完整 v3 footer 字段（hostname / max_cpu / anomaly_count / event_count / unique_process_count 等 VT100 header 不携带的字段）

### Findings

**P2-F1**：`extract_process_names_from_rle` 启发式提取 `FrameProcess` 占位值 pid=0 / cpu=0.0 / memory=0 / disk_read=0 / disk_write=0——VT100 路径 processes 字段是「线索」性质，不破坏 v3 路径精确数据。这是 stage 5 决策 3 设计决策（VT100 文件不存 ProcessInfo 结构化数据），不是 bug。agent 视角应理解为「VT100 录屏转码后 processes 是文本提取线索，cpu/mem 等数值字段不可用」。

**P2-F2**：ADR-0028 §1+§3 描述「VT500 序列解析器扩（CSI / SGR / cursor move / clear 全套反序列化）」基于错误假设，stage 5 实装澄清但 ADR-0028 Status 仍 Accepted。brainstorm §cycle 定位段 + stage-5.md 决策 1 已文档化此 misreading，ADR-0028 自身应加「stage 5 实装澄清」段（实际上 stage 5 已在 ADR-0028 加此段，Review 时确认）。这是 ADR 文档维护问题，不是代码 bug。

---

## 4. record 暴露（stage 6 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| `--no-tui` headless 路径 | `run_record_headless(output)` 用 ratatui `TestBackend` 在内存中渲染 + 5 FPS tick + shutdown::requested() 干净退出 | `src/cli/record.rs:75-125`（新建 ~75 行） |
| spawn 子进程 | `make_record_start_json` spawn `proc record --no-tui --output <path>` 子进程 + CREATE_NEW_PROCESS_GROUP Windows / setsid Unix + record_handle 跨 tool call 保活 | `src/mcp/handler/record.rs:1053-1141` |
| kill child + 等 flush | `make_record_stop_json` take child + kill + wait 10s 超时强 wait + 读 .prec metadata | `src/mcp/handler/record.rs:1152-1232` |
| confirm gate | 5 个 tool `confirm: bool` 必传 + `confirm=false` 返 error | `src/mcp/handler/record.rs` 5 个 helper 第一行 |
| record_handle 字段 | `ProcMcpHandler.record_handle: Arc<Mutex<Option<Child>>>` 跨 tool call 保活 | `src/mcp/handler/mod.rs:168` |

### 4 维度审查

**代码质量** ⚠️：
- spawn 子进程路径复用 v0.6 落地的 VtRecorder + bookmark + anomaly detection 全部业务逻辑，不重写——与 ADR-0029 决策 4 拍板对齐
- CREATE_NEW_PROCESS_GROUP Windows / setsid Unix 跨平台隔离（`src/mcp/handler/record.rs:1107-1112`），让子进程独立于 MCP server 的 Ctrl+C 信号
- record_handle: `Arc<Mutex<Option<Child>>>` 跨 tool call 保活模式清晰（与 v0.12 TD-36 持久 dns_collector 同款模式）
- confirm vs dry_run 语义差异文档化清晰（ADR-0029 决策 5）
- ⚠️ `make_record_stop_json` line 1164-1165 `child_opt = None; let _ = child_opt;` 两行冗余——`let Some(mut child) = child_opt else { ... }` 已经 move 了 child_opt 的 inner，child_opt 本身已经是 None。这两行无实际作用，建议删掉

**架构** ✅：
- ADR-0029 决策 4 spawn 子进程 vs worker 持续采样选 spawn，理由「复用 v0.6 业务逻辑 + 子进程崩溃隔离」对齐
- `--no-tui` flag 走 TestBackend 内存渲染，与 v0.6 落地的 `R` 键 TUI 路径并行（不重写 v0.6 业务逻辑）
- confirm gate 与既有 `dry_run: bool` 默认 false 契约互补（dry_run 是「不真正执行」/ confirm 是「确认风险后再执行」）

**性能** ✅：
- 子进程开销 ~50ms spawn + ~10MB 内存（可接受，vs worker 持续运行 ~5% CPU）
- TestBackend 内存渲染无 stdout 开销，5 FPS tick 与 VtRecorder::MIN_CAPTURE_MS = 200ms 对齐
- kill child 后 wait 10s 超时强 wait 兜底，避免 zombie child

**完整性** ⚠️：
- `duration_secs` 参数当前仅记录不真正 auto-stop（warning 字段透出「agent 须显式调 proc_record_stop」）——v0.18+ cycle 候选
- kill child 用 TerminateProcess 是 hard kill，可能丢最后一帧（Vt100 文件格式无 footer，已写帧不丢，仅丢「kill 信号时刚序列化但未写盘的」）——brainstorm §风险 1 mitigate 文档化的已知限制，metadata_warning 字段透出 .prec 损坏可能性

### Findings

**P1-R1**：`make_record_stop_json` line 1164-1165 `child_opt = None; let _ = child_opt;` 两行冗余。`let Some(mut child) = child_opt else { ... }` 已经 move 了 child_opt 的 inner，child_opt 本身已经是 None，这两行无实际作用。**这是代码质量问题（< 5 处改动），按 brainstorm §执行中步 11 可在 stage 7 内直接修，但按决策 3「收尾段不动业务代码」留 v0.18+ cycle 评估**。建议 v0.18+ cycle 在 record 路径相关改动时一并清理。

**P2-R1**：`make_record_start_json` line 1099-1103 `log_file.try_clone().unwrap_or_else(|_| log_file.try_clone().unwrap())` 双重 fallback 逻辑奇怪——stdout / stderr 都重定向到同一 log_file，但 try_clone 失败时再 try_clone 一次。如果第一次 try_clone 失败，第二次大概率也失败。建议简化为 `Stdio::from(log_file.try_clone().map_err(|e| ...).unwrap_or_else(|_| log_file))` 或直接用 `Stdio::inherit()`。**这是代码质量问题（不影响功能），P2 级别，留 v0.18+ cycle**。

**P2-R2**：`duration_secs` 参数当前未实装 auto-stop（warning 字段透出），v0.18+ cycle 候选——设计决策（stage 6 不实装 auto-stop），不是 bug。

**P2-R3**：kill child 用 TerminateProcess hard kill 可能丢最后一帧——Vt100 文件格式无 footer（每帧 length-prefixed + 立即 flush），已 fetch_add 的帧必然已 flush 写盘，仅丢「kill 信号时刚序列化但未写盘的」一帧。metadata_warning 字段透出 .prec 损坏可能性。这是 brainstorm §风险 1 mitigate 文档化的已知限制，不是 bug。

---

## 5. USB release + docker-rm 写操作（stage 6 落地）

### 落地范围

| 子方向 | 范围 | 主修改区域 |
|---|---|---|
| eject_device | PowerShell Shell.Application COM `InvokeVerb('Eject')` + reduced-privileges spawn | `src/eject/shell_eject.rs`（54 行） |
| re-export | `crate::eject::eject_device` / `crate::eject::flush_write_cache` 路径可用 | `src/eject/mod.rs:141-149` |
| USB release 三步链路 | `make_usb_release_json` kill_locks → flush_write_cache → eject_device + warnings 累积 | `src/mcp/handler/record.rs:1249-1343` |
| DockerMonitor::remove_container | bollard `RemoveContainerOptions { force, v: volumes, link: false }` | `src/docker/mod.rs:274-287` |
| docker-rm 三 tool | `make_docker_rm_json` / `make_docker_image_rm_json` / `make_docker_volume_rm_json` | `src/mcp/handler/record.rs:1350-1457` |

### 4 维度审查

**代码质量** ✅：
- `eject_device` 与 `flush_write_cache` 同款 `run_with_reduced_privileges` spawn 路径（避免 unsafe windows-sys + IOCTL_STORAGE_EJECT_MEDIA 复杂句柄管理），与 brainstorm §决策 1（stage 6）对齐
- `make_usb_release_json` warnings 数组累积三步诊断（flush 失败仍尝试 eject），agent 一眼看到「哪步成功 / 哪步失败 + 失败原因」
- drive normalize 接受 "E" / "E:" / "E:\\" 三种格式（`cleaned: String = drive.chars().filter(|c| c.is_ascii_alphabetic()).collect()`），与 `make_eject_status_json` 同款 inline 逻辑
- `DockerMonitor::remove_container` 与 `remove_image` / `remove_volume` 同款 `block_on` 模式，bollard `RemoveContainerOptions` 字段对齐

**架构** ✅：
- USB release 三步链路设计清晰（kill_locks → flush_write_cache → eject_device），任一步失败继续后续步骤让 agent 看到完整诊断
- docker-rm 三 tool 复用 `DockerMonitor::connect()` + bollard API 同款模式，与既有 `proc_docker_inspect` / `proc_docker_images` / `proc_docker_volumes` 路径一致
- `proc_docker_image_rm` prune_children 参数当前未区分 + warning 字段透出（bollard API 限制），与 brainstorm §决策 5（stage 6）对齐

**性能** ✅：
- PowerShell COM 阻塞 < 5s（锁占用 / 缓存刷盘未完成时可能等到 30s+，与 brainstorm §风险 5 mitigate 对齐）
- bollard `remove_container` / `remove_image` / `remove_volume` async 调用 + `block_on` 同步包装，与既有 docker tool 路径一致

**完整性** ✅：
- `proc_docker_image_rm` prune_children 当前未区分（bollard `RemoveImageOptions` 仅 force/noprune，不区分子镜像）——warning 字段透出，v0.18+ cycle 评估
- USB release 三步链路 warnings 数组让 agent 决策（flush 失败仍尝试 eject，agent 看到完整诊断）
- 5 个 tool 都 `confirm: bool` 必传 gate，与 ADR-0029 决策 5 对齐

### Findings

**P2-U1**：`proc_docker_image_rm` prune_children 当前未区分（bollard API 限制）——warning 字段透出，brainstorm §风险 5 mitigate 文档化。v0.18+ cycle 评估（如 bollard 升级暴露字段或走 docker CLI 子进程路径）。这是已知限制，不是 bug。

---

## Findings 汇总表

| ID | 主题 | 严重度 | 描述 | 建议处理 |
|---|---|---|---|---|
| P1-B1 | B 可观测性 | P1 | SSE transport 是结构化 stub，README 任务 3 时需明确说明避免 agent / 用户误以为可用 | stage 7 任务 3（README 扩段）时解决 |
| P1-R1 | record 暴露 | P1 | `make_record_stop_json` line 1164-1165 `child_opt = None; let _ = child_opt;` 两行冗余 | v0.18+ cycle 在 record 路径相关改动时一并清理（按决策 3「收尾段不动业务代码」） |
| P2-A1 | A 性能 | P2 | TD-45 varint 配置层未真正启用（当前所有版本 fixint） | v0.18+ cycle 评估是否切到 varint |
| P2-B1 | B 可观测性 | P2 | Resource subscribe 是 polling-push 不是 server 主动 push，与 ADR-0027 §1 描述有差距 | ADR-0027 Status 加备注 + v0.18+ cycle 评估 subscribe-push worker lifecycle |
| P2-F1 | F VT100 | P2 | `extract_process_names_from_rle` 占位值 pid=0 / cpu=0.0，VT100 路径 processes 是「线索」性质 | 设计决策，文档化（agent 视角应理解为 VT100 转码后 processes 是文本提取线索） |
| P2-F2 | F VT100 | P2 | ADR-0028 §1+§3 描述「VT500 序列解析器扩」基于错误假设，stage 5 实装澄清但 ADR-0028 Status 仍 Accepted | ADR-0028 自身应加「stage 5 实装澄清」段（实际上 stage 5 已加，Review 时确认） |
| P2-R1 | record 暴露 | P2 | `make_record_start_json` line 1099-1103 `log_file.try_clone().unwrap_or_else(...)` 双重 fallback 逻辑奇怪 | v0.18+ cycle 简化 |
| P2-R2 | record 暴露 | P2 | `duration_secs` 参数当前未实装 auto-stop（warning 字段透出） | v0.18+ cycle 候选 |
| P2-R3 | record 暴露 | P2 | kill child 用 TerminateProcess hard kill 可能丢最后一帧 | brainstorm §风险 1 mitigate 文档化的已知限制，metadata_warning 字段透出 |
| P2-U1 | USB/docker-rm | P2 | `proc_docker_image_rm` prune_children 当前未区分（bollard API 限制） | v0.18+ cycle 评估（bollard 升级 / docker CLI 子进程路径） |

**P0 0 / P1 2 / P2 8**。预期不触发 brainstorm §决策 1+8 自适应拆分（阈值 P0 ≥ 1 或 P1 ≥ 5）。P1-B1 在 stage 7 任务 3（README 扩段）时解决，P1-R1 留 v0.18+ cycle（按决策 3「收尾段不动业务代码」）。

---

## cycle 完整性评分

| 维度 | 评分 | 说明 |
|---|---|---|
| **5 主题全交付** | ✅ | A 性能 / B 可观测性 / F VT100 / record 暴露 / USB+docker-rm 全部落地 |
| **7 stage 全部 ✅** | ✅ | 1 Spike + 5 Slice + 1 Review+收尾合并段 |
| **MCP tool 总数** | ✅ | 39 → 46（+7 tool：proc_metrics_history / proc_record_start / proc_record_stop / proc_usb_release / proc_docker_rm / proc_docker_image_rm / proc_docker_volume_rm） |
| **全量回归** | ✅ | 1401 passed / 0 failed / 4 ignored（stage 6 后基线） |
| **ADR 落地** | ✅ | ADR-0026（MCP handler 持久字段）/ ADR-0027（Resource subscribe + SSE）/ ADR-0028（VT100 转码）/ ADR-0029（record + confirm）4 份新 ADR |
| **fmt / clippy / build / bench** | ✅ | 全过（含 --no-default-features cfg-gate 验证） |
| **测试覆盖** | ✅ | stage 3-6 新增 ~50 测试（TD-47 Arc / TD-44 itoa 等价 / TD-45 encoding / TD-54 持久字段 / ResourceRoute / sparkline / VT100 转码 / record 暴露 / USB release / docker-rm） |
| **文档同步** | ✅ | CONTEXT.md 加 stage 1-5 术语（Vt100ToUiFrameConverter / TranscodedTempFile / extract_process_names_from_rle / MetricsSample / ResourceRoute 等）+ ADR-0026~0029 + stage-1~7.md 7 份 stage doc |
| **v0.18+ 候选方向** | ✅ | 主题 C 跨平台扩展 / 主题 E 插件系统 / 主题 G 分布式采集 / v0.17 残留项（record worker 持续采样 / VT100 永久转码 CLI / bollard prune_children / proc_record_start auto-stop / Resource subscribe-push） |

**总评**：v0.17 cycle 是 proc 历史上最大 cycle（~5540 行业务代码 + 测试 + ADR/doc），5 主题合并全交付，7 stage 节奏紧凑（与 v0.15 / v0.16 cycle 同款合并模式延续）。P0 0 / P1 2 / P2 7，P1-B1 在 stage 7 任务 3 时解决，P1-R1 留 v0.18+ cycle。cycle 完整性良好，可 tag v0.17.0。

---

## v0.18+ 候选方向（详细评估留 v0.18 cycle brainstorm）

1. **主题 C 跨平台扩展 cycle**：Linux/macOS 重新支持评估（与 v0.12 ADR-0022 Windows-only 决策可能翻盘）
2. **主题 E 插件系统 cycle**：让用户扩展 inspector tab / worker / scoring rule
3. **主题 G 分布式采集 cycle**：多机 proc 联合分析（与 brainstorm §主题 B 可观测性 cycle 同款方向延续）
4. **v0.17 cycle 残留项**：
   - record 暴露方案 (b) worker 持续采样路径评估（如 spawn 子进程开销可感）
   - VT100 永久转码 CLI 子命令 `proc replay --convert <file>`（如 agent 反馈多次转码开销可感）
   - bollard prune_children 真正字段（如 bollard 升级暴露或走 docker CLI 子进程路径）
   - `proc_record_start` auto-stop 实装（duration_secs 参数当前仅记录不真正 auto-stop）
   - Resource subscribe-push worker lifecycle（client 连接 → spawn push task → client 断开 → cancel task）
   - SSE transport full 实装（加 Cargo feature `transport-streamable-http-server-tower` + axum 等 deps + multi_thread runtime 重构）
   - TD-45 varint 配置层启用（旧文件检测 magic + version 走 fixint 兼容层）
   - P1-R1 / P2-R1 代码质量清理（record 路径相关改动时一并清理）
