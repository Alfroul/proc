# ADR-0035：TD 清仓 + session 语料卫生（v0.25 维护型轻 cycle 设计定稿）

**Status**: Accepted（v0.25 stage 1 Spike 落地——D1~D4 四终判 + 空会话机制归因 + MCP 持久化现状核查 + TD 逐项终判表）

**Date**: 2026-08-27（v0.25 cycle stage 1 Spike）

**Related**: ADR-0019（worker restart policy——TD-25 文档补段对象）、ADR-0026（v0.17 MCP 持久化基础设施——TD-52/54 已落地的原设计）、ADR-0032（SessionRecorder——治理对象的数据结构）、ADR-0034（RAG——成功段状态机口径来源）

## Context

v0.25 是主线条件未齐窗口期的**维护型轻 cycle**（brainstorm 决策 5：TD 清仓 + session 语料卫生；零挂机）。打包清单三组：① 空会话治理（主项）② 边角 TD 清仓（TD-24/40 实装 + TD-25/34 文档）③ MCP 持久基础设施 + 观测补全（TD-50/52/53/54）。

**Spike 动机**（v0.24 附录 B 范式延续——先实测现状再定方案）：

1. 空会话「96 文件 94 空」现象拍板时只有文件级实证，缺源码级归因（哪条路径落盘、哪条不产生 query 事件）——治理方案（延迟创建 vs 退出清理）的选型前置。
2. MCP 持久化三项（TD-52/53/54）在 tech-debt.md 记为 open，但 v0.17 cycle（ADR-0026）可能已部分落地——实装清单必须先核查现状，避免重复实装。
3. TD-53 有预登记判定标准（brainstorm 风险 1）：fallback 路径清单 + 单实例冲突场景可枚举则做，否则砍。

**Spike 实测两个关键发现**（详见 stage-1 doc「Spike 调查发现」段）：

- **空会话归因修正**：ask/eval 走 `build_runner` 不建 recorder，**不产生任何 session 文件**；空文件唯一来源是 TUI AgentPanel 进面板即建 session 落盘（`enter_agent_session` → `build_session` → `SessionRecorder::start` 构造即 `File::create` + SessionStart 单行）。brainstorm「与 eval run 时间吻合」是相关非因果（同一开发时段用户反复开 TUI 进出 Agent 面板）。实测 102 文件 / 100 空（98%），成对出现间隔 15-30s，全 `-llama-cpp` 后缀。
- **TD-52/54 已落地**：v0.17 stage 3/4 在 feature `mcp-persistent-state`（**默认启用**）下实装了 `snapshot` 持久字段 + `run_snapshot_worker` 1s tick + `system_history` 30s cap + `proc_metrics_history` tool——tech-debt.md 状态回填滞后。stage 3 的 MCP 组实装余量仅剩 TD-50 + TD-53。

## Decision

### D1：空会话治理——延迟创建（首个非 session_start 事件才落盘）✅

**两路对比**：

| 方案 | 机制 | 判定 |
|---|---|---|
| **延迟创建** | SessionStart 条目暂存内存；首个非 session_start 事件到达时 `File::create` + 补写 SessionStart 首行 + 写该事件 | ✅ **终判采纳**——单点改动（`RecorderInner` 内部 lazy 化）；天然覆盖 TUI 中途退出（无 query = 无文件）；不依赖 Drop 语义 |
| 退出清理 | 构造即落盘；退出时（Drop / shutdown 钩子）检查无 query 事件则删文件 | ❌ Drop 语义不可靠（`Arc<Mutex>` poison / clone 方 drop 顺序不定 / TUI 强杀路径）；「先落盘再删」窗口内崩溃仍留空文件；改动面更大（session.rs teardown + recorder 两处） |

**实装规格**（stage 2）：

- `RecorderInner` 加 lazy 状态：`pending_start: Option<LogEvent::SessionStart>`（provider / wall_start 暂存）+ 文件延迟打开（`writer` 变 `Option<BufWriter<File>>` 或 `dir + filename` 暂存）
- 触发口径：**首个非 `SessionStart` 事件**（QueryStarted / Error / TextDelta 聚合段 / ToolStart / …）——Error 也落盘（诊断价值：tokio runtime 失败等场景留 2 行文件）
- `is_enabled()` 语义不变（构造成功即 enabled——目录可写性检查保留在构造时，仅文件创建延迟）；写失败静默降级契约不变
- **成功段口径核对**：无 `QueryStarted` 的文件在 RAG corpus 状态机（`corpus.rs:61` QueryStarted 开段）下永远不产出语料 → 治理不影响 `RagIndex` 正确性；全量重建按现存文件，新行为 = 无空文件可读

