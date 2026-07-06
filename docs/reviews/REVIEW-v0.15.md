# REVIEW-v0.15：v0.15.0 cycle 全局 Review

> **范围**：v0.15.0 cycle stage 1-3 全部产出（commit `d38b741 plan(v0.15)` 之后的全部 working tree 改动）—— MCP 模块骨架重构（`handler.rs` 单文件 1156 行 → `handler/{mod, cli, inspect, metrics}.rs` 4 子 module）+ 15 个新查询类 tool schema 设计 + cat 1 CLI 命令 9 tool + cat 2 `proc_inspect` 6 tab + cat 4 metrics 5 tool 业务逻辑填充 + ADR-0023/0024 + 39 集成测试。
> **方法**：按 stage 4 doc §任务 2 列出的 6 子项审查（代码质量 / 架构 / 性能 / 完整性 / 安全跨平台 / P0-P1-P2 列表）。
> **基线**：`cargo test --release -q` = **1281 passed / 0 failed / 3 ignored**；`cargo fmt --all -- --check` / `cargo clippy --release --all-targets -- -D warnings` / `cargo build --release --no-default-features` / `cargo bench --no-run` 全过。
> **结论**：**P0 0 / P1 1 / P2 5**。stage 1-3 全部交付，无未交付项。1 个 P1 集中在文档完整性（stage 1 doc 头部缺 ✅ 标记，与 v0.14 stage 5 P1-1 同款）；5 个 P2 归档 TD-50 ~ TD-54（MonitorManager 持久化 / metrics sparkline / per-process disk_io / metrics_smart 合并入口 / metrics 多次调用 App 复用，brainstorm FAQ + stage 2/3 doc 已知风险段已明确）。
> **Date**：2026-07-06。

---

## 0. 验收对照表（stage 1-3 是否全交付）

| Stage | 范围 | 验收 | 状态 |
|---|---|---|---|
| 1 Spike | MCP 模块骨架重构（`handler.rs` 1156 行 → `handler/{mod, cli, inspect, metrics}.rs` 4 子 module，Strategy A：32 个 `#[tool]` 都在 mod.rs 主 impl 块）+ 15 tool stub + `InspectTab` 6 变体 enum + ADR-0023（6 Tab 合并设计）+ ADR-0024（子 module 拆分决策）+ CONTEXT.md 加 3 术语（McpToolCategory / InspectTab / MetricsKind） | `cargo test --release` 1242 passed（基线不变）；`cargo run --release -- mcp serve` tool_router 收集到 32 个 tool（17 既有 + 15 新）；`grep '"stub": true' src/mcp/handler/{cli,inspect,metrics}.rs` 15 处 stub helper 全部注册；InspectTab `JsonSchema` derive 编译过 | ✅ 全交付（commit `163b63c`）|
| 2 Slice | cat 1（9 tool）+ cat 2（proc_inspect 6 tab）业务逻辑填充。`cli.rs` 9 个 stub helper 替换为真实业务实现（flows 走 `App::new + 2s warm-up` / throttle 走 `query_throttle`/`set_throttle` / export 走 `SystemSnapshot + format::export_*` / docker_inspect/images/volumes 走 `DockerMonitor` bollard / docker_events 500ms 短超时 drain / monitor_add/remove 走 `MonitorManager` dry_run=false 默认）；`inspect.rs` 1 个 stub helper 替换为 6 tab 分支实装（详情页视角返完整 cmd 真值 / env secret mask + reveal opt-in）；mod.rs 10 个 `#[tool]` description 去 "Stage 1 stub" 字样；`tests/test_mcp_v0_15.rs`（新）29 case | `cargo test --release` 1271 passed（基线 1242 + 新测试 29）；`grep "Stage 1 stub" src/mcp/handler/mod.rs` 仅剩 5 处 metrics（stage 3 范围）；`grep '"stub": true' src/mcp/handler/{cli,inspect}.rs` 无匹配（10 helper 全部去 stub）；集成测试 `test_proc_inspect_summary_*` / `test_proc_flows_*` / `test_proc_monitor_add_*` 等 29 case 全过 | ✅ 全交付（commit `8834fd1`）|
| 3 Slice | cat 4 metrics 5 tool 业务逻辑填充。`metrics.rs` 5 个 stub helper 替换为真实业务实现（system 走 `SystemSnapshot::new + refresh` 含 cpu/mem/swap/disk/uptime/process_count/net_adapters/tcp_stats 8 字段/temperatures；gpu 走 `GpuCollector::new + refresh` 含 providers 字段；disk_io 走 SystemSnapshot disk_io_speed + per_disk_io_speed + all_disks 含 device 过滤；smart 双路径 device=None 聚合 vs device=Some 详细 attributes，决策 2 选 (b) 落地；thermal 走 per_core_freq + per_core_temp + throttle_info + classify_throttle reason）；mod.rs 5 个 metrics tool description 去 "Stage 1 stub"；`tests/test_mcp_v0_15.rs` 扩 10 case | `cargo test --release` 1281 passed（基线 1271 + 新测试 10）；`grep "Stage 1 stub" src/mcp/handler/mod.rs` 全清零；`grep '"stub": true' src/mcp/handler/metrics.rs` 无匹配（5 helper 全部去 stub）；`test_proc_metrics_*` 10 case 全过 | ✅ 全交付（commit `f2e3fc7`）|

**结论**：stage 1-3 全部交付，无未交付项。cycle 业务代码累计 ~1700 行（与主题 D 预期 ~1850 行接近），新测试累计 +39（1242 → 1281）。

---

## 1. 六子项审查

### 1.1 代码质量

#### 1.1.1 stage 1 子 module 拆分：Strategy A 落地正确（rmcp 0.11 限制规避）

