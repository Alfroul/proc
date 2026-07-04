# proc

Rust 编写的交互式 TUI 系统进程管理器。把 **进程管理 + 网络分析 + USB 占用 + 监控 + Docker + 安全评分 + 降频检测 + 磁盘 I/O + 终端录屏 + 告警 + SMART 磁盘健康 + per-process 网络流量 + DNS 查询日志 + 容器 exec** 融合到一个 TUI 中。**Windows-only 应用**（Windows 10 1809+ / Windows 11 x64，详见 [ADR-0022](docs/adr/0022-windows-only-platform.md)）。

> **v0.13.0（2026-07-05）— 性能 baseline cycle**：建立 criterion benchmark suite（6 个 hot path × 多档 fixture = 25 数据点）+ 产出 [PERF-BASELINE 报告](docs/reviews/PERF-BASELINE-v0.13.md)。**用户拍板方案 c**：验证 proc 当前架构在 1000 进程规模下无显著性能瓶颈，跳过 stage 3+ 优化。cycle 全程**不动业务代码**（1115 passed / 0 failed / 3 ignored 基线不变）；4 个候选项（parent_chain Arc 重构 / tui format! 风暴 / record deserialize 加速 / command_palette fuzzy）归档 tech-debt [TD-44/45/46/47](docs/tech-debt.md) 留 v0.14+ cycle 评估（含 1 个侦察报告误读纠错）。

> **v0.12.0（2026-07-04）** Windows-only 平台定位 + UX polish cycle：**ADR-0022 锁定 Windows-only 决策**（移除全部 Linux/macOS 代码——src/ebpf/ 整模块 + src/psi.rs + nvtop / nethogs / unsupported.rs 删 + ~25 文件 cfg gate 清理）/ **签名验证完整度**（SignatureStatus 9 状态机加 Expired/UntrustedRoot/ChainError + TRUSTED_SIGNERS 扩到 24 vendor + 用户配置 `trusted_signers.toml`）/ **FilterExpr 修复**（`mem > 50%` 按总内存换算 silent bug + regex `\/` escape 让 CIDR / URL pattern 能写）/ **6 个 v0.11 REVIEW-13 P2 修复**（diag JSON 加 dns_collector / NetworkIn HashSet O(1) / R17 系统启动白名单 / R18 Downloads 去重 / property_at_index lifetime 修正 / MCP DNS 持久 collector）。全量回归 1115 tests passed / 0 failed。详见 [CHANGELOG](CHANGELOG.md)。

> **已知限制**：**Windows-only 平台**（v0.12 起 proc 转为 Windows-only，Linux / macOS 用户迁移路径 `git checkout v0.11.0`）；Win10 < 1809 admin 下 Schannel event 1793 不 fire；worker restart 3 次失败后仍永久死亡；DNS ETW 仅 Windows 管理员启用（非 admin 走 PowerShell fallback）。详见 [tech-debt](docs/tech-debt.md)。

> **v0.11.0（2026-07-01）** 安全 + 可靠性大版本：**Worker Restart 真正实装**（TD-4 清零，panic 后指数退避 5s/30s/5min 自动重启，3 次失败永久死亡）/ **DNS ETW 替代 PowerShell probe**（CPU 3-5% → < 0.5%，延迟 500ms-1s → < 50ms，PowerShell fallback 保留）/ **FilterExpr v2 网络字段**（`sni/dns_name/remote_addr/remote_port/bytes_out/bytes_in/source`）/ **进程签名验证 R16**（WinVerifyTrust 6 状态机 + BackgroundScorer 异步）/ **进程父子链 R17**（Office → shell / Browser → shell / ScriptInterpreter 三档扣分）/ **可疑启动路径 R18**（%TEMP% / %APPDATA% / Downloads + R16 协同扣分）。全量回归 1146 tests passed / 0 failed。详见 [CHANGELOG](CHANGELOG.md)。

> **已知限制**：Win10 < 1809 admin 下 Schannel event 1793 不 fire（延续 v0.10）；worker restart 3 次失败后仍永久死亡；DNS ETW 仅 Windows 管理员启用（非 admin 走 PowerShell fallback）；Linux ebpf 编译路径未在本机验证（TD-19 延续）。详见 [tech-debt](docs/tech-debt.md)。

> **v0.10.0（2026-06-28）** 跨平台 SNI 对齐：**Windows Schannel ETW SNI 落地**（手写 windows-rs ETW + TDH 动态 schema，event 1793 / TargetName 字段实测修订）/ **ProcessFlow.source 字段**（`Ebpf` / `Schannel` enum，跨平台在 ProcessFlow 数据结构统一）/ **R15 跨平台激活**（白名单同时检查 sni + dns_name）/ **`proc flows` 跨平台 CLI**（表格加「来源」列，JSON 自动加 source 字段）/ **REVIEW-11 P1 修复**（Schannel-only flow 退出感知 + spawn 失败句柄清理）。弥补 v0.7 阶段 8 eBPF 仅 Linux 的缺位。全量回归 959 tests passed / 0 failed。详见 [CHANGELOG](CHANGELOG.md)。
>
> **已知限制**：Win10 < 1809 admin 下 Schannel event 1793 不 fire（worker 启动成功但 UI 显示 0 条）；Linux ebpf 编译路径未在本机验证（v0.8.0 cycle stage 1 主动推迟，本 cycle 不依赖）。详见 [tech-debt TD-19 / TD-20](docs/tech-debt.md)。

> **v0.8.0（2026-06-28）** 小修一波清 + FilterExpr 扩展：**FilterExpr 全 view 支持**（Tree / AppGroup 视图按 `:` 也能用 `cpu > 5 AND name =~ /chrome/` 过滤）/ **错误信息中文化**（不再直出 nom 内部 `TakeWhile1`，改友好提示「缺少字段名/值」）/ **Linux CI 加固**（全量 `cargo test --release` + 测试 bin 数 ≥ 30 校验防 cfg-gate 静默 skip）/ **Linux stub 测试覆盖**（env/dlls/handles/memory 降级路径有早期告警）。全量回归 930 tests passed / 0 failed。详见 [CHANGELOG](CHANGELOG.md)。

> **v0.7.0（2026-06-28）** 新增三大主题：**生态卡位**（`proc mcp serve` LLM agent 接入 / shell 补全 / Ctrl+P 命令面板 / FilterExpr 表达式搜索 `cpu > 5 AND name =~ /chrome/`）/ **平台深度**（Linux PSI 监控 / Win11 EcoQoS 切换 / Win ETW per-process 磁盘 IO / Linux eBPF flow graph）/ **架构债清理**（App 拆 5 个 panel controller）。全量回归 910 tests passed / 0 failed。详见 [CHANGELOG](CHANGELOG.md)。

## 功能

`proc` 把 **进程管理 + 网络分析 + 安全评分 + 资源监控 + USB 占用 + 进程守护 + Docker + 终端录屏 + 告警** 融合到一个 TUI 中，所有面板共享一份系统快照（`SysinfoRegistry` 全局单例），后台 worker 体系（`SnapshotWorker<T>` / `LightWorker` / `HeavyWorker` / `BackgroundScorer`）保证主线程 50ms tick 不阻塞。

### 6 大主面板

| 面板 | 切换 | 能力 |
|---|---|---|
| 进程列表 | `1` | 按 CPU / 内存 / PID / 名称 / 安全分 / 磁盘读 / 磁盘写 / **网络收 / 网络发** 9 字段排序（持久化到 `ui.toml`），模糊搜索，多选批量终止，`v` 切应用分组视图（按 `.exe` 聚合，CPU/内存/进程数三字段）<sup>v0.5.0</sup> |
| 进程树 | `2` | 父子层级展开/折叠，孤儿 / 僵尸 / 残存进程检测，`o` 一键选孤儿、`z` 一键选僵尸 |
| 端口/网络 | `3` | 按端口 / 按进程 / 按远程三种视图；6 种异常模式自动告警（CLOSE_WAIT 堆积、TIME_WAIT 异常、远程地址爆炸等）；网络诊断工具箱 Ping / DNS 反查 / Whois / Traceroute / 端口探测；**`D` 切换 DNS 查询日志子视图**<sup>v0.5.0</sup>；**TCP 传输质量摘要**（重传率 / RST 率告警）<sup>v0.5.0</sup> |
| USB 助手 | `4` | 句柄占用检测（`filelocksmith`）+ 风险分级（Safe / Warning / Critical）+ 缓存刷新 + 安全弹出引导 + 持续监测模式 |
| 监控 | `5` | 按 PID / 端口 / 命令三种 Target；`NotifyOnly` 或 `AutoRestart` 指数退避策略；Critical 告警推 Toast |
| Docker | `6` | 容器列表 + 实时事件流 + 健康检查 + 资源统计；**容器内进程（top）/ 日志模式（logs）/ 镜像 / 卷视图**<sup>v0.5.0</sup>；**`e` exec 进容器嵌入式 PTY**<sup>v0.5.0</sup>；支持命名管道 / TCP 双连接；事件流 `sync_channel(64)` 背压 |

