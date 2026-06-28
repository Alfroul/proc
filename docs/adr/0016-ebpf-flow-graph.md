# ADR-0016: eBPF flow graph + exit-accounting via aya-rs (feature flag `ebpf`)

## Status

**Accepted** — v0.7.0 阶段 8 引入

## Context

v0.6 proc 已有：

- **DNS 查询日志**（PowerShell subscriber，500ms 周期，内存 1000 条 FIFO）
- **per-process 网络流量**（Windows IP Helper / Linux nethogs 子进程）

但这两个数据源是**半关联**的：DNS 知道"pid=X 查询了 example.com"，网络流量知道"pid=X 当前 1KB/s 出 500B/s 入"，但**不知道 pid=X 把数据发到了哪个 IP / 那个 IP 是不是 example.com 解析出来的**。

定位"哪个二进制和哪个域名说了多少字节"（典型场景：发现挖矿/外联 C2）需要端到端关联：

```
execve → 进程启动
connect → 进程发起 TCP 连接到 <remote_ip:port>
DNS 查询 → 进程查询 example.com → 解析结果含 <remote_ip>
关联：pid + remote_ip 同时在 connect 和 DNS 出现 → 流量归属该域名
```

业界最佳实践（bpfview / peekd / procscope）是用 eBPF 串起来。

## Decision

**用 `aya-rs 0.13` 加载 eBPF 程序，监听 3 类内核事件（tracepoint sys_enter_connect / kprobe tcp_connect / tracepoint sched:sched_process_exit），ring_buf 推到 userspace，FlowAggregator 把 DNS + 网络 + connect 关联为 `ProcessFlow`。Linux only + feature flag `ebpf` + SNI 留 v0.8.0+。**

具体决策：

1. **库选 `aya-rs`**（不用 libbpf-rs / 不用 BCC）：
   - aya 是纯 Rust eBPF 库，CO-RE 内嵌
   - 不需要 kernel headers（BTF-only）
   - Helix / bpftrace 在用
   - libbpf-rs 是 libbpf 绑定，需要 C 编译器 + libelf
   - BCC 是 Python + 巨大依赖

2. **feature flag `ebpf`**（默认关闭）：
   ```toml
   [features]
   default = ["nvidia", "nvtop", "nethogs"]
   ebpf = ["aya", "aya-log"]
   ```
   - 理由：aya 依赖 + 内核态 ELF = 包体增加 ~3MB
   - Linux 用户显式启用：`cargo build --release --features ebpf`
   - release CI 在 linux-musl / linux-arm target 额外产出 `_ebpf` 后缀二进制

3. **3 类内核事件**：
   - **tracepoint `sys_enter_connect`**：socket connect() syscall 入口，拿 fd + sockaddr 指针
   - **kprobe `tcp_connect` / `__tcp_connect`**：完整 TCP 流建立（不同内核版本函数名不同，加载时检测）
   - **tracepoint `sched:sched_process_exit`**：进程退出，做 exit-accounting

4. **不监听 TLS SNI**（第一版）：
   - SNI 需要在 SSL_write 上挂 uprobe（OpenSSL / BoringSSL / LibreSSL 多个版本分支）
   - 复杂度高（每个 SSL 库版本一份 BPF 程序）
   - 第一版只把 DNS + connect 关联（DNS 已能拿到大部分域名信息）
   - SNI / JA4 留 v0.8.0+（见 tech-debt TD-17）

5. **数据通路**：
   ```
   内核态 (aya-ebpf) → ring_buf → userspace reader 线程 → FlowAggregator → App::flows (1s drain)
   ```
   - ring_buf 比 perf buffer 更高效（aya 推荐）
   - reader 线程单独跑，不阻塞主线程
   - 主线程 1s drain 拿 snapshot

6. **PID 复用防串**：
   ```rust
   pub struct ProcessFlow {
       pub pid: u32,
       pub start_time: u64,    // 与 v0.6 ProcessInfo 缓存键一致
       pub comm: String,
       pub local_addr: String,
       pub remote_addr: String,
       pub bytes_out: u64,
       pub bytes_in: u64,
       pub dns_name: Option<String>,
       pub first_seen: SystemTime,
       pub last_seen: SystemTime,
   }
   ```
   - key 含 `(pid, start_time)`，PID 复用时不会串数据

