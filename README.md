# proc

Rust 编写的交互式系统进程管理器。集进程管理、网络分析、U 盘占用检测、监控、Docker、安全评分、降频检测、磁盘 I/O 监控、终端录屏回放于一体。Windows 主开发平台，Linux/macOS 可降级运行。

## 功能

`proc` 把 **进程管理 + 网络分析 + 安全评分 + 资源监控 + U 盘占用 + 进程守护 + Docker + 终端录屏 + 告警** 融合到一个 TUI 中，所有面板共享一份系统快照，避免 `System::new_all()` 反复构造（ADR 阶段 5 `SysinfoRegistry` 全局单例）。

### 6 大主面板

| 面板 | 切换 | 能力 |
|---|---|---|
| 进程列表 | `1` | 按 CPU / 内存 / PID / 名称 / 安全分 / 磁盘读 / 磁盘写 **7 字段排序**（持久化到 `ui.toml`），模糊搜索，多选批量终止，`v` 切应用分组视图（按 `.exe` 聚合，CPU/内存/进程数三字段） |
| 进程树 | `2` | 父子层级展开/折叠，孤儿 / 僵尸 / 残存进程检测，`o` 一键选孤儿、`z` 一键选僵尸 |
| 端口/网络 | `3` | 按端口 / 按进程 / 按远程三种视图；6 种异常模式自动告警（CLOSE_WAIT 堆积、TIME_WAIT 异常、远程地址爆炸等）；网络诊断工具箱 Ping / DNS 反查 / Whois / Traceroute / 端口探测 |
| U 盘助手 | `4` | 句柄占用检测（`filelocksmith`）+ 风险分级（Safe / Warning / Critical）+ 缓存刷新 + 安全弹出引导 + 持续监测模式 |
| 监控 | `5` | 按 PID / 端口 / 命令三种 Target；`NotifyOnly` 或 `AutoRestart` 指数退避策略；Critical 告警推 Toast |
| Docker | `6` | 容器列表 + 实时事件流 + 健康检查 + 资源统计；支持命名管道 / TCP 双连接；事件流 `sync_channel(64)` 背压（ADR-0006） |

### 进程深挖（Inspector）<sup>v0.4.0</sup>

进程列表/树中按 `Enter` 进入详情页，顶部 **4 个 Tab** 切换深挖视图（ADR-0004 B2 方案 —— 单一入口，向后兼容原详情页）：

| Tab | 内容 | 数据源 |
|---|---|---|
| **概要** | 分类 / 父进程 / CPU / 内存 / 磁盘 / 运行时长 / exe / cmd / cwd / 端口摘要 / 网络汇总 / 安全分 / 风险因子 | sysinfo + `port_map` |
| **环境** | 进程环境变量列表（`KEY=VALUE`），`/` 大小写不敏感搜索过滤，`↑↓ PgUp PgDn Home End` 滚动 | Win: PEB walk (`NtQueryInformationProcess` + `ReadProcessMemory`)；Linux: `/proc/<pid>/environ` |
| **网络** | 该 PID 的全部监听与连接：协议 / 本地 / 远程 / 状态 / 进程名 | 复用 `port_map::find_ports_by_pid`（ADR 阶段 5 优化路径） |
| **DLL** | 已加载模块（Windows DLL / Linux `.so`），按路径字母排序，`/` 搜索；表格列：路径 / 基址 / 大小 | Win: `CreateToolhelp32Snapshot` 与 `security/dll_check` 同源；Linux: 解析 `/proc/<pid>/maps` 合并 r-xp / r--p / rw-p 多段映射 |

macOS 等非 Win/Linux 平台，环境与 DLL Tab 显示「此平台不支持」降级提示。详情页内 `r` 强制重新采集、`/` 搜索、`Tab/Shift+Tab` 切 Tab、`Esc` 先退搜索再退页面（双层语义）。

### 安全评分

每个进程附带 **0-100 分**（100 = 安全）。基于 **14 项独立检查**（CONTEXT.md `RiskCategory`）：

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

按 `S`（大写）按安全分排序，可疑进程排最前；详情页 **概要 Tab** 展示所有风险因子与扣分。后台评分线程（`BackgroundScorer`）通过 channel 异步计算，不阻塞 UI；Drop 时 take 出 sender 再 join，规避 bounded channel 死锁（ADR-0006）。

### 系统资源监控（侧边栏）

- **降频检测** 实时识别 CPU 降频原因（热 / 功耗 / 空闲），侧边栏显示 `⚠THERMAL` / `⚠POWER`（Win32 `CallNtPowerInformation`）
- **温度** CPU/GPU 颜色分级（< 70°C 绿 / 70-79 黄 / 80-89 橙 / ≥ 90 红）
- **磁盘 I/O** 每磁盘独立读写速率 + 每进程 I/O 速率（`(pid, start_time)` 键防 PID 复用串数据，ADR-0003）
- **GPU** NVIDIA NVML（温度 / 显存 / 功率 / 利用率），cfg-gated 到 Windows（AMD/Intel 列入 0.5.0+ 路线图）
- **侧边栏其他** CPU/内存/交换区使用率 + 火花线图（30 秒历史）、网卡 IP、运行时间

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
| `proc ls --sort cpu --limit 20` | 进程列表 |
| `proc tree` | 进程树 |
| `proc port 8080 [--kill]` | 端口占用查询 / 直接终止 |
| `proc kill <pid> [--force]` | 终止单进程 / 强制终止进程树 |
| `proc pkill <name> [--force --dry-run]` | 按名称批量终止（精确匹配，大小写不敏感）|
| `proc eject <drive> [--locks]` | U 盘占用分析 / 详细句柄列表 |
| `proc monitor add/list/remove` | 监控管理（`--pid` / `--port` / `--command`）|
| `proc record` / `proc replay <file>` | VT100 录屏 |
| `proc export --format json\|csv [-o file] [--sort] [--limit]` | 进程数据导出（含 ISO-8601 本地时间戳，无 chrono 依赖）|
| `proc docker ps / inspect <name> / watch` | Docker 子命令 |

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
| `Tab` / `Shift+Tab` | 详情页内切换 Inspector Tab（概要/环境/网络/DLL）|
| `r`（详情页） | 重新采集 Inspector 数据（环境/网络/模块）|
| `q` / `Esc` | 退出 / 清搜索（详情页内第一次 Esc 只退搜索，第二次才返回列表）|

