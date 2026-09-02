# proc 设计深挖导览（architecture deep-dive）

> **定位**：10 个最值得深挖的设计决策问答地图——每个决策按「是什么 / 为什么这样选 / 边界与反例」三层展开，全部结论可回溯到 ADR / Review / 代码原文（数字与文件路径逐个核对，不自创）。
>
> **读者**：想在 10 分钟内理解 proc 架构取舍的开发者 / 评审者。每条独立可读，顺序无关。
>
> **溯源纪律**：本文每个数字有来源（ADR-0036 D5）。文中 `src/...:行号` 为 v0.26 会话（2026-09-01）核对值；ADR / REVIEW 引用为仓库原文。

---

## 1. PID 复用键控——身份与累计值的分离

**一句话结论**：进程「身份」一律用 `(pid, start_time)` 二元组键控，裸 pid 只用于单 tick 内的易变字段缓存——因为 Windows 会复用 PID，任何跨 tick 的累计 / 归属 / 关联结构用裸 pid 做键都会把死进程的数据算到新进程头上。

**决策与证据**：

- `src/app.rs:181-182`：`prev_process_disk: HashMap<(u32, u64), (u64, u64)>`——TUI 磁盘速度差分基线，注释原文「避免 PID 复用后把死进程的累计 IO 算到新进程头上」
- `src/security/score.rs:417-420`：`invalidate_dead(&HashSet<(u32, u64)>)`——安全评分缓存按 (pid, start_time) 判活失效（`score.rs:50-56` 解析 `{pid}:{start_time}:{exe}` 缓存键）
- `src/mcp/handler/mod.rs:361-394`：MCP snapshot worker 的 `compute_process_disk_speeds` 同款键控（v0.25 TD-53），并有「PID 复用键控」专项单测（`tests/test_mcp_v0_25_stage_3.rs`）
- `src/tui/detail_view.rs:718-727`：Inspector 详情页「还是同一个进程吗」的判定用 (pid, start_time) 元组比对
- `src/flow.rs:21` / `src/net_flow/mod.rs:37` / `src/dns_log/mod.rs:30`：网络流量身份 = 进程 (pid, start_time) → 远端 (addr, port) 二元组

**面试官三层深挖预期**：

- **L1（是什么）**：知道 pid 会被 OS 复用，身份键 = (pid, start_time)。
- **L2（为什么这样选）**：start_time 是 sysinfo 每次采样都带回的字段（`ProcessInfo.start_time`，Unix epoch 秒），零额外系统调用成本；复用检测放在键控层而不是「检测到复用再清理」，让数据结构自身保证不会串数据。
- **L3（边界 / 反例 / 诚实声明）**：`process_cache: HashMap<u32, ProcessInfo>`（`src/collect.rs:1351`）本体是**单键 pid** 的 in-place 缓存——每 2s heavy tick 只更新易变字段（cpu / memory / disk_usage），不比较 start_time；PID 在两个 tick 之间被复用时，缓存里的静态身份字段（name / exe / start_time）会陈旧到该 pid 再次消失为止。这是「省分配」与「身份新鲜度」的显式权衡：身份关键路径（评分 / 差分 / 详情页 / 流量归属）都从**新鲜采样**重新取 (pid, start_time) 校验，不信任缓存的静态字段。另有两个如实归档的残留面：TD-21（overlay 提示单键 pid，窗口窄影响一次评分，维持归档）与 ADR-0005（netflow 非复用场景的速率 0 处理）。

**引用勘误（诚实记录）**：仓库多处（`src/app.rs:181` 注释、CONTEXT.md PID 词条、ADR-0021 Related 段、ADR-0036 D4 表）把该决策引到「ADR-0003」，但现行 ADR-0003 是 smartctl 选型，且 git 全历史不存在 `0003-pid-reuse-start-time-key.md`——幽灵引用（详见 REVIEW-v0.26 Findings）。