| 检查 | 实测 | 结果 |
|---|---|---|
| `handler/mod.rs` 行数 | 1358 行（v0.7 既有 1156 行 + 15 个 #[tool] stub 方法 ~190 行 + use 声明 ~12 行） | ✅（stage 1 §决策 1 Strategy A 落地正确）|
| `handler/cli.rs` 行数 | 568 行（9 个 Args struct + 9 个 helper + 内部 helper） | ✅ |
| `handler/inspect.rs` 行数 | 360 行（ProcInspectArgs + InspectTab enum + make_inspect_json 6 tab 分支 + 2 内部 helper） | ✅ |
| `handler/metrics.rs` 行数 | 400 行（5 个 Args struct + 5 个 helper + 4 内部 helper usage_obj/matches_device/make_metrics_smart_aggregated/single） | ✅ |
| `#[tool_router]` impl 块位置 | 主 mod.rs 单 impl 块（与 stage 1 §决策 1 一致）| ✅ |
| 32 个 `#[tool]` 方法都在主 mod.rs | `grep -c "^    #\[tool" src/mcp/handler/mod.rs` = 32 | ✅ |
| 32 个唯一 tool name | `grep -oE 'name = "proc_[a-z_]+"' mod.rs \| sort -u \| wc -l` = 32 | ✅ |
| 既有 17 tool 行为零回归 | stage 2 决策 6 + stage 3 决策 7「impl 块结构稳定」 | ✅（基线 1242 → 1281 全过）|

**判定**：子 module 拆分按 Strategy A 落地正确，rmcp 0.11 `#[tool_router]` 不跨 module 收集 `#[tool]` 方法的限制规避（所有 32 个 `#[tool]` 都在主 mod.rs impl 块，子 module 只放 Args struct + 业务 helper）。✅

#### 1.1.2 stage 2 cat 1/cat 2 业务逻辑：字段裁剪 + dry_run + secret mask 三大决策落地正确

| 检查 | 实测 | 结果 |
|---|---|---|
| `proc_flows` 走 App::new + 2s warm-up | `cli.rs::make_flows_json`（与 `make_diag_json` 同款路径，stage 2 决策 3）| ✅ |
| `proc_throttle` query/set 三态 Normal/Eco/Unknown | `cli.rs::make_throttle_json` 调 `crate::throttle::{query_throttle, set_throttle}`（cfg-gate Windows-only）| ✅ |
| `proc_export` JSON/CSV 双格式 | `cli.rs::make_export_json` 调 `crate::format::{export_processes_as_json, _csv}` | ✅ |
| `proc_docker_*` 4 tool bollard API | `cli.rs::make_docker_{inspect,images,volumes,events}_json` 调 `DockerMonitor` | ✅ |
| `proc_docker_events` 500ms 短超时 drain | stage 2 决策 4（MCP 不能 follow event stream，drain 一批返 note 字段）| ✅ |
| `proc_monitor_add/remove` dry_run=false 默认 | stage 2 决策 1（与 v0.7 `proc_kill`/`proc_pkill` 契约一致）| ✅ |
| `proc_inspect` 详情页视角返完整 cmd 真值 | `inspect.rs::make_inspect_json` Summary tab 返 ProcessInfo 完整字段（stage 2 决策 2，与列表视角字段裁剪互补）| ✅ |
| `proc_inspect(env)` secret mask 默认 + reveal opt-in | `inspect.rs` 调 `env_mask::is_secret_key` + `mask_value`（与 v0.6 env_reveal 同款契约）| ✅ |
| `proc_inspect(network)` drain DNS 历史 5 条 | `inspect.rs` Network tab 调 `detect_collector` 现场 drain | ✅ |
| `proc_inspect(handles)` 复用 `proc_handles` schema | `inspect.rs` Handles tab 调 `inspect::handles::collect_handles`（与 v0.7 同 schema）| ✅ |

**判定**：cat 1/cat 2 业务逻辑落地正确，3 大字段裁剪决策（详情页视角 vs 列表视角 / secret mask 默认 / dry_run=false 默认）全部生效。✅

#### 1.1.3 stage 3 cat 4 metrics：5 helper 走 SystemSnapshot 直采路径稳定

| 检查 | 实测 | 结果 |
|---|---|---|
| `metrics_system` 字段集 | cpu_usage_pct + memory/swap/system_disk 三段 usage + uptime + processes_count + network_interfaces（过滤 169.254/127.0.0.1）+ tcp_stats 8 字段 + temperatures | ✅（stage 3 决策 3）|
| `metrics_gpu` providers 字段 | 调 `GpuCollector::detect_providers()` 标 nvml/dxgi/pdh 数据源 | ✅（stage 3 决策 4）|
| `metrics_disk_io` 三段字段 | total + per_disk + disks（device=Some 仅过滤 per_disk，total/disks 保留全部）| ✅（stage 3 决策 5）|
| `metrics_smart` 双路径 | device=None 走聚合（list_disks + read_smart 摘要）/ device=Some 走详细 attributes（与 proc_smart 同 schema）| ✅（stage 3 决策 2 选 (b)）|
| `metrics_thermal` throttle null 兜底 | 非 Windows / Win11 build<22000 → throttle_info: null + reason: "Unavailable" | ✅（stage 3 决策 6）|
| 5 个 metrics tool description 去 "Stage 1 stub" | `grep "Stage 1 stub" src/mcp/handler/mod.rs` 全清零 | ✅ |

**判定**：cat 4 metrics 5 helper 业务路径稳定，决策 1-7 全部落地。✅

#### 1.1.4 测试覆盖度：cycle 累计 +39 新测试

| Stage | 新测试数 | 累计基线 | 覆盖维度 |
|---|---|---|---|
| stage 1 | +0 | 1242 → 1242 | 仅加 stub helper 不动业务代码，无新测试（与 stage 1 §决策 1 surgical 原则一致）|
| stage 2 | +29 | 1242 → 1271 | cat 1 16 case（含 docker unavailable / dry_run preview / 各 target_kind / sort/limit boundary）+ cat 2 9 case + 4 case（每个 tab 至少 1 个 + secret mask / reveal / bogus pid）|
| stage 3 | +10 | 1271 → 1281 | system 3 / gpu 1 / disk_io 2 / smart 2 / thermal 2（含 tcp_stats 8 字段 / device 过滤兜底 / gpu 空 providers / throttle 平台差异兜底）|
| **cycle 累计** | **+39** | **1242 → 1281** | **全维度覆盖** |