详情页内的 `k` / `w` / `c` 保持原语义（终止 / 加监控 / 复制信息）。各面板有额外快捷键，底部状态栏有提示。

## 命令行

```bash
proc                                              # 启动 TUI
proc ls --sort cpu --limit 20                     # 列出进程
proc tree                                         # 进程树
proc port 8080                                    # 查看占用 8080 的进程
proc port 8080 --kill                             # 终止占用 8080 的进程
proc kill 1234                                    # 终止进程
proc kill 1234 --force                            # 强制终止进程树
proc pkill chrome.exe                             # 按名称终止
proc pkill chrome.exe --force --dry-run           # 强制 + 预览不实际终止
proc eject E:                                     # 检测 E 盘占用
proc eject E: --locks                             # 详细句柄列表
proc monitor add --pid 1234                       # 监控 PID
proc monitor add --port 8080                      # 监控端口
proc monitor add --command "cargo run"            # 监控并自动重启
proc monitor list / remove 1                      # 列出 / 删除监控
proc record                                       # 启动 TUI 并录制
proc replay recording.prec                        # 回放
proc export --format json --limit 20              # 导出 JSON 到 stdout
proc export --format csv -o procs.csv --sort mem  # 按内存导出 CSV 到文件
proc docker ps / inspect <name> / watch           # Docker 子命令
```

## 平台支持

Windows 是主开发平台。Linux/macOS 可编译运行，依赖 Win32 API 的功能不可用，启动时状态栏会一次性提示降级清单。

| 功能 | Windows | Linux | macOS |
|---|---|---|---|
| 进程列表 / 树 | ✅ | ⚠️ 基础 | ⚠️ 基础 |
| 进程分类（用户/系统/服务） | ✅ Win32 | ⚠️ 启发式 | ⚠️ 启发式 |
| 安全评分（签名） | ✅ | ⚠️ 仅行为 | ⚠️ 仅行为 |
| U 盘助手 | ✅ | ❌ | ❌ |
| 降频检测 | ✅ | ❌ | ❌ |
| 每磁盘 I/O 速率 | ✅ | ❌ | ❌ |
| 每进程磁盘 I/O | ✅ | ✅ | ✅ |
| 进程级带宽（EStats） | ✅ | ❌ | ❌ |
| Toast 通知 | ✅ | ❌ | ❌ |
| 网络诊断 | ✅ | ✅ | ✅ |
| Docker / 录屏 / 告警 | ✅ | ✅ | ✅ |
| GPU（NVIDIA NVML） | ✅ | ❌ | ❌ |

## GPU 路线图

当前仅支持 NVIDIA NVML（可选 feature `nvidia`，`cargo build --no-default-features` 可禁用）。AMD/Intel 暂未支持，列入 **0.5.0+** 路线图（0.3.0 / 0.4.0 聚焦 Inspector 与稳定性，暂不动 GPU）：

- **AMD**：Linux sysfs DRM 已有清晰路径；Windows 需 ADL SDK
- **Intel**：Linux sysfs i915 已有清晰路径；Windows 待评估

核心需求是 GPU 温度 + 显存使用率，对齐现有 NVML 路径。如需优先支持请提 issue。

## FAQ

**需要管理员权限吗？** 基本功能不需要。管理员权限可启用进程级带宽监控（EStats）、终止某些系统进程、完整句柄枚举。非管理员自动降级。

**GPU 信息不显示？** 当前仅支持 NVIDIA。非 NVIDIA 显卡静默跳过，其他功能不受影响。

**终端异常？** 退出后执行 `reset` 恢复。

**配置文件在哪？** `~/.config/proc/` 下：`theme.txt`（主题索引）、`ui.toml`（排序偏好）、`alerts.toml`（告警规则）、`proc.log`（运行日志）、`recordings/`（默认录制路径）。

**如何查看详细日志？** 日志默认写到 `~/.config/proc/proc.log`（启动时覆盖旧文件）。用 `RUST_LOG` 调级别：

```bash
RUST_LOG=proc=debug proc                 # debug 级别（端口扫描 / 评分耗时等）
RUST_LOG=proc::security=trace proc       # 仅安全模块 trace
RUST_LOG=proc::port_map=debug proc ls    # CLI 子命令也生效
```

未设置 `RUST_LOG` 时默认级别为 `info`。日志在每次启动时被覆盖（truncate）—— 如需保留历史请配合外部 logrotate。

**LICENSE 何时添加？** 已在 0.2.0 添加 MIT，文件位于仓库根目录。

## License

MIT
