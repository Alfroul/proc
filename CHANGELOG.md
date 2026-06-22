# Changelog

本项目的所有重要变更都会记录在此文件中。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
并遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 阶段 9 — Slice：E2 exec 进容器（PTY 集成）

- Added (E2): `src/docker/exec.rs` 新模块 —— `ContainerExec` 句柄持有 portable-pty master / writer / child / reader 通道。`start(container, cmd, image)` 用本地 PTY spawn `docker exec -it <container> <shell>` 子进程；docker CLI 处理所有 daemon 通信（命名管道 / TCP / unix socket）+ 远端 PTY 分配。reader 线程循环 `master.reader.read` → `sync_channel(64)` 背压 → 主线程 tick `drain()` 拼接字节喂 `vt100::Parser::process`。`write_all(bytes)` 转发按键字节（含 ANSI 转义）到 PTY writer。`resize(cols, rows)` 同步 PTY 尺寸（SIGWINCH 由 `-t` 自动转发容器）。`is_finished()` 检测 child 退出。`detect_default_shell(image)` 纯函数按镜像名推断 shell（alpine/busybox → `/bin/sh`，ubuntu/debian/centos/fedora/rust/golang/python/node → `/bin/bash`，其它 → `/bin/sh` 兜底）。详见 `docs/adr/0007-container-exec-pty-bridge.md`。
- Added (E2): `src/tui/container_exec_view.rs` —— 嵌入式终端渲染。`draw()` 用 `Layout::vertical` 分顶部（容器名 + 退出提示）/ PTY 输出区 / 底部（PTY 尺寸 + 快捷键）。PTY 区遍历 `vt100::Screen::cell(r, c)`，每 cell 取 `contents()` / `fgcolor()` / `bgcolor()` / `bold()` / `italic()` / `underline()` / `inverse()` 写到 ratatui buffer；光标位置取反色高亮。`vt_color_to_ratatui(Color)` 把 vt100 `Color` 枚举（Default → Reset / Idx → Indexed / Rgb → Rgb）转 ratatui Color；`vt_attrs_to_modifier(cell)` 把 bold/italic/underline/inverse 转 Modifier。
- Added (E2): `src/app_panel.rs::AppMode::ContainerExec` 新变体 —— 从 DockerPanel 按 `e` 进入；`Ctrl+D` / `Ctrl+\` / 子进程退出时切回 DockerPanel。`PanelContext` 新增 `pending_container_exec: &'a mut Option<String>` 字段，DockerPanel 按 `e` 时塞容器名 + 返回 `SwitchMode(ContainerExec)`，`App::switch_mode` 取出启动 PTY。
- Added (E2): `src/app.rs::App` 新增 `container_exec: Option<ContainerExec>` / `container_exec_vt: Option<vt100::Parser>` / `pending_container_exec_target: Option<String>` / `container_exec_exit_msg: Option<String>` 4 个字段。`enter_container_exec()` 从 target 启动 PTY（失败回退 DockerPanel + 错误提示）；`tick_container_exec()` 每帧 drain PTY 字节喂 vt100 + 检测 child 退出；`handle_container_exec_key(key)` 把 `KeyEvent` 转 ANSI 字节序列（Enter=`\r`、Ctrl+C=`\x03`、Ctrl+D=`\x04`、Ctrl+\\=`\x1c`、Backspace=`\x7f`、Tab=`\t`、Up=`\x1b[A`、Down=`\x1b[B`、Right=`\x1b[C`、Left=`\x1b[D`、Home/End/PageUp/PageDown/Delete 全 ANSI 序列、Alt+x=`\x1b` + x、普通字符直接字节）写 PTY writer；`resize_container_exec(cols, rows)` 同步 PTY + vt100 parser 尺寸。`switch_mode` 在退出 ContainerExec 时主动 drop PTY + parser（避免 fd 泄漏）。
- Changed (E2): `src/view_models/docker_panel.rs::DockerPanel::handle_key` 新增 `e` 分支 → `enter_exec_mode(ctx)` 设置 `pending_container_exec` + 返回 `SwitchMode(ContainerExec)`。容器视图 + 容器运行状态检查；非容器视图 / 未选中容器 / 容器未运行时返回 `Consumed` + 友好错误消息。
- Changed (E2): `src/tui/layout.rs::draw_main_panel` 新增 `AppMode::ContainerExec` 分支调用 `container_exec_view::draw`；`draw_footer` 新增 exec 模式快捷键栏；`tab_index` 把 ContainerExec 映射到 Docker tab。
- Changed (E2): `src/tui/mod.rs::handle_events` 新增 `Event::Resize` 分支（之前忽略）触发 `App::notify_terminal_resized`；`run_app` 在 draw 之后若 `mode == ContainerExec`，按 area 实际尺寸调 `resize_container_exec(area.w, area.h)`。
- Added (CLI): `src/cli.rs::DockerSub::Exec { container, cmd: Vec<String> }` 子命令；`src/main.rs::run_docker_exec(monitor, container, cmd)` —— 验证容器存在 + 根据 image 推断 shell + spawn `docker exec -it` 子进程透传 stdio（docker CLI 接管用户终端，等价直接调 `docker exec`）。
- Added (Cargo): 新依赖 `portable-pty = "0.9"`（跨平台 PTY 抽象，Windows ConPTY / Linux POSIX PTY）、`vt100 = "0.15"`（ANSI 字节流解析 + Screen 状态）。
- Added (ADR-0007): `docs/adr/0007-container-exec-pty-bridge.md` —— 解释为何选 spawn `docker exec -it` 子进程而非走 bollard exec Attached 流（方案 B/C）：① 方案 C 描述的「portable-pty master/slave pair + bollard Attached 流双向中转」技术上不成立（PTY slave 端需子进程才有意义）；② 方案 B 可行但放弃（不引入 portable-pty 违背 stage-9.md 明确要求）；③ 方案 A 让 docker CLI 处理所有 daemon 连接差异（Docker Desktop 命名管道 / WSL Docker TCP / Linux unix socket），proc 不感知。
- Added: `tests/test_container_exec.rs`（新）—— 集成测试：`detect_default_shell` 多 image 推断矩阵 / PTY 字节转换纯函数 / `ContainerExec::start` 不 PATH docker 时优雅报错（cfg-gate：仅在 Docker 可用的 CI 环境 smoke）/ `PtyChunk::default` 空。
- Test: 总测试数 568 → 预计 **575+**（+7 左右）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过。
- Note: 已知限制 —— 需要 PATH 有 `docker` 二进制（与既有 `proc docker compose` 一致）；Windows ConPTY 需 Windows 10 1809+；首次 spawn 延迟 ~50ms（docker CLI 启动 + daemon 连接）；exec 模式下 Ctrl+C 走 KeyEvent 转发容器（raw mode 下 crossterm 不传 SIGINT），其它模式下 Ctrl+C 走全局 shutdown 不变。

### 阶段 8 — Slice：D3 DNS 查询日志（Windows PowerShell Get-WinEvent）