---

## 2. 采集三路与「ETW → sysinfo delta」改道

**一句话结论**：进程 / 系统数据走 sysinfo（跨 tick 增量缓存），网络与磁盘事件走手写 windows-rs ETW（三 provider），句柄 / TCP 质量 / 进程控制走 NT API 直调——而 v0.25 给 MCP 暴露 per-process 磁盘速率时**放弃了再开一路 ETW**，改在既有 worker 里做 sysinfo 差分。

**决策与证据**：

- 三路分层（README 架构图「采集层」）：sysinfo（进程 / CPU / 内存，Heavy 2s 增量）+ ETW ×3（`src/dns_log/etw.rs` DNS 查询 / `src/schannel_etw/provider.rs` TLS SNI / `src/disk_io_etw/provider.rs` 磁盘 IO）+ NT API（`src/inspect/handles.rs` 句柄枚举 / `src/estats.rs` TCP estats / `src/process_control.rs` 优先级与 kill）
- 改道决策：[ADR-0035](adr/0035-td-cleanup-and-session-hygiene.md) D2——原方案「handler 持久 disk_io_etw worker」被三条理由否决，改道 sysinfo delta（v0.25 stage 3 落地，`compute_process_disk_speeds` 纯函数 + `run_snapshot_worker` 局部基线 + `per_process` top-N 响应段）

**面试官三层深挖预期**：

- **L1（是什么）**：三种采集通道各管一摊；磁盘 per-process 速率走 sysinfo 差分而非 ETW。
- **L2（为什么否决 ETW worker——三理由原文）**：① NT Kernel Logger 是**全局单实例**——MCP server（`proc mcp`）与 TUI（`proc`）同机并存时互抢 session，后启动者恒失败（DNS 用的非独占 provider 可多实例，先例不可复制）；② MCP server 常态**非提权**运行——该 ETW 路径恒 None，实装等于死代码；③ 启动延迟 ~1s + x86 cfg-gate。三条都有现场可复现的失败模式，不是偏好问题。
- **L3（边界 / 代价）**：sysinfo `disk_usage` 在 Windows 来自进程 IO counters，**含非磁盘 IO（命名管道等）**，精度低于 ETW——处理方式是在响应里加 `source: "sysinfo-delta"` 字段声明口径（让 agent 可判读），而不是假装精度相同；触发条件复刻 TUI：只在 `refresh_heavy_incremental` 返 `Ok(true)`（cache 确实刷新）时算差分，否则会把速度刷成 0。

---

## 3. worker 指数退避重启 + 止损状态机

**一句话结论**：采集 worker panic 后按 5s / 30s / 5min 指数退避自动 respawn，3 次失败永久死亡（止损），1 小时无 panic 计数归零——「自动恢复瞬时故障」与「不让 panic loop 拖垮系统」同时成立。

**决策与证据**：[ADR-0019](adr/0019-worker-restart-policy.md)（v0.11 落地）+ TD-24 止损补丁（v0.25 stage 2，[ADR-0035](adr/0035-td-cleanup-and-session-hygiene.md)）：

- 状态机 `RestartState { retry_count, last_crash, last_restart, last_reset }`（`src/workers/restart.rs`，纯逻辑 14 单测）+ `WorkerManager::restart_tick` 每 1s 检查退避到期
- TD-24 修复的缺陷：原实现 spawn 失败不计数——环境持续不支持该 worker 时 `retry_count` 永不增长，banner 永远显示 Restarting 的**无限重试**；补 `on_respawn_failed(now)`（saturating +1 + backoff 从失败点重算）后 MAX_RETRIES 触发 permanent_failure 止损
- 三态 banner（restarting / restarted / permanent failure）让用户知道是否需要手动重启 proc

**面试官三层深挖预期**：