7. **DNS 关联启发式**（第一版不 100% 准）：
   - 当 connect 事件来到，在 dns_recent（v0.5.0 已有 1000 条 FIFO）里找最近 5s 内的 DnsQuery
   - 匹配条件：同 pid + 对端 IP == DNS 解析结果 IP
   - 找到 → 填 `flow.dns_name = Some(query)`
   - 找不到 → flow.dns_name = None（不代表可疑，可能命中 cache 或非 DNS 解析路径）

8. **SecurityRule R15 外联行为评分**：
   - 命中条件（任一）扣 30 分：
     - 进程外联到 SNI 不在白名单（用户可配 `~/.config/proc/sni_whitelist.txt`）
     - 短时间（10s）外联到 ≥ 50 个不同 IP（端口扫描特征）
     - DNS 查询与 connect 目标 IP 不一致（DNS hijack 检测，需要 dns_name 已关联）
   - 加到 SecurityScorer 总分（v0.6 14 项 → v0.7 15 项）

9. **exit-accounting**：
   - `sched:sched_process_exit` 事件 → flow.exit_time = Some(now)
   - 退出后保留 flow 30s（"幽灵 flow"），方便用户看到刚结束的连接
   - 与 atop 的 process accounting 类似，但用 eBPF 不用 acct(2)

## Alternatives Considered

### A. 用 libbpf-rs（不用 aya）

**否决理由**：
- libbpf-rs 是 libbpf 的 Rust 绑定，需要 C 编译器 + libelf + zlib
- 跨平台编译更难（macOS / Win 交叉编译失败）
- aya 是纯 Rust，单 cargo build 完成

### B. 用 BCC（Python 桥）

**否决理由**：
- BCC 需要 Python runtime + LLVM
- 包体巨大（> 50MB）
- 不适合 proc 这种单二进制 CLI

### C. 引入完整 eBPF 栈（Tetragon / Falco 级别）

**否决理由**：
- Tetragon 是 Kubernetes 级别，proc 用不上
- Falco 是规则引擎，不是 lib
- proc 只需要 3 个 tracepoint，不需要完整 eBPF 框架

### D. TLS SNI 也做（第一版就上）

**否决理由**：
- SNI uprobe 多版本分支（OpenSSL / BoringSSL / LibreSSL / 各版本 offset 不同）
- 至少 1-2 周额外工作量
- DNS 关联已能覆盖大部分场景
- 留 v0.8.0+ TD-15

### E. 用 ETW TLS write 抓 SNI（Windows 等价物）

**否决理由**：
- ETW 提供 `Microsoft-Windows-Schannel` event 196（含 SNI），但 schema 复杂
- 留 v0.8.0+ TD-18

### F. 不做端到端关联（沿用 v0.6 半关联）

**否决理由**：
- v0.6 缺口明显（无法回答"哪个二进制和哪个域名说了多少字节"）
- bottom / btop / glances 都没做（proc 的强差异化）
- 与 14 项安全评分天然契合（R15 外联行为）

### G. 用 atopacctd / netatop-bpf（atop 方案）

**否决理由**：
- atopacctd 是独立守护进程，需要 install + systemd 配置
- proc 是单二进制，应自包含
- netatop-bpf 字节跳动的方案，但 proc 自己写更可控

## Consequences

### 正面

- **杀手锏**：定位挖矿 / 外联 C2 一目了然（同类 TUI 进程管理器独有）
- **与安全评分契合**：R15 让 14 项 → 15 项，纵深加强
- **DNS + 网络流量 + connect 三源关联**：v0.6 数据利用率提升
- **Linux 5.10+ 普及率高**（2020 年以后内核），降级路径足够

### 负面