- Added (D3): `src/dns_log/mod.rs` 新模块 —— `DnsLogCollector` trait 抽象 DNS 查询日志数据源（`fn drain(&mut self) -> Vec<DnsQuery>` / `fn provider_name(&self) -> &'static str`），`Send + Sync` 让 collector 可跨 worker 线程传递。参考阶段 6/7 的 trait 模式（[`crate::gpu::GpuProvider`] / [`crate::net_flow::NetFlowCollector`]）。`DnsQuery { timestamp, pid, process_name, query_name, query_type, result }` derive `Serialize/Deserialize` 仅用于内存 round-trip 测试；**永不持久化到磁盘**（隐私）。`DnsResult { Success(Vec<IpAddr>) | NxDomain | Timeout | Error(String) }` + `from_windows_status(status, results)` 把 Win32 错误码映射到语义结果。`parse_query_type(raw)` 把数字（"1" → "A"，"28" → "AAAA"，"65" → "HTTPS" 等）转 RFC 1035 助记符；`parse_query_results(raw)` 把 `;` 分隔的 IP 列表（含 TTL 后缀）解析为 `Vec<IpAddr>`，非法分片忽略不抛错。`detect_collector() -> Option<Box<dyn DnsLogCollector>>`：Windows 上启动 PowerShell collector；Linux/macOS 返回 None。
- Added (D3): `src/dns_log/windows_dns.rs::PowershellDnsCollector` —— Windows 路线走 PowerShell `Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-DNS-Client/Operational'; Id=3010}` 子进程（不走 ETW）。spawn 长跑 `powershell.exe -NoProfile -NonInteractive -Command <SCRIPT>`，脚本内部 ~400ms 节奏轮询新事件，每事件一行 JSON emit（`ts/pid/name/qtype/status/results`）。reader 线程 `BufReader::read_line` 流式解析 + PID 名 lookup（sysinfo `refresh_processes_specifics` 每 10s 刷一次缓存） + `sync_channel(1000)` 推到 collector drain。`Arc<Mutex<Option<Child>>>` 共享给 reader 保活 + Drop 时主动 kill（避免 reader 阻塞在 `read_line`）。详见 `docs/adr/0006-dns-subprocess-not-etw-dbus.md`。
- Added (D3): `src/dns_log/windows_dns.rs::parse_powershell_event(line)` 纯函数 —— 解析 PowerShell JSON 行为 `DnsQuery`。容忍 PID 0（System Idle，噪声）/ 负 PID（PowerShell 偶发）丢弃；status 解析 u32 失败仍保留事件（标 `Error("unparsed:...")`）。9 个单测覆盖 success/nxdomain/timeout/invalid PID/PID 0/garbage status/non-JSON/mnemonic qtype/时间戳边界。
- Added (D3): `src/dns_log/unsupported.rs::PowershellDnsCollector` 占位 —— 非 Windows 平台 `pub use unsupported as windows`，`new()` 直接返回 `Err`。
- Added (D3): `src/dns_log/worker.rs::DnsLogWorker` —— 复用 `SnapshotWorker<DnsLogSnapshot>`，**500ms poll**（DNS 查询高频，比阶段 7 NetFlow 的 1s 更短）。`spawn(collector: Box<dyn DnsLogCollector>)` 启动 worker，body 调 `collector.drain()` 推送（空 Vec 跳过）。
- Changed (D3): `src/app.rs::App` 新增 `dns_log_worker: Option<DnsLogWorker>` + `dns_log_recent: VecDeque<DnsQuery>`（cap=1000 FIFO）字段。`App::new()` 调 `detect_collector().map(spawn)`：Windows 上启动 worker；其它平台字段为 None。`App::tick_dns_log()`（每次 100ms tick 调）drain worker 最新 snapshot，逐条 push_back 到 VecDeque，超 1000 pop_front。
- Changed (D3): `src/app_panel.rs::PanelContext` 新增 `dns_log_recent: &'a mut VecDeque<DnsQuery>` 字段 —— 让 PortPanel 在 DNS 子视图中按 `c` 清空（无需额外间接）。
- Added (D3): `src/view_models/port_panel.rs` DNS 子视图 —— 新字段 `dns_view_active: bool` / `dns_cursor: usize` / `dns_scroll: usize` / `dns_follow: bool`（默认 true）/ `dns_search: SearchState`。按 `D`（大写，小写 `d` 留给 anomaly dismiss）进入 DNS 子视图，`Esc`/`D` 退出。激活时接管所有按键：`↑↓` 移动光标 / `/` 搜索（域名 + 进程名）/ `c` 清空 / `f` 切换 follow / `PageUp/Down`/`Home/End`。`dns_filtered_indices(recent)` 返回搜索命中的索引列表，跨 view + key handler 共享避免重算。
- Added (D3): `src/tui/port_table.rs::draw_dns_view` —— 渲染 DNS 日志列表（时间 / PID / 进程名 / 类型 / 域名 / 结果 6 列），结果按 `DnsResult` 变体着色（Success=绿 / NxDomain+Error=黄 / Timeout=亮红）。标题栏显示 collector 状态 + 条数 + follow 状态；搜索激活时底部显示搜索框。`dns_view_active == true` 时 `draw()` 优先走 DNS 视图，覆盖常规端口视图。
- Added (D3): `src/tui/detail_view.rs::draw_network_tab` —— Network Tab 顶部加「最近 5 条 DNS 查询」面板（按当前 PID 过滤 `dns_log_recent`）。无 DNS 数据时（worker 未启动 / 此 PID 未查 DNS）省略，避免占垂直空间。
- Changed (D3): `src/tui/layout.rs::draw_footer` —— DNS worker 活动时状态栏左侧显示 `📡DNS(仅内存)` 指示（隐私提示）；PortMap 模式快捷键栏追加 `DDNS日志` 提示；DNS 子视图激活时改用专用快捷键栏（`↑↓滚动 / 搜索 / c清空 / f切换follow / D/Esc退出`）。
- Added (D3): `src/anomaly.rs::AnomalyDetector::detect_new_dns_from_new_process` —— R9 异常规则：一个进程首次发起 DNS 查询，且其名称不在 whitelist（小写进程名集合）→ Warning，每个 PID 仅触发一次（跨调用维护 `seen_pids: HashSet<u32>`）。6 个单测覆盖 whitelist hit/miss、PID 去重、未知进程名（`?`）触发、空 queries、跨调用 `seen_pids` 持久。
- Added (D3): `src/cli.rs::Command::Dns { tail, since }` —— `proc dns --tail` 流式输出 DNS 查询日志（Ctrl+C 退出），`--since 1h` 留 TODO（DNS 不持久化，需走 Windows EventLog 历史读取路径）。`src/main.rs::run_dns` dispatch。
- Added (ADR-0006): `docs/adr/0006-dns-subprocess-not-etw-dbus.md` —— 解释：① Windows 为什么选 PowerShell `Get-WinEvent` 而非 ETW（ROI：500 行 unsafe FFI vs 150 行子进程）；② Linux 为什么放弃 systemd-resolved DBus（DBus 接口不暴露 per-query 信号，stage-8.md 原计划有误），pcap/eBPF 工程量超 stage 范围列为未来 feature。共同原则：复用既有依赖 + 最小 native 表面 + 子进程开销可接受（与 ADR-0004/0005 一致）。
- Added: `tests/test_dns_log.rs`（新）—— 11 个集成测试：VecDeque cap=1000 FIFO 行为 / 批量 push 顺序 / clear / mock collector + worker round-trip / 空 collector drop 不卡死 / `detect_collector` 跨平台不 panic / `parse_query_type` 6 种常见类型 / `parse_query_results` IPv4+IPv6 混合 / `DnsResult::badge` 4 个变体 / `DnsLogSnapshot::default` 空。
- Added: 模块内嵌单测 —— `dns_log/mod.rs` 9 个（Display / Display 零值 / serde round-trip / Windows status 4 个分支 / badge 稳定 / parse_query_type 已知 + 未知 + trailing NUL / parse_query_results 多种格式 / clone+eq）、`dns_log/windows_dns.rs` 9 个（success / nxdomain / timeout / 负 PID / PID 0 / 不可解析 status / 非 JSON / 助记符 qtype / 时间戳边界）、`anomaly.rs` 6 个 dns_anomaly_tests。
- Test: 总测试数 532 → **568**（+36：11 集成 + 25 lib 内嵌）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过；`cargo fmt --all -- --check` 干净；阶段 8 新代码（trait + powershell impl + worker + UI + anomaly）`-W clippy::must_use_candidate` 0 新增警告（baseline 16 个全在 src/app.rs 现有 getter，surgical 原则不动）。
- Note: 隐私 —— DNS 查询含敏感信息（用户访问的域名），**永不持久化**到磁盘。`App::dns_log_recent` 仅内存；录屏（`record/`）路径不序列化 DNS 数据（不在 `SystemSnapshot` 内）；状态栏 `📡DNS(仅内存)` 指示让用户知道采集状态。
- Note: 已知限制 —— Linux/macOS 暂不支持 DNS 采集（DBus 不暴露 per-query 信号；pcap/eBPF 工程量大留作未来 feature）；仅覆盖 event 3010（QueryResultsEx）；PowerShell 启动延迟 ~300ms（首次 spawn 后 ~1s 开始收到事件）；PID 名 lookup 10s 刷新一次，新进程可能暂时显示 `?`。

### 阶段 7 — Slice：D1 per-process 网络流量（Windows IP Helper + Linux nethogs）

