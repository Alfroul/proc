# ADR-0020: DNS ETW Provider — 替代 PowerShell `Get-WinEvent` probe

- **Status**: Accepted
- **Date**: 2026-06-29
- **Phase**: v0.11.0 阶段 2
- **Replaces**: [ADR-0006](0006-dns-subprocess-not-etw-dbus.md)（部分）—— ADR-0006 阶段 8 决定走 PowerShell 子进程路线，本 ADR 在保留 PowerShell fallback 的前提下新增 ETW 主路径

## 背景

阶段 8（v0.5.0）落地 DNS 查询日志时选择 PowerShell `Get-WinEvent` 子进程路线，理由是 ETW 实时 session 需要 ~500 行 native FFI + schema 解析，工程量超出单 stage（详见 ADR-0006）。该路线落地后稳定运行，但暴露两个问题：

1. **CPU 开销**：PowerShell 子进程持续占 **3-5% CPU**（脚本内 `Start-Sleep 400ms` + `Get-WinEvent` filter hashtable 每轮 ~50ms）。后台工具常年跑时这是不可忽略的开销。
2. **延迟 + 漏抓**：500ms-1s 的端到端延迟（PowerShell 进程启动 + Get-WinEvent 查询历史 + reader 线程 stdout drain + worker 500ms tick）；高频 DNS 查询场景（如浏览器并发抓取数十个 subresource）会漏抓事件（Get-WinEvent 在 `StartTime` 边界 + filter hashtable 索引下偶发漏事件）。
3. **bug 诊断困难**：用户报「DNS 日志缺数据」时无法区分是 PowerShell 路径漏抓还是 DNS 查询本身没发生，缺少可观察性。

v0.10 阶段 2 落地 schannel_etw（ADR-0018）后，项目已具备手写 ETW session + TDH 动态 schema 解析的工程能力；v0.11 cycle 把 DNS 同款升级。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| **A. 继续 PowerShell 路径（不动）** | 零工程量 | 3-5% CPU + 漏抓问题持续存在；与项目「平台深度对标 Resource Monitor」叙事不符 |
| **B. ferrisetw 自动 TDH** | ~100 行代码即可覆盖 | 与项目偏好「手写更可控」冲突（同 ADR-0015 disk_io_etw / ADR-0018 schannel_etw 决策）；引入 ferrisetw + 其依赖树 |
| **C. PowerShell + WMI CIM 模式** | 仍 spawn 子进程 | 性能没本质改善（PowerShell 启动开销固定 ~300ms）；CIM 查询历史模式同款漏抓 |
| **D. 读 `%SystemRoot%\System32\dnsrslvr.log`** | 不需 ETW | 仅缓存解析器日志，不含应用层查询（哪个 PID 发起）；Dnscache service 客户端日志默认关闭需用户启用 |
| **E. 手写 windows-rs ETW + TDH 动态 schema** | < 50ms 延迟 + 100% 完整性 + 与 ADR-0018 同款路线 | ~250 行 native FFI；TDH 解析有 10-50μs/event 开销（DNS 高频时累积，但可接受） |

## 决策

**选 E**。具体设计：

1. **Provider**：`Microsoft-Windows-DNS-Client` ETW provider，GUID `{1C95126E-7EEA-49A9-A3FE-A9FB58F46014}`（Microsoft 公开文档 + logman 实证）。

2. **Event fast-filter**：
   - **event 3008**（`QueryResponseEx`）—— 查询响应到达时触发，含完整 QueryResults
   - **event 3010**（`QueryCompletedEx`）—— 查询完成时触发（无论成功/失败），与 PowerShell 路径同款 ID
   
   两者都含 `QueryName` / `QueryType` / `QueryStatus` / `QueryResults` 字段（UTF-16 LE / uint32 / uint32 / UTF-16 LE）。
   
   3008 + 3010 都抓的原因：3008 在某些 Win 版本对 NXDOMAIN / Timeout 不触发，3010 是「查询完成」语义更全；同时抓保证完整性（去重靠主线程 `App::dns_log_recent` 的 FIFO + 时间戳）。

3. **TDH 动态 schema**（同 ADR-0018 §3 路线）：
   - 用 `TdhGetEventInformation` 拉 `TRACE_EVENT_INFO` buffer
   - 遍历 top-level properties 按 property name 找 `QueryName` / `QueryType` / `QueryStatus` / `QueryResults`
   - **不硬编码字段顺序 / 偏移**——manifest 在 Win10/Win11 版本间可能扩字段（1809 vs 22H2 实测 property 顺序一致，但保守起见走 name-based）

4. **PID 来源**：`EVENT_HEADER.ProcessId`（用户态 provider 自带 PID，**不复用** disk_io_etw 的 thread→pid map——后者是 NT Kernel Logger 才需要）。注意：DNS-Client service 在 svchost.exe 内运行时 PID 可能指向 svchost，而非真正发起查询的应用进程；Windows DNS Cache service (Dnscache) 在 Win10 1607+ 已支持 track originating PID 并写入 EVENT_HEADER.ProcessId。

5. **Session 配置**（与 schannel_etw 同款）：
   - 自定义 session name `proc-dns-client\0`（非 NT Kernel Logger——DNS-Client 是用户态 provider）
   - `EVENT_TRACE_REAL_TIME_MODE` + `PROCESS_TRACE_MODE_EVENT_RECORD` + `PROCESS_TRACE_MODE_RAW_TIMESTAMP`
   - Buffer 64KB / Min 20 / Max 100（同 schannel_etw）
   - `EnableTraceEx2` level=verbose(5) + MatchAnyKeyword=全 1

