# proc

Rust 编写的交互式系统进程管理器，集进程管理、网络分析、U盘占用检测、进程监控、Docker 管理、安全评分于一体。

## 功能概览

| 面板 | 快捷键 | 核心能力 |
|---|---|---|
| 进程列表 | `1` | 按 CPU/内存/名称/PID 排序，模糊搜索，多选批量终止 |
| 进程树 | `2` | 树形展示父子关系，孤儿/僵尸检测，折叠/过滤 |
| 端口/网络 | `3` | 三种视图（按端口/按进程/按远程），网络诊断工具箱，异常检测 |
| U盘助手 | `4` | 占用检测，风险分级，缓存刷新，安全弹出引导 |
| 进程监控 | `5` | 按 PID/端口/命令监视，崩溃自动重启，Toast 通知 |
| Docker | `6` | 容器列表，实时事件流，健康检查，资源统计 |

## 功能详情

### 进程列表

- 按 CPU%/内存%/名称/PID 排序（`←→` 切换）
- 模糊搜索（`/`），多选（`Space`），全选（`a`）
- 批量终止（`k`）或强制终止进程树（`K`）
- 进程分类标签：用户（蓝）/系统（红）/服务（黄）
- 分页浏览（`PageUp`/`PageDown`）
- 侧边栏：CPU/内存/交换区使用率 + 火花线图、GPU 信息、磁盘空间、网卡 IP、温度、运行时间
- 详情页（`Enter`）：进程完整信息 + 剪贴板复制（`c`）+ 添加监控（`w`）
- 安全评分徽章：基于数字签名、父进程链、路径等 6 维度评估进程可信度

### 进程树

- 树形展示进程父子关系，展开/折叠（`Enter`）
- 孤儿进程一键选择（`o`），僵尸/残存进程一键选择（`z`）
- 过滤：全部 / 仅用户进程 / 仅系统进程（`f`）
- 搜索（`/`），多选终止，单个终止（`k`）

### 端口/网络

三种视图通过 `g` 键循环切换：

- **按端口视图** — 每行一个连接，按状态分组（ESTABLISHED → LISTEN → 其他TCP → UDP）
- **按进程视图** — 每行一个进程，聚合 TCP/UDP 连接数、各状态计数、远程地址数，管理员模式下显示实时收发速率
- **按远程视图** — 每行一个远程 IP，聚合连接和进程，显示 IP 分类标签（本机/内网/公网）和云厂商识别（AWS/GCP/Azure/Cloudflare 等）

网络诊断工具箱（远程视图按 `d`）：

| 工具 | 说明 |
|---|---|
| Ping | 测试可达性和延迟 |
| DNS 反查 | IP → 主机名解析 |
| Whois | 查询 IP 归属信息 |
| Traceroute | 追踪路由路径 |
| 端口探测 | 扫描 15 个常用端口 |

异常检测（`a`）：自动检测 CLOSE_WAIT 堆积、单 IP 高连接数、连接突增、新 LISTEN 端口、TIME_WAIT 洪水等 6 种异常模式，Critical 级别 Toast 通知。

### U盘助手

- 自动检测可移除设备，扫描所有进程（含系统隐藏进程）的文件句柄占用
- 占用进程按风险分级：危险（红）/警告（黄）/安全（绿）/系统（白）
- 一键终止安全进程，写入缓存刷新（`Write-VolumeCache`）
- 持续监测模式（`w`）：每 5 秒扫描，确认无占用后提示安全弹出

### 进程监控

- 三种监控类型：按 PID、按端口、按命令
- 崩溃自动重启：指数退避策略（1s → 30s），可配置最大重试次数
- 端口监视：检测端口占用/释放事件
- 状态管理：暂停/恢复（`s`）、手动重启（`r`）、删除（`d`）
- Windows Toast 通知：进程崩溃、端口变化、严重异常自动推送

### Docker 监控

- 容器列表：名称、镜像、端口映射、状态、运行时间
- 实时事件流（`a`）：监听容器 start/stop/die/health_status 事件
- 健康检查状态展示，容器资源统计（CPU/内存/网络/块 I/O）
- 容器操作：重启（`r`）、停止（`s`）
- 支持命名管道（Windows Docker Desktop）和 TCP（WSL Docker）两种连接方式

### 安全评分

每个进程附带安全评分徽章（A/B/C/D/F 五级），基于以下维度：

- 数字签名验证（Authenticode）
- 父进程链完整性
- 可执行文件路径检查
- 命令行参数可疑模式
- 进程关系异常检测
- 综合加权评分

### 录屏与回放

VT100 终端录屏，完整捕获每帧渲染内容（包括光标、搜索状态等），支持回放、暂停、逐帧查看、倍速播放。

- `proc record` — 启动 TUI 并自动录制
- `proc replay <file>` — 回放录制文件
- 回放控制：Space 播放/暂停，←→ 逐帧/快进，+/- 调速，Home/End 跳转，Q 退出