- Added (D1): `src/net_flow/mod.rs` 新模块 —— `NetFlowCollector` trait 抽象 per-process 字节速率数据源（`fn per_process_rates(&mut self) -> Vec<ProcessNetRate>` / `fn provider_name(&self) -> &'static str`）。`Send + Sync` 让 collector 可跨 worker 线程传递。参考阶段 6 [`crate::gpu::GpuProvider`] trait 模式。`ProcessNetRate { pid, start_time, bytes_sent_per_sec, bytes_recv_per_sec }` 带 `start_time` 字段防 PID 复用（ADR-0003）。`detect_collector() -> Option<Box<dyn NetFlowCollector>>` 按平台 / feature / 二进制可用性返回活跃 collector，无可用时返回 None（主线程 net 列保持 0）。
- Added (D1): `src/net_flow/windows.rs::IphelperCollector` —— Windows IP Helper 路线（不走 ETW）。`per_process_rates` 每次调用：调 `GetTcpTable2` + `SetPerTcpConnectionEStats` + `GetPerTcpConnectionEStats`（复用 [`crate::estats`] 同款 Win32 调用）拿每条 IPv4 TCP 连接的累计 `DataBytesIn` / `DataBytesOut`，同时调 `netstat2::get_sockets_info` 拿连接 → PID 映射，按 PID 聚合。内部维护 `last_per_pid` 累计缓存，差值 / elapsed = bytes/sec。PID 复用检测：当前累计 < 上次累计 → 视为新进程，速率按 0 计。详见 `docs/adr/0005-netflow-windows-iphelper-not-etw.md`。
- Added (D1): `src/net_flow/nethogs.rs::NethogsCollector` —— 仅 `target_os = "linux"` + `feature = "nethogs"` 下编译。`try_new` 先用 `nethogs --version` 探测二进制，可用时 spawn `nethogs -t -d 2 -v 3` 子进程，stdout 喂 `parse_nethogs_line` 纯函数解析（每行 PID + direction + KB/sec）。`Drop` 时 `child.kill() + wait()` 干净退出。`unsafe impl Sync` 套 `Mutex<Child>` 让 collector 满足 trait 的 `Send + Sync` 要求。
- Added (D1): `src/net_flow/nethogs.rs::parse_nethogs_line(line)` 纯函数 —— 解析 nethogs tracemode 行（tab/多空格分隔），从 `name/pid/user` token 提取 PID，过滤 PID 0（kernel），返回 `(pid, direction, kbps)`；解析失败 / 缺列 / 负速率返回 None。模块内嵌 9 个单测覆盖 happy path / garbage / 边界。
- Added (D1): `src/net_flow/unsupported.rs::IphelperCollector` 占位 —— macOS / 非 Windows 非 Linux 平台的 `pub use unsupported as windows` 路径，`new()` 直接返回 `Err`。
- Added (D1): `src/net_flow/worker.rs::NetFlowWorker` —— 复用 `SnapshotWorker<NetFlowSnapshot>`，1s poll。`spawn(collector: Box<dyn NetFlowCollector>)` 启动 worker，body 调 `collector.per_process_rates()` 推送。
- Added (D1): `src/collect.rs::ProcessInfo` 新增 `net_sent_rate: u64` / `net_recv_rate: u64` 字段（默认 0，worker 不可用时保持 0）。5 处 `ProcessInfo { ... }` 构造点全部同步（collect.rs × 2 / eject/locks.rs / record/conversions.rs × 1 + 内部测试 14 处）。
- Added (D1): `src/collect.rs::SortField` 新增 `NetSent` / `NetRecv` 两个排序变体。`label()` 返回「↑网络」/「↓网络」；`next` / `prev` 循环 DiskWrite ↔ NetSent ↔ NetRecv ↔ Cpu；`as_str` / `parse_from_str` 持久化标识 `net_sent` / `net_recv`；`sort_processes` 新增降序比较分支（`.then(pid)` tie-breaker）。
- Changed (D1): `src/app.rs::App` 新增 `net_flow_worker: Option<NetFlowWorker>` 字段。`App::new()` 调 `detect_collector().map(spawn)`：平台支持时启动 worker，不支持时字段为 None。`App::update_net_rates()`（heavy refresh 时调）drain worker 最新一份，按 PID 贴回 `cached_processes.net_sent_rate` / `net_recv_rate`；无 worker / 无新帧时保留当前值不强制清零。`get_filtered_sorted_processes` 排序 match 加 NetSent / NetRecv 分支。
- Changed (D1): `src/tui/process_table.rs` 排序字段为 NetSent / NetRecv 时新增「↑网络」「↓网络」两列（与磁盘R/磁盘W 同款 layout），用 `format::format_speed` 格式化。
- Changed (D1): `src/tui/sidebar.rs` 在 NET ↓/↑ 速率下方追加 Top 3 上行流量进程 mini list（参考 Mission Center 同款），按 `net_sent + net_recv` 降序取前 3；过滤全 0 行避免误导。
- Changed (D1): `src/cli.rs::Command::Ls` / `Command::Export` sort help 字符串加 `disk_read | disk_write | net_sent | net_recv` 选项；`src/main.rs::run_ls` / `run_export` sort matcher 加对应分支；`run_ls` 在 NetSent / NetRecv 排序时输出 ↑网络/s 和 ↓网络/s 两列。
- Added (Cargo): `[features] default = ["nvidia", "nvtop", "nethogs"]`，新增 `nethogs = []`（无 native 依赖，走子进程）。仅 `target_os = "linux"` 平台生效。`cargo build --no-default-features --features nvidia` 仍可编译（关闭 nethogs + nvtop）。
- Added (ADR-0005): `docs/adr/0005-netflow-windows-iphelper-not-etw.md` —— 解释 Windows 为什么选 IP Helper 而非 ETW：① ETW 实时 session 需要单独消费者线程 + `ProcessTrace` 阻塞调用 + ~500 行 unsafe 脚手架，ROI 不匹配；② IP Helper 复用 [`crate::estats`] 已测的同款 Win32 调用，1s poll 下 CPU < 1%；③ `SetPerTcpConnectionEStats` 在非管理员下通常仍可工作；④ `NetFlowCollector` trait 抽象让未来切回 ETW 是 additive（新增 impl + detect_collector 分支）。
- Added: `tests/test_net_flow.rs`（新）—— 11 个集成测试：`ProcessNetRate` Display smoke / `detect_collector` 跨平台不 panic / `SnapshotWorker<NetFlowSnapshot>` spawn+drop 生命周期 + try_recv_latest 推送 / `sort_processes` 在 NetSent/NetRecv 上降序 + tiebreak / `SortField::NetSent/NetRecv` as_str/parse_from_str 往返 / `next`/`prev` 循环覆盖 Net 变体 / `ProcessNetRate` clone+eq / `NetFlowSnapshot::default()` 空。
- Added: 模块内嵌单测 —— `net_flow/mod.rs` 3 个（Display / Display 零值 / detect_collector 不 panic）、`net_flow/nethogs.rs` 10 个（parse down/up/无 user/refreshing/closed/pid 0/garbage/negative/multi-space/kbps 转换 + 端到端聚合）。
- Test: 总测试数 506 → **532**（+26：13 集成 + 13 lib 内嵌）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过；阶段 7 新代码（trait + 4 个 impl + worker + Display）`-W clippy::must_use_candidate` 0 警告。
- Note: 已知限制 —— Windows IP Helper 仅覆盖 IPv4 TCP（与 [`crate::estats`] 一致），IPv6 路径（`GetPerTcp6ConnectionEStats`）和 ETW 全协议覆盖留作后续 additive 工作；UDP 无 per-PID 字节速率概念（无连接字节计数）；非管理员模式下部分其它进程的连接可能拿不到字节（`SetPerTcpConnectionEStats` 失败按 0 计，UI 显示 `0B/s`）。

### 阶段 6 — Slice：B1 AMD/Intel GPU via nvtop

- Added (B1): `src/gpu.rs::GpuProvider` trait 抽象 GPU 数据源 —— `fn list_gpus(&self) -> Vec<GpuInfo>` / `fn refresh(&mut self)` / `fn provider_name(&self) -> &'static str`。`Send + Sync` 让 provider 可跨 worker 线程传递；`list_gpus` 取 `&self`（缓存由 refresh 维护），让多 provider 场景能并发查询无需 `&mut`。
- Added (B1): `src/gpu.rs::NvmlProvider` —— 封装现有 Windows DXGI + NVML + PDH 三层路径为 `GpuProvider` impl。`NvmlState` / `PdhState` / `collect_dxgi_adapters` 全部保留；新增 `pdh_util: Option<u32>` 字段缓存 PDH 单次采样结果（PDH 是状态机，不能在 `list_gpus(&self)` 里推进）。NVML feature 关闭时仍返回所有 vendor 的 VRAM（DXGI 覆盖），仅退化 utilization/temp/power enrichment。非 Windows 平台 provider 类型保留为占位（detect_providers 不构造）。
- Added (B1): `src/gpu.rs::NvtopProvider` —— 仅 `target_os = "linux"` + `feature = "nvtop"` 下编译。`refresh` spawn `nvtop -s -o json` 子进程，stdout 喂 `parse_nvtop_json`；失败保留旧缓存不 panic。`is_available()` 通过 `nvtop --version` 探测 PATH。
- Added (B1): `src/gpu.rs::parse_nvtop_json(content)` 纯函数 —— 解析 nvtop JSON 输出（device/temperature/memory{used,total}/gpu_utilization/power{used,total} 五字段），缺字段按 0/None 退化，非法 JSON 返回空 Vec。`infer_vendor(name)` 字符串匹配 NVIDIA/GeForce/Quadro/RTX/GTX → Nvidia；AMD/Radeon → Amd；Intel → Intel；其它 → Unknown。utilization > 100 时 clamp 到 100。
- Added (B1): `src/gpu.rs::detect_providers() -> Vec<Box<dyn GpuProvider>>` —— 根据平台 / feature / 二进制可用性返回活跃 provider 列表。Windows 始终加 `NvmlProvider`；Linux + nvtop feature + nvtop 在 PATH 时加 `NvtopProvider`；macOS 等返回空 Vec。多 provider 并存支持混合 GPU 笔记本（Intel iGPU + NVIDIA dGPU）。
- Changed (B1): `src/gpu.rs::GpuCollector` 内部从 `Option<NvmlState> + Option<PdhState>` 改为 `Vec<Box<dyn GpuProvider>>`；`new()` 调 `detect_providers()`，`refresh()` 遍历所有 provider refresh + list_gpus 聚合。外部 API（`new` / `refresh -> Vec<GpuInfo>`）零修改 —— `collect.rs::LightWorker` 调用点不变。
- Added (Cargo): `[features] default = ["nvidia", "nvtop"]`，新增 `nvtop = []`（无 native 依赖，走子进程）。`cargo build --no-default-features --features nvidia` 仍可编译；`--features nvtop`（关 nvidia）在 Linux 上仍工作，在 Windows 上无 GPU provider（sidebar GPU 区为空，可接受）。
- Added (ADR-0004): `docs/adr/0004-gpu-via-nvtop-subprocess.md` —— 解释选 nvtop 子进程而非 libdrm 直接绑定 / WMI 的理由：依赖管理干净（无 bindgen）、跨厂商一次覆盖（AMD+Intel+NVIDIA 一套解析器）、与 ADR-0003 SMART `smartctl` 同类取舍（30s/1s poll 子进程开销可接受）、失败优雅降级。Windows AMD/Intel 留 TODO（DXGI 仅 VRAM，方案 D WMI 后续迭代）。
- Added: `tests/test_gpu.rs`（新）—— 6 个集成测试：fixture 多厂商解析 / 字段完整 / malformed 输入 / 空数组 / `detect_providers` 不 panic + provider_name 非空 / `GpuCollector::refresh` 返回 Vec 不 panic。
- Added: `tests/fixtures/nvtop_sample.json` —— 三厂商样本（NVIDIA RTX 4070 + AMD RX 7900 XTX + Intel Arc A770），含 temperature/memory/utilization/power 完整字段。
- Added: 模块内嵌单测 —— `gpu.rs` 8 个测试（parse_nvtop_json 多厂商/缺字段/garbage/clamp + infer_vendor 7 个品牌串 + detect_providers 不 panic + GpuCollector default）。
- Note: sidebar.rs 已通过 `for gpu in gpu_info` 循环支持多 GPU，阶段 6 无需改动（surgical 原则）。Windows AMD/Intel GPU 在 DXGI 路径下已显示 VRAM，仅缺 utilization/temp/power（NVML enrichment 限定 NVIDIA）。
- Test: 总测试数 492 → **506**（+14：8 lib + 6 集成）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过；阶段 6 新代码（gpu trait + providers + parse_nvtop_json + detect_providers）`-W clippy::must_use_candidate` 0 警告（baseline 遗留 16 个全在 src/app.rs 现有 getter，surgical 原则不动）。

