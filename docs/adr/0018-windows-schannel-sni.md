# ADR-0018: Windows Schannel ETW SNI via 手写 windows-rs ETW + TDH 动态 schema

## Status

**Accepted** — v0.10.0 阶段 1 引入。决策从「硬编码偏移（仿 ADR-0015 disk_io_etw）」改为「TDH 动态 schema 解析」。

**v0.10.0 阶段 2 实测修订（2026-06-28）**：阶段 1 推测的 provider GUID + event ID + 字段名全部错误（详见 §3 schema 表），实装路径走 TDH 动态 schema 路线本身不受影响（按 property name `TargetName` 查字段），但 §3 的「待实测修订」标注已删除。

**v0.10.0 阶段 3 落地（2026-06-28）**：worker → App → UI / CLI / R15 全链路打通。`ProcessFlow.source: FlowSource` 字段（Copy enum 区分 `Ebpf` / `Schannel`）+ `App::overlay_flow_sni_schannel`（drain SchannelEtwWorker → 按 pid 覆盖 / 新建 flow）+ `port_table::draw_flow_view` 跨平台对齐 + R15 条件 1 同时检查 sni + dns_name + `proc flows` CLI 跨平台入口。**TD-18 标 ✅ Fixed**。详见 [CHANGELOG.md](../../CHANGELOG.md) v0.10.0 阶段 3 段。

## Context

v0.7 阶段 8 落地了 eBPF flow graph（ADR-0016），Linux 用户在 proc 里能看到「进程 → 域名 → 字节数」的端到端关联。但 Windows 没有 eBPF，这套能力长期缺位（v0.7 release notes 标 **TD-18**：Windows Schannel SNI）。

v0.9 cycle 计划在 Linux 上扩 `ProcessFlow.sni / ja4 / bytes_out / bytes_in`（eBPF uprobe on `SSL_write` + JA4 RFC 9503 + kprobe on `tcp_sendmsg`），但 v0.9 cycle 整体推迟——`ProcessFlow.sni` 字段在 v0.10 阶段 1 一并扩上（`src/ebpf/flow.rs`），ja4 留待 eBPF 那边实现时再加。

Windows 抓 SNI 的可选路径：

1. **ETW `Microsoft-Windows-Schannel`**：Schannel SSP 在 TLS handshake 时 fire 事件，含明文 SNI。Win10 1809+ 加了更细粒度的 TLS handshake 事件（含 SNI 字段）。
2. **ETW kernel network stack**：内核网络栈 ETW provider（`Microsoft-Windows-Kernel-Network`）只给 packet 元数据，不含 TLS payload，抓不到 SNI。
3. **WinDivert**：用户态 packet filter 驱动，能拿原始 ClientHello packet 自己 parse SNI。需驱动签名（用户态部分不需要但 kernel 部分需要），性能差（每 packet userspace round-trip）。
4. **FiddlerCore / HTTP Toolkit**：商业 / 半商业，做 MITM 才能解密 TLS——抓不到 SNI 之外的明文不是 MITM 的话其实直接看 ClientHello 也行，但需注入 system proxy，对用户侵入太大。
5. **System Informer netmgr**：实际上 System Informer 也没解决 Windows SNI 问题（network tab 只显示 IP + 字节数，不显示域名）。

路径 1 最契合 proc 风格（v0.7 阶段 7 已落地 disk_io_etw 手写 ETW，模式成熟）。但与 disk_io_etw 不同：**Schannel 是用户态 provider，schema 在 manifest 里**；disk_io_etw 是内核态 provider（NT Kernel Logger），schema 是 `Win32` 文档化的 MOF class（`DiskIo_TypeGroup1`）。

## Decision

**手写 windows-rs ETW（`Win32_System_Diagnostics_Etw`）开 `Microsoft-Windows-Schannel` session 监听 TLS handshake 事件，用 `TdhGetEventInformation` 动态解析 schema 提取 SNI + 关联 PID，覆盖 eBPF 在 Windows 上的缺位。Windows only cfg-gate + 管理员权限 + 失败降级。**

具体决策（关键差异：与 ADR-0015 disk_io_etw **不走同款「硬编码偏移」路线**，走「TDH 动态 schema」路线）：