### 告警系统

可配置的阈值告警规则，支持 CPU/内存/磁盘/网络/连接数等指标，连续命中触发，自动分级（Info/Warning/Critical），集成 Toast 通知。

## 主题

6 种内置主题，按 `t` 切换：Dark、Catppuccin、Dracula、Nord、Solarized、Tokyo Night。

## 快捷键

| 键 | 功能 |
|---|---|
| `1-6` | 切换面板 |
| `t` | 切换主题 |
| `q` | 退出 |
| `↑↓` | 移动光标 |
| `Enter` | 详情/展开/折叠 |
| `Space` | 多选 |
| `/` | 搜索 |
| `k` / `K` | 终止 / 强制终止 |
| `Esc` | 清除搜索/关闭弹窗 |

各面板有额外快捷键，底部状态栏有提示。

## 命令行

```bash
proc                                              # 启动交互式 TUI
proc ls                                           # 列出所有进程
proc ls --sort cpu --limit 20                     # 按CPU排序显示前20
proc tree                                         # 进程树
proc port                                         # 显示所有端口映射
proc port 8080                                    # 查看占用8080端口的进程
proc port 8080 --kill                             # 终止占用8080端口的进程
proc kill 1234                                    # 终止进程
proc kill 1234 --force                            # 强制终止进程树
proc eject E:                                     # 检测E盘占用进程
proc eject E: --locks                             # 详细句柄列表
proc monitor add --pid 1234                       # 监控指定PID
proc monitor add --port 8080                      # 监控端口
proc monitor add --command "cargo run"            # 监控并自动重启命令
proc monitor list                                 # 列出所有监控
proc monitor remove 1                             # 删除监控
proc record                                       # 启动 TUI 并录制
proc replay recording.prec                        # 回放录制文件
proc docker                                       # Docker容器面板
proc docker ps                                    # 列出容器
proc docker inspect <name>                        # 容器详情
proc docker watch                                 # 监听事件
```

## 安装

```bash
git clone https://github.com/<your-username>/proc.git
cd proc
cargo build --release
```

二进制在 `target/release/proc.exe`。也可 `cargo install --path .` 安装到 `~/.cargo/bin/`。

## 技术栈

| 组件 | 方案 |
|---|---|
| 语言 | Rust 2024 Edition |
| TUI | ratatui 0.29 + crossterm 0.28 |
| CLI | clap 4 |
| 进程信息 | sysinfo 0.34 + Win32 API 兜底 |
| 端口映射 | netstat2（GetExtendedTcpTable） |
| 进程级带宽 | Win32 GetPerTcpConnectionEStats |
| 文件句柄 | filelocksmith（NtQuerySystemInformation） |
| 进程分类 | Win32 EnumServicesStatusExW |
| 安全评分 | Win32 Authenticode 签名验证 |
| Toast 通知 | WinRT ToastNotification |
| Docker | bollard 0.18（命名管道 + TCP） |
| 异步 | tokio |
| GPU | nvml-wrapper（可选 feature） |
| 序列化 | serde + bincode（录屏格式） |

## 项目结构

```
src/
├── main.rs              # CLI 入口 + 子命令路由
├── app.rs               # 应用状态机 + 事件循环
├── cli.rs               # clap 子命令定义
├── collect.rs           # sysinfo 数据采集 + tasklist/WinAPI 兜底
├── classify.rs          # 进程分类（用户/系统/服务）
├── kill.rs              # 进程终止
├── tree.rs              # 进程树构建
├── port_map.rs          # 端口映射 + 聚合 + IP 分类 + 云厂商检测
├── estats.rs            # Windows EStats TCP 连接带宽采集
├── anomaly.rs           # 网络异常检测
├── diag.rs              # 网络诊断工具箱
├── gpu.rs               # GPU 信息采集
├── format.rs            # 格式化工具
├── error.rs             # 统一错误类型
├── security/            # 安全评分（签名/父链/路径/命令行）
├── alert/               # 阈值告警（规则引擎 + 状态管理）
├── docker/              # Docker 监控（连接/事件/健康/统计）
├── monitor/             # 进程监控（watchdog/端口/快照/通知）
├── eject/               # U盘助手（设备/句柄/分级/缓存）
├── record/              # 录屏（VT100 帧捕获 + 旧格式兼容）
└── tui/                 # TUI 界面（布局/主题/各面板组件）
```

约 15000 行 Rust 代码，10 个测试文件。

## FAQ

**需要管理员权限吗？**
基本功能不需要。管理员权限可启用进程级带宽监控（EStats）、终止某些系统进程、完整句柄枚举。非管理员自动降级。

**GPU 信息不显示？**
依赖 NVIDIA NVML 库，非 NVIDIA 显卡暂不支持。`cargo build --release --no-default-features` 可禁用。

**退出后终端异常？**
执行 `reset` 恢复。

## License

MIT