- **L1（是什么）**：catch_unwind 截获 panic → crash 事件进 channel → 主线程按退避表 respawn；3 次封顶；1h 重置。
- **L2（为什么这样选）**：对比表四方案（立即重启 = panic loop CPU 100% + crash 文件爆炸；永久死亡 = v0.10 行为，单 worker 故障拖垮整个工具；手动按钮 = 无人值守场景失效）；退避序列 5s/30s/300s 让「3 次 ≈ 15min」形成有限止损窗口。
- **L3（边界 / 例外）**：两个显式例外文档化——docker worker 不接入（DockerPanel 自管生命周期，重进面板即重建；ADR-0019 决策 8）；reset 窗口 1h 意味着「偶发 panic 后稳定运行」不积累惩罚，但 banner 三态是用户感知止损的唯一通道（无人看屏幕时 permanent failure 静默存在——接受，因为替代方案是无限重试）。测试口径也值得讲：mock 用非 canonical thread_name 走 `spawn_one` 的 `_ => false` 分支制造**确定性** spawn 失败，不依赖管理员权限环境。

---

## 4. session 空会话治理——延迟创建 vs 退出清理

**一句话结论**：session JSONL 只在**首个非 session_start 事件**到达时才创建文件（SessionStart 届时补写首行）——用「不依赖 Drop」的惰性物化根治了 98% 空会话噪声，而不是「先落盘、退出时删」。

**决策与证据**：[ADR-0035](adr/0035-td-cleanup-and-session-hygiene.md) D1（v0.25 stage 2 落地，`src/agent/session_log.rs`）：

- 现象归因（Spike 修正过一次）：102 文件 100 空、成对出现间隔 15-30s、全带 `-llama-cpp` 后缀——空文件唯一来源是 TUI 进 Agent 面板即 `SessionRecorder::start` 构造即 `File::create`；「与 eval run 时间吻合」是相关非因果
- 退出清理方案的 Drop 三缺陷（原文）：`Arc<Mutex>` poison 后清理代码不跑 / 多 clone 方 drop 顺序不定 / TUI 强杀路径没有 Drop；外加「先落盘再删」窗口内崩溃仍留空文件

**面试官三层深挖预期**：

- **L1（是什么）**：RecorderInner 持 `path` + `pending_start` + `writer: Option`，物化收敛在 `write_entry` 单点。
- **L2（为什么延迟创建赢）**：单点改动、天然覆盖「TUI 中途退出」（无 query = 无文件）、不依赖 Drop 语义；而 Error 事件也触发落盘（保留 2 行诊断文件的价值判断）。
- **L3（边界 / 代价）**：SessionStart 的 wall_start 与文件 mtime 有轻微偏差（文件名时间戳仍是构造时刻）；「ask / eval 不落盘」是**现状锚**而非治理引入——测试显式锚定该现状，防未来 recorder 接进 build_runner 时口径漂移；对 RAG 无影响（无 QueryStarted 的文件在语料状态机下本就不产出语料，`tests/test_agent_rag.rs` 全绿即锚）。

---

## 5. 录屏双格式——VT100 字节流 vs UiFrame 结构化帧

**一句话结论**：同一套录屏能力维护两种文件格式（v0.6 VT100 字节流 `.prec` / v0.14 起结构化 UiFrame v3），VT100 老文件通过**临时转码**享受 v3 全部回放能力（搜索 / 倒放 / 书签），转码失败回退正向播放。

**决策与证据**：`src/record/`（vt100.rs / frame.rs / vt100_to_uiframe.rs 等）+ [ADR-0028](adr/0028-vt100-to-uiframe-converter.md)（v0.17 落地）：