1. **不引 ferrisetw / schannel-rs，手写 windows-rs ETW**：
   - 项目偏好「更可控」（与 ADR-0015 一致）：windows-rs 已是项目依赖（v0.6 +），只加 `Win32_System_Diagnostics_Etw` + `Win32_System_Diagnostics_Tdh` feature
   - schannel-rs crate 最后一次 release 2019 年，SSPI API 也不直接给 ETW
   - ferrisetw fallback 同 ADR-0015：未启用，schema 解析变脆可切换

2. **`Microsoft-Windows-Schannel-Events` provider GUID = `{91CC1150-71AA-47E2-AE18-C96E61736B6F}`**：
   - **阶段 2 实测修订**：阶段 1 推测的 `{37D2C3CD-...}`（`Security: SChannel`）实测对 curl TLS handshake **不 fire 任何 event**——管理员 + `logman create trace` + 3 个 curl https:// 触发后 ETL 只含 NT Kernel Logger 元数据，无 Schannel provider 事件
   - 实际能 fire 1793（SNI）/ 257/258/1025-1028/1537/1538 等 Schannel 事件的是 `Microsoft-Windows-Schannel-Events` provider（GUID `{91CC1150-71AA-47E2-AE18-C96E61736B6F}`），通过 `logman query providers | Select-String channel` 可枚举
   - 注意与 EventLog 里的 `Schannel` source（GUID `{1f678132-5938-4686-9fdc-c8ff68f15c85}`，写到 System Log）区分——后者是经典 EventLog provider（event IDs 36864-36888），不含 SNI 明文
   - manifest-based ETW provider 在 `C:\Windows\System32\schannel.dll` 资源里，TdhGetEventInformation 能拉到 schema

3. **TDH 动态 schema 解析（与 disk_io_etw 硬编码偏移的根本差异）**：
   ```text
   ADR-0015 (disk_io_etw)            ADR-0018 (schannel_etw)
   ──────────────────────────────    ──────────────────────────────
   NT Kernel Logger                  Microsoft-Windows-Schannel-Events
   MOFF class (DiskIo_TypeGroup1)    Manifest schema (TDH 动态)
   偏移硬编码（x64 36B 固定）          TdhGetEventInformation 按 property name 查
   schema 跨版本稳定（Win8+）          schema 跨版本可能变（manifest 可扩字段）
   ```
   原因：disk_io 是内核 provider schema 自 Win8 起稳定（公开 Win32 文档）；
   Schannel 是用户态 manifest-based provider，schema 在 `schannel.dll` 资源里，
   manifest 加字段时 layout 可能变（即便当前实测只有 `ContextHandle + TargetName`
   2 个 top-level property，未来 Windows 版本可能扩到 3-4 个）。TDH API
   （`TdhGetEventInformation` + `EventPropertyInfoArray`）按 property name
   （`TargetName`）查字段，**不硬编码 offset**，跨 Win10/Win11 版本兼容。

4. **event 1793 / SNI 字段实测结论（阶段 2 实测，2026-06-28）**：
   - **阶段 1 推测的 event 196 实测完全不出现**：管理员 + `logman create trace proc-schannel-probe -p {91CC1150-...} 0xFFFF...FFFF 255 -ets` + 3 个 `curl https://` 触发后，ETL 抓到的 Schannel events 是 257/258/1025-1028/1283/1284/1537/1538/1793/1794 共 10 个 EventID，**完全没有 196**
   - **SNI event = 1793**（Task = `DeleteSecurityContext` 28672, Opcode = 1 Start, Level = 4 Informational, Keyword = `0x8000000000000000`）
   - **SNI 字段 = `TargetName`**（UTF-16 LE null-terminated string，不是阶段 1 推测的 `ServerName`）
   - **完整 schema**（实测 + tracerpt XML 字段对账）：
     ```text
     EVENT_RECORD.EventHeader.EventDescriptor.Id = 1793
     EVENT_RECORD.EventHeader.EventDescriptor.Opcode = 1
     EVENT_RECORD.EventHeader.ProcessId = 发起 TLS handshake 的 PID（curl 的 PID）
     EVENT_RECORD.UserData layout：
       offset 0..7   ContextHandle  (u64 pointer，TLS context 句柄)
       offset 8..    TargetName     (UTF-16 LE null-terminated string)
     ```
   - **实装走 TDH 动态 schema**：不硬编码上述 layout（manifest 加字段时仍能工作），按 property name `TargetName` 查字段；调用 `TdhGetEventInformation` 拉 `TRACE_EVENT_INFO` → 遍历 `EventPropertyInfoArray` 找 `TargetName` → 用 `TdhGetPropertySize` 累加算 data offset → 从 `UserData[offset..]` 读 UTF-16 LE 串