### 阶段 5 — Slice：D2 TCP 质量 + B3 SMART 磁盘健康

- Added (D2): `src/collect.rs::TcpStats` 新增 4 个传输质量字段 —— `retransmitted_segs` / `reset_segs` / `failed_connections` / `out_segs`（u64）。Windows 走 `GetTcpStatisticsEx2` + `MIB_TCPSTATS2`，IPv4 + IPv6 各跑一次累加；Linux 解析 `/proc/net/snmp` 的 TCP 行；其它平台保留 0。
- Added (D2): `src/port_map.rs::TcpSnmpStats` 结构 + `parse_proc_net_snmp_tcp(content)` 纯函数解析器。按列名匹配（不是按位置），新内核加列时自动跳过；header 缺失或 numeric 缺失都返回 `default()` 不抛错。模块内嵌 4 个单测覆盖 typical / 缺失 / 多列 / InErrs+InCsumErrors 合并。
- Added (D2): `src/port_map.rs::PortEntry.rtt_ms: Option<u32>` —— per-connection RTT 字段占位。`netstat2` 不暴露 RTT，Windows `GetPerTcpConnectionEStats` 是管理员专属重型 API，阶段 5 不强制采集；`None` 在 UI 渲染为 `-`，避免误读为零延迟。
- Added (D2): `src/anomaly.rs` 新增 `detect_with_tcp_stats` 入口 + `R7 高重传率`（retransmit/out_segs > 5%）+ `R8 高 RST 率`（rst/out_segs > 2%）2 个 anomaly 规则。out_segs < 1000 时跳过（样本量太小，统计噪音大）。模块内嵌 6 个单测。
- Added (D2): `src/view_models/port_panel.rs::tick` 接入 `detect_with_tcp_stats`，3s 一次把 `TcpStats` 喂给 detector，让 R7/R8 在阈值触发时生成 anomaly。
- Added (D2): `src/tui/port_table.rs::draw_net_traffic_bar` 追加「重传 / RST / 失败」三段，按 retrans > 5% / rst > 2% 上色（danger / warning / success）；out_segs=0 时显示 `-`。
- Added (D2): `src/tui/detail_view.rs::draw_network_tab` 新增「RTT」列（默认 `-`，对应 `PortEntry.rtt_ms`）。
- Added (D2): `src/cli.rs::Command::Port` 新增 `--stats` 标志；`src/main.rs::run_port` 输出 TCP 传输质量摘要（established/listen/time_wait/close_wait/retrans/rst/failed/out_segs + 重传率/RST率）。
- Added (B3): `src/smart/mod.rs` 新模块 —— `SmartData`（device/model/serial/temperature/health/attributes）+ `SmartHealth`（Ok/Warning/Failing/Unknown，`badge()` 返回 ✓/⚠/✗/-）+ `SmartAttribute`（id/name/value/threshold/raw_value/failing）+ `parse_smartctl_json(content)` 纯函数 + `read_smart(device)` 跨平台分发 + `list_disks()`。Windows 走 smartctl 子进程（装了 smartmontools 的话），失败退化到 WMI `MSStorageDriver_FailurePredictStatus`（只给 health，无属性表）；Linux 走 smartctl 子进程；macOS 同 Linux。模块内嵌 7 个单测。
- Added (B3): `src/collect.rs::SmartWorker` —— 独立后台 worker，`sync_channel(1)` + Drop shutdown + join，30s poll 周期。单盘失败不阻塞其它盘。`SystemSnapshot::new` 预热阶段 `recv_first(2s)` 拿首帧，避免 sidebar SMART 徽章空白。
- Added (B3): `src/collect.rs::SystemSnapshot::smart_data()` 访问器 + `refresh_light()` try_recv 覆盖缓存。
- Added (B3): `src/tui/sidebar.rs::format_smart_badge()` —— 把 SmartWorker 缓存的 health.badge() + 温度追加到每行磁盘后；空 Vec 时返回空字符串（无 SMART 数据时 sidebar 不变）。
- Added (B3): `src/cli.rs::Command::Smart { device: Option<String> }` + `src/main.rs::run_smart_list` / `run_smart_detail`。`proc smart` 列出所有磁盘 + 健康/温度/属性数；`proc smart <device>` 输出完整 SMART 属性表（comfy-table）。
- Added (B3): `src/error.rs::ProcError::Smart { message, source }` 变体 + `smart()` / `smart_with()` / `smart_msg()` 三个构造器。
- Added (ADR-0003): `docs/adr/0003-smart-subprocess-vs-library.md` —— 解释为什么选 smartctl 子进程而非 libatasmart：libatasmart 维护停滞（2013 年最后一版）、完全不支持 NVMe、Windows 完全不支持；smartmontools 持续维护、JSON schema 7.0+ 起稳定、跨平台覆盖、依赖管理干净、30s poll 周期下子进程开销可接受。Windows WMI `MSStorageDriver_FailurePredictStatus` 作为降级路径（无详细属性，仅预测布尔）。
- Added: `tests/test_smart.rs`（新）—— 3 个集成测试：fixture sample 解析 / failing 样本解析 / `list_disks` 不 panic。
- Added: `tests/test_tcp_stats.rs`（新）—— 5 个集成测试：`parse_proc_net_snmp_tcp` 真实格式 / 紧凑格式 / 无 Tcp 段 / 垃圾输入 / Windows `SystemSnapshot::tcp_stats()` 字段存在 smoke。
- Added: `tests/fixtures/smartctl_sample.json` —— 真实 smartctl 输出样本（Samsung SSD 850 EVO，15 个 ATA SMART 属性）。
- Test: 总测试数 466 → **492**（+26）。`cargo clippy --all-targets -- -D warnings` 0 警告；`cargo build --release --no-default-features` 编译通过。

### 阶段 4 — Slice：A4 优先级/affinity + A1 句柄 Tab + A3 内存映射 Tab