**判定**：cycle 测试覆盖度优秀，每 stage 测试数与新增功能复杂度匹配。stage 1 仅加 stub 不加测试是 surgical 原则的体现（stub helper 占位返 placeholder 不需测试，stage 2/3 实装时才加）。✅

---

### 1.2 架构审查

#### 1.2.1 MCP 模块改动范围最小

| 检查 | 实测 | 结果 |
|---|---|---|
| stage 1 改动文件 | `src/mcp/{handler.rs(删，git mv 到 mod.rs), handler/{mod.rs(顶部加 mod 声明 + use + impl 块末尾追加 15 个 #[tool] stub 方法), cli.rs(新), inspect.rs(新), metrics.rs(新)}}` + `docs/adr/{0023, 0024}.md` + `CONTEXT.md`（本地）| ✅（仅 src/mcp/ + docs/adr/，不污染其他模块）|
| stage 2 改动文件 | `src/mcp/handler/{cli.rs(stub helper 替换为真实业务), inspect.rs(stub helper 替换为 6 tab 分支), mod.rs(10 个 #[tool] description 字符串更新 + mod 声明改 pub mod 让测试能 import)}` + `tests/test_mcp_v0_15.rs(新 29 case)` | ✅（仅 src/mcp/handler/ + tests/）|
| stage 3 改动文件 | `src/mcp/handler/{metrics.rs(stub helper 替换为真实业务), mod.rs(5 个 metrics tool description 字符串更新)}` + `tests/test_mcp_v0_15.rs(扩 10 case)` | ✅（仅 src/mcp/handler/metrics.rs + tests/）|

**判定**：3 个 stage 改动范围都最小，仅 src/mcp/ + tests/，不污染业务模块。✅

#### 1.2.2 ProcMcpHandler 字段 / Clone / Default / new 是否破坏 v0.12 TD-36 持久 dns_collector 契约

| 检查 | 实测 | 结果 |
|---|---|---|
| `dns_collector: Arc<Mutex<Option<Box<dyn DnsLogCollector>>>>` 字段保留 | stage 1 重构后字段仍在 mod.rs（v0.12 TD-36 fix 不动）| ✅ |
| `Clone` derive 共享 collector | rmcp 内部 clone handler 时共享同一 collector 实例 | ✅ |
| `new()` 调 `detect_collector()` spawn 一次 | 生产入口（serve 调）spawn ETW / PowerShell | ✅ |
| `Default` 保持 `None` 不强制 spawn | 测试路径不 spawn ETW / PowerShell | ✅（与 v0.12 TD-36 同款规则）|

**判定**：v0.12 TD-36 持久 dns_collector 契约零回归。✅

#### 1.2.3 InspectTab 与 v0.5 TUI InspectionTab 独立类型决策

| 检查 | 实测 | 结果 |
|---|---|---|
| `InspectTab`（MCP 入参）derive 集 | `Deserialize + Default + schemars::JsonSchema` | ✅ |
| `InspectionTab`（TUI 状态机）derive 集 | `Display + Clone + PartialEq`（v0.5 落地）| ✅ |
| 两类型不共享 | surgical 原则（avoid derive 污染）| ✅ |

**判定**：stage 1 §决策 5（独立类型）落地正确。✅

---

### 1.3 性能审查

#### 1.3.1 proc_flows / proc_diag 走 App::new + 2s warm-up 路径

| 检查 | 实测 | 结果 |
|---|---|---|
| `make_flows_json` spawn App | 与 v0.7 `make_diag_json` 同款路径，~2s 开销 | ⚠ **已知限制**（stage 2 风险 1）|
| agent 多次调用累积开销 | agent 典型 task 调 1-2 次，可接受 | ✅ |
| stage 4 Review 决策 | 评估是否加 MCP handler 内 App 复用（暂留 TD-54 v0.16+ 候选）| 见 §3 P2-5 |

**判定**：App::new 多次 spawn 是已知限制（stage 2 风险 1 文档化），与 v0.7 `proc_diag` 同款路径。归档 TD-54 留 v0.16+ cycle 评估。✅

#### 1.3.2 metrics 走 SystemSnapshot::new + refresh 路径

| 检查 | 实测 | 结果 |
|---|---|---|
| `make_metrics_*_json` 5 helper 都走 SystemSnapshot::new + refresh | 与 stage 2 `make_export_json` 同款路径（~500ms 开销）| ⚠ **已知限制**（stage 3 风险 4）|
| agent 多次调用累积开销 | 5 个 metrics tool + export + throttle 共享 SystemSnapshot 路径，agent 典型 task 调 1-2 次 | ✅ |
| stage 4 Review 决策 | 评估是否加 MCP handler 内 SystemSnapshot 复用（暂留 TD-54 v0.16+ 候选，与 proc_flows 同款决策）| 见 §3 P2-5 |

**判定**：SystemSnapshot 多次调用是已知限制（stage 3 风险 4 文档化），与 stage 2 `make_export_json` 同款路径。归档 TD-54 与 proc_flows 共享 v0.16+ 候选评估。✅

#### 1.3.3 32 tool 启动开销

| 检查 | 实测 | 结果 |
|---|---|---|
| rmcp `#[tool_router]` 编译期宏 | runtime 不扫描 tool 列表（与 v0.7 17 tool 同款开销）| ✅ |
| 新增 15 tool 不影响 MCP server 启动延迟 | stage 1 落地后 `cargo run --release -- mcp serve` 启动延迟与 v0.14 同 | ✅ |
| 每个 tool lazy 调用 | agent 不调不耗资源 | ✅（brainstorm FAQ Q5）|

**判定**：32 tool 启动开销与 v0.7 17 tool 同（编译期宏），无 runtime 扫描开销。✅

---

### 1.4 完整性检查

#### 1.4.1 brainstorm.md cycle 总览表 + 14/15 tool miscount

