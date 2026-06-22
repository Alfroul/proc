# proc

Rust 编写的交互式 TUI 系统进程管理器。把 **进程管理 + 网络分析 + USB 占用 + 监控 + Docker + 安全评分 + 降频检测 + 磁盘 I/O + 终端录屏 + 告警 + SMART 磁盘健康 + per-process 网络流量 + DNS 查询日志 + 容器 exec** 融合到一个 TUI 中。Windows 主开发平台，Linux/macOS 可降级运行。

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

macOS 等非 Win/Linux 平台，环境 / DLL / 句柄 / 内存 Tab 显示「此平台不支持」降级提示。详情页内 `r` 强制重新采集、`/` 搜索、`Tab/Shift+Tab` 切 Tab、`+` / `-` 调整优先级、`a` 调整 affinity、`Esc` 先退搜索再退页面（双层语义）。

### 安全评分

每个进程附带 **0-100 分**（100 = 安全）。基于 **14 项独立检查**：

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

Windows 平台专属。`DnsLogCollector` trait + `PowershellDnsCollector` 实现：spawn 长跑 `powershell.exe` 子进程订阅 `Microsoft-Windows-DNS-Client/Operational` channel event 3010，reader 线程解析 JSON 行 + sysinfo PID 名 lookup + `sync_channel(1000)` 推到主线程。500ms 周期 drain，主线程 cap=1000 FIFO。

- **TUI 内**：端口面板按 `D`（大写）激活 DNS 子视图，显示最近 DNS 查询列表
- **详情页 Network Tab**：底部展示该 PID 最近 5 条 DNS 查询
- **CLI**：`proc dns --tail` 流式输出新事件
- **异常规则 R9**：新 PID 首次发起 DNS 查询且不在白名单 → Warning

**隐私承诺**：DNS 查询记录**永不持久化**到磁盘，仅在内存中保留最近 1000 条；`record/frame.rs` 序列化类型不含 `DnsQuery`。

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
| `proc smart [device]`<sup>v0.5.0</sup> | SMART 磁盘健康（省略 device 列出所有磁盘） |
| `proc dns [--tail]`<sup>v0.5.0</sup> | DNS 查询日志（仅 Windows，内存 only） |
| `proc monitor --add --pid N` / `--remove ID` | 监控管理（按 `--pid` / `--port` / `--command`） |
| `proc record` / `proc replay <file>` | VT100 录屏 |
| `proc export --format json\|csv [-o file] [--sort] [--limit]` | 进程数据导出（含 ISO-8601 本地时间戳） |
| `proc docker ps / inspect / top / logs / images / volumes / image-rm / volume-rm / compose / events / exec`<sup>v0.5.0</sup> | Docker 11 子命令 |

### 主题与持久化

**10 个内置主题**：Dark / Catppuccin / Dracula / Gruvbox / One Dark / Rose Pine / Nord / Solarized / Tokyo Night / Light。`t` 循环切换，选择持久化到 `~/.config/proc/theme.txt`。

**用户偏好持久化**：进程列表排序字段、首次启动引导 flag 都写入 `~/.config/proc/ui.toml`。

**首次启动**：`ui.toml` 缺失时显示一次性引导提示「按 `?` 查看快捷键」，按 `?` 后写盘 `first_run=false`，下次启动不再提示。

## 快速开始

```bash
git clone https://github.com/Alfroul/proc.git
cd proc
cargo build --release
./target/release/proc         # 启动 TUI
```

也可 `cargo install --path .` 装到 `~/.cargo/bin/`。

## 快捷键

按 `?` 在 TUI 内查看完整列表（带分组、可滚动）。

| 键 | 功能 |
|---|---|
| `1-6` | 切换面板 |
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
| **`a`**<sup>v0.5.0</sup> | 详情页 Summary Tab：调整 CPU affinity |
| **`Ctrl+F`**<sup>v0.5.0</sup> | 详情页句柄 / 内存 Tab：搜索过滤 |
| **`D`（大写）**<sup>v0.5.0</sup> | 端口面板：切换 DNS 查询日志子视图 |
| **`e`**<sup>v0.5.0</sup> | Docker 面板：exec 进容器（嵌入式 PTY） |
| **`l`**<sup>v0.5.0</sup> | Docker 面板：进入容器日志视图 |
| **`t`**<sup>v0.5.0</sup> | Docker 面板：查看容器内进程（docker top） |
| **`Tab`（Docker 面板）**<sup>v0.5.0</sup> | 切换 Containers / Images / Volumes 三视图 |
| `r`（详情页） | 重新采集 Inspector 数据（环境/网络/模块/句柄/内存） |
| `q` / `Esc` | 退出 / 清搜索（详情页内第一次 Esc 只退搜索，第二次才返回列表） |

详情页内的 `k` / `w` / `c` 保持原语义（终止 / 加监控 / 复制信息）。各面板有额外快捷键，底部状态栏有提示。

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

Windows 是主开发平台。Linux/macOS 可编译运行，依赖 Win32 API 的功能不可用，启动时状态栏会一次性提示降级清单。

