# ADR-0029：record 暴露 + 写操作 confirm 机制

**Status**：Accepted（**v0.17 stage 6 partial 落地 duration_secs 仅记录参数；v0.18 stage 2 补全 auto-stop**，详见 §关键设计点 6 + Migration path）
**Date**：2026-07-07（v0.17.0 阶段 1 落地决策，brainstorm 决策 4 + 5 拍板）/ 2026-07-10 扩 §6 auto-stop（v0.18 stage 1 Spike 决策）
**Related**：ADR-0009（v0.7 MCP server 设计）、ADR-0025b（v0.16 cycle 决策不暴露 record）、ADR-0026（MCP handler 持久字段策略）、v0.6 落地的 pending_record_confirm TUI 路径、v0.7 proc_kill / v0.15 proc_monitor_add dry_run 契约

## 背景（Context）

v0.16 ADR-0025b 推迟项「record 暴露评估」：v0.16 cycle 不暴露 `proc_record_start` / `proc_record_stop`，理由是 (1) MCP stdio 无 TTY（v0.6 落地的 `run_record` 走 TUI 路径需 TTY）；(2) 替代路径（worker 持续采样）工作量大且收益边际；(3) MCP-level confirm 机制待评估；(4) agent 视角应聚焦「分析已有录屏」。v0.16 cycle 7 tool 范围只覆盖 replay / bookmarks / usb_status（操作已存在文件）。

v0.17 cycle 用户拍板全做 5 主题，其中包括 record 暴露 + USB release + docker-rm 写操作（brainstorm §主题 D2 + §FAQ Q1 + Q2）。stage 6 落地 5 个新 tool：

- `proc_record_start` / `proc_record_stop`（record 暴露）
- `proc_usb_release`（kill + flush + eject 三步破坏性操作）
- `proc_docker_rm` / `proc_docker_image_rm` / `proc_docker_volume_rm`（删除容器 / 镜像 / 卷，bollard API）

5 个 tool 都是不可逆破坏性操作，需 confirm 机制让 agent 显式确认风险。

## 决策（Decision）

### 1. record 暴露方案 (a) spawn `proc record` 子进程（brainstorm 决策 4 拍板）

`proc_record_start` tool spawn `proc record --no-tui --output <path>` 子进程（无 TTY，子进程独立写 `.prec`），handler 持 `record_handle: Arc<Mutex<Option<Child>>>` 字段（ADR-0026 落地）跨 tool call 保活；`proc_record_stop` tool kill child + 等待 `.prec` 文件 flush。

`--no-tui` flag（`src/cli/record.rs` + `src/cli/def.rs` 落地）让 `proc record` 走 headless 路径（不 attach TUI），与 v0.6 落地的 `R` 键 TUI 路径并行。stage 1 Spike 加 flag 声明 + stub 分支返 "v0.17-stage-6 未实装" 错误；stage 6 Slice 实装 headless 路径（recorder / bookmark / anomaly detection 复用 v0.6 业务逻辑）。

### 2. confirm 机制方案 A 参数 `confirm: bool`（brainstorm 决策 5 拍板）

5 个新 tool 都 `confirm: bool` 必传参数：

```rust
pub struct RecordStartArgs {
    pub confirm: bool,         // 必传 true 以确认录屏风险
    pub file_path: String,
    pub duration_secs: Option<u64>,
}

pub struct UsbReleaseArgs {
    pub confirm: bool,         // 必传 true 以确认破坏性操作
    pub drive: String,
    pub kill_pids: Vec<u32>,
    pub dry_run: Option<bool>,
}
// ... docker_rm / image_rm / volume_rm 同款
```

`confirm=false` 时返 `ok: false + error: "confirm=true 必传以确认风险"`。`confirm=true` 时才真正执行。

### 3. `confirm` vs `dry_run` 语义差异（互补不冲突）

| 参数 | 语义 | 适用场景 | 默认值 |
|---|---|---|---|
| `dry_run: bool` | 「不真正执行」（预演）| 可逆写操作（kill 进程可重启 / monitor_add 可 remove / bookmarks_add 可 delete）| `false`（agent 想要安全可以 `dry_run=true` opt-in 预演）|
| `confirm: bool` | 「确认风险后再执行」| 不可逆破坏性操作（录屏捕获屏幕 / kill + flush + eject 三步 / 删容器 / 删镜像 / 删卷）| 必传（无默认值，agent 必须显式 `confirm=true`）|