5. **PID 关联**：
   - 与 disk_io_etw 不同：Schannel event 自带 `EventHeader.ProcessId`（用户态 provider，不需要 thread→pid map）
   - 极少数 svchost 转发场景（如 LSASS 代理握手）：保留 ProcessId 原值，不在阶段 1-2 反查 socket→pid（留 v0.10.1+ 优化）

6. **覆盖 v0.9 推迟的 `ProcessFlow.sni` 字段**：
   - v0.10 阶段 1 在 `ProcessFlow` struct 加 `pub sni: Option<String>`（`#[serde(default)]`）
   - eBPF Linux 路径暂填 `None`（v0.9 推迟，uprobe on `SSL_write` 复活时填）
   - Windows Schannel 路径在 v0.10 阶段 2-3 由 Schannel worker 填
   - ja4 字段**本周期不加**——纯 eBPF 范畴，与 Schannel 路径无关

7. **降级路径（与 disk_io_etw 同款 cfg-gate + admin 检查）**：
   - Linux / macOS：worker 是 None，ProcessFlow.sni 在 eBPF 路径填
   - Windows 非管理员：`StartTraceW` 失败 → warn 日志 + 降级（sni 永远 None）
   - **Win10 < 1809**（v0.10 阶段 4 REVIEW-11 P2-1 确认）：provider GUID 在 Win10 早期版本就存在，`StartTraceW` + `EnableTraceEx2` + `OpenTraceW` 全部成功，worker 启动成功（不是 None），但 event 1793 是 Win10 1809+ 才有的精细化 TLS handshake 事件（build 17763+），永不 fire → accum 永远空 → UI 显示「Schannel Flow graph（0 条）」误导用户。无法在用户态探测 event 是否会 fire（除非真跑 handshake 触发），归档为 [tech-debt TD-20](../tech-debt.md)（v0.11+ 评估用 `RtlGetVersion` 探测 build number < 17763 直接返 None）。
   - x86 (32-bit) Windows：直接拒绝（cfg-gate），同 disk_io_etw
   - TDH 解析失败的 event：drop 该事件，不影响其他
   - **trace_thread spawn 失败**（v0.10 阶段 4 REVIEW-11 P1-2）：spawn 失败时清理 `stop_session(session_handle)` + `CloseTrace(trace_handle)` 后返 None（不让 session 名 `proc-schannel-sni\0` 被占用导致后续 proc 重启失败）。

8. **WorkerManager 集成（阶段 2 落地，阶段 1 不接）**：
   - 阶段 1 只跑通最简 Schannel session（callback hex 打印 raw UserData）
   - 阶段 2：新增 `schannel_etw_worker: Option<SchannelEtwWorker>`（Windows only），与 disk_io_etw 同款 `SnapshotWorker<Vec<SniRecord>>` 模板，1s tick → 主线程 drain
   - `proc diag` 加 `schannel_etw` worker 行

## Alternatives Considered

### A. 沿用 sysinfo + DNS 关联（不升级）

**否决理由**：
- HTTPS 流量命中 DNS cache 时（系统 hosts / 缓存命中），无对应 DnsQuery 事件，关联不到
- Chrome / Firefox 用 DoH（DNS over HTTPS）时，DNS 查询走加密通道，proc 抓不到
- v0.7 阶段 8 的 ProcessFlow.dns_name 字段在大流量场景命中率 < 70%（实测观察）

### B. 硬编码偏移（仿 ADR-0015 disk_io_etw）

