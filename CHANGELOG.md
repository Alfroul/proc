# Changelog

本项目的所有重要变更都会记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
并遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 新增

- **进程列表排序字段持久化**：`←`/`→`/`S` 切换排序时写入 `~/.config/proc/ui.toml`，下次启动恢复上次选择（解决"每次打开都要手动切内存"的痛点）。新增 `src/ui_state.rs` 模块 + 4 个单元测试。
- **3 个新主题**：Gruvbox（暖色复古）、One Dark（Atom 默认配色）、Rose Pine（柔和现代）。`THEMES` 从 7 个扩展到 10 个，`t` 循环切换。

## [0.2.0] - 2026-06-15

本次发布聚焦于代码质量、性能优化和用户体验打磨。共修复 6 个真实 Bug，完成 6 项架构整洁，新增 9 项 UX 改进，4 项跨平台与文档增强，5 项测试覆盖，3 项性能优化。

### 阶段 8 — Review：全局审查 + 脏区域优化（收尾）

- 全局代码审查通过：`cargo test --release`（297 个测试，0 失败）、`cargo clippy --all-targets -- -D warnings`（0 警告）、`cargo fmt --all -- --check`（无 diff）、`cargo build --release --no-default-features`（通过）。
- 33 项问题逐一核对：#1 VT100 RGB、#2 跨平台时区、#3 watchdog try_wait、#4 sysinfo 散落、#5 排序 O(N²)、#6 Arc<Vec> 共享、#7 panels/tui 重命名、#8 AppMode 死代码、#9 tick ≤ 50 行（实测 33 行）、#10 replay ≤ 50 行（实测 24 行）、#11 deprecated 删除、#12 scan_ports 不再 new_all、#13 help_panel.rs、#14 主题持久化、#15 Ctrl+C handler、#16 时间格式含月-日、#17 README 隐藏快捷键公开、#18 THEMES 长度 = 7、#19 Command::Export、#20 read_line 已废弃、#21 Command::Pkill、#22 README 平台支持、#23 LICENSE/CHANGELOG、#24 CI workflow、#25 README GPU 路线图、#26 test_record_color、#27 test_scorer_concurrency、#28 test_kill_tree、#29 skeleton 合并、#30 test_platform_compat、#31 select_nth_unstable、#32 脏区域（见下方"性能优化"）、#33 tick_history_sample 抽离 —— **全部落地**。
- Performance: 脏区域优化经真实测量后决定**不动代码** —— ratatui 内置 buffer diff 已实现 Cell 级增量传输，`App::tick` 已用 `needs_draw` 判断避免无谓重绘，每帧 draw 调用成本 < 15ms（20 fps 预算 50ms）。激进 dirty rect 优化的复杂度收益比差，且引入回归风险。完整分析见 [`docs/dirty-rect-analysis.md`](docs/dirty-rect-analysis.md)。
- Performance 基线回归（500 进程基准，见 `tests/test_stage8_perf_regress.rs`）：`rebuild_sorted_cache` **38.2 µs**（< 5ms 目标，130× 裕量）、top-N `select_nth_unstable` + 局部排序 **6.1 µs**（< 1ms 目标，160× 裕量）。无回退。
- Removed: `SystemSnapshot` 中未使用的 `prev_process_disk` / `prev_process_disk_time` 字段（被 `App` 同名字段独立实现，注释明确标记 `#[allow(dead_code)]` "used via App, not directly in SystemSnapshot"），同时移除 `per_disk_io_speed` 上过时的 TODO 注释（功能已实现）。
- Pedantic 现状：`cargo clippy -- -W clippy::pedantic` 共 ~287 个 `format!` 风格建议、~119 个 `#[must_use]` 缺失、~63 个 cast 精度提示等，全部为风格偏好而非 bug。`-D warnings` 等级 0 警告。本阶段不修 pedantic，留作未来风格统一批次。



本次打磨按 8 个阶段组织，预期产出如下（细节见 `docs/stages/`）。

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
- Changed: `VT100_VERSION` 提升到 2，旧版本 v1 文件回放时给出友好错误（详见 ADR 0003）。
- Added: `docs/adr/0003-VT100-RGB-颜色编码方案.md`。

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
- Added: `docs/adr/0005-Ctrl+C-优雅退出设计.md`。
- Updated: `README.md` 命令行章节补充 `proc pkill` 示例。

### 阶段 5 — Slice：性能优化

- Performance: 引入 `SysinfoRegistry` 全局单例（`src/collect.rs` 顶部 `SYSINFO_REGISTRY` + `sysinfo_with`），消除 5 处散落的 `sysinfo::System::new_all()` 调用 —— `port_map::scan_ports`、`eject::locks::find_volume_lockers_with_processes`（原循环内每个未命中 PID `new_all` 一次，最差 O(N × 200ms)）、`kill::kill_single` 非 Windows 分支、`kill::find_processes_by_name` 全部改为只读访问 SysinfoRegistry 快照。详见 `docs/adr/0004-sysinfo-单例收敛方案.md`。
- Removed: `SystemSnapshot::processes()` 老 deprecated 方法（被 `cached_processes_vec` 替代），同步迁移 `tests/test_alert.rs`、`tests/test_process_list.rs`、`tests/test_skeleton.rs` 中 9 处调用，删除已无意义的 `test_incremental_refresh_consistent_with_full`。
- Performance: `App::rebuild_sorted_cache` 从 O(N²) 改为 O(N) —— 一次性构造 `PID → idx` HashMap 替代循环内 `Vec::position` 查找，同时把内部 `Vec<(class, ProcessInfo)>` 改为 `Vec<(class, &ProcessInfo)>` 借引用，省去每帧全字段深拷贝。
- Performance: top-N 进程排序使用 `slice::select_nth_unstable_by` —— 500 进程时比较次数从 ~4500 (O(N log N)) 降到 ~786 (O(N) + O(K log K))，sparkline 历史采样路径受益。
- Performance: `BackgroundScorer::request` 签名从 `Vec<ProcessInfo>` / `Vec<PortEntry>` 改为 `Arc<Vec<...>>`，为下游消费者共享而非拷贝铺路；score 线程循环改用 `as_ref()` 切片迭代。
- Performance: `global_cpu_history` / `global_mem_history` 显式落到 light refresh（每秒）采样；`proc_history` 仍依赖 heavy refresh 的新 cached_processes，只在 heavy 帧推数据，retain 清理保留在 light 中。sparkline 现每秒一格。

### 阶段 6 — Slice：架构整洁

- Refactor: `src/panels/` 重命名为 `src/view_models/`，避免与 `src/tui/`（纯渲染层）在目录名层面混淆 —— 前者持有面板状态 + 业务逻辑（MVVM 中的 ViewModel 角色），后者无状态。`ProcessPanel` 等类型名保持不变，避免改名风暴；外部 import 路径由 `crate::panels::*` 改为 `crate::view_models::*`（涉及 `src/lib.rs` 和 `src/app.rs`）。详见 `docs/adr/0002-panels-vs-tui-命名重构决策.md`。
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