- Added (A4): `src/process_control.rs` 新模块 —— `PriorityClass`（6 档：Idle / BelowNormal / Normal / AboveNormal / High / Realtime）+ `get_priority` / `set_priority` / `get_affinity` / `set_affinity`。Windows 走 `SetPriorityClass` / `GetProcessAffinityMask` / `SetProcessAffinityMask`；Linux 走 `setpriority(PRIO_PROCESS)` / `sched_getaffinity` / `sched_setaffinity`；macOS 返回 `PermissionDenied`。`bump_up` / `bump_down` 实现 Realtime/Idle 边界 clamp；`to_nice` / `from_nice` 完成与 Linux nice 的 6 档映射（19/10/0/-5/-10/-20）。
- Added (A1): `src/inspect/handles.rs` 新模块 —— `collect_handles(pid)` Windows 上用 `GetModuleHandleW("ntdll.dll") + GetProcAddress` 手动加载 `NtQuerySystemInformation` / `NtQueryObject`，枚举 `SystemExtendedHandleInformation` 按 PID 过滤，对每个匹配句柄 `DuplicateHandle` 到当前进程后 `NtQueryObject(ObjectTypeInformation)` 拿类型名（`File` / `Key` / `Mutant` 等 11 档分类）；Linux 走 `/proc/<pid>/fd/*` + `readlink` 拿目标路径。`find_lockers(path)` Windows 复用 filelocksmith（内部已用 worker thread + 200ms 超时规避 `NtQueryObject(ObjectNameInformation)` 同步阻塞），Linux 遍历 `/proc/*/fd/*`。`parse_handle_kind` 把 NT type_name 字符串归类到 `HandleKind`，独立单测覆盖。
- Added (A3): `src/inspect/memory.rs` 新模块 —— `collect_memory(pid)` Windows 走 `VirtualQueryEx` 遍历整个进程地址空间（上限 `0x7FFF_FFFF_FFFF`，wrapping_add 防 0xFFFF... 溢出），按 `MEMORY_BASIC_INFORMATION.State` 分类 Commit/Reserve/Free，`PAGE_PROTECTION_FLAGS` 映射为 `rwxg` 风格字符串；Linux 解析 `/proc/<pid>/maps` 6 列格式，`parse_maps_line` 纯函数 + 单测覆盖 typical .so / `[heap]` / 匿名 / `---p` / malformed 5 类。`parse_smaps_block` 提取 Size/Rss（kB → bytes，缺失 Rss 退化为 Size），独立单测。
- Added: `src/inspect/mod.rs::HandleKind` 新增 `label()` 方法，返回 12 档稳定字符串（UI / CLI / 测试 anchor 复用）。
- Added (ADR-0002): `docs/adr/0002-inspector-tab-extension-mechanism.md` —— 解释 Inspector 继续用 enum + match 而非 trait object：编译期穷尽性 > 运行时灵活性、数据量小 vtable 开销无意义、独立字段便于按需加载、`label()` 作测试 anchor。
- Changed: `src/app.rs::App::switch_mode(ProcessDetail)` 在原 `inspect_with_ports` 之外追加 `collect_handles` / `collect_memory` 同步加载（失败退化为空 Vec），`r` 刷新同步覆盖三个数据源。
- Changed: `src/app.rs::handle_detail_key` 新增 `+` / `=` / `-` 处理 —— 调 `bump_priority(pid, up/down)` 实时改优先级并写 `status_message` / `op_history`。
- Added: `src/app.rs::App::bump_priority` / `bump_selected_priority` —— A4 优先级调整的共享实现，详情页和列表页统一走这里。
- Changed: `src/view_models/process_panel.rs::handle_list_key` 新增 `+` / `=` / `-` 分支 + `focused_pid` helper（多选时取最后选中，否则 cursor）+ `bump_priority_into` 自由函数把错误/成功塞 `status_message`。
- Changed: `src/tui/detail_view.rs` —— `InspectionTab::Handles` / `Memory` 走真实渲染（`draw_handles_tab` / `draw_memory_tab`），阶段 1 占位 `draw_construction_placeholder` 删除。Summary Tab 新增「优先级」+「Affinity」两行（同步查 `process_control::get_priority` / `get_affinity`，单次 < 1ms）。快捷键栏追加 `+/-=优先级`。
- Changed: `src/cli.rs` 新增 4 个命令 —— `Command::Who { target_path }`（位置参数，避开全局 `--path` 冲突）/ `Command::Handles { pid, file }` / `Command::Priority { pid, set }` / `Command::Affinity { pid, set }`。
- Changed: `src/main.rs` 新增 `run_who` / `run_handles` / `run_handles_pid` / `run_priority` / `run_affinity` / `parse_priority_class` / `pid_to_name` 7 个函数，复用 comfy-table 表格输出。`proc who` 空结果时提示「需要管理员权限枚举系统进程句柄」。
- Changed: `Cargo.toml` 启用 `Win32_System_LibraryLoader` feature（GetModuleHandleW / GetProcAddress 加载 ntdll 函数指针，避免改 Wdk feature 列表）。
- Added: 模块内嵌单测 —— `process_control.rs`（7 测试：label 唯一 / bump_up Realtime clamp / bump_down Idle clamp / to_nice 单调 / from_nice 往返 / Default / self_get_priority smoke）+ `inspect/handles.rs`（5 测试：parse_handle_kind 11 档 / Other 分类 / 空字符串 Unknown / format_raw_handle 16 进制 / self_collect_handles smoke）+ `inspect/memory.rs`（8 测试：parse_maps_line typical/heap/anon/noaccess/malformed + parse_proc_maps 多段 + parse_smaps_block 提取 Rss / 退化 Size + self_memory_collect 非空）。
- Added: `tests/test_inspect.rs` 追加 5 个集成测试（collect_handles / collect_memory / find_lockers 自身进程 smoke + 跨平台降级）。
- Added: `tests/test_priority.rs`（新）—— 4 个 round-trip 测试（get_priority 不 panic / set→get 往返 BelowNormal→Normal / parse_priority_class 6 档 / Linux nice 映射）。
- Added: `tests/test_inspector.rs` 追加 6 个测试（Handles/Memory Tab 切换加载 inspection_handles_data/inspection_memory_data / `r` 同步刷新 / `+`/`-` 调优先级写 status_message / Detail 占位 draw 不再触发）。
- Test: 总测试数 428 → **466**（+38）。`cargo clippy --all-targets -- -D warnings` 0 警告；阶段 4 新代码（process_control / inspect::handles / inspect::memory / cli 新命令 / main 新分发函数）`-W clippy::must_use_candidate` 0 警告。baseline 遗留 15 个 `must_use_candidate` 警告（src/app.rs 现有 getter + src/tui/security_badge.rs），不在阶段 4 范围（surgical 原则）；`cargo build --release --no-default-features` 编译通过（129 lib 测试绿）。

### 阶段 3 — Slice：E4 docker top + E1 docker logs + E3 镜像/volume/compose

- Added (E4): `src/docker/top.rs` —— `ContainerTopProcess`（pid/user/command/cpu_time/started）+ `get_container_top()` 调用 bollard `top_processes` + `parse_top_output()` 文本表格纯解析器 + `parse_top_response()` 结构化响应解析器。文本解析器容忍 CMD 列内空格（按 cmd_idx 取后续整段），无 PID/CMD 表头时返回空 Vec 降级。`DockerMonitor::container_top()` 暴露同步接口。
- Added (E1): `src/docker/logs.rs` —— `LogLine`（timestamp/message/is_stderr）+ `parse_log_timestamp()` 纯解析器（剥离 RFC3339 前缀，支持 `Z` / `±HH:MM` 时区，保留 ANSI 颜色码）+ `collect_container_logs()` 一次性拉日志 + `make_follow_options()` 构造 follow 配置。`DockerMonitor::collect_logs()` 暴露。
- Added (E1): `src/docker/logs_worker.rs` —— 后台日志 worker：独立 tokio runtime + `sync_channel(64)` 背压 + `try_send` 满即丢 + 周期 `try_recv` shutdown 信号。chunk 大小 16 行 / 4KB 字符；环形缓冲上限 5000 行（`LogViewer::MAX_BUFFER_LINES`）。Drop 句柄触发 worker 干净退出。
- Added (E3): `src/docker/images.rs` —— `ImageInfo`（id/short_id/repo_tags/created/size/containers）+ `list_images()` / `remove_image()` + `Display` 实现 + `in_use()` / `display_name()` 辅助。`<none>:<none>` tag 过滤掉。
- Added (E3): `src/docker/volumes.rs` —— `VolumeInfo`（name/driver/mountpoint/created/size/in_use）+ `list_volumes()` / `remove_volume()` + `Display` 实现。`list_volumes` 反查所有容器 mounts 给 `in_use` 标记；`size` 通过 `du` 风格递归算 mountpoint 大小。无 chrono 依赖手写 `days_from_civil` + `parse_rfc3339_to_unix`。
- Added (E3): `DockerMonitor` 新增 `container_top()` / `collect_logs()` / `list_images()` / `remove_image()` / `list_volumes()` / `remove_volume()` 6 个方法，统一走 `runtime.block_on` 同步包装。
- Added (E3): `src/view_models/docker_panel.rs` —— `DockerViewMode` 枚举（Containers/Images/Volumes，`Tab` 循环）+ `LogViewer`（buffer/scroll/follow/container）+ `DeleteTarget`（两次 `d` 确认）+ `LogsWorker` 句柄（drop 退出）。`DockerPanel` 新增 9 字段（view_mode/images/volumes/show_top_processes/top_processes/log_viewer/logs_worker/delete_pending 等）+ 9 个交互方法（`cycle_view_mode`/`refresh_images`/`refresh_volumes`/`handle_delete`/`toggle_top_processes`/`enter_logs_mode`/`exit_logs_mode`/`toggle_logs_follow`/`clear_logs`）。`handle_key` 重写：日志模式优先吃快捷键，`Tab`/`t`/`l`/`f`/`c`/`d` 全接入。
- Added (E4/E1): `src/tui/docker_panel.rs` —— 视图路由按 `view_mode` 分发到容器/镜像/volume 三种列表；标题栏新增 `[容器][镜像][卷]` 高亮 Tab；详情弹窗加进程区块（`t` 触发，最多 10 行 + 折叠）；日志覆盖层占下 60% 屏，stderr 红色，滚动条跟随/手动切换；删除确认状态走 status_message。
- Changed: `src/cli.rs` —— `Command::Docker` 改成嵌套子命令 `DockerSub`（Ps/Inspect/Top/Logs/Images/Volumes/ImageRm/VolumeRm/Compose/Events）。Compose 子命令用 `trailing_var_arg + allow_hyphen_values` 转发参数给 `docker-compose`，环境变量 `PROC_DOCKER_COMPOSE` 覆盖二进制路径。
- Changed: `src/main.rs::run_docker` 拆成 11 个子分发函数（`run_docker_ps`/`_inspect`/`_top`/`_logs`/`_images`/`_volumes`/`_image_rm`/`_volume_rm`/`_compose`/`_events`）。`logs --follow` 走 logs_worker 跟随模式 + Ctrl+C 优雅退出。
- Added: `tests/test_docker.rs` 追加 22 个测试 —— `parse_top_output` 多格式（typical/empty/header-only/args_with_spaces/structured_response）+ `parse_log_timestamp` 多格式（Z/offset/no_ts/ansi/short）+ LogChunk/LogsWorker 行为 + ImageInfo/VolumeInfo Display + 6 个 DockerSub CLI 解析覆盖 + ViewModel 状态机。
- Updated: `tests/test_skeleton.rs` 原 `test_cli_docker_parsing` 适配新嵌套结构（`docker events` 替代 `docker --watch`）。
- Added: 模块内嵌单测：`top.rs`（8 测试）+ `logs.rs`（8 测试）+ `logs_worker.rs`（6 测试）+ `images.rs`（6 测试）+ `volumes.rs`（9 测试）+ `view_models/docker_panel.rs`（6 测试）。
- Test: 总测试数 353 → **428**（+75）。`cargo clippy --all-targets -- -D warnings` 0 警告。`cargo build --release --no-default-features` 编译通过。

