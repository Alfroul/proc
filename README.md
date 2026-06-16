# proc

Rust 编写的交互式系统进程管理器。集进程管理、网络分析、U 盘占用检测、监控、Docker、安全评分、降频检测、磁盘 I/O 监控、终端录屏回放于一体。Windows 主开发平台，Linux/macOS 可降级运行。

## 功能

| 面板 | 切换 | 能力 |
|---|---|---|
| 进程列表 | `1` | 按 CPU/内存/PID/名称/安全分/磁盘读写排序，模糊搜索，多选批量终止，`v` 切应用分组视图 |
| 进程树 | `2` | 父子层级，孤儿/僵尸/残存检测，`o`/`z` 一键选中异常 |
| 端口/网络 | `3` | 按端口/进程/远程三种视图，网络诊断（Ping/DNS/Whois/Traceroute/端口探测），异常检测 |
| U 盘助手 | `4` | 句柄占用检测 + 风险分级 + 缓存刷新 + 安全弹出引导 |
| 监控 | `5` | 按 PID/端口/命令监视，崩溃自动重启（指数退避），Toast 通知 |
| Docker | `6` | 容器列表、实时事件流、健康检查、资源统计 |

### 安全评分

每个进程附带 0-100 分（100 = 安全）。基于 14 项独立检查：Authenticode 数字签名、父进程链完整性、可执行文件路径、命令行可疑模式、网络行为、名称仿冒、资源异常、子进程爆炸、权限提升、svchost 完整性、DLL 加载、令牌权限审计、签名信誉缓存。

按 `S`（大写）按安全分排序，可疑进程排最前。详情页（`Enter`）展示所有风险因子。

### 系统监控

- **降频检测**：实时识别 CPU 降频（热/功耗/空闲），侧边栏显示 `⚠THERMAL` / `⚠POWER`
- **温度**：CPU/GPU 温度颜色分级（< 70°C 绿 / 70-79 黄 / 80-89 橙 / ≥ 90 红）
- **磁盘 I/O**：每磁盘独立读写速率 + 每进程 I/O 速率追踪
- **GPU**：NVIDIA NVML（温度、显存、功率、利用率）

### 录屏回放

VT100 终端完整录屏（包含光标、搜索状态等每帧渲染内容）。`proc record` 启动录制，`proc replay <file>` 回放（播放/暂停、逐帧、倍速、跳转）。

### 告警系统

可配置阈值规则（CPU/内存/磁盘/网络/连接数/温度/降频），连续命中触发，自动分级 Info/Warning/Critical，Critical 推 Toast。默认规则无需配置即工作；自定义规则放 `~/.config/proc/alerts.toml`。

```toml
[[rule]]
metric = "CpuUsage"
op = "GT"
threshold = 90.0
consecutive_hits = 3
severity = "Warning"
```

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
| `q` / `Esc` | 退出 / 清搜索 |

各面板有额外快捷键，底部状态栏有提示。

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

## 主题

10 种内置主题，`t` 切换：**Dark / Catppuccin / Dracula / Gruvbox / One Dark / Rose Pine / Nord / Solarized / Tokyo Night / Light**。选择持久化到 `~/.config/proc/theme.txt`。

进程列表的排序字段也持久化到 `~/.config/proc/ui.toml` —— 切到内存排序后，下次启动直接是内存视图。

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

当前仅支持 NVIDIA NVML（可选 feature `nvidia`，`cargo build --no-default-features` 可禁用）。AMD/Intel 暂未支持：

- **AMD**：Linux sysfs DRM 已有清晰路径；Windows 需 ADL SDK
- **Intel**：Linux sysfs i915 已有清晰路径；Windows 待评估

核心需求是 GPU 温度 + 显存使用率，对齐现有 NVML 路径。时间表未定，如需优先支持请提 issue。

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