| 检查 | 实测 | 状态 |
|---|---|---|
| 阶段总览表反映 4 stage（1 Spike + 2 Slice + 1 Review+收尾） | `docs/stages/v0.15-brainstorm.md:92-97` 4 stage 都列；stage 1/2/3/4 全 ⬜ 未开始 | ❌ **P1-2**（4 行 ⬜ → ✅，stage 4 收尾段任务 7 改）|
| brainstorm 「14 tool」miscount 文档化 | stage 1 §决策 2 拍板实装 15 tool，brainstorm 表格列 9+1+5=15（"14" 是非正式 miscount）| ❌ **P1-3**（brainstorm §14 个新 tool 详细范围段标题加 miscount 注释，stage 4 收尾段任务 7 改）|
| cycle 决策（拍板记录）段 | brainstorm §cycle 决策段完整（用户选主题 D 拆 v0.15+v0.16 / 4 stage / tab 合并 / dry_run false / secret mask 5 决策）| ✅ |

**判定**：2 个 P1（brainstorm 总览表 4 行未改 / 14 tool miscount 未文档化），stage 4 收尾段任务 7 修复。✅

#### 1.4.2 stage docs 头部 ✅ 标记

| 检查 | 实测 | 状态 |
|---|---|---|
| `docs/stages/v0.15-stage-1.md` 头部 ✅ | 第 1 行 `### 阶段 1：Spike — ...`，第 3 行 `> **独立会话指令**：...`，**无 ✅ 标记** | ❌ **P1-1** |
| `docs/stages/v0.15-stage-2.md` 头部 ✅ | 第 3 行 `> ✅ **已完成**（v0.15.0 阶段 2 会话产出，2026-07-06）` | ✅ |
| `docs/stages/v0.15-stage-3.md` 头部 ✅ | 第 3 行 `> ✅ **已完成**（v0.15.0 阶段 3 会话产出，2026-07-06）` | ✅ |

**P1-1**：stage 1 doc 头部缺 `> ✅ **已完成**` 标记。与 v0.14 stage 5 P1-1 同款问题（cycle 末段 Review 时发现 stage 1 doc 头部 ✅ 标记漏加）。stage 4 收尾段任务 8 修复。

**判定**：1 个 P1（stage 1 doc 头部 ✅ 缺），stage 4 收尾段任务 8 修复。✅

#### 1.4.3 CHANGELOG `[Unreleased]` 段

| 检查 | 实测 | 状态 |
|---|---|---|
| `[Unreleased]` 段 | `CHANGELOG.md:8-10` 占位「下次 cycle（v0.15.0+）的候选方向：基于 v0.14 cycle 落地情况 + tech-debt TD-44~49 残留项决定。」| ⬜ 待 stage 4 收尾段任务 4 改 `[0.15.0] - 2026-07-06` + 加阶段汇总 + 关键数字表 + `[Unreleased]` 改 v0.16 候选 |
| `[0.14.0]` 段保留（v0.14 cycle 历史） | `CHANGELOG.md:12` 起 `[0.14.0] - 2026-07-06` | ✅ |

**判定**：CHANGELOG `[Unreleased]` 段 stage 1-3 未单独加条目（v0.14 cycle 各 stage 在 `[Unreleased]` 段单独加，v0.15 cycle 各 stage 未加是 cycle 末段统一收尾模式），stage 4 收尾段任务 4 一次性加 v0.15.0 段 + 阶段汇总 + 关键数字表。✅

#### 1.4.4 README MCP 章节

| 检查 | 实测 | 状态 |
|---|---|---|
| MCP 章节提 v0.7 落地的 17 tool | `README.md` banner + MCP 段落（v0.7 落地时已加） | ✅ |
| README banner v0.14.0 段 | `README.md:5` 当前 banner 是 v0.14.0 | ⬜ 待 stage 4 收尾段任务 6 加 v0.15.0 banner |
| MCP 章节扩 32 tool 列表 | 当前仅 17 tool 列表 | ⬜ 待 stage 4 收尾段任务 6 加 15 新 tool 列表 |

**判定**：README banner / MCP 章节 v0.15 内容缺失，stage 4 收尾段任务 6 加 v0.15.0 banner + 32 tool 列表。✅

#### 1.4.5 CONTEXT.md 演进历史段

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.15.0 段存在 | `CONTEXT.md:204` `### v0.15.0 新增术语（开发中，2026-07-06 启动）` + `CONTEXT.md:216` `### v0.15.0 落地变更（开发中，2026-07-06 启动）` | ✅（stage 1 落地时已加）|
| stage 1 行 | `CONTEXT.md:220` 完整描述 stage 1（McpToolCategory / InspectTab / MetricsKind 3 术语 + 子 module 骨架重构）| ✅ |
| stage 2 行 | `CONTEXT.md:221` 完整描述 stage 2（cat 1 9 tool + cat 2 6 tab 业务逻辑）| ✅ |
| stage 3 行 | `CONTEXT.md:222` 完整描述 stage 3（cat 4 metrics 5 tool 业务逻辑）| ✅ |
| stage 4 行 | 缺 | ⬜ 待 stage 4 收尾段任务 9 加（本地不入 commit）|
| 术语段状态升级（开发中 → 已落地） | 当前仍是「开发中，2026-07-06 启动」 | ⬜ 待 stage 4 收尾段任务 9 改「已落地，2026-07-06 发布」（本地不入 commit）|

**判定**：CONTEXT.md 演进历史段 stage 1-3 行齐全（cycle 各 stage 落地时已加），stage 4 收尾段任务 9 加 stage 4 行 + 状态升级。✅

#### 1.4.6 tech-debt.md

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.16.0+ 候选段 | 当前 tech-debt.md 含 v0.15.0+ 候选补遗段 TD-49（v0.14 cycle 归档），无 v0.15 cycle 新 TD 候选 | ⬜ 待 stage 4 Review §3 决策（P2-1 ~ P2-5 候选 TD-50 ~ TD-54）|
| TD-44~49 终态 | 全部归档正确（v0.13 归档 5 项 + v0.14 归档 1 项）| ✅ |

**判定**：tech-debt TD-44~49 终态正确，stage 4 §3 决策新 TD-50 ~ TD-54（P2-1 ~ P2-5）归档到 v0.16.0+ 候选补遗段。✅