**stage 2 回归测试口径**（brainstorm 风险 3 mitigate）：

- TUI 路径（AgentPanel 语义）：进面板不发问 → 无文件；发问 → 文件含 SessionStart + QueryStarted（首两行）
- ask / eval 路径：**不落盘是现状而非治理引入**——测试锚定「跑 ask/eval 后 sessions 目录不新增文件」（防未来接线变化时口径漂移）
- 中途退出边界：query 进行中 TUI 退出 → 已落盘文件保留（有 QueryStarted）

### D2：MCP 持久化——TD-52/54 状态回填 + TD-53 改道 sysinfo delta ✅

**现状核查结论**：

| 项 | 现状 | stage 3 动作 |
|---|---|---|
| TD-54（持久 snapshot） | ✅ v0.17 stage 3 已落地：`ProcMcpHandler.snapshot` + `run_snapshot_worker`（`mod.rs:293`）1s tick refresh + refresh_heavy_incremental；feature `mcp-persistent-state` 默认启用 | tech-debt.md 回填 ✅ Fixed（无代码） |
| TD-52（sparkline 30s 历史） | ✅ v0.17 stage 4 已落地：`system_history: VecDeque<MetricsSample>` 30s cap，single worker 兼任 push；`proc_metrics_history` tool 已注册 | tech-debt.md 回填 ✅ Fixed（无代码） |
| TD-50（proc_smart 重叠） | 未落地（无 x-deprecated 痕迹） | 实装：`proc_smart` tool schema 加 `x-deprecated: true` hint + README 注记（~30 行，方案 a——不删 tool 保外部 client） |
| TD-53（per-process disk_io） | 未落地（metrics.rs:189 注释明确暂不暴露） | **改道实装**（见下） |

**TD-53 改道终判**（预登记标准执行）：

- **原方案否决**（handler 持久 disk_io_etw worker，dns_collector 先例模式）：① NT Kernel Logger **全局单实例**——MCP server（`proc mcp`）与 TUI（`proc`）同机并存时互抢 session，后启动者恒失败（dns_collector 先例不可复制：DNS 用非独占 provider，可多实例）；② MCP server 常态非提权运行——ETW 恒 None，实装等于死代码；③ 启动延迟 ~1s + x86 cfg-gate。
- **新方案采纳**（TD-54 落地后解锁）：`run_snapshot_worker` 每 tick 已刷新 `process_cache`（含 `disk_usage` 累计值）——在 worker 内做 **sysinfo delta 计算**（TUI `update_disk_speeds`（`app.rs:1781`）同款：prev tick 的 `disk_usage` 差分 / elapsed）填 `ProcessInfo.disk_read_speed/write_speed` → `make_metrics_disk_io_json` 加 per-process top-N 段（按 read+write 降序，默认 top 10）。
- **预登记标准核对**：fallback 路径清单 ✅ 可枚举（worker 不存在 → helper 走既有 fallback；process_cache 空 → 段为空数组）；单实例冲突场景 ✅ 可枚举（sysinfo 路径无 session 独占，冲突不存在）。
- **精度声明**：sysinfo `disk_usage` 在 Windows 来自 process IO counters（含非磁盘 IO——命名管道等），口径与 TUI 非管理员档一致；响应段加 `source: "sysinfo-delta"` 字段声明口径，agent 可判读。
- **规模**：~60-100 行（worker delta 计算 ~30 + 响应段 ~30 + 测试 ~40）vs 原方案 ~200-300。

### D3：TD 打包项逐项终判表（清单封闭——只砍不加）✅

| TD | 终判 | 落点 | 依据（Spike 实测） |
|---|---|---|---|
| TD-24 | ✅ 做 | stage 2 实装 | `manager.rs:207-213` spawn_one 失败仅不调 on_respawned；`RestartState` 加 `on_respawn_failed(now)` retry_count += 1，MAX_RETRIES 后 permanent_failure 止损 |
| TD-25 | ✅ 做 | stage 1 doc | ADR-0019 追加「不实装 docker worker restart：DockerPanel 自管」段 |
| TD-34 | ✅ Obsolete | stage 1 doc | plan.md 已不在 v0.13+ 流程（brainstorm 替代）；TD 标 Obsolete |
| TD-40 | ✅ 做 | stage 2 实装 | `trusted_signers.rs:74` 裸 `Regex::new`——`RegexBuilder::size_limit(64 * 1024)` + 极端 regex 拒绝测试 |
| TD-50 | ✅ 做 | stage 3 实装 | `proc_smart` 标 `x-deprecated: true` hint（不删 tool） |
| TD-52 | ✅ 回填 | stage 3 doc | v0.17 stage 4 已落地（见 D2 表） |
| TD-53 | ✅ 做（改道） | stage 3 实装 | sysinfo delta 路径（见 D2） |
| TD-54 | ✅ 回填 | stage 3 doc | v0.17 stage 3 已落地（见 D2 表） |
| TD-11 | 维持归档 | — | `watchdog.rs:87` 裸 `Command::new` 仍在；v0.7 决策理由仍立（用户自定义命令威胁模型不同，强制 restricted_spawn 破坏合法用例） |
| TD-20 | 维持归档 | — | 无版本探测代码；v0.10 理由仍立（Win10 1809 已 7+ 年，RtlGetVersion 行为不可靠） |
| TD-21 | 维持归档 | — | `app.rs:1927` overlay 仍单键 pid（start_time 仅新建 flow 填充用）；v0.10 理由仍立（窗口窄 + 影响一次评分） |
| TD-22 | ✅ Fixed 回填 | stage 1 doc | `provider.rs:484` 签名已 `info_buf: &[u8] -> Option<&EVENT_PROPERTY_INFO>`（lifetime elision 正确传播，注释明确「不再撒谎说 'static」）——后续重构中已修未回填 |