- 为什么两种格式并存：VT100 录的是屏幕字节流（CSI / SGR / 光标序列），保真且紧凑，但**没有结构化帧索引**——倒放需要反向解释器（clear / cursor move / SGR 的逆操作，~1000+ 行），FilterExpr 时间轴搜索（timestamp / cpu / mem / name / anomaly.severity 五维）依赖 UiFrame 结构
- 转码路径：`Vt100ToUiFrameConverter` 增量解析 + 累积屏幕 buffer + 30 FPS 切片 → `<file>.tmp.v3`（RAII 临时文件，回放结束删除）→ 走 v3 Player；`proc replay` 与 MCP `proc_replay_info` / `proc_replay_search` 双路径自动检测透明转码；失败 fallback `VtPlayer` 正向 replay
- 否决的备选：永久转码（用户要管理第二份文件，转码后 size 2-3×）与纯反向解释器（~1000+ 行还不享受 v3 能力）

**面试官三层深挖预期**：

- **L1（是什么）**：双格式 + 自动临时转码 + 失败回退。
- **L2（为什么临时转码）**：不破坏原文件、复用 v3 Player 全部能力（书签 sidecar / footer / 搜索）、开销 ~3s/30min session 可接受（agent 一次性调用场景）。
- **L3（边界）**：转码帧的字段填充是**降级的**——VT100 字节流不含 anomaly / cpu / mem，转码后这些字段恒默认值（0 / 空 Vec），`anomaly.severity` 搜索在转码帧上无意义；agent 视角需理解「VT100 转码帧无系统指标」。多次调用的累积 ~3s × N 开销留了永久转码 CLI 的后手（v0.18+ 候选，未排期）。

---

## 6. 安全纵深四件套

**一句话结论**：proc 是持 `SeDebugPrivilege` 能读任意进程内存的工具，被攻破即成 credential theft 跳板——所以安全投入分四层：18 项评分（帮用户看别人）、self-mitigation 5 策略（硬化自己）、restricted spawn（管住子进程）、env mask + 录屏确认（管住输出）。

**决策与证据**：[ADR-0008](adr/0008-self-mitigation-policy.md)（v0.6 落地）+ `src/security/`：

- **评分 18 项**：0-100 分（100 = 安全）扣分制——v0.6 14 项 → v0.7 R15 网络命中 → v0.11 扩到 18（R16 签名验证 / R17 父子链 / R18 可疑启动路径，[ADR-0021](adr/0021-process-signature-verification.md)）；`BackgroundScorer` 异步跑不阻塞 UI
- **self-mitigation 5 策略**：`SetProcessMitigationPolicy` 开 DEP(Permanent) / ASLR(HighEntropy) / ProhibitDynamicCode / DisableExtensionPoints / ImageLoad——**不开 ProcessSignaturePolicy**：nvml-wrapper 的 nvml.dll 签名状态不受控，强制签名可能让 GPU 监控直接挂；调用在 `main` 第一行，失败 `warn` 不 panic（启动健壮性 > 完美加固）；DEP 在 release 二进制上预期失败（`/NXCOMPAT` 已默认开启且 Permanent），进 failed Vec 继续——符合契约
- **restricted spawn**：elevated 时 spawn PowerShell DNS 子进程前 `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)` 剥离 SeDebugPrivilege——**只接 DNS 一处**（docker exec / smartctl 自身需特权，不接入）；probe 一并走 restricted（TD-10）
- **env mask**：详情页 Env Tab 默认脱敏（`{前 2 字符}***(原长 B)`），secret pattern 12 关键字族匹配，按 `v` 显式切换且**录屏时强制 mask**；录屏启动前 `y/n` 确认（录屏会捕获屏幕全部内容）

**面试官三层深挖预期**：

- **L1（是什么）**：四层各自对象（被观察进程 / 自己 / 子进程 / 输出）。
- **L2（为什么不开 Signature）**：兼容性取舍的最强保护策略 vs 未签名 native 依赖——「挡住 80% 注入路径且一定能启动」优于「挡 100% 但可能起不来」；同理说明 warn-not-panic。
- **L3（边界）**：restricted spawn 不是全量——威胁模型按「子进程是否接受外部数据」逐点接入而非一刀切；评分是启发式（18 项扣分制会误报，定位是排查辅助不是判决）；watchdog 的用户自定义命令不强制 restricted（用户自己写的命令，威胁模型不同，TD-11 维持归档）。