`dry_run` 是 modifier（不改 action 类型），`confirm` 是 gate（不通过则不执行）。两者互补——`proc_usb_release` 同时有 `confirm` 必传 + `dry_run` opt-in，agent 可 `confirm=true + dry_run=true` 预演破坏性操作。

## 关键设计点

### 1. spawn 子进程复用 v0.6 落地的 `run_record` 业务逻辑

`proc_record_start` spawn `proc record --no-tui --output <path>` 子进程，复用 v0.6 落地的 `run_record` 路径全部业务逻辑（recorder / bookmark / anomaly detection），不重写。`--no-tui` flag 让 record 走 headless 路径，与 v0.6 落地的 `R` 键 TUI 路径并行。

### 2. MCP handler 不持 worker 状态

与 v0.12 TD-36 持久 `dns_collector` 模式不同——`dns_collector` 是查询类 worker 持有成本低（drain 一次返空），`record_handle` 是录制类 worker 持有成本高（child 进程持续运行 ~30 FPS × 165 µs/frame = ~5% CPU + ~10MB 内存）。spawn 子进程让 child 进程独立于 MCP server 主路径，崩溃隔离（record worker 崩溃不影响 MCP server）。

### 3. 子进程崩溃隔离

record worker 崩溃（如 `.prec` 文件写盘失败 / recorder panic）不影响 MCP server——`proc_record_stop` 调用时检测 child handle 状态，如 child 已退出返 `ok: false + error: "record subprocess exited unexpectedly"` + 已写入的 `.prec` 文件元数据。MCP server 主路径不受影响。

### 4. `confirm` 与 v0.6 落地的 `pending_record_confirm` TUI 路径同款契约

v0.6 落地的 `pending_record_confirm` TUI 路径：用户按 `R` 弹确认（警告会捕获屏幕所有内容含 DNS 域名 / 进程 cmd），按 `y` 确认 / `n` 取消。MCP 路径用 `confirm: bool` 必传参数同款契约——agent 传 `confirm=true` 显式确认 = 用户按 `y`。

### 5. `--no-tui` flag 重构成本低

stage 1 Spike 加 flag 声明 + stub 分支（返 "v0.17-stage-6 未实装" 错误）。stage 6 实装 headless 路径 ~50 行（让 record 走 `Recorder::submit_frame` 路径绕过 `tui::setup_terminal() + tui::run_app()`），与 v0.6 落地的 `R` 键 TUI 路径并行。

### 6. record auto-stop 子进程 `--duration` flag（v0.18 stage 2 落地，stage 1 Spike 落地 flag stub）

**v0.17 stage 6 落地的 `duration_secs` 参数仅记录不真正 auto-stop**——`make_record_start_json` 把 `duration_secs` 原样回显 `expected_duration_secs`，warning 字段透出「agent 须显式调 proc_record_stop」（`src/mcp/handler/record.rs:1135-1139`）。这与本 ADR §1 决策 1 「record 暴露方案 (a) spawn 子进程」描述「`duration_secs: Option<u64>`」可推断 auto-stop 应自动生效有差距（REVIEW-v0.17 §4 Findings P2-R2）。

**v0.18 stage 1 Spike 落地**（flag 声明 + 解析 stub）：

| 修改点 | 范围 | stage |
|---|---|---|
| `src/cli/def.rs` `Command::Record` 加 `duration: Option<u64>` 字段 | flag 声明 + clap 解析 | 1 Spike |
| `src/cli/record.rs` `run_record` / `run_record_headless` 接受 duration 参数 | 参数传递（stub：暂存不真正实装 timer thread，stage 2 实装） | 1 Spike |
| `src/mcp/handler/record.rs` `make_record_start_json` spawn 时传 `--duration <secs>` | spawn 路径加 flag（stub：stage 2 实装）+ 移除 warning 字段 | 2 Slice |

**v0.18 stage 2 实装路径**（stage 1 Spike 设计，stage 2 实装）：

1. **CLI 加 `--duration` flag**：`src/cli/def.rs` `Command::Record` 加 `duration: Option<u64>` 字段（已 stage 1 落地）
2. **`run_record_headless` 内 timer thread**：spawn `std::thread::spawn(move || { sleep N secs; shutdown::request(); })` + 主循环检 `shutdown::requested()` 退出
3. **MCP handler spawn 时传 flag**：`make_record_start_json` spawn 时加 `.arg("--duration").arg(secs.to_string())`（如 `duration_secs` Some）+ 移除 warning 字段