### 阶段 2 — Slice：H4 Miri CI + B2 per-core CPU 频率/温度

- Added (H4): `.github/workflows/miri.yml` —— Linux runner + nightly + miri 组件，跑 `test_scorer_concurrency`（5 个并发测试）+ `test_workers`（4 个 SnapshotWorker 测试）共 9 个并发测试。首次接入用 `continue-on-error: true` 容错，后续稳定后移除。
- Added (B2): `src/collect.rs::LightSnapshot` 新增 `per_core_freq: Vec<u64>`（MHz）+ `per_core_temp: Vec<Option<f32>>`（°C）字段，与 `sysinfo::System::cpus()` 顺序对齐；worker loop 每 1s 推一份，主线程 `refresh_light` 时 `try_recv` 更新缓存。新增 `SystemSnapshot::per_core_freq()` / `per_core_temp()` 公开访问器。
- Added (B2): 跨平台采集策略 —— Linux 优先读 `/sys/devices/system/cpu/cpuN/cpufreq/scaling_cur_freq`（sysfs kHz），拿不到时退回 sysinfo；Windows 走 sysinfo 注册表 `~MHz`；macOS 走 sysinfo sysctl。温度走 sysinfo `Components`，通常只能拿到全局 CPU 温度（填到第 0 核），per-core 留 None。`parse_scaling_cur_freq` 抽成纯函数便于跨平台测。
- Added (B2): `src/tui/sidebar.rs` 折叠/展开模式 —— 折叠保持现状（13 行）；展开追加 per-core 表格（核/频率/温度，最多 8 行，>8 核按温度降序取 top-8）。`App::sidebar_height()` 改为根据 `sidebar_expanded` 动态返回 13 / 23。
- Added (B2): `App` 新增 `sidebar_expanded: bool` 字段，`c` 键切换（详情页的 `c` 复制进程信息在 `handle_detail_key` 里走 ProcessDetail 分支，不被这里抢键）。切换时持久化到 `~/.config/proc/ui.toml` 的 `sidebar_expanded` 字段，下次启动恢复。`src/ui_state.rs` 新增 `load_sidebar_expanded` / `save_sidebar_expanded`，`write_state` 升级到 3 字段（sort_field + first_run + sidebar_expanded），老 ui.toml（无新字段）按缺失默认 false 处理。
- Added: `src/collect.rs` 内嵌 `collect_tests` 3 个测试（scaling_cur_freq 解析）+ `src/tui/sidebar.rs` 内嵌 6 个测试（select_cores_for_display 截断/对齐 + per_core_line 渲染）+ `src/ui_state.rs` 新增 4 个 sidebar_expanded 解析测试 + `tests/test_per_core_freq.rs` 4 个集成测试（snapshot 返回 ≥1 频率 + parse 纯函数 + App c 键状态机 + 持久化往返）。

### 阶段 1 — Spike：文档基础设施 + Inspector 可扩展 Tab 骨架（0.5.0 起点）

- Added: 0.5.0 开发宪法 `plan.md`（11 阶段拆分 + 会话规则 + 验证矩阵）、领域词汇表 `CONTEXT.md`（含规划中术语标注）、`docs/adr/0001-phased-project-adoption.md`、`docs/stages/stage-1..11.md` 目录骨架。
- Added: `InspectionTab` 枚举从 4 变体扩为 6 变体（+`Handles` / `Memory`），`label` / `next` / `prev` / `all` 同步更新，循环正确（Memory ⇄ Summary）。
- Added: `src/tui/detail_view.rs` 新增 `draw_construction_placeholder` —— Handles / Memory Tab 渲染「建设中（阶段 4 上线）」占位文本，不崩溃。
- Added: `src/inspect/mod.rs` 新增 `HandleInfo` / `HandleKind` / `MemoryRegion` / `MemoryState` 类型骨架（字段 + `Default`，不实现采集），阶段 4 直接填实现。
- Added: `App` 结构体新增 `inspection_handles_data` / `inspection_memory_data` 两个 `Option<Vec<...>>` 占位字段（始终 None），`App::new()` 默认初始化正确。
- Added: `tests/test_inspector.rs` 适配 6 变体（更新 `inspection_tab_all_*` / `next_cycles` / `prev_cycles` / `labels` / `tab_key_cycles_inspector_tabs` / `backtab_cycles_in_reverse`），并新增 5 个加固不变量测试（`all_in_next_cycle_order` / `next_prev_are_inverse_for_all_six` / `next_six_times_returns_to_start` / `labels_are_all_distinct` / `memory_tab_is_last_in_all`）。
- Added: `tests/test_skeleton.rs` 新增 `test_app_inspection_handles_and_memory_default_none` —— 锁定占位字段初始 None。

## [0.4.0] - 2026-06-17

本次发布聚焦于 Inspector：进程详情页升级为多 Tab 深挖视图（环境变量 / 网络连接 / 已加载模块）。阶段 12（数据层）+ 阶段 13（TUI）共 2 个阶段，5 modified + 3 new，核心 +482 / -13。无 API 破坏；详情页原有快捷键全部保留（向后兼容）。

实测数据：

- 测试 **325 passed / 0 failed / 2 ignored**（baseline 291 → +34）
- pedantic `must_use_candidate`：**0**
- ADR-0004 落地一致性：✅ B2 方案无偏差

### 阶段 12 — Inspector v1 数据层 / Round 8

- Added (ADR-0004): `src/inspect/` 新模块 + 3 个子模块（`env` / `dlls` / `net`），顶层 `inspect::inspect(pid)` 聚合成 `InspectionData { env, dlls, net }`。
- Added: Windows 环境变量采集 —— `OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION)` + `NtQueryInformationProcess(ProcessBasicInformation)` 走 PEB → ProcessParameters → Environment。x64 上偏移 0x20 / 0x80 / 0x3F0 注释完整；32-bit 显式拒绝（避免错误偏移）。
- Added: Windows 模块列表 —— `CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32)` + `MODULEENTRY32W`，与 `security/dll_check.rs` 同源。
- Added: Linux 环境变量 —— 读 `/proc/<pid>/environ`，NUL 分隔 + `=` 分隔。
- Added: Linux 模块列表 —— 解析 `/proc/<pid>/maps`，BTreeMap 合并同 path 的多段映射（r-xp / r--p / rw-p），取最低 base + span 求和。
- Added: `Cargo.toml` 新增 Windows feature `Win32_System_Diagnostics_Debug`（ReadProcessMemory）+ `Wdk_System_Threading`（NtQueryInformationProcess）。
- Added: 跨平台降级 —— 非 Win/Linux 平台 env/dlls 返回 `ProcError::PermissionDenied`，由 TUI 层显示降级提示。
- Added: `tests/test_inspect.rs` 7 个集成测试 + `src/inspect/*.rs` 内嵌单元测试（self env/dlls/net 数据正确性 + unknown PID + Linux proc_maps 解析）。

### 阶段 13 — Inspector v1 TUI / Round 9

- Added (ADR-0004): `InspectionTab` 枚举（Summary / Env / Network / Dlls），`label()` / `next()` / `prev()` / `all()` 4 个方法全部 `#[must_use]`，`#[derive(Default)]` 保证 Summary 为默认 Tab。
- Added: `App` 结构体新增 4 字段（`inspection_tab` / `inspection_data` / `inspection_search` / `inspection_scroll`），集中在「Inspector」分组；`App::new()` 默认值正确。
- Added: `switch_mode(ProcessDetail)` 预加载 `inspection_data` + 重置 tab/scroll/search —— 进入详情页立即可见数据。
- Added: `handle_detail_key` 重写 —— 搜索 active 时优先吃输入且吞掉 Tab/BackTab（避免误触丢搜索内容）；Tab 切换重置 scroll；`r` 重新 `inspect()` + status_message 提示；Esc 双层（先退搜索，再退页面）。
- Added: `src/tui/detail_view.rs` 整体重写 —— Tab 栏（当前 Tab `accent + Bold + Underlined`）+ 主体内容区，4 个 Tab 分别渲染，每个 Tab 处理 empty / no-match / data-None 三态降级提示。
- Changed: Summary Tab 保留原详情页全部内容（分类 / 父进程 / 状态 / CPU / 内存 / 磁盘 / 运行时长 / exe / cmd / cwd / 端口 / 网络摘要 / 安全分 / 风险因子 / 快捷键）—— **零回归**。
- Changed: Dlls Tab 按 path 字母排序；Network Tab 不接搜索（数据量通常小）。
- Added: `tests/test_inspector.rs` 22 个集成测试 —— InspectionTab 枚举行为 + App state 默认 + Tab/BackTab 切换 + 搜索 + 刷新 + 滚动 + 跨平台 smoke。