---

## 7. MCP 46 不变锚 + deprecated 双轨 + snapshot 复用

**一句话结论**：MCP tool 总数 46 自 v0.17 起冻结为**不变锚**（每个 cycle 用 grep + 运行时断言双重核对），重叠 tool 用「description 文本 + `_meta.x-deprecated` schema」双轨标记而不删除，tool 响应性能靠 handler 持久 snapshot 字段（1s tick worker 刷新）复用而不是每次调用重建。

**决策与证据**：[ADR-0026](adr/0026-mcp-handler-persistent-fields.md)（v0.17）+ [ADR-0035](adr/0035-td-cleanup-and-session-hygiene.md) D2（v0.25）：

- **不变锚的用途**：46 是 eval 基线可比性的锚（tool 集不变 → agent 行为面不变）——v0.22 起每个 cycle 完工都断言 `grep 'name = "proc_' | wc -l == 46` + 运行时 `list_tool_names().len() == 46`
- **deprecated 双轨**：`proc_smart` 与 CLI 直采能力重叠——v0.17 在 description 写 `[Deprecated]` 文本 hint（静态 grep 测试锚定），v0.25 TD-50 再加 schema 层 `_meta: {"x-deprecated": true}`（rmcp `#[tool(meta = ...)]`，MCP 规范官方扩展键；运行时断言 `proc_smart_tool_attr().meta`）——不删的理由：外部 MCP client 可能已依赖，删除是 breaking change
- **snapshot 复用**：`ProcMcpHandler` 持 `snapshot: Arc<Mutex<Option<SystemSnapshot>>>` 等持久字段（`mcp-persistent-state` feature 默认启用），`run_snapshot_worker` 1s tick `refresh_heavy_incremental`——对比被否决的「每次调用新建 SystemSnapshot」（TD-54 归档时实测 `proc_flows` 单次 ~200ms 含 2s warm-up，多次调用累积 500ms-2s）与 TTL 缓存（freshness 不如 tick）

**面试官三层深挖预期**：

- **L1（是什么）**：tool 数冻结 + 双轨弃用 + 持久字段复用。
- **L2（为什么）**：外部接口的删除成本与内部 API 不同（没有编译器帮你找调用方）→ hint 不删除；eval 可比性要求 agent 可见面稳定 → 把「面」变成回归断言。
- **L3（边界）**：`flows` 走 `App::new` ~2s warm-up 的例外如实归档（数据源是 Schannel worker 非 SystemSnapshot，不在 snapshot 复用范围——TD-54 关闭注记）；`x-deprecated` 依赖 rmcp 0.11 的 `Tool.meta` 序列化，rmcp major 升级（CI audit 漏洞修复路径）时需复核。

---

## 8. OpenAI tools 协议 over llama.cpp + GBNF 否决证据链

**一句话结论**：agent 循环用 llama-server 的 **OpenAI chat/completions + tools 协议**（`tool_choice=required` + 自定义 `proc_finish` 收尾 tool 构成可靠循环），GBNF 语法约束这条修复路径被 12 次 400 响应**结构性否决**（绑定 llama.cpp b8685 版本）。

**决策与证据**：[ADR-0030](adr/0030-builtin-ai-agent.md) D3/D7（v0.20）+ [ADR-0033](adr/0033-eval-experiments-and-record-tools.md) 附录 B（v0.23 冒烟实测）+ [REVIEW-v0.22](reviews/REVIEW-v0.22.md) 观察 1：