| 功能 | Windows | Linux | macOS |
|---|---|---|---|
| 进程列表 / 树 | ✅ | ⚠️ 基础 | ⚠️ 基础 |
| 进程分类（用户/系统/服务） | ✅ Win32 | ⚠️ 启发式 | ⚠️ 启发式 |
| 安全评分（签名） | ✅ | ⚠️ 仅行为 | ⚠️ 仅行为 |
| USB 助手 | ✅ | ❌ | ❌ |
| 降频检测 | ✅ | ❌ | ❌ |
| per-core 频率<sup>v0.5.0</sup> | ✅ | ✅ sysfs cpufreq | ❌ |
| per-core 温度<sup>v0.5.0</sup> | ✅ ACPI | ✅ hwmon | ❌ |
| 每磁盘 I/O 速率 | ✅ | ❌ | ❌ |
| 每进程磁盘 I/O | ✅ | ✅ | ✅ |
| **GPU（多厂商）**<sup>v0.5.0</sup> | ✅ NVIDIA via NVML | ✅ AMD/Intel/NVIDIA via nvtop | ❌ |
| **SMART 磁盘健康**<sup>v0.5.0</sup> | ✅ smartctl + WMI 降级 | ✅ smartctl | ✅ smartctl |
| **per-process 网络流量**<sup>v0.5.0</sup> | ✅ IP Helper | ✅ nethogs 子进程 | ❌ |
| **DNS 查询日志**<sup>v0.5.0</sup> | ✅ PowerShell | ❌（pcap 留 0.6.0+） | ❌ |
| **TCP 传输质量**<sup>v0.5.0</sup> | ✅ GetTcpStatisticsEx2 | ✅ /proc/net/snmp | ❌ |
| **进程句柄 Tab**<sup>v0.5.0</sup> | ✅ NtQuerySystemInformation | ✅ /proc/\<pid\>/fd | ❌ |
| **内存映射 Tab**<sup>v0.5.0</sup> | ✅ VirtualQueryEx | ✅ /proc/\<pid\>/maps | ❌ |
| **进程优先级 / affinity**<sup>v0.5.0</sup> | ✅ SetPriorityClass / SetProcessAffinityMask | ✅ setpriority / sched_setaffinity | ❌ |
| **文件占用反查（who）**<sup>v0.5.0</sup> | ✅ filelocksmith | ⚠️ lsof 启发式 | ⚠️ lsof 启发式 |
| **Docker**（ps/inspect/top/logs/images/volumes/exec） | ✅ | ✅ | ✅ |
| 进程级带宽（EStats） | ✅ | ❌ | ❌ |
| Toast 通知 | ✅ | ❌ | ❌ |
| 网络诊断 | ✅ | ✅ | ✅ |
| 录屏 / 告警 / 监控 | ✅ | ✅ | ✅ |

## FAQ

**需要管理员权限吗？** 基本功能不需要。管理员权限可启用进程级带宽监控（EStats）、终止某些系统进程、完整句柄枚举。非管理员自动降级。

**GPU 信息不显示？**
- Windows：仅显示 NVIDIA（via NVML），其他显卡走 DXGI 显示 VRAM（utilization/temp/power 仅 NVIDIA）
- Linux：安装 `nvtop` 后自动启用 AMD / Intel / NVIDIA 全厂商监控
- macOS：暂不支持

**DNS 日志记录写到哪？** **永不持久化**。仅在内存中保留最近 1000 条，退出 proc 即丢失。这是隐私设计（详见 ADR-0006）。如需长期记录请用 Windows 事件查看器导出 `Microsoft-Windows-DNS-Client/Operational` channel。

**容器 exec 跟直接 `docker exec` 有什么区别？** TUI 内按 `e` 进入的是嵌入式 PTY 视图（`portable-pty` + `vt100` crate），ANSI 渲染在 ratatui 内部；CLI `proc docker exec` 直接透传 stdio，等价 `docker exec -it`。两者底层都 spawn `docker exec -it <container> <shell>` 子进程，docker CLI 处理所有 daemon 通信。

**smartctl 未安装？** Linux/macOS 必须装 `smartctl`（smartmontools 包）；Windows 装 smartctl 后 proc 自动用，未装时退化到 WMI `MSStorageDriver_FailurePredictStatus`（仅预测失败聚合状态，无详细属性）。

**终端异常？** 退出后执行 `reset` 恢复。

**配置文件在哪？** `~/.config/proc/` 下：`theme.txt`（主题索引）、`ui.toml`（排序偏好）、`alerts.toml`（告警规则）、`proc.log`（运行日志）、`recordings/`（默认录制路径）。

**如何查看详细日志？** 日志默认写到 `~/.config/proc/proc.log`（启动时覆盖旧文件）。用 `RUST_LOG` 调级别：

```bash
RUST_LOG=proc=debug proc                 # debug 级别
RUST_LOG=proc::security=trace proc       # 仅安全模块 trace
RUST_LOG=proc::port_map=debug proc ls    # CLI 子命令也生效
```

未设置 `RUST_LOG` 时默认级别为 `info`。日志在每次启动时被覆盖（truncate）—— 如需保留历史请配合外部 logrotate。

## License

MIT（仓库根目录 LICENSE 文件）