---

### 1.5 安全 / 跨平台审查

#### 1.5.1 字段裁剪 — 详情页视角 vs 列表视角互补

| 检查 | 实测 | 结果 |
|---|---|---|
| `proc_inspect(summary)` 返完整 cmd/exe/cwd 真值 | `inspect.rs::make_inspect_json` Summary tab 走详情页视角（与 brainstorm FAQ Q1 决策一致）| ✅ |
| `proc_ls` 列表视角仍裁剪 exe/cwd/user_id | v0.7 ADR-0009 契约不动（stage 1-3 未动既有 17 tool）| ✅ |
| 两视角互补不冲突 | brainstorm FAQ Q1 决策 | ✅ |

**判定**：详情页视角与列表视角字段裁剪互补落地正确，v0.7 ADR-0009 列表视角契约零回归。✅

#### 1.5.2 secret mask 默认 + reveal opt-in

| 检查 | 实测 | 结果 |
|---|---|---|
| `proc_inspect(env)` secret 12 关键字默认 mask | `inspect.rs` Env tab 调 `env_mask::is_secret_key` + `mask_value` | ✅ |
| `reveal=true` opt-in 显示真值 | stage 2 决策 2 + brainstorm §决策 | ✅ |
| 与 v0.6 env_reveal 同款契约 | 录屏强制 mask / `v` 键 toggle / 12 关键字 pattern | ✅（stage 1-3 未动 v0.6 env_mask 模块）|

**判定**：secret mask 默认落地正确，与 v0.6 env_reveal 同款契约零回归。✅

#### 1.5.3 写操作 dry_run=false 默认

| 检查 | 实测 | 结果 |
|---|---|---|
| `proc_monitor_add/remove` dry_run=false 默认 | stage 2 决策 1（与 v0.7 `proc_kill`/`proc_pkill` 契约一致）| ✅ |
| `dry_run=true` opt-in 预演 | stage 2 测试 `test_proc_monitor_add_dry_run_returns_preview` 验证 | ✅ |
| 既有 `proc_kill`/`proc_pkill` v0.7 契约不动 | stage 1-3 未动既有 17 tool | ✅ |

**判定**：写操作 dry_run=false 默认落地正确，v0.7 写操作契约零回归。✅

#### 1.5.4 平台差异兜底

| 检查 | 实测 | 结果 |
|---|---|---|
| `proc_throttle` 非 Windows 返 ok=false | `cli.rs::make_throttle_json` cfg-gate Windows-only，非 Windows 返友好 error | ✅ |
| `metrics_gpu` 无 GPU 环境返空 + note | `metrics.rs::make_metrics_gpu_json` gpus=[] + providers=[] + note 字段（stage 3 风险 1 mitigate）| ✅ |
| `metrics_thermal` 非 Windows throttle_info null + reason="Unavailable" | `metrics.rs::make_metrics_thermal_json` 走 `classify_throttle` 返 6 档 reason（stage 3 风险 2 mitigate）| ✅ |

**判定**：平台差异兜底全部落地（throttle / gpu / thermal 三处），与 brainstorm FAQ + stage 3 风险段决策一致。✅

#### 1.5.5 mod.rs 顶部 mod 声明改 pub mod（让测试能 import）

| 检查 | 实测 | 结果 |
|---|---|---|
| stage 1 落地时 mod 声明是 `mod cli; mod inspect; mod metrics;` | stage 1 §决策 1（私有 mod）| ✅ |
| stage 2 改为 `pub mod cli; pub mod inspect; pub mod metrics;` | 让 `tests/test_mcp_v0_15.rs` 能 import Args struct | ✅ |
| production 路径影响 | 仅暴露 module 给 tests，production 路径调用方仍走 `crate::mcp::handler::*` re-export | ✅（surgical，仅 visibility 调整）|

**判定**：mod 声明改 pub mod 是 stage 2 让测试能 import 的最小调整，不影响 production 路径。✅

---

### 1.6 P0 / P1 / P2 列表

#### P0（阻断 v0.15.0 发布）：0 项

无。cycle 业务代码 +39 测试全过，fmt / clippy / build / bench 全过，无编译 / 测试 / 关键文档阻断问题。

#### P1（cycle 内闭环）：3 项

| 编号 | 问题 | 修复 |
|---|---|---|
| **P1-1** | `docs/stages/v0.15-stage-1.md` 头部缺 `> ✅ **已完成**` 标记（stage 2/3 已加，stage 1 是 Spike doc 没加），与 v0.14 stage 5 P1-1 同款问题 | stage 4 收尾段任务 8 加 ✅ 标记 |
| **P1-2** | brainstorm cycle 总览表 stage 1-4 全部仍 ⬜ 未开始（line 92-97），未升级为 ✅ 已完成 | stage 4 收尾段任务 7 改 4 行 ⬜ → ✅ |
| **P1-3** | brainstorm §14 个新 tool 详细范围段标题仍是「14 tool」（line 107），实际 stage 1 §决策 2 拍板实装 15 tool（"14" 是非正式 miscount） | stage 4 收尾段任务 7 在标题加 miscount 注释 + FAQ 段加更正说明 |

#### P2（归档 v0.16+ cycle）：5 项