### 进程深挖（Inspector）<sup>v0.5.0</sup>

进程列表/树中按 `Enter` 进入详情页，顶部 **6 个 Tab** 切换深挖视图：

| Tab | 内容 | 数据源 |
|---|---|---|
| **概要** | 分类 / 父进程 / CPU / 内存 / 磁盘 / 运行时长 / exe / cmd / cwd / 端口摘要 / 网络汇总 / 安全分 / 风险因子 / **优先级 + affinity**<sup>v0.5.0</sup> | sysinfo + `port_map` |
| **环境** | 进程环境变量列表（`KEY=VALUE`），`/` 大小写不敏感搜索过滤，`↑↓ PgUp PgDn Home End` 滚动 | Win: PEB walk (`NtQueryInformationProcess` + `ReadProcessMemory`)；Linux: `/proc/<pid>/environ` |
| **网络** | 该 PID 的全部监听与连接 + **最近 5 条 DNS 查询**<sup>v0.5.0</sup>：协议 / 本地 / 远程 / 状态 / 进程名 | 复用 `port_map::find_ports_by_pid` |
| **DLL** | 已加载模块（Windows DLL / Linux `.so`），按路径字母排序，`/` 搜索；表格列：路径 / 基址 / 大小 | Win: `CreateToolhelp32Snapshot`；Linux: 解析 `/proc/<pid>/maps` 合并 r-xp / r--p / rw-p 多段映射 |
| **句柄**<sup>v0.5.0</sup> | 进程打开的所有句柄：File / RegistryKey / Event / Semaphore / Mutant / Section / Process / Thread / Token；**`Ctrl+F` 句柄内搜索** | Win: `NtQuerySystemInformation` + `DuplicateHandle` + `NtQueryObject`；Linux: `/proc/<pid>/fd` |
| **内存映射**<sup>v0.5.0</sup> | VirtualQueryEx / `/proc/<pid>/maps` 内存区域：基址 / 大小 / 状态 / 保护 / 映射文件名 | Win: `VirtualQueryEx`；Linux: `/proc/<pid>/maps` |

macOS 等非 Win/Linux 平台，环境 / DLL / 句柄 / 内存 Tab 显示「此平台不支持」降级提示。详情页内 `F5` 强制重新采集（v0.6.0 起替代 `r`，`r` 兼容期显示 deprecation）、`y` 复制进程信息到剪贴板（vim yank，v0.6.0 起替代 `c`）、`v` 切换 Env Tab 的 secret 脱敏（录屏强制 mask）、`/` 搜索、`Tab/Shift+Tab` 切 Tab、`+` / `-` 调整优先级、`Esc` 先退搜索再退页面（双层语义）。

### 安全评分

每个进程附带 **0-100 分**（100 = 安全）。基于 **18 项独立检查**（v0.11.0 起扩到 18 项，新增 R16 签名验证 / R17 父子链 / R18 可疑路径 + 与既有 v0.6 path_check 协同扣分；详见 [ADR-0021](docs/adr/0021-process-signature-verification.md) + [CONTEXT.md](CONTEXT.md) R16/R17/R18 段）：