**范围影响**：stage 3 业务实装 ~450 → **~100-150 行**（TD-50 + TD-53 改道）；cycle 总规模 ~1900 → **~1100-1200 行级**。无新增项（新发现债一律记 TD 留档——本 Spike 无新发现债：TD-22 属已修项回填非新债）。

### D4：验收锚（零挂机 cycle 的质量论证）✅

- **不变锚**：MCP tool **46**（TD-50 只加 hint 不删；TD-53 是既有 tool 响应扩展）/ agent catalog **47** / Cargo deps **+0**（全部用现有 API：regex 自带 RegexBuilder / sysinfo 既有字段 / serde_json）
- **回归数字递增预期**：stage 2 完工 1725 + N（语料三路径 + TD-24 状态机 + TD-40 regex 测试）；stage 3 完工再 + M（TD-53 worker/响应段测试）；anthropic 档同步 +N/+M
- **tool 语义不变原则**（brainstorm 风险 4）：TD-52/53 只加响应字段不改既有字段含义；`x-deprecated` 是 schema hint 非删除；queries.toml 70q 不涉新字段（sparkline / per-process disk_io 无对应 query）
- **RAG 索引无影响锚**：既有 `tests/test_agent_rag.rs` 全绿即锚（空文件本就无成功段，D1 口径核对见上）

## Consequences

- **正向**：session 语料目录停止空文件增长（98% 噪声消失，TD-59 轮转压力同步缓解）；stage 3 规模砍 2/3（TD-52/54 回填替代实装）；TD-53 避开单实例陷阱（改道方案无特权要求、与 TUI 同机共存）。
- **代价**：延迟创建后 SessionStart 的 wall_start 与文件 mtime 有轻微偏差（构造时刻 vs 首事件时刻——文件名时间戳仍是构造时刻，无实际影响）；TD-53 sysinfo delta 精度低于 ETW（IO counters 含非磁盘 IO，已用 source 字段声明）。
- **中性**：ask/eval 不落盘现状显式化为测试锚（原为隐式行为）；TD-22/34/52/54 四项状态回填让 tech-debt.md 与代码重新对齐。

## stage 2 实装注记（2026-08-30 落地）

- **D1 落地**：`RecorderInner` 持 `path: PathBuf` + `pending_start: Option<(String, String)>`（provider, wall_start）+ `writer: Option<BufWriter<File>>`；物化收敛在 `write_entry` 单点（`File::create` + 补写 SessionStart ts 0/seq 0 + 写当前事件；届时失败清暂存静默放弃）。TextDelta 聚合段经 `flush_pending` → `write_entry` 同样触发物化，与 D1 触发口径一致。
- **TD-24 落地**：`on_respawn_failed(now)` = retry_count `saturating_add(1)` + `last_crash = Some(now)`（backoff 从失败尝试点重新起算；`Some` 保持 `restart_tick` pending 过滤器命中）。生产路径只在 `decide_restart` 为 Some（retry_count < MAX_RETRIES）时被调，计数不越界。mock 口径：非 canonical thread_name 直插 `restart_history` → `spawn_one` 的 `_ => false` 分支确定性失败（不依赖环境，规避 disk-io-etw 需管理员的不确定性）。
- **TD-40 落地**：`RegexBuilder::new(pattern).size_limit(64 * 1024).build()`；测试用 `(a{300}){300}`（展开 ~90K 指令）验证编译期拒绝。
- **实测**：新增 9 测试（三路径 5 + TD-24 3 + TD-40 1），回归双档 1734 / 1758（+9 双档同量，0 failed）；`tests/test_agent_rag.rs` 全绿（D1 口径核对实证——无 QueryStarted 的文件在 corpus 状态机下不产语料，治理不影响索引）；MCP tool 46 / catalog 47 / deps +0 锚在位。