**与 ADR-0029 决策 4「spawn 子进程复用 v0.6 业务逻辑 + 子进程崩溃隔离」对齐**：

- 子进程 timer 随 child 退出自动终止，无 zombie timer（vs MCP handler timer 需手动 cancel）
- MCP handler 不持 timer 状态（与 §关键设计点 2「MCP handler 不持 worker 状态」对齐）
- 复用 v0.6 落地的 `shutdown::requested()` 干净退出路径（与 `--no-tui` headless 路径同款）
- 子进程崩溃时 timer 也随之终止（vs MCP handler timer 需独立检测 child 退出）

**与 brainstorm 决策 4 拍板「子进程 `--duration` flag」对齐**：v0.18 cycle 不用 MCP handler timer 方案（备选 (b) 已否决），与 ADR-0029 决策 4 spawn 子进程模式延续。

## 备选方案（Alternatives）

### record 暴露方案

#### (a) spawn `proc record` 子进程（**本 ADR 选此**，brainstorm 决策 4 拍板）

**接受**：复用 v0.6 落地的 `run_record` 路径全部业务逻辑 + 子进程崩溃隔离 + `--no-tui` flag 重构成本低。

#### (b) MCP handler 启 worker 线程

**否决**：worker 持续运行影响主路径（~5% CPU 持续占用）+ lifecycle 管理复杂（client 关闭后 worker 仍跑？什么时候停止？写文件失败如何反馈给已断连的 client？）+ 需重写 record 路径绕过 TUI。

#### (c) 推迟到 v0.18+ cycle

**否决**：v0.17 cycle 用户拍板全做 5 主题，包括 record 暴露。

### confirm 机制方案

#### A 参数 `confirm: bool`（**本 ADR 选此**，brainstorm 决策 5 拍板）

**接受**：与 v0.6 落地的 `pending_record_confirm` TUI 路径同款契约 + 简单（一个 bool 参数，无需 token 状态机）+ agent 视角明确（`confirm=true` 时才真正启动，`false` 时返 error 让 agent 知道需要确认）。

#### B 两次调用 confirm（token 状态机）

**否决**：复杂（agent 第一次调 `proc_record_start` 返 token + warning，第二次调 `proc_record_confirm(token)` 真正启动）。类似 2FA 但 MCP 协议本身已有 client-server 信任关系，无需 token 状态机。

#### C 仅文档警告，无参数

**否决**：agent 视角不明确（description 字段写警告但 agent 仍可调，无法强制 confirm）。需显式 `confirm=true` 让 agent 显式确认风险。

### record auto-stop 位置方案（v0.18 cycle 拍板）

#### (a) 子进程 `--duration` flag（**v0.18 cycle 选此**，brainstorm 决策 4 拍板）

**接受**：与 ADR-0029 §决策 1「spawn 子进程复用 v0.6 业务逻辑 + 子进程崩溃隔离」对齐——子进程 timer 随 child 退出自动终止无 zombie timer / MCP handler 不持 timer 状态（与 §关键设计点 2「MCP handler 不持 worker 状态」对齐）/ 复用 v0.6 落地的 `shutdown::requested()` 干净退出路径（与 `--no-tui` headless 路径同款）。

#### (b) MCP handler 启 timer 线程

**否决**：MCP handler 持 timer 状态——child 退出时需独立检测并 cancel timer（lifecycle 管理复杂）/ client 异常断开时 timer 持续运行（zombie timer 占内存 + CPU）/ 与 §关键设计点 2「MCP handler 不持 worker 状态」决策冲突。

#### (c) 推迟到 v0.19+ cycle

**否决**：v0.18 cycle 是 v0.17 残留项补全 cycle，duration_secs warning 字段透出已一个 cycle 不补全影响 agent 体验（agent 必须显式调 proc_record_stop 才能停止，违反 `duration_secs: Option<u64>` 参数语义）。

## 结果（Consequences）