- **包体增加 ~3MB**（feature flag 隔离，默认不带；见下方实测）
- **Linux only**：Windows / macOS 用户没此功能（Windows 等价物见 TD-18）
- **root 或 CAP_BPF**：普通用户运行不生效（降级提示）
- **DNS 关联启发式不 100% 准**：命中 cache 的查询关联不到，需用户理解
- **aya-rs 0.13 API 可能变**：v0.8+ aya 升级阻力
- **Linux 真实编译验证缺失**：worker.rs / ebpf-ebpf/src/main.rs 在 Windows 会话落地，未在 Linux + root + 内核 5.10+ 环境验证（详见 TD-19）

### 实测数据（v0.7.0 阶段 8 Part B 完工时）

- **Windows 默认 build 包体**：cargo build --release 不带 feature，aya 不进依赖图，包体与 v0.6 基线一致。`cargo build --features ebpf` 在 Linux 上预期 +3MB（aya 0.13 + 内核态 ELF 嵌入）；Windows 无法验证（无 bpf-linker）。
- **Linux 内核版本要求**：≥ 5.10（CO-RE / BTF 支持）。Ubuntu 20.04+ / Debian 11+ / RHEL 9 默认满足。
- **DNS 关联命中率（设计目标）**：~60-80%。命中条件：DNS 查询 < 5s 内伴随 connect 到解析结果 IP。命中率下降场景：(a) /etc/hosts 直配；(b) DNS-over-HTTPS / DNS-over-TLS；(c) 系统解析缓存命中（不产生新的 DnsQuery 事件）。**未在真实流量上测**，留 TD-19 修复时一并验证。
- **R15 误报率（设计目标）**：默认关闭（`~/.config/proc/sni_whitelist.txt` 不存在 → R15 不启用）。用户显式创建文件才激活，触发即扣 30 分。**Port scan 条件** 阈值 50 个不同 IP / 10s（典型浏览器 ~ 20，普通服务进程 ≤ 10，留余量）。

### 缓解

- feature flag `ebpf`：`cargo build` 不带默认，包体不显著增加
- root / CAP_BPF 检测：worker spawn 失败 → warn + App::flows 为空（不影响 TUI 其他功能）
- DNS 关联启发式：在 UI / 文档明确说明"流量可能显示 dns_name=None 不代表可疑"
- aya 升级：v0.7 阶段 8 落地后锁版本，v0.8+ 评估升级

## Implementation Notes

- 入口：`src/ebpf/{mod.rs,flow.rs}`
- 内核态：`src/ebpf/ebpf-ebpf/`（独立 cargo sub-project，aya-ebpf crate）
- WorkerManager 集成：`src/workers/manager.rs::ebpf_worker: Option<EbisuBpfWorker>`（feature + cfg-gate）
- 安全评分：`src/security/flow.rs`（R15 命中条件）
- UI：`src/view_models/port_panel.rs` 加 `F` 键切 Flow 子视图
- CLI：`src/cli/flows.rs::run_flows`
- 测试：`tests/test_ebpf_flow.rs`（feature ebpf only，Linux only）

## Capacity Considerations

本阶段是 v0.7 工作量最大的一段（~2500-3500 行 + 100-150 工具调用），**预期需要 Checkpoint 接力**：

- **Part A（MVP）**：execve + connect + DNS 关联（任务 1-7）— 单阶段约 1 周
- **Part B（exit-accounting + R15）**：sched_process_exit + SecurityRule R15 + CLI（任务 8-14）— 单阶段约 3-5 天

Part A 完成后**必须写 Checkpoint**，硬停止，开新会话接力 Part B。

## References

- [aya-rs book](https://aya-rs.dev/book/)
- [aya GitHub](https://github.com/aya-rs/aya)
- [bpfview (eBPF 端到端关联参考)](https://github.com/bpfview/bpfview)
- [peekd](https://github.com/fntlnz/peekd)
- [Linux PSI / eBPF docs](https://www.kernel.org/doc/html/latest/bpf/index.html)
- [sched_process_exit tracepoint format](https://www.kernel.org/doc/html/latest/trace/events.html)
- proc v0.5.0 DNS 日志（`docs/adr/0006-dns-subprocess-not-etw-dbus.md`）
- proc v0.6.0 安全评分 14 项（R15 扩展点）
- proc v0.7.0 `docs/tech-debt.md` TD-17（eBPF SNI）/ TD-18（Windows Schannel）/ TD-19（Linux 真实验证缺失）