- **两层 tool registry**：entry 4 tool 起步 + `proc_help(category)` 元 tool 动态发现其余——单轮 tool-context 从 ~15K 降至 ~1.5K token 峰值（96% 减少），这是 2B 模型可驱动 47 tool 的前提；few-shot 对话示例实测让 E2B 角色扮演编造结果 → 删
- **GBNF 否决链**：动机——E2B 基线 FULL 70q 的 `output_degraded` 21 次（占失败 55%，proc_finish 语法泄漏型为主），GBNF 语法约束预期直接消灭；实测——grammar + tools 同传的请求全部被 llama-server b8685 以 400 拒绝，错误体原文 `"Cannot use custom grammar constraints with tools."`，两场景冒烟 12 请求零生成，错误在**请求校验层**（与 query 内容无关，第三场景不必跑）；结论绑定版本——升级 llama.cpp 后互斥校验若放开可重开（TD-61 观察项）
- **grammar 的实际去处**：`tool_call.gbnf` 仍 `include_str!` 嵌入 binary，`agent.toml grammar_file` 留逃生舱——但明确「不进 ReAct 主循环：约束形状与自然语言回答互斥」

**面试官三层深挖预期**：

- **L1（是什么）**：OpenAI 协议 + required + finish tool；GBNF 在该 server 版本与 tools 互斥。
- **L2（为什么这样设计循环）**：小模型需要**结构化强制**而不是措辞引导——required 保证每轮必有 tool call，proc_finish 把「结束」也变成一次 tool 调用（而不是靠停止条件猜）；两层 registry 解决的是 token 预算不是能力。
- **L3（边界 / 数据意识）**：这条否决是**证据驱动**的完整样本——从失败直方图（21/70 泄漏型）提出假设，冒烟 12 请求即判定性否决，负结果归档 ADR 附录并绑定版本边界；被问「为什么不换纯 JSON completion + grammar 自解析」时的答案：那是协议层重写非配置开关，且 tools 协议路径已被 70q 基线验证可用——修复一个 30% 的退化不值得重写已工作的 70%。

---

## 9. RAG 经验召回全链 + 判读纪律

**一句话结论**：RAG 用零依赖的 BM25-lite keyword 检索 + per-query 预注入（user message 前缀，800 token 硬预算）+ 双语料源（session JSONL 主 / eval trace bootstrap）+ 污染排除（exact + 词元覆盖率 0.6），实验结论按方差带标尺判读——**「机制成立但 E2B 兑现不了通过率」**两分归档，默认 off。

**决策与证据**：[ADR-0034](adr/0034-rag-experience-recall.md) D1-D5（v0.24）+ `src/agent/rag/`：

- **D1 检索**：`score = Σ idf(t) × tf(t)`（tf 上限 3 防刷分），CJK 按 2-gram 切分避免词典依赖，top-k=3 + min_score=1.0 门槛；否决 embedding（百级条目上向量检索收益无法兑现 + 重依赖）与 rusqlite（无复杂查询需求）
- **D2 注入**：否决惰性 meta tool（E2B 惰性发现链已被证明不可靠）选预注入；注入位置在 user message 前缀的三个理由——临近性（小模型对近期 token 注意更强）/ 变量隔离（不碰 system.md，与 prompt 实验文件级解耦）/ 可观测（注入内容与 query 同段进 session log）
- **D4 污染防护**：bootstrap 语料含 eval 同款 query 时检索等于发答案——exact 一律排除 + 双向 min 分母覆盖率 ≥ 0.6 排除；边界诚实声明：同场景同意图不同措辞互检是设计目的不是污染
- **D5 判读与结论**：主指标（检索准不准 / 注入有没有被用上）与通过率增益分离——实测召回 12/15 = 80% / 引用 8/15 = 53% / 干扰 0 / `output_degraded` -12（超带改善）但净通过 +2（带内）→ 机制成立、E2B 兑现不了，`enabled` 默认 off（改默认门槛 = 净通过差 ≥ +7 且 L2 方向性，与换模型同级；引用率勘误见 REVIEW-v0.26——v0.24 归档汇总行的 57% 是 8/14 口径笔误，表格原始值 8/15 = 53%）