### 验证矩阵

- `cargo fmt --all -- --check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test --release` ✅ 325 passed / 0 failed / 2 ignored（baseline 291 → +34：7 inspect 数据层 + 22 inspector TUI + 5 inspect 内嵌单元测试）
- `cargo build --release --no-default-features` ✅
- `cargo clippy ... -W clippy::pedantic | grep must_use_candidate` ✅ 0

### ADR 状态

- ADR-0004（Inspector B2 升级详情页）Status: **Accepted**（阶段 12-13 落地，2026-06-17）

### P2 改善建议（本仓库未单独维护 tech-debt.md，长期项记在此处）

- P2-6：`detail_view::draw_summary` 每帧重扫端口 → 已在 0.5.0 打磨中修复（复用 `port_panel.port_entries`）
- P2-7：`parse_utf16_env` 双 NUL 截断改 `find` → 0.5.0+ 可选
- P2-8：`End` 设为 `usize::MAX / 2` 加注释 → 0.5.0+ 可选

## [0.3.0] - 2026-06-17

本次发布聚焦于资源生命周期治理、性能优化、CI 加固、文档/帮助打磨、错误链完善，并伴随二进制体积优化。阶段 5-9 共 5 个 Round 累积：69 文件，+1043 / -249。无 API 破坏；错误类型 `ProcError` 转 struct form 含 source chain（ADR-0005）。

实测数据（来自内部 review 记录，未单独入仓）：

- 测试 **291 passed / 0 failed / 2 ignored**（baseline 283 → +8）
- 二进制体积 **-6.1%**（7.3 MB → 6.9 MB，ADR-0007 profile.release）
- pedantic `must_use_candidate`：**0**

### 阶段 5 — 资源生命周期 / Round 3

- Fixed (ADR-0006): `BackgroundScorer::Drop` 死锁修复 —— take 出 `request_tx` 后再 join，避免 bounded channel 满时 `try_send(Shutdown)` 失败导致 worker 卡死。新增 `tests/test_scorer_concurrency.rs::test_scorer_request_drops_when_busy` 验证（修复前 60s+ 不退出，修复后 0.03s）。
- Changed (ADR-0006): `docker/events.rs` 和 `monitor/port_watcher.rs` 事件通道从无界 `channel()` 改为 `sync_channel(64)` + `try_send` + `Full` → `tracing::warn!`，避免慢消费者积压。
- Changed: `diag.rs` 资源生命周期重构（+109 / -50），消除多处散落的临时句柄。
- Performance: `port_map::scan_ports` 走 `SysinfoRegistry` 全局单例，消除每次扫描 `System::new_all()` 的 ~200ms 开销；`eject::locks::find_volume_lockers_with_processes` 的最差 O(N × 200ms) 路径同步消除。

### 阶段 6 — 性能优化 / Round 4

- Fixed (ADR-0003): PID 复用导致旧实例的安全评分缓存过继给新进程 —— `ScoreCache::cache_key` 加 `start_time` 字段（`{pid}:{start_time}:{exe}`），`App::update_disk_speeds` 的 `prev_process_disk` 键改为 `(pid, start_time)` 元组。新增 `test_score_cache_pid_reuse_isolation` 回归测试。
- Performance: `format_speed` 抽取到 `src/format.rs`，统一磁盘 / 网络速率格式化。
- Performance: 500 进程基准（`tests/test_stage8_perf_regress.rs`）`rebuild_sorted_cache` **38.2 µs**（< 5ms 目标，130× 裕量）。

### 阶段 7 — CI / Cargo / Round 5

- Added (ADR-0007): `[profile.release]` 保守版（`opt-level=3` + `lto="thin"` + `codegen-units=1` + `strip="debuginfo"`，**不开** `panic="abort"`）。二进制体积 -6.1%。
- Changed: `tokio` features 精简为 `["rt", "rt-multi-thread", "macros", "net", "sync", "time"]`，`cargo build --no-default-features` 通过。
- Added: CI 新增 `check-macos` / `msrv` (1.85) / `audit` (RustSec) job。
- Fixed: `dll_check.rs::truncate_path` 改为手写实现以兼容 Rust 1.85（`str::split_once` 边界 case）。

### 阶段 8 — 文档 / 帮助 / Round 6

- Added: **首次启动引导** —— `~/.config/proc/ui.toml` 不存在时显示一次性提示。
- Added: **进程列表排序字段持久化** —— `←`/`→`/`S` 切换排序时写入 `ui.toml`，下次启动恢复。新增 `src/ui_state.rs` 模块，含 9 个解析测试覆盖 sort_field / first_run。
- Added: **3 个新主题** —— Gruvbox（暖色复古）/ One Dark（Atom 默认配色）/ Rose Pine（柔和现代），`THEMES` 从 7 个扩展到 10 个，`t` 循环切换。
- Changed: `help_panel.rs` 重构为结构化数据 + 滚动，新增 `sections_are_non_empty` / `every_shortcut_has_a_label` 内嵌不变量测试。

### 阶段 9 — error.rs source chain / Round 7

- Changed (ADR-0005): `ProcError` 7 个变体全部转 struct form，统一含 `#[source] source: Option<Box<dyn StdError + Send + Sync>>`。配套 14 个 `xxx()` / `xxx_with()` helper。全仓库 13 个调用站点迁移，无 `ProcError::Variant { ... }` 字面量构造（`IoError` 仍保留 `#[from] std::io::Error` 向后兼容 `?`）。
- Added: `tests/test_skeleton.rs::test_proc_error_source_chain` 验证 source chain 可遍历到根因。
- Fixed: pedantic `must_use_candidate` 全部消除（0 残留）。

### 阶段 11 — 批量修复 + 发布（本次发布）

- Fixed (REVIEW-10 P1-3): `SecurityScorer::invalidate_dead` 改用 `(pid, start_time)` 元组精确清理，避免 PID 复用场景下陈旧 entry 残留（之前仅靠 30s TTL `evict_expired` 兜底）。新增 `parse_alive_key` 解析键前两段。
- Docs: README GPU 路线图补"AMD/Intel 列入 0.5.0+ 路线图"。

### 验证矩阵

- `cargo fmt --all -- --check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test --release` ✅ 291 passed / 0 failed / 2 ignored（baseline 283 → +8：1 PID 复用 + 6 ui_state + 1 source chain）
- `cargo build --release --no-default-features` ✅
- `cargo clippy ... -W clippy::pedantic | grep must_use_candidate` ✅ 0

### ADR 状态

- ADR-0003（PID 复用 start_time 键）Status: **Accepted**（阶段 6 落地，2026-06-16）
- ADR-0005（error.rs source chain）Status: **Accepted**（阶段 9 落地）
- ADR-0006（sync_channel(64) 背压）Status: **Accepted**（阶段 5 落地，2026-06-16）
- ADR-0007（profile.release 保守版）Status: **Accepted**（阶段 7 落地，2026-06-16）

## [0.2.1] - 2026-06-16

Patch 版本，三阶段累积修复：跨平台编译（阶段 2 cfg-gate）+ 文案一致性（阶段 3 纯文案 10 项）+ 鲁棒性（阶段 4 鲁棒性 10 项）。无 API 破坏；行为上有几处错误从"静默吞没"改为"显式提示 / 返回 Result"。

### 阶段 2 — cfg-gate 跨平台降级（Linux 编译修复）

- Fixed: `src/classify.rs` 和 `src/eject/{device,locks,cache,classify}.rs` 在顶级 `use windows::Win32::*` 但无 `#[cfg(target_os="windows")]` gate —— Linux 编译失败、`check-linux` CI 为红。改用**模块级 cfg gate**（详见 ADR-0002）。
- Changed: `src/eject/mod.rs` 顶层暴露跨平台结构体 `RemovableDevice` / `HandleLock` / `UsbScanResult`，原 Windows 实现下沉到 `windows_impl`，非 Windows 走 `stub_impl`（全部返回 `Err(ProcError::UsbDetect)`）。
- Changed: `format_size` 上移到 `eject/mod.rs`，避免循环依赖。
- Changed: `Cargo.toml` `filelocksmith` 依赖移至 `[target.'cfg(windows)'.dependencies]`。
- Changed: `App::new` 启动 `status_message`（非 Windows）补全完整降级清单。

### 阶段 3 — 纯文案一致性（10 项）

- Changed: TUI 文案与 README 快捷键表对齐 —— `A`/`R`/`Shift+←→` 等此前未公开的快捷键全部补到 README。
- Changed: 启动时降级提示文案统一（之前零散）。
- Changed: 杂项中英混排、错别字、过期路径修正（10 项，详见 git log）。

### 阶段 4 — 鲁棒性（10 项）