- **stage 1 Spike 落地**：5 个新 Args struct（RecordStartArgs / RecordStopArgs / UsbReleaseArgs / DockerRmArgs / DockerImageRmArgs / DockerVolumeRmArgs）+ 5 个 stub helper（返 placeholder JSON 含 `received_confirm` 字段）+ `src/cli/record.rs` 加 `--no-tui` flag stub + `src/cli/def.rs` `Command::Record` 加 `no_tui: bool` 字段
- **stage 6 Slice 实装**：spawn 子进程路径 + `--no-tui` headless 路径 + 5 个 tool 业务逻辑填充（kill_locks / flush_write_cache / eject_device / bollard remove_container / remove_image / remove_volume）
- **5 个 tool 都 `confirm: bool` 必传**：agent 必须显式确认风险才执行，与既有 `dry_run: bool` 默认 false 契约互补

### 负面（Trade-offs）

- **`--no-tui` flag 重构风险**：v0.6 落地的 `run_record` 路径深度耦合 TUI（recorder / bookmark / anomaly detection 都假设 TUI 上下文）。stage 6 实装时如 `--no-tui` 重构超预期 → 触发 brainstorm §决策 8 自适应拆分规则（stage 6a record 暴露 / stage 6b USB release + docker-rm 写操作）
- **子进程开销**：~50ms spawn + ~10MB 内存（每录屏 session）。可接受（vs worker 持续运行 ~5% CPU）
- **`confirm` 与既有 `dry_run` 语义可能混淆**：agent 视角可能混淆（dry_run 是「不真正执行」/ confirm 是「确认风险后再执行」）。mitigate：ADR-0029 文档化语义差异 + 5 个 tool 的 description 字段明确说明 confirm 必传 + dry_run opt-in 语义

## Migration path

- **v0.17 stage 1 Spike**（本 ADR 落地）：5 个 Args struct + 5 个 stub helper + `--no-tui` flag stub + `mcp-persistent-state` feature flag（`record_handle` 字段 cfg-gate）
- **v0.17 stage 6 Slice**：spawn 子进程路径 + `--no-tui` headless 路径 + 5 个 tool 业务逻辑填充 + `proc_record_start` / `proc_record_stop` 跨 tool call 保活 child handle（**`duration_secs` 仅记录参数不真正 auto-stop**，warning 字段透出）
- **v0.18 stage 1 Spike**：扩 §6 auto-stop 段 + stage 1 落地 `--duration` flag 声明 stub（`src/cli/def.rs` Command::Record 加 `duration: Option<u64>` 字段 + `src/cli/record.rs` 解析 stub 暂存不真正实装 timer thread）
- **v0.18 stage 2 Slice**：`run_record_headless` 内 timer thread（spawn thread sleep N secs → `shutdown::request()`）+ MCP handler `make_record_start_json` spawn 时传 `--duration <secs>` flag + 移除 warning 字段（auto-stop 已实装）+ ADR-0029 Status 备注 「auto-stop 已补全」
- **v0.19+ cycle**：评估 record 暴露方案 (b) worker 持续采样路径（如 spawn 子进程开销可感）/ 评估 confirm 机制方案 B 两次调用 token（如 agent 反馈单参数 confirm 不够安全）

## 相关 ADR / 文档

- [ADR-0009](0009-mcp-server.md)：v0.7 MCP server 设计（agent 视角字段裁剪原则延续）
- [ADR-0025b](0025b-mcp-record-not-exposed.md)：v0.16 cycle 决策不暴露 record（推迟理由 + v0.17+ cycle 评估路径）
- [ADR-0026](0026-mcp-handler-persistent-fields.md)：MCP handler 持久字段策略（`record_handle` 字段落地基础）
- v0.6 落地的 `pending_record_confirm` TUI 路径：`src/record/mod.rs` + `src/app.rs`
- v0.7 proc_kill / proc_pkill `dry_run` 契约：`src/mcp/handler/mod.rs::make_kill_json` / `make_pkill_json`
- v0.15 proc_monitor_add `dry_run` 契约：`src/mcp/handler/cli.rs::make_monitor_add_json`
- v0.16 proc_bookmarks_add `dry_run` 契约：`src/mcp/handler/record.rs::make_bookmarks_add_json`
- [`docs/stages/v0.17-stage-1.md`](../stages/v0.17-stage-1.md) §决策 2（6-7 个新 tool 范围）+ §决策 3（stub 返回格式）+ §决策 5（CONTEXT.md 新术语 8 个，含 ConfirmRequired / RecordSubprocess / HeadlessRecord）