**否决理由（落选）**：
- Schannel 是用户态 manifest-based provider，schema 在 `schannel.dll` 资源里；
  阶段 2 实测当前 Win11 26100 是 2 个 top-level property（`ContextHandle` + `TargetName`），
  但未来 Windows 版本扩展字段时硬编码偏移会失效
- TDH API（`TdhGetEventInformation`）就是为这种场景设计的——manifest-based
  provider 用动态 schema 解析是标准做法（按 property name 查，不硬编码偏移）

### C. 用 ferrisetw（自动 TDH 解析）

**否决理由（落选）**：
- 与 ADR-0015 同款：用户偏好「更可控」，手写 ~250-400 行 windows-rs 已是项目依赖
- ferrisetw ~500KB + KrabsETW 间接依赖，手写直接复用 v0.7 disk_io_etw 模式
- **保留为 fallback**：若 TDH 解析在跨版本上变脆可切换；ADR-0018 文档已留接口签名

### D. WinDivert + ClientHello parser

**否决理由**：
- WinDivert 需驱动签名（商业），用户态抓 packet 性能差（每 packet userspace round-trip）
- ClientHello parser 还要处理 TLS fragment / TCP segment 重组（HTTPS 通常 ALPN 协商后 fragment），实现复杂度高
- Schannel ETW 是 LSASS 已经解好的明文，零解析成本

### E. FiddlerCore / HTTP Toolkit MITM

**否决理由**：
- 商业 / 半商业（FiddlerCore 商业 license）
- 需注入 system proxy，破坏用户其它 HTTPS 应用
- proc 是观察工具，不能干扰系统网络栈

### F. EventLog Schannel source（GUID `{1f678132-...}`）

**否决理由**：
- EventLog source 写到 System Log 的 event IDs 36864-36888 是「错误 / 警告」级别（fatal alert / cert 问题），**正常 TLS handshake 不写 EventLog**
- ETW manifest-based provider 才有 informational-level events（含 SNI），需开 ETW real-time session

### G. ProcessFlow 加 ja4 字段一起扩

**否决理由（用户明确指示）**：
- 用户在 v0.10 启动指令包明确：「ja4 留 ebpf 那边」
- ja4 是 Linux eBPF 路径的范畴（v0.9 阶段 3 计划），与 Windows Schannel 无关
- surgical 原则：本周期只扩 sni，ja4 留待 eBPF 那边实现时再加

## Consequences

### 正面

- **跨平台对齐**：Windows 用户终于能看到 SNI（v0.7-v0.8 长期缺位），与 Linux eBPF 路径在 `ProcessFlow.sni` 字段统一
- **零新增依赖**：windows-rs 已是项目依赖，只加 `Win32_System_Diagnostics_Tdh` feature
- **TDH 路线稳健**：动态 schema 解析跨 Win10/Win11 版本兼容，不依赖未公开偏移
- **复用 disk_io_etw 模式**：StartTraceW + OpenTraceW + ProcessTrace + EventRecordCallback 4 个 API 与 ADR-0015 完全一致，模式成熟

### 负面

- **阶段 1 推测的 schema 三项全错**（阶段 2 实测修订）：provider GUID `{37D2C3CD-...}` 实测不 fire event、event ID 196 实测完全不出现、字段名 ServerName 应为 `TargetName`。阶段 2 实测拿到正确 schema 后实装走 TDH 路线不受影响（按 property name `TargetName` 查）
- **管理员权限**：非 admin 模式降级（sni 永远 None，UI 显示降级提示）
- **`Microsoft-Windows-Schannel-Events` provider 跨 Win 版本兼容**：阶段 2 在 Win11 26100 实测；Win10 1809 / 21H2 等版本若 event 1793 schema 不同（如新增字段 / 改字段名），TDH 按 property name 查仍能容错——但若 manifest 把 `TargetName` 改名则需要重新探测
- **Linux 用户 sni 仍空**：v0.9 eBPF uprobe 路径推迟，Linux 路径 sni 也填 None
- **TDH 性能开销**：每 event 调 `TdhGetEventInformation` 解 schema 比硬编码偏移慢（约 10-50μs/event），但 Schannel handshake 频率远低于 disk IO（每秒几十次 vs 数千次），可接受；同时实装加了 fast filter `event_id == 1793` 先过滤，避免对每个 Schannel event 都跑 TDH