- **签名** Authenticode 数字签名
- **父子链** 父进程链完整性（如 `explorer.exe → chrome.exe` 是否合理）
- **路径** 可执行文件路径合法性（如 `C:\Windows\System32\` vs 临时目录）
- **命令行** 可疑模式（编码 base64、隐藏窗口、可疑参数等 20+ 模式）
- **网络行为** 连接数 / 远程地址 / 监听端口异常
- **名称仿冒** `scvhost.exe` / `chr0me.exe` 等同形异义攻击
- **资源异常** CPU / 内存峰值
- **子进程爆炸** 短时间内派生大量子进程
- **权限提升** 是否请求了过高权限
- **svchost 完整性** 系统服务宿主完整性
- **DLL 加载** 加载来源合法性
- **令牌权限** 令牌权限审计
- **信誉缓存** 文件 SHA256 信誉（流式哈希 + 64MB 上限防 OOM）

按 `S`（大写）按安全分排序，可疑进程排最前；详情页 **概要 Tab** 展示所有风险因子与扣分。后台评分线程（`BackgroundScorer`）通过 channel 异步计算，不阻塞 UI；缓存键 `{pid}:{start_time}:{exe}` 防 PID 复用串数据。

### 系统资源监控（侧边栏）

- **降频检测** 实时识别 CPU 降频原因（热 / 功耗 / 空闲），侧边栏显示 `⚠THERMAL` / `⚠POWER`（Win32 `CallNtPowerInformation`）
- **per-core 频率 + 温度**<sup>v0.5.0</sup> 每核独立显示当前频率 + 温度；颜色分级（< 70°C 绿 / 70-79 黄 / 80-89 橙 / ≥ 90 红）；Linux 走 `/sys/devices/system/cpu/cpufreq` + hwmon，Windows 走 `CallNtPowerInformation`
- **磁盘 I/O** 每磁盘独立读写速率 + 每进程 I/O 速率（`(pid, start_time)` 键防 PID 复用串数据）
- **GPU** 多厂商支持：Windows DXGI + NVML（NVIDIA enrichment：温度 / 显存 / 功率 / 利用率）+ PDH utilization；**Linux 走 nvtop 子进程覆盖 AMD / Intel / NVIDIA 全厂商**<sup>v0.5.0</sup>
- **SMART 磁盘健康徽章**<sup>v0.5.0</sup> 侧边栏每个挂载点显示 `✓` (Ok) / `⚠` (Warning) / `✗` (Failing) / `-` (Unknown)
- **侧边栏其他** CPU/内存/交换区使用率 + 火花线图（30 秒历史）、网卡 IP、运行时间

### AMD / Intel GPU 支持（多厂商）<sup>v0.5.0</sup>

阶段 6 引入 `GpuProvider` trait 抽象，多 impl 架构：

| Provider | 平台 | 覆盖 |
|---|---|---|
| `NvmlProvider` | Windows | DXGI（所有 vendor 的 VRAM）+ NVML（NVIDIA enrichment）+ PDH utilization |
| `NvtopProvider` | Linux | nvtop 子进程 JSON 输出，覆盖 AMD / Intel / NVIDIA 全厂商 |

`detect_providers()` 根据 feature flag + 平台 + 二进制可用性返回活跃列表。Linux 用户安装 `nvtop` 后自动启用 AMD/Intel 温度 / 显存 / 利用率监控。Windows AMD/Intel 列入 0.6.0+ 路线图。

### per-process 网络流量<sup>v0.5.0</sup>

进程列表新增 **网络收 / 网络发** 两列（字节/秒），1s 周期采集。`NetFlowCollector` trait + 多 impl：

| 实现 | 平台 | 路径 |
|---|---|---|
| `IphelperCollector` | Windows | IP Helper（`GetTcpTable2` + `GetPerTcpConnectionEStats` + netstat2 PID join，**不走 ETW**） |
| `NethogsCollector` | Linux | nethogs 子进程解析 |

CLI `proc ls --sort net_recv --limit 10` 按流量排序；TUI 内 `←→` 切换排序字段时可选中网络列。

### DNS 查询日志<sup>v0.5.0</sup>

Windows 平台专属。<sup>v0.11.0</sup> **主路径走 ETW**（手写 windows-rs `Microsoft-Windows-DNS-Client` real-time session，event 3008/3010 + TDH 动态 schema 解析，延迟 < 50ms + 100% 完整性）；ETW 启动失败时**降级到 PowerShell fallback**（spawn 长跑 `powershell.exe` 子进程订阅 `Microsoft-Windows-DNS-Client/Operational` channel event 3010）。两者都通过 `DnsLogCollector` trait 抽象，reader 线程 / ETW callback 解析事件 + sysinfo PID 名 lookup + `sync_channel(1000)` 推到主线程。500ms 周期 drain，主线程 cap=1000 FIFO。详见 [ADR-0020](docs/adr/0020-dns-etw-provider.md)。

- **TUI 内**：端口面板按 `D`（大写）激活 DNS 子视图，显示最近 DNS 查询列表
- **详情页 Network Tab**：底部展示该 PID 最近 5 条 DNS 查询
- **CLI**：`proc dns --tail` 流式输出新事件
- **异常规则 R9**：新 PID 首次发起 DNS 查询且不在白名单 → Warning

**怎么知道当前用的是哪个 collector？** 跑 `proc diag`，末尾的 `dns_collector: <kind>` 行反映实际类型（`etw` / `powershell` / `none`）。报「DNS 日志缺数据」类 bug 时附上此行。

**隐私承诺**：DNS 查询记录**永不持久化**到磁盘，仅在内存中保留最近 1000 条；`record/frame.rs` 序列化类型不含 `DnsQuery`。

### v0.6.0 安全加固

阶段 2 引入 4 项独立的安全机制（详见 [SECURITY.md](SECURITY.md) 与 [ADR-0008](docs/adr/0008-self-mitigation-policy.md)）：

| 机制 | 作用 |
|---|---|
| **self-mitigation** | 启动时最早调用 `apply_self_mitigations()`，通过 `SetProcessMitigationPolicy` 给自己上 5 项保护：DEP（Permanent）/ ASLR（HighEntropy）/ ProhibitDynamicCode / DisableExtensionPoints / **ImageLoad（NoRemote + NoLow + PreferSystem32）**。**不开 ProcessSignaturePolicy**（会让 nvml-wrapper 未签名 native 依赖挂） |
| **env 脱敏** | 详情页 Env Tab 默认 mask 显示 `{前2字符}***(原长B)`；secret pattern 匹配 12 关键字（KEY/TOKEN/SECRET/PASSWORD/PASSWD/PWD/CREDENTIAL/PRIVATE/AUTH/API/DSN/CONNECTION_STRING）+ `DATABASE_URL` 特例 + `*_AUTHORIZATION` 后缀。按 `v` 切换 reveal，**录屏时强制 mask** |
| **录屏防护** | 用户主动按 `R` 触发录屏时先弹确认（警告会捕获屏幕所有内容含 DNS 域名 / 进程 cmd），按 `y` 确认 / `n` 取消 |
| **restricted spawn** | elevated 时 spawn PowerShell DNS 子进程前调 `CreateRestrictedToken` + `DISABLE_MAX_PRIVILEGE` 剥离继承的 `SeDebugPrivilege`，防子进程被劫持后变 credential theft 跳板。**仅接入 DNS spawn**（docker exec / nvtop 因自身需 privileged token 不接入） |

### v0.6.0 可观测性

阶段 3 引入完整的诊断链路：

- **日志 rotate**：`tracing-appender::RollingFileAppender::daily` 每天一个文件 `~/.config/proc/proc.logYYYY-MM-DD`，自动清理 7 天前的日志（不再启动时 truncate）。
- **crash report**：panic 时写到 `~/.config/proc/crashes/crash-{YYYYMMDD-HHMMSS}.txt`（主线程 panic）或 `crash-worker-{name}-{ts}.txt`（worker 线程 panic），含时间戳 + proc 版本 + panic info + `Backtrace::force_capture()`。**不上传任何位置**，用户报 bug 时手动附上。
- **worker metrics**：每个 worker 自身记录 `poll_count / poll_total_us / poll_max_us / channel_full_count / last_error`（atomic），主线程聚合到 `proc diag` 输出。
- **`proc diag` 子命令**：JSON 输出所有 worker 的 avg/max/polls/drops 指标，便于 bug 报告附上。`?` 帮助页 Workers 区段也展示精简版（带 `✓` / `⚠` 健康徽章）。
- **worker crash banner**：worker 线程用 `catch_unwind` 包 body，panic 时通过 `crash_tx` 通知主线程，TUI 顶部渲染红色 banner；按 `D` 清空。

### v0.6.0 性能优化

阶段 4 把 `ProcessInfo` 的 `format!` String 分配全部换成 Arc：

| 字段 | 旧类型 | 新类型 | 收益 |
|---|---|---|---|
| `name` | `String` | `Arc<str>` | heavy worker 一次分配，clone 走原子计数 |
| `cmd` | `Vec<String>` | `Arc<[String]>` | 同上 |
| `exe` / `cwd` / `user_id` | `Option<String>` | `Option<Arc<str>>` | 同上 |
| `status` | `String`（`format!("{:?}", sysinfo::ProcessStatus)`） | `ProcessStatus` Copy 枚举（13 变体） | 零分配，按 sysinfo 0.34.2 真实命名对齐 |
| `name_lower` | （每次搜索 `to_lowercase`） | `Arc<str>` 预计算 | 搜索 hot path 不再每按键重建 |
| `query_lower` | （每次按键 `to_lowercase`） | `SearchState` 缓存 | 同上 |

500 进程 × 1.5s 重采下，每秒堆分配减少 90%+；搜索框逐字符输入累积延迟从 ~50ms 降到 μs 级。

### TCP 传输质量<sup>v0.5.0</sup>

`TcpStats` 扩 4 个传输质量字段：

| 字段 | 数据源 |
|---|---|
| `retransmitted_segs` | Win `GetTcpStatisticsEx2` / Linux `/proc/net/snmp` |
| `reset_segs` | 同上 |
| `failed_connections` | 同上 |
| `out_segs` | 同上 |

CLI `proc port --stats` 输出 TCP 质量摘要；TUI 内异常规则：

- **R7**：重传率 > 5% → Warning
- **R8**：RST 率 > 2% → Warning

### SMART 磁盘健康<sup>v0.5.0</sup>

跨平台 SMART 数据采集：

| 平台 | 主路径 | 降级 |
|---|---|---|
| Linux / macOS | `smartctl --json --attributes <device>` 子进程 | — |
| Windows | `smartctl` 子进程 | WMI `MSStorageDriver_FailurePredictStatus`（聚合状态） |

`SmartData` 结构：device / model / serial / temperature / health（4 档 Ok / Warning / Failing / Unknown）/ attributes。CLI `proc smart [device]` 输出；TUI 侧边栏磁盘徽章。

30s poll 周期（`SmartWorker` 独立 worker，与 `LightWorker` 解耦），Drop 时 shutdown + join。

### Docker 深化<sup>v0.5.0</sup>

阶段 3 把 Docker 面板从「容器列表 + 事件流」扩到 lazydocker 级别：

- **容器内进程**（`t` 切到 docker top 视图，CLI `proc docker top <name>`）：复用 `bollard::top_processes`
- **日志模式**（`l` 进入日志视图，CLI `proc docker logs <name> [--follow] [--tail N]`）：独立 `LogsWorker` tokio runtime + `sync_channel(64)` 背压 + 5000 行环形 buffer + follow 模式
- **镜像视图**（`Tab` 切到 Images，CLI `proc docker images`）：本地镜像列表 + `in_use()` 判定 + 两次 `d` 确认删除（CLI `proc docker image-rm <id> [--force]`）
- **卷视图**（`Tab` 切到 Volumes，CLI `proc docker volumes`）：volume 列表 + in_use 反查 + 两次 `d` 确认删除（CLI `proc docker volume-rm <name> [--force]`）
- **docker-compose 薄封装**（CLI `proc docker compose up -d` 等，转发给 docker-compose）
- **事件流**（CLI `proc docker events`）

### 容器 exec（嵌入式 PTY）<sup>v0.5.0</sup>

阶段 9 引入 `AppMode::ContainerExec`：从 DockerPanel 选中容器后按 `e` 进入嵌入式 PTY 模式。本地 `portable-pty` spawn `docker exec -it <container> <shell>` 子进程；docker CLI 处理所有 daemon 通信（命名管道 / TCP / unix socket）+ 远端 PTY 分配；`vt100` crate 解析 ANSI 字节流喂 `ratatui` 渲染。

| 按键 | 行为 |
|---|---|
| `e`（Docker 面板） | 进入 exec 模式（用 image 推断默认 shell：alpine→`/bin/sh`，ubuntu/debian→`/bin/bash`，其它兜底 `/bin/sh`） |
| 普通 ANSI 键 | 透传（Enter=`\r` / Tab=`\t` / Backspace=`\x7f` / 方向键=`\x1b[A/B/C/D` / Ctrl+C=`\x03` / Ctrl+D=`\x04` / Ctrl+\\=`\x1c`） |
| `Ctrl+D` / `exit` / `Ctrl+\` / 子进程退出 | 自动切回 DockerPanel + 提示「容器 xxx 已退出」 |

CLI `proc docker exec <container> [cmd...]` 直接 spawn `docker exec -it` 透传 stdio（CLI 用户终端 = 远端 PTY）。

### 进程守护与告警

**监控**（面板 `5`）：按 PID / 端口 / 命令监视目标进程；`NotifyOnly` 仅通知，`AutoRestart { max_retries, base_backoff, max_backoff }` 指数退避自动重启；watchdog 子进程长跑时可通过 Ctrl+C 干净关停（`try_wait()` 轮询）。

**告警**：可配置阈值规则（CPU / 内存 / 磁盘 / 网络 / 连接数 / 温度 / 降频 7 类指标，6 种比较运算符），连续命中触发防抖，状态机 `Pending → Firing → Resolved`，自动分级 Info / Warning / Critical，Critical 推 Toast。默认规则无需配置即工作；自定义规则放 `~/.config/proc/alerts.toml`：

```toml
[[rule]]
metric = "CpuUsage"
op = "GT"
threshold = 90.0
consecutive_hits = 3
severity = "Warning"
```

### 终端录屏回放

VT100 终端完整录屏（v2 格式，保留 RGB 颜色 —— v1 旧版会褪色，已废弃）：

- `proc record` 启动录制（TUI 内按 `R` 大写开关），状态栏显示 REC 指示
- `proc replay <file>.prec` 回放：播放/暂停、逐帧 `←→`、倍速 `0.5× / 1× / 2× / 4×`、`Shift+←→` 跳 10 帧
- Ctrl+C 优雅退出，保证录制文件正常 flush（全局 `shutdown` 模块统一信号）
- 时间戳格式 `MM-DD HH:MM:SS`，与操作日志对齐

### CLI 子命令

不只是 TUI —— 命令行直接消费同一套采集层：

| 子命令 | 用途 |
|---|---|
| `proc ls --sort cpu --limit 20` | 进程列表（sort: cpu/mem/name/pid/disk_read/disk_write/net_sent/net_recv） |
| `proc tree` | 进程树 |
| `proc port 8080 [--kill] [--stats]` | 端口占用查询 / 终止 / TCP 质量摘要 |
| `proc kill <pid> [--force]` | 终止单进程 / 强制终止进程树 |
| `proc pkill <name> [--force --dry-run]` | 按名称批量终止（精确匹配，大小写不敏感） |
| `proc eject <drive> [--find-locks]` | USB 占用分析 / 详细句柄列表 |
| `proc who <path>`<sup>v0.5.0</sup> | 反查「谁占用这个文件 / 目录」 |
| `proc handles [--pid N] [--file path]`<sup>v0.5.0</sup> | 枚举指定 PID 的所有句柄 / 反查占用路径的 PID 列表 |
| `proc priority <pid> [--set normal]`<sup>v0.5.0</sup> | 查询 / 设置优先级（idle/belownormal/normal/abovenormal/high/realtime） |
| `proc affinity <pid> [--set 0xFF]`<sup>v0.5.0</sup> | 查询 / 设置 CPU affinity mask |
| `proc throttle <pid> on\|off`<sup>v0.7.0</sup> | Windows 11 EcoQoS / Efficiency Mode 切换（🍃 标记由 HeavyWorker 批量 query 维护） |
| `proc smart [device]`<sup>v0.5.0</sup> | SMART 磁盘健康（省略 device 列出所有磁盘） |
| `proc dns [--tail]`<sup>v0.5.0</sup> | DNS 查询日志（仅 Windows，内存 only） |
| `proc flows [--limit N] [--json]`<sup>v0.7.0 · 跨平台 v0.10.0</sup> | ProcessFlow 列表（Linux 走 eBPF connect+DNS 关联，Windows admin 走 Schannel ETW SNI；表格含「来源」列；ADR-0016 + ADR-0018） |
| `proc diag`<sup>v0.6.0</sup> | worker metrics JSON 输出（avg/max/polls/drops），bug 报告附上 |
| `proc monitor --add --pid N` / `--remove ID` | 监控管理（按 `--pid` / `--port` / `--command`） |
| `proc record` / `proc replay <file>` | VT100 录屏 |
| `proc export --format json\|csv [-o file] [--sort] [--limit]` | 进程数据导出（含 ISO-8601 本地时间戳） |
| `proc docker ps / inspect / top / logs / images / volumes / image-rm / volume-rm / compose / events / exec`<sup>v0.5.0</sup> | Docker 11 子命令 |
| `proc mcp serve`<sup>v0.7.0</sup> | 启动 MCP server（stdio transport），把上述 17+ 子命令暴露为 `proc_*` MCP tools 供 Claude Desktop / Cursor 等 LLM agent 调用 |

### MCP server（LLM agent 接入）<sup>v0.7.0</sup>

`proc mcp serve` 把 proc 的进程 / 网络 / DNS / Docker 能力暴露为 [MCP](https://modelcontextprotocol.io/) tools，让 Claude Code / Cursor / Windsurf 等客户端直接调用。详见 [`docs/adr/0009-mcp-server.md`](docs/adr/0009-mcp-server.md)。

**Claude Desktop 配置**（macOS：`~/Library/Application Support/Claude/claude_desktop_config.json`；Windows：`%APPDATA%\Claude\claude_desktop_config.json`）：

```json
{
  "mcpServers": {
    "proc": {
      "command": "proc",
      "args": ["mcp", "serve"]
    }
  }
}
```

**Cursor / Windsurf 配置**（项目 `.cursor/mcp.json` 或全局）：同上 schema，把 `command` 改成 proc 的绝对路径（如 `C:\Users\YOU\.cargo\bin\proc.exe`）。

**手动调试**：`npx mcp-inspector proc mcp serve` 在浏览器里看 schema、试调用。

**可用 tool**（17 个）：

| Tool | 对应 CLI | 返回 JSON |
|---|---|---|
| `proc_ls` | `proc ls` | `{ ok, sort, count, processes[] }` |
| `proc_tree` | `proc tree` | `{ ok, roots[] }`（递归） |
| `proc_port` | `proc port` | `{ ok, count, ports[] }` |
| `proc_kill` | `proc kill` | `{ ok, pid, result }` |
| `proc_pkill` | `proc pkill` | `{ ok, total, killed, failed, results[] }` |
| `proc_eject` | `proc eject` | `{ ok, devices[] | locks[] }` |
| `proc_who` | `proc who` | `{ ok, count, lockers[] }` |
| `proc_handles` | `proc handles --pid` | `{ ok, count, handles[] }` |
| `proc_priority` | `proc priority` | `{ ok, pid, action, priority }` |
| `proc_affinity` | `proc affinity` | `{ ok, pid, action, affinity_mask }` |
| `proc_smart` | `proc smart` | `{ ok, disks[] | disk }` |
| `proc_dns` | `proc dns` | `{ ok, count, queries[] }`（drain 一次，非 tail） |
| `proc_diag` | `proc diag --json` | `{ ok, workers[] }` |
| `proc_monitor_list` | 监控配置快照 | `{ ok, count, monitors[] }` |
| `proc_docker_ps` | `proc docker ps` | `{ ok, count, containers[] }` |
| `proc_docker_top` | `proc docker top` | `{ ok, count, processes[] }` |
| `proc_docker_logs` | `proc docker logs` | `{ ok, count, lines[] }`（非 follow） |

未暴露（对 LLM 无意义）：`proc_record` / `proc_replay` / `proc_export`。后续阶段追加：`proc_psi` / `proc_throttle` / `proc_disk_io` / `proc_flows`。

**字段裁剪**：`proc_ls` 不返回 `exe` / `cwd` / `user_id`，避免 LLM 上下文泄漏敏感路径（详见 ADR-0009）。

### 主题与持久化

**10 个内置主题**：Dark / Catppuccin / Dracula / Gruvbox / One Dark / Rose Pine / Nord / Solarized / Tokyo Night / Light。`t` 循环切换，选择持久化到 `~/.config/proc/theme.txt`。

**用户偏好持久化**：进程列表排序字段、首次启动引导 flag 都写入 `~/.config/proc/ui.toml`。

**首次启动**：`ui.toml` 缺失时显示一次性引导提示「按 `?` 查看快捷键」，按 `?` 后写盘 `first_run=false`，下次启动不再提示。

## 安装

```bash
# 方式 1：cargo binstall（5 秒装预编译版，推荐）
cargo install cargo-binstall
cargo binstall proc

# 方式 2：从源码编译（5 分钟）
git clone https://github.com/Alfroul/proc.git
cd proc
cargo build --release
./target/release/proc

# 方式 3：Windows 包管理器（v0.6.0+）
winget install Alfroul.proc
scoop install proc
```

### Shell 补全（v0.7.0+）

```bash
# 在线生成补全脚本到 stdout，重定向到对应 shell 的补全目录。
proc completions --shell bash    > ~/.bash_completion.d/proc
proc completions --shell zsh     > ~/.zsh/completions/_proc
proc completions --shell fish    > ~/.config/fish/completions/proc.fish
proc completions --shell powershell > $PROFILE
```

Release artifact 也附带预生成的 4 个补全文件（`completions/` 目录），scoop / winget 安装时会一并部署。

也可 `cargo install --path .` 装到 `~/.cargo/bin/`。

## 快捷键

按 `?` 在 TUI 内查看完整列表（带分组、可滚动）。

| 键 | 功能 |
|---|---|
| `1-6` | 切换面板 |
| **`Ctrl+P`**<sup>v0.7.0</sup> | **命令面板**（fuzzy 搜命令：kill / port panel / theme / sort by cpu / ...，~40 项直达，替代记忆键位） |
| `v` | 切换进程视图（列表 ↔ 应用分组） |
| `t` | 切换主题（持久化） |
| `?` | 帮助页 |
| `↑↓` / `PgUp PgDn` | 移动 / 翻页 |
| `Enter` | 详情 / 展开 / 折叠 |
| `Space` | 多选 |
| `/` | 搜索 |
| `←→` | 切换排序字段（持久化） |
| `S` | 直达安全分排序 |
| `k` / `K` | 终止 / 强制终止进程树 |
| `A` | 告警弹窗 |
| `R`（大写） | VT100 录制开关 |
| `Shift+←→` | 回放：跳 10 帧 |
| **`Tab` / `Shift+Tab`** | 详情页内切换 Inspector Tab（概要 / 环境 / 网络 / DLL / 句柄 / 内存）<sup>v0.5.0</sup> |
| **`+` / `-`**<sup>v0.5.0</sup> | 详情页 Summary Tab：调整进程优先级（Idle ↔ Realtime 6 档） |
| **`F5`**<sup>v0.6.0</sup> | 详情页：强制刷新 Inspector 数据（替代 'r'，对齐 Mission Center / htop；旧 'r' 兼容期显示 deprecation warning） |
| **`y`**<sup>v0.6.0</sup> | 详情页：复制进程信息到剪贴板（vim yank，替代 'c'；旧 'c' 显示 deprecation） |
| **`v`**<sup>v0.6.0</sup> | 详情页：切换 Env Tab 的 secret 脱敏（录屏中强制 mask） |
| **`Ctrl+F`**<sup>v0.5.0</sup> | 详情页句柄 / 内存 Tab：搜索过滤 |
| **`D`（大写）**<sup>v0.5.0</sup> | 端口面板：切换 DNS 查询日志子视图 |
| **`e`**<sup>v0.5.0</sup> | Docker 面板：exec 进容器（嵌入式 PTY） |
| **`l`**<sup>v0.5.0</sup> | Docker 面板：进入容器日志视图 |
| **`t`**<sup>v0.5.0</sup> | Docker 面板：查看容器内进程（docker top） |
| **`Tab`（Docker 面板）**<sup>v0.5.0</sup> | 切换 Containers / Images / Volumes 三视图 |
| `r`（详情页） | 重新采集 Inspector 数据（环境/网络/模块/句柄/内存） |
| `q` / `Esc` | 退出 / 清搜索（详情页内第一次 Esc 只退搜索，第二次才返回列表） |

> **v0.6.0 键位变更**：详情页 `r` → **`F5`**（刷新，对齐 Mission Center / htop）、`c` → **`y`**（vim yank 复制）、新增 `v` 切换 Env Tab secret 脱敏；Docker 面板 `r` → **`Shift+R`**（restart / 刷新镜像或卷）。旧 `r` / `c` 在 v0.6.0 兼容期会显示 deprecation warning 指引新键位，v0.7.0 移除。

详情页内的 `k` / `w` 保持原语义（终止 / 加监控）。各面板有额外快捷键，底部状态栏有提示。

## 命令行

```bash
proc                                              # 启动 TUI
proc ls --sort cpu --limit 20                     # 列出进程
proc ls --sort net_recv --limit 10                # 按下载速率排序
proc tree                                         # 进程树
proc port 8080                                    # 查看占用 8080 的进程
proc port 8080 --kill                             # 终止占用 8080 的进程
proc port --stats                                 # TCP 传输质量摘要
proc kill 1234                                    # 终止进程
proc kill 1234 --force                            # 强制终止进程树
proc pkill chrome.exe                             # 按名称终止
proc pkill chrome.exe --force --dry-run           # 强制 + 预览不实际终止
proc eject E:                                     # 检测 E 盘占用
proc eject E: --find-locks                        # 详细句柄列表
proc who Cargo.toml                               # 反查谁占用此文件
proc handles --pid 1234                           # 枚举 PID 1234 的所有句柄
proc handles --file Cargo.toml                    # 反查占用此路径的所有 PID
proc priority 1234                                # 查询 PID 1234 优先级
proc priority 1234 --set high                     # 设为 High
proc affinity 1234                                # 查询 affinity mask
proc affinity 1234 --set 0xFF                     # 设为 0xFF（前 8 核）
proc smart                                        # 列出所有磁盘 + 健康
proc smart /dev/sda                               # 查看 /dev/sda SMART 详情
proc smart '\\.\PhysicalDrive0'                   # Windows 物理磁盘 0
proc dns --tail                                   # 流式输出新 DNS 事件（仅 Windows）
proc diag                                         # 输出 worker metrics JSON（bug 报告附上）
proc monitor --add --pid 1234                     # 监控 PID
proc monitor --add --port 8080                    # 监控端口
proc monitor --add --command "cargo run"          # 监控并自动重启
proc monitor --remove 1                           # 删除监控（按 ID）
proc record                                       # 启动 TUI 并录制
proc replay recording.prec                        # 回放
proc export --format json --limit 20              # 导出 JSON 到 stdout
proc export --format csv -o procs.csv --sort mem  # 按内存导出 CSV 到文件
# Docker 11 子命令
proc docker ps                                    # 容器列表（默认）
proc docker inspect <name>                        # 容器详情
proc docker top <name>                            # 容器内进程
proc docker logs <name>                           # 一次性输出日志
proc docker logs <name> --follow --tail 100       # 跟随模式 + 末尾 100 行
proc docker images                                # 本地镜像
proc docker volumes                               # 卷列表
proc docker image-rm <id>                         # 删除镜像（两次确认）
proc docker image-rm <id> --force                 # 强制删除（即便 in_use）
proc docker volume-rm <name>                      # 删除卷
proc docker compose up -d                         # 转发给 docker-compose
proc docker events                                # 监听事件流（Ctrl+C 停）
proc docker exec <container>                      # exec 进容器（推断 shell）
proc docker exec <container> bash -lc "env"       # exec 指定命令
```

## 平台支持

**v0.12.0 起 Windows-only**（详见 [ADR-0022](docs/adr/0022-windows-only-platform.md)）。Windows 10 1809+ / Windows 11 x64。Linux / macOS 用户迁移路径：`git checkout v0.11.0`（最后含 Linux 代码的 release）。

**release CI 仅覆盖 1 个 target**（v0.12.0+）：`x86_64-pc-windows-msvc`。`cargo binstall proc` / `winget install Alfroul.proc` / `scoop install proc` 任选一种安装（Linux / macOS 的 binstall / winget / scoop 包不再发布，详见 [ADR-0022](docs/adr/0022-windows-only-platform.md)）。

| 功能 | Windows 10 1809+ / Windows 11 x64 |
|---|---|
| 进程列表 / 树 | ✅ |
| 进程分类（用户/系统/服务） | ✅ Win32 |
| 安全评分（签名） | ✅ WinVerifyTrust R16<sup>v0.11.0</sup> + 9 状态机<sup>v0.12.0</sup> + trusted_signers.toml<sup>v0.12.0</sup> |
| USB 助手 | ✅ |
| 降频检测 | ✅ |
| per-core 频率<sup>v0.5.0</sup> | ✅ CallNtPowerInformation |
| per-core 温度<sup>v0.5.0</sup> | ✅ ACPI |
| 每磁盘 I/O 速率 | ✅ |
| 每进程磁盘 I/O | ✅ sysinfo（IO 性能计数器） |
| **每进程磁盘 I/O（ETW 高精度）**<sup>v0.7.0</sup> | ✅ NT Kernel Logger + DiskIo TypeGroup1（管理员；非管理员降级到 sysinfo） |
| **GPU（多厂商）**<sup>v0.5.0</sup> | ✅ NVIDIA via NVML + DXGI / PDH utilization |
| **SMART 磁盘健康**<sup>v0.5.0</sup> | ✅ smartctl + WMI 降级 |
| **per-process 网络流量**<sup>v0.5.0</sup> | ✅ IP Helper |
| **DNS 查询日志**<sup>v0.5.0 · v0.11.0 ETW</sup> | ✅ ETW（默认）/ PowerShell（fallback，管理员判定 `proc diag` 看 `dns_collector`） |
| **TCP 传输质量**<sup>v0.5.0</sup> | ✅ GetTcpStatisticsEx2 |
| **进程句柄 Tab**<sup>v0.5.0</sup> | ✅ NtQuerySystemInformation |
| **内存映射 Tab**<sup>v0.5.0</sup> | ✅ VirtualQueryEx |
| **进程优先级 / affinity**<sup>v0.5.0</sup> | ✅ SetPriorityClass / SetProcessAffinityMask |
| **Windows 11 EcoQoS 切换**<sup>v0.7.0</sup> | ✅ SetProcessInformation(ProcessPowerThrottling) + 进程列表 🍃 标记 |
| **Schannel ETW TLS SNI**<sup>v0.10.0</sup> | ✅ `Microsoft-Windows-Schannel-Events` ETW event 1793（Win10 1809+ 管理员 + TDH 动态 schema） |
| **文件占用反查（who）**<sup>v0.5.0</sup> | ✅ filelocksmith |
| **v0.6.0 安全加固**（self-mitigation / env mask / restricted spawn） | ✅ |
| **v0.6.0 可观测性**（log rotate / crash report / worker metrics） | ✅ |
| **Docker**（ps/inspect/top/logs/images/volumes/exec） | ✅ |
| 进程级带宽（EStats） | ✅ |
| Toast 通知 | ✅ |
| 网络诊断 | ✅ |
| 录屏 / 告警 / 监控 | ✅ |

## FAQ

**需要管理员权限吗？** 基本功能不需要。管理员权限可启用进程级带宽监控（EStats）、终止某些系统进程、完整句柄枚举。非管理员自动降级。

**为什么 v0.12 移除 Linux / macOS 支持？**<sup>v0.12.0</sup> 详见 [ADR-0022](docs/adr/0022-windows-only-platform.md)。简短理由：(1) **维护成本**——Linux eBPF flow graph（ADR-0016）+ Linux PSI 监控（ADR-0013）+ Linux nvtop GPU + Linux nethogs 网络流量这些「Linux 杀手锏」需要 Linux 真机环境持续验证，开发者主要在 Windows 开发，Linux 路径长期挂着 `未在本机验证` 标签（TD-19）成为债务黑洞；(2) **聚焦**——proc 的核心价值在 Windows 平台深度（WinVerifyTrust / ETW / Schannel / EcoQoS / NT Kernel Logger / Win32 API），把精力集中在 Windows 让 Windows 体验做到最佳；(3) **简化**——Cargo.toml 删 libc / aya / aya-log 等 Linux 依赖，CI / release 只跑 1 个 target（之前 5 个），工具链简化让迭代更快。Linux / macOS 用户如需 v0.12+ 新功能（如 trusted_signers / mem% 修复 / 9 状态机）欢迎 fork。

**Linux / macOS 用户怎么办？**<sup>v0.12.0</sup> 三个选项：(1) **停留在 v0.11.0**——`git checkout v0.11.0` 或在 [releases](https://github.com/Alfroul/proc/releases) 下载 v0.11.0 二进制；v0.11.0 是最后含 Linux 代码的 release，含完整 eBPF / PSI / nvtop / nethogs 路径。(2) **fork 继续 Linux 维护**——proc 是 MIT 协议，欢迎社区 fork 维护 Linux 分支。(3) **替代方案**——Linux 推荐 [btop](https://github.com/aristocratos/btop)（TUI）/ [htop](https://htop.dev/)（TUI）/ [sysdig](https://sysdig.com/)（eBPF）；macOS 推荐 [Activity Monitor](https://support.apple.com/guide/mac-help/mchlp2529/mac)（内置）/ [iStat Menus](https://bjango.com/mac/istatmenus/)。详见 ADR-0022 migration path 段。

**GPU 信息不显示？**
- Windows：仅显示 NVIDIA（via NVML），其他显卡走 DXGI 显示 VRAM（utilization/temp/power 仅 NVIDIA）
- Linux：安装 `nvtop` 后自动启用 AMD / Intel / NVIDIA 全厂商监控
- macOS：暂不支持

**DNS 日志记录写到哪？** **永不持久化**。仅在内存中保留最近 1000 条，退出 proc 即丢失。这是隐私设计。如需长期记录请用 Windows 事件查看器导出 `Microsoft-Windows-DNS-Client/Operational` channel。

**容器 exec 跟直接 `docker exec` 有什么区别？** TUI 内按 `e` 进入的是嵌入式 PTY 视图（`portable-pty` + `vt100` crate），ANSI 渲染在 ratatui 内部；CLI `proc docker exec` 直接透传 stdio，等价 `docker exec -it`。两者底层都 spawn `docker exec -it <container> <shell>` 子进程，docker CLI 处理所有 daemon 通信。

**smartctl 未安装？** Windows 装 smartctl 后 proc 自动用，未装时退化到 WMI `MSStorageDriver_FailurePredictStatus`（仅预测失败聚合状态，无详细属性）。

**录屏会泄漏什么？**<sup>v0.6.0</sup> 录屏（VT100 recording）会捕获屏幕所有内容含 DNS 域名 / 进程 cmd / env 真值（如果 reveal 打开）。v0.6.0 起按 `R` 触发录屏时**先弹确认对话框**（按 `y` 确认 / `n` 取消），并在录屏期间强制 Env Tab 走 mask 模式（即便 `env_reveal=true` 也强制 mask）。录屏文件存 `~/.config/proc/recordings/*.prec`，**永不自动上传**。

**self-mitigation 开了哪些策略？**<sup>v0.6.0</sup> 5 项：DEP（Permanent）/ ASLR（HighEntropy）/ ProhibitDynamicCode / DisableExtensionPoints / **ImageLoad（NoRemote + NoLow + PreferSystem32）**。**不开 ProcessSignaturePolicy**（会让 nvml-wrapper 未签名 native 依赖挂）。详见 [ADR-0008](docs/adr/0008-self-mitigation-policy.md)。可在 Process Explorer → Properties → Image File → Mitigation flags 验证。

**crash report 在哪？**<sup>v0.6.0</sup> `~/.config/proc/crashes/` 下：主线程 panic → `crash-{YYYYMMDD-HHMMSS}.txt`；worker 线程 panic → `crash-worker-{name}-{ts}.txt`。文件含时间戳 + proc 版本 + panic info + `Backtrace::force_capture()`。报 bug 时把对应文件附上。

**日志为什么不覆盖了？**<sup>v0.6.0</sup> v0.5.0 以前启动时 `File::create` truncate 覆盖旧日志，崩溃前最后一段全丢。v0.6.0 起改为 `tracing-appender::RollingFileAppender::daily`，每天一个文件 `proc.logYYYY-MM-DD`，自动清理 7 天前的日志。

**如何启用 eBPF flow graph？**<sup>v0.7.0 · v0.12 移除</sup> ~~仅 Linux 平台，需自行编译~~。**v0.12 起 Windows-only，eBPF flow graph 路径已删（ADR-0022）**——`src/ebpf/` 整模块删除，Cargo.toml `ebpf` feature flag 删除。Linux 用户迁移路径：`git checkout v0.11.0` 仍可用旧 eBPF 路径（详见 ADR-0016，Status 改 Superseded by ADR-0022）。Windows 用户走 [ADR-0018 Schannel ETW TLS SNI](docs/adr/0018-windows-schannel-sni.md) 路径替代——端口面板按 `F` 进入 Flow 子视图，CLI 用 `proc flows`。

**eBPF 需要什么权限？**<sup>v0.7.0 · v0.12 移除</sup> ~~root 或 `CAP_BPF` + `CAP_PERFMON` capability，内核 ≥ 5.10~~。**v0.12 起 Windows-only，eBPF 路径已删**（见上一条「如何启用 eBPF flow graph」与 [ADR-0022](docs/adr/0022-windows-only-platform.md)）。

**R15 安全评分（外联行为）怎么触发？**<sup>v0.7.0</sup> 默认不启用。需要显式创建 `~/.config/proc/sni_whitelist.txt`（一行一个允许的域名，`#` 开头注释），R15 才激活。两条命中条件（任一扣 30 分）：dns_name 不在白名单 / 10s 内 ≥ 50 个不同 IP（端口扫描特征）。空文件 = "所有 dns_name 都不在白名单"（用户自负）。详见 [ADR-0016](docs/adr/0016-ebpf-flow-graph.md#securityrule-r15-外联行为评分)。

**进程名旁边的 🔒 / ⚠️ / ❓ 是什么？**<sup>v0.11.0</sup> 签名状态标记（ADR-0021）：🔒 Trusted（签名链追溯到微软 / 已知 CA）/ ⚠️ Unsigned 或 Revoked（无签名 / 签名被吊销）/ ❓ Unknown（验证失败 / 非管理员运行）。`Pending`（启动后头 1-2 个 heavy refresh 内的默认值）和 `Signed`（已签名但非受信 CA）不显示 emoji 避免列宽波动。Inspector Summary Tab 显示完整状态（如「签名: 受信签名 (微软/已知 CA)」）。CLI 走 `proc ls --filter 'security_score < 80'` 可过滤出扣分进程。详见 [ADR-0021](docs/adr/0021-process-signature-verification.md)。

**proc 显示我的应用是 ⚠️（无签名），但我明明有签名？**<sup>v0.11.0 · v0.12.1 改进</sup> 三步排查：(1) proc 是否以管理员身份运行？**非 elevated 时 `verify_signature` 直接返回 Unknown（不调 `WinVerifyTrust`），v0.12.1 起进程列表不显示任何 emoji（Unknown 与 Pending / Signed 同款空串），进 Inspector Summary Tab 才能看到「未知（需管理员权限）」状态**——以管理员身份重启 proc 即可激活 WinVerifyTrust 真实验证；(2) 签名链是否完整？中间证书缺失会让 `WinVerifyTrust` 返 `TRUST_E_SUBJECT_NOT_SIGNED` 或链断裂错误（落入 Unknown / ChainError）；(3) 是不是 `.cat` 文件签名？驱动 + 系统组件走 `.cat` 关联签名，`WinVerifyTrust` 直接验 `.exe` 会返 Unsigned——这是已知限制，留 TD。跑 `proc ls --json | jq '.[] | select(.signature_status=="Unsigned")'` 拿到完整列表后用 `sigcheck /a your.exe`（Sysinternals）交叉验证。

**Windows Flow graph 怎么用？**<sup>v0.10.0</sup> Windows admin 自动启用 Schannel ETW worker（不需要 feature flag）：启动 proc → 端口面板按 `F` 切到 Flow 子视图 → curl / 浏览器触发 TLS handshake → 看到 SNI 列表。CLI 走 `proc flows` 同款显示，`proc flows --json` 输出含 `"source": "schannel"` 字段。非管理员 / Win10 < 1809 / x86 进程 → worker 启动失败 / 不 fire，UI 显示降级提示。详见 [ADR-0018](docs/adr/0018-windows-schannel-sni.md)。

**Windows 用户在 Flow 子视图看到 0 条怎么办？**<sup>v0.10.0</sup> 三步排查：(1) `winver` 查 Windows 版本，需 Win10 1809+（build 17763+）/ Win11；(2) 以管理员身份启动 proc（非管理员 worker 启动失败）；(3) 触发 TLS handshake 后等 1-2s（Schannel event 1793 在 DeleteSecurityContext 时 fire，连接关闭瞬间才有）。仍 0 条？跑 `proc diag` 看 `schannel_etw` worker 行的 `poll_count` 是否增长——若增长但 SNI 空，说明 callback 收到 event 但 TDH 解析失败，请附 `~/.config/proc/logs/proc.log` 报 issue。

**proc 显示 R17 可疑父子链命中，但我的 Word/Excel 启动 cmd 是合法脚本？**<sup>v0.11.0</sup> R17 是「典型 macro attack 链」启发式检测——扣分不代表恶意，只是符合攻击模式。三步排查：(1) Inspector Summary Tab 顶部红色警告下方有完整 chain（`WINWORD.EXE → cmd.exe`），核对是不是你预期的脚本；(2) 进入详情页看 `命令行:` 字段（`proc inspect <pid>` 同款），确认 cmd 参数是预期脚本路径而非 base64 / encoded payload；(3) 如果确实合法想消除警告，目前 R17 内置 pattern 不支持白名单（v0.11.0 后续会加），但可以在 `~/.config/proc/lineage_rules.toml` 配自定义规则替代默认检测——例如把 weight 调低到 5：
   ```toml
   # 注：内置 OfficeToShell/BrowserToShell/ScriptInterpreter 仍会扣 35/25/15，
   # 此处仅用于追加自定义规则（无法 override 内置 weight）。
   [[rule]]
   name = "my_editor_to_shell"
   parent_pattern = "(?i)my_editor"
   child_pattern = "(?i)(cmd|powershell)"
   weight = 5
   ```
   详见 [tech-debt](docs/tech-debt.md)（R17 内置 pattern 白名单待加）。

**proc 显示 R18 命中（`suspicious_path_*` / `[⚠ 可疑位置]`），但我的便携应用就放在 AppData？**<sup>v0.11.0</sup> R18 是「malware 常见启动位置」启发式检测——`%TEMP%` / `%APPDATA%` / `%LOCALAPPDATA%` / `%USERPROFILE%\Downloads` 都是用户可写目录，合法便携应用确实会放在这些位置。三步排查：(1) 看进程是否同时命中 R16（未签名）——Inspector Summary Tab 顶部若有「未签名 + 可疑路径协同命中（双重特征强信号）」警告，强烈建议扫描病毒；签名应用（🔒 / 空 emoji）通常合法。(2) 便携应用确实需要放 AppData 时，可在 `~/.config/proc/path_rules.toml` 配置中接受该路径（虽然不能直接白名单内置 kind，但可以确认权重符合预期）：
   ```toml
   # 自定义可疑目录（不会影响内置 Temp/AppData/LocalAppData/Downloads 判定）
   [[suspicious_dir]]
   name = "my_portable_app"
   path = "%USERPROFILE%\\my_portable"  # 支持 %VAR% / ${VAR} / $VAR 占位符
   weight = 5                            # 缺省 25
   reason = "便携应用残留路径"
   ```
   (3) R18 与 v0.6 path_check（temp_dir / downloads_dir）**叠加扣分**——Temp 路径会同时扣 25（temp_dir）+ 20（suspicious_path_temp）+ 协同 10 = 55 分（未签名情况下）。这是 surgical 原则下的设计：保留 v0.6 path_check 不动，R18 作为独立入口叠加扣分（同 R17 + v0.7 office_spawning_shell 的处理模式）。如果只能接受单次扣分，参考 [CONTEXT.md](CONTEXT.md) R18 段了解评分逻辑。

**worker 崩溃了怎么办？**<sup>v0.6.0 · v0.11.0 自动重启</sup> TUI 顶部会渲染红色 banner（`[worker name] panicked: <message>`），按 `D` 清空。同时 crash report 写到 `crashes/crash-worker-*.txt`。**v0.11.0 起自动热恢复**：worker panic 后按指数退避（5s / 30s / 5min）自动 respawn；3 次失败永久死亡需重启 proc（`WorkerManager::restart` 实装，TD-4 清零）。banner 三态显示（restarting / restarted / permanent failure）。详见 [ADR-0019](docs/adr/0019-worker-restart-policy.md)。

**为什么我的 DNS 日志延迟还是高 / 漏抓？**<sup>v0.11.0</sup> v0.11 起默认走 ETW（CPU < 0.5%、延迟 < 50ms），但管理员权限是硬要求——非管理员自动降级到 PowerShell probe（v0.5.0 路径保留）。跑 `proc diag`，末尾的 `dns_collector: <kind>` 行反映实际类型（`etw` / `powershell` / `none`）。如果是 `powershell` 想用 ETW：以管理员身份启动 proc 即可。Linux / macOS 没有 DNS-Client ETW provider，DNS 日志功能不可用。

**FilterExpr `cpu > 5` 在 Flow 子视图（`F` 进入）为什么过滤掉所有 flow？**<sup>v0.11.0</sup> v0.11 阶段 8 REVIEW-13 P1-2 修复——Flow 视图走 `apply_network` 求值上下文，process 字段（cpu/mem/name/...）在该 ctx 下永远 false（无 ProcessInfo），用户写后会过滤掉所有 flow。CLI `proc flows --filter 'cpu > 5'` 会打印 warn 提示「Flow 字段：sni/dns_name/remote_addr/remote_port/bytes_out/bytes_in/source，详见 ADR-0011」+ 退出 1；TUI 同款 UX 缺口留 TD 归档（需更深状态机协调）。Flow 视图正确语法：`sni =~ /google\.com$/` / `remote_port = 443` / `source = schannel` / `dns_name in ("a.com", "b.com")`。

**R16 / R17 / R18 触发条件分别是什么？**<sup>v0.11.0</sup> 三档评分规则：
- **R16 签名**（第 1 步接入，ADR-0021）：每个进程 `.exe` 走 `WinVerifyTrust` 6 状态机——Unsigned 扣 20 / Revoked 扣 35 / Signed（已签名但非受信 CA）扣 10 / Unknown（验证失败 / 非管理员）扣 5（仅 Windows）/ Trusted / Pending 不扣分。
- **R17 可疑父子链**（第 17 步）：`OfficeToShell`（Word/Excel/PowerPoint → cmd/powershell/wscript）扣 35；`BrowserToShell`（Chrome/Edge/Firefox → cmd/powershell）扣 25；`ScriptInterpreter`（wscript/cscript/mshta 直接运行）扣 15。可在 `~/.config/proc/lineage_rules.toml` 加自定义规则。
- **R18 可疑启动路径**（第 18 步）：`%TEMP%` 扣 20 / `%APPDATA%` / `%LOCALAPPDATA%` / `%USERPROFILE%\Downloads` 各扣 15；与 R16 协同（Unsigned/Revoked + 可疑路径同时命中）额外扣 10。系统目录（Program Files / Windows / System32）白名单不扣分。可在 `~/.config/proc/path_rules.toml` 加自定义目录。
- 跨规则**叠加扣分**（surgical 原则——保留 v0.6 path_check / v0.7 office_spawning_shell 不动，R17/R18 作为独立入口叠加）：典型 macro attack 模式 `未签名 + 临时目录 + Word → cmd` 可累加扣到 100+ 分。

**R16 状态机 9 个变体分别扣多少分？**<sup>v0.12.0</sup> v0.12 阶段 3 TD-26 扩 `WinVerifyTrust` HRESULT 状态机，从 v0.11 6 变体扩到 9 变体：
- `Trusted` / `Pending`：不扣分（受信 CA 或尚未触发验证）
- `Signed`：扣 10（已签名但非受信 CA）
- `ChainError`：扣 10（证书链断裂 / 名称不匹配 / 签名无效——验证不完整）
- `Expired` / `UntrustedRoot`：各扣 15（证书过期 / 不受信根，曾经受信但有问题）
- `Unsigned`：扣 20（无签名）
- `Revoked`：扣 35（签名被吊销，曾经受信但被 CA 撤销）
- `Unknown`：扣 5（Windows 非管理员降级 / 验证 API 错误）

badge 显示：🔒 Trusted / ⚠️ Unsigned / Revoked / Expired / UntrustedRoot / ❓ ChainError / 空 Pending / Signed / Unknown（v0.12.1：Unknown 从 ❓ 改空串避免非 admin 全屏噪音）。详见 [ADR-0021](docs/adr/0021-process-signature-verification.md)。

**proc 显示 Adobe / Docker / Cisco 等进程是 ⚠️ 或扣分（Signed），但我配了 `trusted_signers.toml` 还是不升级到 🔒？**<sup>v0.12.0</sup> v0.12 阶段 3 TD-27 落地，三步排查：(1) **CompanyName 字段值**：`trusted_signers.toml` 匹配的是 FileVersion Information 的 `CompanyName`（不是 X.509 certificate subject CN）。用 `sigcheck /a your.exe`（Sysinternals）或 PowerShell `(Get-Item your.exe).VersionInfo.CompanyName` 看真实值。(2) **regex 大小写**：用户 `vendor_pattern` 默认大小写敏感——需要不敏感请加 `(?i)` 前缀（如 `(?i)^adobe`）。内置 24 vendor 列表（v0.12 扩）已含 Adobe / Cisco / Oracle / VMWare / Docker / Red Hat / Apache / Python / GitHub / Electron / AMD 等，零配置即生效。(3) **配置文件位置 + 格式**：`~/.config/proc/trusted_signers.toml`（Windows = `C:\Users\{user}\.config\proc\trusted_signers.toml`），格式 `[[signer]] name / vendor_pattern / reason(可选)`，TOML 解析失败 / regex 编译失败会**静默降级为空**——查 `~/.config/proc/proc.log` 看 `trusted_signers rule「xxx」vendor_pattern 正则编译失败` 警告。示例：
   ```toml
   [[signer]]
   name = "my_company"
   vendor_pattern = "(?i)^MyCompany"  # (?i) 让匹配大小写不敏感
   reason = "内部应用"
   ```

**终端异常？** 退出后执行 `reset` 恢复。

**FilterExpr 怎么写 CIDR / URL / 含 `/` 的正则？**<sup>v0.12.0</sup> v0.12 阶段 4 TD-28 起支持 `\/` 转义。例子：`remote_addr =~ /192\.168\.1\.0\/24/`（CIDR）/ `sni =~ /https:\/\/example\.com/`（URL）/ `cmd =~ /C:\/Users\/admin/`（Windows 路径）。parser 把 `\/` 转成单 `/`（regex crate 不接受 `\/` 作为有效转义），其他 `\X`（如 `\.` `\d` `\w`）原样保留让 regex 解释。旧表达式（无 `\/`）行为不变。

**FilterExpr `mem > 5%` 命中几乎全部进程？**<sup>v0.12.0</sup> v0.12 阶段 4 TD-30 修复 silent bug——v0.11 前 `mem > 5%` 字面量被解释为「mem_bytes > 5.0」（字节值与百分号数字直接比较），几乎全部进程命中。修复后 `mem + %` 按 `mem / total_memory * 100` 与百分号字面量比较（`mem > 50%` 在 16GB 系统上等价 `mem > 8GB`）。`cpu > 5%` 与 `cpu > 5` 仍等价（cpu 自身就是 0-100 标度）；`disk_read > 5%` / `net_sent > 5%` 等没有自然除数的字节字段保留 legacy 行为（surgical：不在 EvalCtx 加 disk_total / net_total 等字段）。total_memory 在测试场景（panel_with_procs 传 0）退回 legacy 行为避免 div by zero。

**配置文件在哪？** `~/.config/proc/` 下：`theme.txt`（主题索引）、`ui.toml`（排序偏好）、`alerts.toml`（告警规则）、`proc.logYYYY-MM-DD`（运行日志，daily rotate 保留 7 天）、`crashes/`（panic crash report）、`recordings/`（默认录制路径）。

**如何查看详细日志？** 日志默认写到 `~/.config/proc/proc.logYYYY-MM-DD`（每天 rotate，保留 7 天）。用 `RUST_LOG` 调级别：

```bash
RUST_LOG=proc=debug proc                 # debug 级别
RUST_LOG=proc::security=trace proc       # 仅安全模块 trace
RUST_LOG=proc::port_map=debug proc ls    # CLI 子命令也生效
```

未设置 `RUST_LOG` 时默认级别为 `info`。

**如何报 worker 性能问题？**<sup>v0.6.0</sup> 跑 `proc diag` 输出所有 worker 的 metrics JSON（avg_us/max_us/polls/drops/last_error），附在 bug 报告里。TUI 内按 `?` 进入帮助页也可看精简版（带 `✓` / `⚠` 健康徽章）。

## Benchmark

v0.13.0 起仓库含 criterion benchmark suite（6 个 hot path：搜索 / 排序 / heavy refresh / TUI 渲染 / 录屏序列化 / FilterExpr apply）。本地跑：

```bash
cargo bench                                  # 跑全部 6 个 benchmark
cargo bench --bench bench_refresh_heavy      # 单独跑一个
```

输出在 `target/criterion/<name>/<fixture>/new/estimates.json`（含 mean / median / stddev）。**不在 CI 跑**——criterion 在 GitHub Actions 共享 runner 抖动大，仅本地手跑。

当前 baseline 数字（13th Gen Intel i7-13700HX / Win11 / Rust 1.95.0）见 [`docs/reviews/PERF-BASELINE-v0.13.md`](docs/reviews/PERF-BASELINE-v0.13.md)。**核心结论**：proc 当前架构在 1000 进程规模下无显著性能瓶颈；唯一 mean > 5 ms 的 hot path（parent_chain 16.5 ms @ 1000 进程）在 worker 独立线程不阻塞 UI 帧预算。报「卡」时附 `cargo bench` 数字可大幅降低定位成本。

## License

MIT（仓库根目录 LICENSE 文件）