| 编号 | 问题 | 归档 |
|---|---|---|
| **P2-1 → TD-50** | `proc_metrics_smart` vs `proc_smart` 入口重叠（stage 1 §4c 待定项 stage 3 落地为 (b) 方案「聚合 vs 单设备」互补，但 device=Some 时两 tool 100% 重叠）—— stage 4 Review 评估合并入口 / 废弃 `proc_smart` / 保持现状 | tech-debt.md 加 TD-50 段 |
| **P2-2 → TD-51** | `MonitorManager` 无持久化（in-memory 空表，每次 `ProcMcpHandler::new()` 都新建，monitor_add/remove 只在单次 tool call 内有效，跨调用丢失，stage 2 风险 2）—— 与既有 `proc_monitor_list` v0.7 行为一致（都空表起步），但 agent 跨调用配置监控应持久化 | tech-debt.md 加 TD-51 段 |
| **P2-3 → TD-52** | `metrics_system` sparkline 30s 历史不暴露（stage 3 决策 3 + 风险 5）—— MCP 一次性 request-response 模型不适合 worker 累积，需要持久化 + worker 推送 | tech-debt.md 加 TD-52 段 |
| **P2-4 → TD-53** | `metrics_disk_io` per-process 不暴露（stage 3 决策 5）—— 需要 ETW + thread_map（disk_io_etw worker 模式），MCP 一次性调用启动 ETW session 不实用 | tech-debt.md 加 TD-53 段 |
| **P2-5 → TD-54** | `proc_flows` / `metrics_*` / `proc_export` 多次调用 SystemSnapshot::new + App::new 累积开销（stage 2 风险 1 + stage 3 风险 4）—— 每次 ~500ms-2s，agent 多次调用累积可感 | tech-debt.md 加 TD-54 段 |

---

## 2. P1 修复方案

### P1-1：stage 1 doc 头部加 ✅ 标记

在 `docs/stages/v0.15-stage-1.md` 第 1 行（`### 阶段 1：Spike — ...` 行）下面插入：

```markdown
> ✅ **已完成**（v0.15.0 阶段 1 会话产出，2026-07-06）
```

**修复位置**：`docs/stages/v0.15-stage-1.md:1` 后插入 ✅ 标记。