### 缓解

- 阶段 4 review 时管理员 + logman/tracerpt 复跑 schema 探测（用本文档 §3 schema 表作 ground truth 对账），跨 Win 版本（Win10 22H2 / Win11 24H2 / 25H2）抽样验证
- 降级路径明确：catch + warn + fallback（sni = None，UI 显示「需管理员」提示）
- TDH 解析失败的 event drop 不影响其他（不污染 accum）
- `EnableTraceEx2` 用 `MatchAnyKeyword=0xFFFF_FFFF_FFFF_FFFF`（全 1）抓所有 keyword 的 Schannel events，避免漏 keyword=`0x8000_0000_0000_0000`（1793 keyword）

## Implementation Notes

- 入口：`src/schannel_etw/{mod.rs,provider.rs,parser.rs}`（阶段 2 实装）
  - `mod.rs`：跨平台 stub + `pub type SchannelEtwWorker = SnapshotWorker<Vec<SniRecord>>`
  - `provider.rs`（Windows cfg-gate）：`try_spawn_windows` 启动 ETW session + ProcessTrace 线程 + SnapshotWorker body；callback fast-filter `event_id == 1793` → TDH 解析 `TargetName` 字段
  - `parser.rs`：`SniRecord` 数据结构（跨平台）+ `read_utf16_le_until_null` 纯函数（单测覆盖）
- WorkerManager 集成：`src/workers/manager.rs::schannel_etw_worker: Option<SchannelEtwWorker>`（阶段 2 落地）+ `metrics_snapshot()` 追加 `schannel_etw` 行
- App 集成：`src/app.rs::App::overlay_flow_sni_schannel`（阶段 3，把 SniRecord merge 到 ProcessFlow.sni）
- ProcessFlow.sni 字段：阶段 1 已扩（`src/ebpf/flow.rs`）
- CLI / UI / Inspector 不动（接口不变；阶段 3 加 source 字段后才显式区分 Linux ebpf / Windows schannel）
- 实测验证脚本：管理员 PowerShell `logman create trace proc-schannel-probe -p {91CC1150-71AA-47E2-AE18-C96E61736B6F} 0xFFFF_FFFF_FFFF_FFFF 255 -ets` → curl https://example.com → `tracerpt -of XML` 解 ETL → 看 EventID 1793 + `TargetName` 字段

## References

- [StartTraceW - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew)
- [OpenTraceW - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-opentracew)
- [TdhGetEventInformation - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/tdh/nf-tdh-tdhgeteventinformation)
- [TRACE_EVENT_INFO (tdh.h) - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/tdh/ns-tdh-trace_event_info)
- [EVENT_PROPERTY_INFO (tdh.h) - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/tdh/ns-tdh-event_property_info)
- [TdhGetPropertySize - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/tdh/nf-tdh-tdhgetpropertysize)
- [Schannel Events - Win32 docs](https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2012-r2-and-2012/dn786445(v=ws.11))（EventLog source，不含 ETW event 1793 schema）
- [RFC 6066 TLS Extensions §3 Server Name Indication](https://datatracker.ietf.org/doc/html/rfc6066#section-3)
- 阶段 2 实测命令（管理员 + curl https://example.com 触发 1793 event）：
  `logman create trace proc-schannel-probe -p {91CC1150-71AA-47E2-AE18-C96E61736B6F} 0xFFFFFFFFFFFFFFFF 255 -ets`
  （`logman query providers | Select-String channel` 可枚举所有 Schannel 相关 provider GUID）
- proc v0.7 阶段 7 [ADR-0015](./0015-etw-per-process-disk-io.md)（disk_io_etw 手写 ETW 先例 + 模式复用；阶段 2 落地时发现 disk_io_etw 的 thread_local 模式有跨线程不传递的隐藏 bug，schannel_etw 改成在 ProcessTrace spawn 闭包内设置 thread_local）
- proc v0.7 阶段 8 [ADR-0016](./0016-ebpf-flow-graph.md)（ProcessFlow 数据结构 + eBPF flow graph 先例）