**面试官三层深挖预期**：

- **L1（是什么）**：keyword 检索 + 前缀注入 + 排除 + 带内判读 off。
- **L2（为什么零依赖）**：语料量级（百级条目）决定向量检索收益无法兑现；deps +0 是 v0.22 以来的不变锚纪律。
- **L3（方法论层——最值得讲的部分）**：方差带标尺（单次 run ±3 通过数 / ±6 失败模式计数以内不可单独归因）是 v0.23 最重要遗产；两分结论框架让「机制成立」与「底座兑现不了」分离归档，为模型升级重启决策留下数据输入而不是一笔糊涂账；负结果三连（GBNF 互斥 / prompt v3 带外向下 revert / RAG 两分）共同构成「用数据关闭假设」的完整叙事。

---

## 10. Windows-only 权衡 + eval 科学方法论

**一句话结论**：v0.12 起主动收窄为 Windows-only（删除 ~1000 行 Linux 代码与 4 个 CI target），用「深度换广度」；同期建立的 eval 方法论（方差带 / 单变量隔离 / 预登记拍板标准 / 负结果归档）让所有 agent 改动**用数据决策而不是用感觉**。

**决策与证据**：[ADR-0022](adr/0022-windows-only-platform.md)（v0.12）+ [ADR-0033](adr/0033-eval-experiments-and-record-tools.md) / [ADR-0032](adr/0032-eval-harness.md)（v0.22-23）：

- **收窄的账**：TD-19（eBPF Linux 真机验证）三个 cycle 推迟、用户从未在 Linux 上跑过 proc、v0.11 Review 多个 P2 耗在 cfg gate 一致性上——**删除后** ~1000 行 Linux 代码 + ~30 文件 cfg gate + CI 5 target → 1 target，换来的精力投进 Windows 深度（ETW / Schannel SNI / EcoQoS / estats / 签名验证）；Linux 用户迁移路径显式化（停在 v0.11.0 或 fork）
- **eval 方法论五件**：70 query 三级基准（L0 23 / L1 27 / L2 20）+ 确定性失败模式分类（不上 LLM-as-judge）+ 方差带标尺（±3/±6）+ 预登记拍板标准（换默认 = 净通过差 ≥ +7 且 L2 方向性——先定标准后看数据）+ report-only 不 gate（与验收测试双入口分工，防两套阈值漂移）
- **口径一次锁定**：serde schema 即结果 JSON contract（roundtrip 测试锚字段名）；`is_degraded_output` 在 FULL 实跑前就位——若沿用「非空即过」口径 eval 虚高 30%

**面试官三层深挖预期**：

- **L1（是什么）**：平台收窄的事实 + eval 工具链存在。
- **L2（为什么敢收窄）**：成本数据说话（推迟记录 / Review 精力分布 / 零真实使用）；「跨平台范例」的学习价值 vs 维护成本的显式取舍；沉没成本（ADR-0016 eBPF 设计投入）不构成继续的理由。
- **L3（如果重做 / 反问）**：诚实答案——如果目标包含 Linux 用户群，一开始就该按平台分包而不是 cfg gate 散布；eval 方法论的可迁移性：方差带思维（先测噪声再判信号）适用于任何有随机性的系统评测，pre-registration（先定门槛后看数据）是防 cherry-picking 的机制而非仪式。

---

## 附：阅读路线建议

- **只读三条**：#8（agent 循环与否决证据链）→ #9（RAG 与判读纪律）→ #3（可靠性状态机）——覆盖「AI 工程 + 方法论 + 系统工程」三个面试高频面
- **系统方向**：#1 → #2 → #3 → #5（数据身份 → 采集 → 可靠性 → 持久化格式）
- **安全方向**：#6 单条即成章（纵深四层 + 每层的威胁模型）