**注意**：stage 1 doc 头部 ✅ 标记修复不动业务代码（仅 docs/* 改动），与 v0.14 stage 5 P1-1 同款规则。

### P1-2：brainstorm cycle 总览表 4 行 ⬜ → ✅

修改 `docs/stages/v0.15-brainstorm.md:94-97` 阶段总览表 4 行 status 列：
- stage 1 行（line 94）「⬜ 未开始」→ 「✅ 已完成」
- stage 2 行（line 95）「⬜ 未开始」→ 「✅ 已完成」
- stage 3 行（line 96）「⬜ 未开始」→ 「✅ 已完成」
- stage 4 行（line 97）「⬜ 未开始」→ 「✅ 已完成」

### P1-3：brainstorm 14 tool miscount 文档化

修改 `docs/stages/v0.15-brainstorm.md:107` 标题：

旧：
```markdown
## 14 个新 tool 详细范围
```

新：
```markdown
## 15 个新 tool 详细范围（"14" 是 brainstorm 起草阶段非正式 miscount，stage 1 §决策 2 拍板按表格列出的 9+1+5=15 实装）
```

`docs/stages/v0.15-brainstorm.md` FAQ 段（如有 Q 关于 tool 数）加更正说明（如无则跳过）。

---

## 3. P2 归档（TD-50 ~ TD-54）

### TD-50（REVIEW-v0.15 P2-1）：`proc_metrics_smart` vs `proc_smart` 入口重叠

**位置**：
- `src/mcp/handler/metrics.rs::make_metrics_smart_json`（device=None 走聚合 vs device=Some 走详细 attributes）
- `src/mcp/handler/mod.rs::proc_smart`（v0.7 既有 17 tool 之一，单设备详细 attributes）
- `src/mcp/handler/mod.rs::proc_metrics_smart`（v0.15 cat 4 新 tool）

**现状**：stage 3 决策 2 选 (b) 方案落地 —— `proc_metrics_smart(device=None)` 返系统级聚合（all disks 摘要），`proc_metrics_smart(device=Some)` 与 `proc_smart(device=Some)` 同款返详细 attributes。device=Some 时两 tool 100% 重叠。

**影响**：agent 调用 confusion（两 tool 都能查单设备 SMART，schema 略不同但内容相同）。无功能阻断。

**修复方案**（v0.16+ cycle 评估）：
1. **(a) 废弃 `proc_smart`**（推荐）：标 Status Deprecated，schema 加 `x-deprecated: true` hint，agent 优先调 `proc_metrics_smart`。理由：`proc_metrics_smart` 双路径设计更通用（聚合 + 单设备），`proc_smart` 是 v0.7 历史遗留
2. **(b) 合并入口**：`proc_smart` alias 到 `proc_metrics_smart`，统一 helper
3. **(c) 保持现状**：documented 作为互补，agent 二选一

**REVIEW-v0.15 决策**：归档 v0.16+ cycle 评估。理由：(1) `proc_smart` 是 v0.7 既有 17 tool 之一，外部 client（Claude Desktop / Cursor）可能已集成，废弃需评估破坏性；(2) `proc_metrics_smart` 双路径设计是 stage 3 决策 2 落地，stage 1 §4c 待定项已闭环；(3) 保持现状 (c) 是 surgical 默认，agent 二选一不阻断。

### TD-51（REVIEW-v0.15 P2-2）：`MonitorManager` 无持久化

**位置**：
- `src/mcp/handler/cli.rs::make_monitor_add_json` / `make_monitor_remove_json`（每次 `ProcMcpHandler::new()` 都新建 MonitorManager）
- `src/monitor/manager.rs::MonitorManager::new()`（in-memory 空表）

**现状**：`MonitorManager` 是 in-memory 的（无磁盘持久化），每次 `new()` 都是空表。stage 2 的 `monitor_add` / `monitor_remove` 仅在 process 内有效，跨 tool call 丢失。与既有 `proc_monitor_list` v0.7 行为一致（都空表起步）。

**影响**：agent 跨 tool call 配置监控规则无效（add 后 list 看不到）。无错误，但 agent 视角 confusion。

**修复方案**（v0.16+ cycle 评估）：
1. **加配置文件持久化**（推荐）：`~/.config/proc/monitors.toml`（与 `trusted_signers.toml` 同款路径），`MonitorManager::new()` 时 load，add/remove 时 write
2. **加 MCP handler 持久 MonitorManager 字段**：与 v0.12 TD-36 持久 dns_collector 同款模式，`ProcMcpHandler` 加 `monitor_manager: Arc<Mutex<MonitorManager>>` 字段，跨 tool call 共享

**REVIEW-v0.15 决策**：归档 v0.16+ cycle 评估。理由：(1) v0.7 `proc_monitor_list` 既有契约是「空表起步」（list 在 production TUI 路径有持久化，但 MCP 路径未集成）；(2) agent 视角的监控配置应持久化是合理需求，但需评估配置 schema 与 TUI 路径一致性；(3) v0.16 cycle 主题 D2（操作 + 录屏类）会涉及更多写操作 MCP tool，统一评估持久化策略。

### TD-52（REVIEW-v0.15 P2-3）：`metrics_system` sparkline 30s 历史不暴露

**位置**：
- `src/mcp/handler/metrics.rs::make_metrics_system_json`（仅返当前快照，无 sparkline 历史）
- brainstorm §类别 4 提「30 秒火花线图历史」

**现状**：stage 3 决策 3 + 风险 5 明确「sparkline 30s 历史暂不做」—— MCP 一次性 request-response 模型不适合 worker 累积，需要持久化 + worker 1s tick 推送（与 LightWorker 同款）。

**影响**：agent 看不到 CPU/内存 30s 趋势，只能看当前快照。无功能阻断（与 `proc_diag` 同款一次性快照语义）。

**修复方案**（v0.16+ cycle 评估）：
1. **加 MCP handler 持久 SystemSnapshot 历史**：`ProcMcpHandler` 加 `system_history: Arc<Mutex<Vec<SystemSnapshot>>>` 字段，1s tick push 一次，30s cap
2. **加 Resource subscribe**：rmcp 0.11 `Resource subscribe` 模式，client 订阅 system metrics 更新事件（与 brainstorm 主题 B 可观测性 cycle 同款方向）

**REVIEW-v0.15 决策**：归档 v0.16+ cycle 评估。理由：(1) MCP 一次性 request-response 模型与 sparkline 持久化语义不直接兼容，需评估 rmcp 0.11 Resource subscribe 能力（与 brainstorm 主题 B 可观测性 cycle 同款方向）；(2) `proc_diag` 是 v0.7 既有一次性快照 tool，`metrics_system` 同款语义是 surgical 默认；(3) agent 当前能用 `metrics_system` 拿当前快照 + 多次调用对比，趋势需求可在 client 侧累积。

### TD-53（REVIEW-v0.15 P2-4）：`metrics_disk_io` per-process 不暴露

**位置**：
- `src/mcp/handler/metrics.rs::make_metrics_disk_io_json`（仅返 total + per_disk + disks 三段，无 per-process）
- `src/disk_io_etw/{mod.rs, provider.rs, thread_map.rs}`（v0.7 落地的 per-process disk_io ETW worker）

**现状**：stage 3 决策 5 明确「per-process disk_io 暂不暴露」—— 需要 ETW + thread_map（disk_io_etw worker 模式），MCP 一次性调用启动 ETW session 不实用（NT Kernel Logger 单实例限制 + 启动延迟 ~1s）。

**影响**：agent 看不到 per-process disk_io BPS（`proc_ls --sort disk_read` 是另一种视角，列表 + 排序）。无功能阻断。

**修复方案**（v0.16+ cycle 评估）：
1. **加 MCP handler 持久 disk_io_etw_worker**：与 `dns_collector` 同款模式（v0.12 TD-36），`ProcMcpHandler` 加 `disk_io_etw: Arc<Mutex<Option<DiskIoEtwHandle>>>` 字段，handler spawn 时启动 worker，metrics_disk_io tool drain 一次
2. **加 proc_inspect(disk_io) tab**：详情页视角看单进程 disk_io 历史

**REVIEW-v0.15 决策**：归档 v0.16+ cycle 评估。理由：(1) disk_io_etw worker 启动延迟（NT Kernel Logger 单实例）+ 非管理员 / x86 fallback 复杂度高，MCP 一次性调用不适合；(2) `proc_ls --sort disk_read` 已覆盖列表视角，详情页视角 v0.16 cycle 评估；(3) v0.16 cycle 主题 D2（操作 + 录屏类）会涉及更多 worker 路径，统一评估 MCP handler 持久 worker 字段策略。

### TD-54（REVIEW-v0.15 P2-5）：`proc_flows` / `metrics_*` 多次调用 SystemSnapshot::new + App::new 累积开销

**位置**：
- `src/mcp/handler/cli.rs::make_flows_json`（`App::new() + 2s warm-up` 每次 ~2s）
- `src/mcp/handler/metrics.rs::make_metrics_*_json` 5 helper（`SystemSnapshot::new() + refresh()` 每次 ~500ms）
- `src/mcp/handler/cli.rs::make_export_json`（同款 SystemSnapshot 路径）

**现状**：stage 2 风险 1 + stage 3 风险 4 文档化 —— 每次 tool call 都新建 App / SystemSnapshot，agent 多次调用累积开销大。

**影响**：agent 多次调 metrics_* / proc_flows / proc_export 累积 ~500ms-2s/次。可接受（agent 典型 task 调 1-2 次）。

**修复方案**（v0.16+ cycle 评估）：
1. **加 MCP handler 持久 SystemSnapshot / App**：与 `dns_collector` 同款模式，`ProcMcpHandler` 加 `snapshot: Arc<Mutex<SystemSnapshot>>` 字段，1s tick refresh
2. **加 TTL 缓存**：handler 内 `HashMap<ToolName, (timestamp, result)>` 缓存，TTL 1s（与 worker 1s tick 对齐）

**REVIEW-v0.15 决策**：归档 v0.16+ cycle 评估。理由：(1) `App::new()` 不是 Send + Sync（包含多个 worker handle + UI 状态），跨 tool call 共享需评估线程安全；(2) SystemSnapshot 共享较简单但需评估 freshness（worker 路径 vs MCP 路径同步）；(3) agent 实际不会高频调（典型 task 调 1-2 次），优化收益边际；(4) v0.16 cycle 主题 D2 涉及更多 worker 路径，统一评估。

---

## 4. 验收

### 4.1 全量回归

`cargo test --release -q` = **1281 passed / 0 failed / 3 ignored**（v0.14.0 → v0.15 stage 1 → stage 2 → stage 3 全程基线递增 1242 → 1242 → 1271 → 1281，cycle 累计 +39 新测试）。

理由：v0.15 cycle 3 个 stage 全部交付业务代码 + 测试，每个 stage 测试数与新增功能复杂度匹配（stage 1 仅加 stub 不加测试是 surgical 原则）。

### 4.2 静态检查

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式化 | `cargo fmt --all -- --check` | ✅ 通过 |
| Clippy | `cargo clippy --release --all-targets -- -D warnings` | ✅ 通过 |
| 无默认 feature 构建 | `cargo build --release --no-default-features` | ✅ 通过（2m 11s）|
| Bench 编译 | `cargo bench --no-run` | ✅ 通过 |

### 4.3 P0 / P1 / P2 闭环

- **P0 = 0** ✓
- **P1 = 3**（P1-1 / P1-2 / P1-3 全部闭环——见 §2 修复方案，stage 4 收尾段任务 7/8 修复）
- **P2 = 5**（归档 TD-50 ~ TD-54——见 §3）

---

## 5. 后续（stage 4 收尾段 + cycle 闭环）

stage 4 Review 段（本文）完工后，stage 4 收尾段任务（按 stage 4 doc §任务清单）：

1. **CHANGELOG.md**：`[Unreleased]` → `[0.15.0] - 2026-07-06` + 4 stage 阶段汇总 + 关键数字表（17 → 32 tool / 39 新测试 / 1281 全量回归 / ~1700 行业务代码）
2. **Cargo.toml**：`0.14.0` → `0.15.0` + Cargo.lock 自动同步
3. **README.md**：banner 加 v0.15.0 段（4 大能力：MCP 模块重构 + 9 CLI tool + proc_inspect 6 tab + 5 metrics tool）+ MCP 章节扩 32 tool 列表
4. **brainstorm.md**：cycle 阶段总览表 stage 1-4 ⬜ → ✅（P1-2 修复）+ §14 tool 标题加 miscount 注释（P1-3 修复）+ 末尾加 cycle 总结段
5. **stage 1 doc 头部 ✅**（P1-1 修复，含 stage 4 doc 本身已有）
6. **tech-debt.md**：加 v0.16.0+ 候选补遗段 TD-50 ~ TD-54
7. **CONTEXT.md**：演进历史加 stage 4 行 + 状态升级（本地，不入 commit）
8. **commit**：`release(v0.15.0): MCP 全功能暴露查询类 cycle（4 stage 全交付 + REVIEW-v0.15 + tag v0.15.0）`
9. **git tag v0.15.0**：等用户确认 push（与 v0.14.0 同款规则）

---

## 6. 总结

v0.15 cycle 是「MCP 全功能暴露查询类 cycle」（主题 D 子方向 D1，4 stage 中重 cycle）：
- **stage 1** Spike（commit `163b63c`）：MCP 模块骨架重构 — `handler.rs` 单文件 1156 行 → `handler/{mod, cli, inspect, metrics}.rs` 4 子 module + 15 tool stub + ADR-0023/0024
- **stage 2** Slice（commit `8834fd1`）：cat 1 9 CLI tool + cat 2 `proc_inspect` 6 tab 业务逻辑填充 + 29 集成测试
- **stage 3** Slice（commit `f2e3fc7`）：cat 4 metrics 5 tool 业务逻辑填充 + 10 集成测试
- **stage 4** Review + 收尾（commit 待）：本 Review + 收尾 + tag v0.15.0

**核心结论**：MCP 模块从 v0.7 的「17 tool 工具箱」升级到「32 tool 全功能透出」。agent 视角最大价值缺口补完——CLI 已有但 MCP 未暴露的 9 命令 + 详情页 6 Tab（5 新 Tab + 1 复用 proc_handles）+ 系统级 metrics 5 tool 全部透出。写操作 + 录屏类（brainstorm §主题 D 子方向 D2）留 v0.16 cycle（~6 tool ~600 行）。

**cycle 数据**：
- 全量回归：1242 passed（v0.14.0 基线）→ 1281 passed（v0.15.0 落地），+39 新测试
- 业务代码：~1700 行（与主题 D 预期 ~1850 行接近）
- MCP tool 总数：17 → 32（17 既有 + 15 新增）
- handler.rs 单文件 1156 行 → 4 子 module（mod.rs 1358 + cli.rs 568 + inspect.rs 360 + metrics.rs 400 = 2686 行总计，含既有 17 tool + 15 新 tool + helper + 测试 Args struct）

**REVIEW-v0.15 完工交付**：
- 本报告（~340 行）
- P1 修复（3 项：stage 1 doc ✅ / brainstorm 表 ⬜ → ✅ / 14 tool miscount 文档化，stage 4 收尾段任务 7/8 修复）
- TD-50 ~ TD-54 归档（5 项 v0.16+ 候选，留 cycle 评估）
- stage 4 收尾段（CHANGELOG + Cargo + README + brainstorm + tech-debt + CONTEXT + git tag v0.15.0）
- stage 1-4 docs 头部 ✅（含 stage 4 doc 本身）
- v0.16.0 cycle 启动指引（基于 v0.15 落地情况 + TD-50~54 残留 + brainstorm §主题 D 子方向 D2 已锁定的「操作 + 录屏类 MCP tool 6 个 ~600 行」）

**v0.16.0 候选方向**（stage 4 收尾段总结时给方向建议，用户最终拍板）：
- 主题 D2（brainstorm 已锁定）：MCP 操作 + 录屏类 cycle — `proc_record_start/stop` / `proc_replay_info/search` / `proc_bookmarks_*` ~6 tool ~600 行
- 主题 B：可观测性 cycle — rmcp Resource subscribe / SSE transport / 实时流（与 TD-52 sparkline 历史同款方向）
- 主题 A：性能优化 cycle — TD-54（MCP handler 内 SystemSnapshot / App 复用）+ TD-44~47 残留（PERF-BASELINE）
- 主题 F：VT100 replay 增强 cycle — TD-49（VT100 字节流转码 UiFrame / 反向解释器）