- Fixed (P1.20): `init_tracing` 静默忽略 `create_dir_all` / `File::create` / `set_global_default` 错误 —— 改为 `eprintln!` 显式提示用户"日志不可用"。
- Changed (P1.21): `AlertManager::load_or_default` 改为 `try_load` 返回 `Result`，调用方决定 fallback；新增 `Default` impl 给 `AlertManager`。
- Fixed (P1.22): `record::reader::Player::open` 不再对恶意 / 损坏的录制文件无限信任 header_len —— 上限 64 KB，超出立即 `bail!`，避免 OOM。新增 `test_player_rejects_oversized_header` 回归测试。
- Fixed (P1.23): `security::hash_cache::HashReputation::hash_file` 从 `std::fs::read` 改为 `BufReader` 流式 + 64 MB 上限 —— 多 GB 安装器 / 敌意文件不再能撑爆内存。新增 2 个单元测试（80 MB 大文件不爆 + 超 cap 字节不影响摘要）。
- Fixed (P1.24): `local_offset_hours` 中 `-(bias + ...) as i64 / 60` 触发 `-(i32::MIN)` UB（Rust 一元 `-` 优先级高于 `as`）。抽出 `bias_minutes_to_offset_hours(i32) -> i64` 纯函数，先 `as i64` 再取负。新增针对 `i32::MIN` 的回归测试。
- Fixed (P1.10): `monitor::watchdog` 收到 Ctrl+C 时 `child.kill()` 静默忽略错误 —— 改为 `tracing::warn!` 记录。
- Fixed (#8): CHANGELOG 引用本 plan 重新编号前的旧 ADR 编号 / dirty-rect 分析文档全部失效 —— 改为泛指（"详见 ADR-0001" / "详见 CHANGELOG 阶段记录"）。
- Added (#10): tracing 补 6 处关键路径观测点 —— `tick_light_refresh` 重刷失败 warn（已有，保留）、`BackgroundScorer` 评分耗时 debug、`scan_ports` 耗时 debug、`find_volume_lockers` 耗时 debug、`record::writer` 每 100 帧写入字节数 debug、`docker::events` 断线重连 warn + 上限 10 次放弃。
- Changed (#12): `init_tracing` 补注释说明 `File::create` 默认 truncate 行为 —— 启动时覆盖旧日志，防止长期运行后 `proc.log` 无限增长。如需保留历史请走 `tracing-appender`（ADR-0006 已规划）。
- Added (#13): `init_tracing` 接入 `EnvFilter::try_from_default_env()`，默认级别 `info`，`RUST_LOG=proc=debug` 等环境变量生效（之前因缺 `env-filter` feature 不生效）。`tracing-subscriber` 启用 `env-filter` feature。README FAQ 增 RUST_LOG 用法说明。

### 验证矩阵

- `cargo fmt --all -- --check` ✅
- `cargo clippy --all-targets -- -D warnings` ✅
- `cargo test --release` ✅ 283 passed / 0 failed / 2 ignored（阶段 3 baseline 279 → +4：1 record header cap + 2 hash streaming + 1 bias_minutes UB 回归）
- `cargo build --release --no-default-features` ✅

Linux 端仍由 GitHub Actions `check-linux` job 验证；本机 WSL vhdx 仍损坏。

### ADR 状态

- ADR-0002（cfg gate）Status: **Accepted**（阶段 2 落地，2026-06-15，本次随 0.2.1 一起发布）

## [0.2.0] - 2026-06-15

本次发布聚焦于代码质量、性能优化和用户体验打磨。共修复 6 个真实 Bug，完成 6 项架构整洁，新增 9 项 UX 改进，4 项跨平台与文档增强，5 项测试覆盖，3 项性能优化。

### 阶段 8 — Review：全局审查 + 脏区域优化（收尾）

- 全局代码审查通过：`cargo test --release`（297 个测试，0 失败）、`cargo clippy --all-targets -- -D warnings`（0 警告）、`cargo fmt --all -- --check`（无 diff）、`cargo build --release --no-default-features`（通过）。
- 33 项问题逐一核对：#1 VT100 RGB、#2 跨平台时区、#3 watchdog try_wait、#4 sysinfo 散落、#5 排序 O(N²)、#6 Arc<Vec> 共享、#7 panels/tui 重命名、#8 AppMode 死代码、#9 tick ≤ 50 行（实测 33 行）、#10 replay ≤ 50 行（实测 24 行）、#11 deprecated 删除、#12 scan_ports 不再 new_all、#13 help_panel.rs、#14 主题持久化、#15 Ctrl+C handler、#16 时间格式含月-日、#17 README 隐藏快捷键公开、#18 THEMES 长度 = 7、#19 Command::Export、#20 read_line 已废弃、#21 Command::Pkill、#22 README 平台支持、#23 LICENSE/CHANGELOG、#24 CI workflow、#25 README GPU 路线图、#26 test_record_color、#27 test_scorer_concurrency、#28 test_kill_tree、#29 skeleton 合并、#30 test_platform_compat、#31 select_nth_unstable、#32 脏区域（见下方"性能优化"）、#33 tick_history_sample 抽离 —— **全部落地**。
- Performance: 脏区域优化经真实测量后决定**不动代码** —— ratatui 内置 buffer diff 已实现 Cell 级增量传输，`App::tick` 已用 `needs_draw` 判断避免无谓重绘，每帧 draw 调用成本 < 15ms（20 fps 预算 50ms）。激进 dirty rect 优化的复杂度收益比差，且引入回归风险。完整分析见 CHANGELOG 阶段 8 记录。
- Performance 基线回归（500 进程基准，见 `tests/test_stage8_perf_regress.rs`）：`rebuild_sorted_cache` **38.2 µs**（< 5ms 目标，130× 裕量）、top-N `select_nth_unstable` + 局部排序 **6.1 µs**（< 1ms 目标，160× 裕量）。无回退。
- Removed: `SystemSnapshot` 中未使用的 `prev_process_disk` / `prev_process_disk_time` 字段（被 `App` 同名字段独立实现，注释明确标记 `#[allow(dead_code)]` "used via App, not directly in SystemSnapshot"），同时移除 `per_disk_io_speed` 上过时的 TODO 注释（功能已实现）。
- Pedantic 现状：`cargo clippy -- -W clippy::pedantic` 共 ~287 个 `format!` 风格建议、~119 个 `#[must_use]` 缺失、~63 个 cast 精度提示等，全部为风格偏好而非 bug。`-D warnings` 等级 0 警告。本阶段不修 pedantic，留作未来风格统一批次。



本次打磨按 8 个阶段组织，预期产出见下方各阶段小节。

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
- Changed: `VT100_VERSION` 提升到 2，旧版本 v1 文件回放时给出友好错误（详见「阶段 2 — 录制 RGB」小节的决策记录）。

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
- Updated: `README.md` 命令行章节补充 `proc pkill` 示例。

### 阶段 5 — Slice：性能优化

- Performance: 引入 `SysinfoRegistry` 全局单例（`src/collect.rs` 顶部 `SYSINFO_REGISTRY` + `sysinfo_with`），消除 5 处散落的 `sysinfo::System::new_all()` 调用 —— `port_map::scan_ports`、`eject::locks::find_volume_lockers_with_processes`（原循环内每个未命中 PID `new_all` 一次，最差 O(N × 200ms)）、`kill::kill_single` 非 Windows 分支、`kill::find_processes_by_name` 全部改为只读访问 SysinfoRegistry 快照。详见 CHANGELOG 阶段 5 记录。
- Removed: `SystemSnapshot::processes()` 老 deprecated 方法（被 `cached_processes_vec` 替代），同步迁移 `tests/test_alert.rs`、`tests/test_process_list.rs`、`tests/test_skeleton.rs` 中 9 处调用，删除已无意义的 `test_incremental_refresh_consistent_with_full`。
- Performance: `App::rebuild_sorted_cache` 从 O(N²) 改为 O(N) —— 一次性构造 `PID → idx` HashMap 替代循环内 `Vec::position` 查找，同时把内部 `Vec<(class, ProcessInfo)>` 改为 `Vec<(class, &ProcessInfo)>` 借引用，省去每帧全字段深拷贝。
- Performance: top-N 进程排序使用 `slice::select_nth_unstable_by` —— 500 进程时比较次数从 ~4500 (O(N log N)) 降到 ~786 (O(N) + O(K log K))，sparkline 历史采样路径受益。
- Performance: `BackgroundScorer::request` 签名从 `Vec<ProcessInfo>` / `Vec<PortEntry>` 改为 `Arc<Vec<...>>`，为下游消费者共享而非拷贝铺路；score 线程循环改用 `as_ref()` 切片迭代。
- Performance: `global_cpu_history` / `global_mem_history` 显式落到 light refresh（每秒）采样；`proc_history` 仍依赖 heavy refresh 的新 cached_processes，只在 heavy 帧推数据，retain 清理保留在 light 中。sparkline 现每秒一格。

### 阶段 6 — Slice：架构整洁

- Refactor: `src/panels/` 重命名为 `src/view_models/`，避免与 `src/tui/`（纯渲染层）在目录名层面混淆 —— 前者持有面板状态 + 业务逻辑（MVVM 中的 ViewModel 角色），后者无状态。`ProcessPanel` 等类型名保持不变，避免改名风暴；外部 import 路径由 `crate::panels::*` 改为 `crate::view_models::*`（涉及 `src/lib.rs` 和 `src/app.rs`）。详见 ADR-0001。
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