## stage 3 实装注记（2026-08-30 落地）

- **TD-53 落地（D2 改道方案）**：三层接线——① `src/collect.rs` `SystemSnapshot::process_cache_mut()` 可变访问器（`process_cache()` 的对称补全，worker 填 speed 字段的最小开口）；② `src/mcp/handler/mod.rs` `compute_process_disk_speeds(processes, prev, prev_time, now)` 纯函数（TUI `update_disk_speeds` 同款口径：`(pid, start_time)` 键控 `disk_usage` 差分 / elapsed，`saturating_sub` 防 counter 回退，首观测只建基线，尾部无条件重建基线淘汰死亡进程——`pub` 让集成测试合成数据单测）+ `run_snapshot_worker` 局部基线状态（`prev_disk` / `prev_disk_at`——worker 私有不进 handler 字段，风险 2 并发顾虑的规避）+ **只在 `refresh_heavy_incremental` 返 `Ok(true)` 时触发**（pending tick cache 未变，重算会把速度刷成 0——TUI 只在 `Ok(true)` 分支调 update_disk_speeds 的同款理由）；③ `metrics.rs` `metrics_disk_io_json_from_snapshot` 加 `per_process` 段（`process_cache()` 引用按 read+write 饱和和降序 + truncate top-N）+ `MetricsDiskIoArgs.top: Option<usize>`（默认 10）。**附带收益**：`proc_ls` 的 `disk_read_bps/disk_write_bps`（读 `ProcessInfo.disk_read_speed/write_speed`）随 worker 填充同步受益。**接线面**：agent 内部分发 `dispatch.rs` 同步传 `usize_of(args, "top")`；`test_mcp_v0_15.rs` 两处既有调用点机械补第二参。
- **TD-50 落地**：`deprecated_meta() -> rmcp::model::Meta`（`{"x-deprecated": true}`）+ `#[tool(meta = deprecated_meta())]`——rmcp 0.11 `#[tool]` 宏的 `meta` 属性生成 `Tool.meta`，序列化为 MCP 规范官方扩展键 `_meta`。description 的 `[Deprecated]` 文本层 hint（v0.17）不动（v0_17 静态 grep 测试锚定）。**测试升级**：v0.17 用源码 grep 静态断言是因为 `tool_router()` 私有；本 stage 发现宏生成 `pub fn {fn}_tool_attr() -> rmcp::model::Tool` 可从外部直接调用——运行时 schema 断言（`proc_smart_tool_attr().meta` 含 `x-deprecated: true` + `proc_metrics_smart_tool_attr().meta` None 阴性对照）。
- **实测**：新增 8 测试（TD-53 单元 4——差分精确断言（`checked_sub` 构造精确 elapsed）/ 首观测基线 / counter 回退 saturating / 基线重建 + PID 复用键控；TD-53 响应段 2——per_process 结构 / source 口径 / 降序 / top 截断 + 既有字段不缺锚；TD-50 运行时 1；tool 46 运行时锚 1），回归双档 1742 / 1766（+8 双档同量，0 failed，79 行 test result——78 基线 + 1 新测试二进制 test_mcp_v0_25_stage_3）；MCP tool 46 / catalog 47 / deps +0（Cargo.lock 零 diff）锚在位。

## stage 2/3 实装清单预览

**stage 2（Slice A：语料卫生 + 边角清仓，~150 业务 + ~150 测试 + ~100 doc）**：

- `src/agent/session_log.rs`：D1 延迟创建（RecorderInner lazy 化）
- `src/workers/restart.rs` + `manager.rs`：TD-24 `on_respawn_failed`（状态机 + try_respawn 接线）
- `src/security/trusted_signers.rs`：TD-40 `RegexBuilder::size_limit`
- `tests/`：三路径语料回归（TUI 面板语义 / ask/eval 不落盘锚 / 中途退出边界）+ TD-24 状态机单测 + TD-40 拒绝测试

**stage 3（Slice B + Review：~100-150 业务 + ~100 测试 + ~400 doc）**：

- `src/mcp/handler/mod.rs`：TD-53 worker delta 计算（run_snapshot_worker 内）
- `src/mcp/handler/metrics.rs`：TD-53 per-process top-N 段 + source 字段；TD-50 `x-deprecated` hint
- `docs/tech-debt.md`：TD-52/54 回填 + TD-50/53 关闭 + 状态总检
- `README.md`：proc_smart 注记；REVIEW-v0.25 + CHANGELOG + Cargo 0.24.0 → 0.25.0 + tag `v0.25.0`