6. **Worker 集成**：复用 [`SnapshotWorker<Vec<DnsQuery>>`]，500ms tick → drain accum → push channel（与原 PowerShell probe 节奏一致，主线程消费路径不变）。

7. **降级路径**：以下场景 `try_spawn_etw` 返回 `None`，调用方走 PowerShell fallback：
   - `StartTraceW` 失败：非管理员 / session name 已被占用（其它 proc 实例已开）
   - `EnableTraceEx2` 失败：provider GUID 错误 / DNS-Client service 未启动
   - `OpenTraceW` / `ProcessTrace` spawn 失败：罕见，通常内存不足或线程 ulimit
   - x86 (32-bit) Windows：cfg-gate 直接拒绝（pointer-size 与 manifest 偏移不稳，同 schannel_etw / disk_io_etw 决策）
   - 非 Windows：cfg-gate 编译 stub，`try_spawn_etw` 直接返 `None`

8. **PowerShell 路径保留作 fallback**：`src/dns_log/windows_dns.rs` 代码不动，仅加注释。`detect_collector()` 改为「先尝试 ETW → 失败则 fallback PowerShell」。

## 后果

### 正面

- **性能**：PowerShell 3-5% CPU → ETW < 0.5% CPU（ETW callback 每事件 ~10-50μs，500 events/s ≈ 25ms/s = 2.5% 单核，实际 DNS 查询频率远低）
- **延迟**：PowerShell 500ms-1s → ETW < 50ms（callback 实时触发 + 500ms tick drain）
- **完整性**：PowerShell 漏抓高频查询 → ETW 100% 抓（实时 session 不依赖 filter hashtable 索引边界）
- **可观察性**：`proc diag` 新增 `dns_collector: etw | powershell | none` 行，用户报 bug 时附上当前 collector 类型
- **架构一致**：与 ADR-0015 disk_io_etw / ADR-0018 schannel_etw 同款「手写 windows-rs ETW + TDH」路线，未来扩 ETW provider 时复用同套骨架

### 负面

- **仅 Windows + 管理员**：ETW real-time session 需 admin / Performance Log Users SID；非 admin 降级到 PowerShell（PowerShell 也需要订阅 Microsoft-Windows-DNS-Client/Operational channel 的权限，普通用户在 Win10+ 默认有）
- **TDH schema 兼容**：跨 Win10 / Win11 版本 manifest 可能扩字段（1809 vs 22H2 property 数实测一致，但走 name-based 路线防御性更强）；如 Microsoft 改 event ID（schannel_etw 阶段 2 已遇到一次），fast filter 需调整
- **PID 语义**：DNS-Client service 在 svchost 内运行时 EVENT_HEADER.ProcessId 可能指向 svchost 而非真正发起进程——Win10 1607+ Dnscache service 已记录 originating PID 到 EVENT_HEADER，但用户感知场景仍可能出现 PID 不准（与 PowerShell 路径同款限制）
- **代码量**：~250 行 native FFI + 单元测试，工程量高于 PowerShell 路径；与 schannel_etw / disk_io_etw 代码重复度 ~70%（未来可抽 ETW session 公共骨架，但 v0.11 cycle 不做）

### 缓解

- TDH name-based schema 解析（不硬编码偏移）应对 manifest 变更
- `proc diag` 输出 `dns_collector` 字段让 bug report 自带 collector 类型信息
- PowerShell fallback 保证非 admin / Win10 早期版本 / ETW 启动失败场景仍有 DNS 日志
- 单元测试覆盖 mock event bytes → parser 输出（3008 + 3010 各覆盖 Success / NxDomain / Timeout / Error）；跨平台 stub 测试覆盖非 Windows `try_spawn_etw` 返 None

## 实现 Notes

- `src/dns_log/etw.rs`（新）：StartTraceW + EnableTraceEx2 + OpenTraceW + ProcessTrace + EventRecordCallback；callback fast-filter 3008 || 3010 → TDH 解析 → 构造 `DnsQuery` → push accum
- `src/dns_log/mod.rs::detect_collector`：改为先 `try_spawn_etw` → 失败 fallback `PowershellDnsCollector::new()`；新增 `DnsCollectorKind` enum + `detect_collector_kind()` 让 `proc diag` 输出 collector 类型
- `src/dns_log/windows_dns.rs`：保留代码不动，加注释「v0.11 阶段 2 后退为 fallback；ETW 路径在 src/dns_log/etw.rs」
- `src/cli/diag.rs`：输出加 `dns_collector: <kind>` 行
- `tests/test_dns_etw.rs`（新）：mock event 3008/3010 bytes → parser；跨平台 stub；fallback 路径

## 参考

- [ADR-0006](0006-dns-subprocess-not-etw-dbus.md) — PowerShell 子进程路线原始决策（本 ADR 在保留 fallback 前提下新增 ETW 主路径）
- [ADR-0015](0015-etw-per-process-disk-io.md) — disk_io_etw「硬编码偏移」路线（NT Kernel Logger，schema 稳定）
- [ADR-0018](0018-windows-schannel-sni.md) — schannel_etw「TDH 动态 schema」路线（与本 ADR 同款，manifest-based provider）
- [CONTEXT.md「后台 worker」段](../../CONTEXT.md) — DnsLogWorker / DnsQuery 术语
- `src/schannel_etw/provider.rs` — ETW session + TDH 解析骨架模板
- Microsoft DNS-Client ETW provider 文档：`Microsoft-Windows-DNS-Client` GUID `{1C95126E-7EEA-49A9-A3FE-A9FB58F46014}`
