# ADR-0015: ETW per-process disk IO via 手写 windows-rs NT Kernel Logger

## Status

**Accepted** — v0.7.0 阶段 7 引入。决策从「ferrisetw」改为「手写 windows-rs ETW」。

## Context

v0.6 proc 有 per-process 磁盘 IO 字段（`ProcessInfo::disk_read_speed / disk_write_speed`），但 Windows 实现走 sysinfo 的 `Process::disk_usage()`，sysinfo 在 Windows 又走 `IO_*.ProcessId` 性能计数器。这套方案有几个问题：

1. **精度低**：性能计数器按 1s 采样间隔平均，无法捕捉瞬时 IO burst
2. **粒度只到 bytes/采样周期**：不能区分一个进程的 read vs write 时间分布
3. **非管理员权限下数据缺失**：性能计数器需要 PERFORMANCE_MONITORING 用户组

Resource Monitor（Windows 内置）走 ETW（Event Tracing for Windows），精度高、零开销、按 PID 准确。System Informer 的 `etwmon.c` 同款方案。

proc v0.6 已有 per-process 网络流量（走 IP Helper 不走 ETW，因为 ETW kernel network 在 Win8+ 没有给 PID 字段）。但磁盘 IO 走 ETW 是更优解。

## Decision

**手写 windows-rs ETW（`Win32_System_Diagnostics_Etw`）开 NT Kernel Logger session 监听 `EVENT_TRACE_FLAG_DISK_IO` + DiskIo TypeGroup1 事件，按 PID 聚合得到 per-process R/W BPS，覆盖 sysinfo 数据源。Windows only cfg-gate + 管理员权限 + 失败降级。**

具体决策（决策从原计划「ferrisetw」改为「手写」）：

1. **不引 ferrisetw，手写 windows-rs ETW**：
   - 项目偏好「更可控」（用户在阶段 7 启动指令包明确指出）
   - windows-rs 已是项目依赖（v0.6 +），只加 `Win32_System_Diagnostics_Etw` feature
   - NT Kernel Logger API 简单：`StartTraceW` + `OpenTraceW` + `ProcessTrace` ~250 行
   - ferrisetw fallback 写入 ADR 但未启用——若 schema 解析变脆可切换

2. **NT Kernel Logger（固定 session name + GUID）**：
   - session name = "NT Kernel Logger"，GUID = `{9e814aad-3204-11d2-9a82-006008a869e3}`
   - API 简单（自 Win7 起稳定，所有 Windows 一致）
   - 劣势：单实例（同时只能一个进程用）。资源监视器 / 另一个 proc 实例占用 → 降级

3. **DiskIo_TypeGroup1 schema 硬编码偏移**（x64 only）：
   ```text
   offset  type   field
   0       u32    TransferSize
   4       u32    DiskNumber
   8       u64    Irp           (pointer-sized)
   16      u64    FileObject    (pointer-sized)
   24      u64    HighResResponseTime
   32      u32    IssuingThreadId
   ```
   schema 自 Win8 起稳定。32-bit Windows 拒绝（pointer size 不同），Win11 ARM64 应可工作（pointer 8-byte）但未测——Windows 11 仅 x64 渠道发布，实际可忽略

4. **PID 映射策略**：
   - DiskIo 事件本身**不含 PID**，只有 `IssuingThreadId`（在 UserData 偏移 32）
   - 用 `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)` 全量枚举线程 → 维护 `thread_id → pid` map
   - **不用 sysinfo `Process::tasks()`**：sysinfo 0.34.2 在 Windows 上实测 `tasks()` 经常返回 None，初始化时机不稳定
   - 5s 全量刷新（含同步预填，避免 callback 第一次拉到空 map）

5. **read/write 区分**：用 `EVENT_HEADER.EventDescriptor.Opcode` 字段
   - opcode 0x02 = read completion
   - opcode 0x03 = write completion
   - 其它 opcode 一律按 read 计（保守）

6. **覆盖 sysinfo 数据源（不新增列）**：
   - v0.6 `ProcessInfo::disk_read_speed / disk_write_speed` 字段保留
   - `App::update_disk_speeds` 先跑 sysinfo delta（fallback）
   - 新增 `App::overlay_disk_speeds_etw` 用 ETW 数据**覆盖** sysinfo 值（ETW 更准）
   - ETW 缺失的 PID（thread_map 没刷到）保留 sysinfo 值
   - UI / CLI / Inspector 全部不动（接口不变）

7. **降级路径**：
   - Linux / macOS：worker 是 None，ProcessInfo 沿用 sysinfo
   - Windows 非管理员：NT Kernel Logger 需要 admin，`StartTraceW` 失败 → warn 日志 + 降级 sysinfo
   - NT Kernel Logger 被占用：同上降级
   - x86 (32-bit) Windows：直接拒绝（cfg-gate），降级 sysinfo
   - ETW schema 长度 < 36 bytes：drop 该事件，不影响其他

8. **WorkerManager 集成**：
   - 新增 `disk_io_etw_worker: Option<DiskIoEtwWorker>`（Windows only）
   - 复用 `SnapshotWorker<DiskIoMap>` 模板，与 v0.6 NetFlowWorker 同款
   - `proc diag` 加 `disk_io_etw` worker 行

## Alternatives Considered

### A. 沿用 sysinfo（不升级）

**否决理由**：
- 精度低（性能计数器 1s 平均）
- 非管理员数据缺失
- v0.6 已是 sysinfo 上限，无法再优化

### B. 用 ferrisetw（v0.7 阶段 7 早期计划）

**否决理由（落选）**：
- 用户在阶段 7 启动指令包明确：「ferrisetw 不引依赖直接用 windows-rs ETW API...更可控」
- ferrisetw 是 KrabsETW Rust 移植，~500KB + 间接依赖；手写 ~250 行 windows-rs 已是项目依赖
- 手写 NT Kernel Logger API 在 sysinfo 风格项目中已有先例（throttle.rs / collect.rs 都直用 windows-rs）
- **保留为 fallback**：若 schema 解析变脆可切换；ADR-0015 文档已留接口签名

### C. 自写 ETW + TdhGetEventInformation（不用 ferrisetw，但动态 schema）

**否决理由**：
- `TdhGetEventInformation` + property offset 解析 ~500 行 unsafe FFI
- 仅适用于 schema 频繁变化的 provider；DiskIo_TypeGroup1 自 Win8 起稳定，硬编码偏移够用

### D. 用 Event Traces for Windows API + 用户态 provider

**否决理由**：
- 用户态 provider 不提供 disk IO（disk IO 是内核 provider）
- 必须用 KernelTrace

### E. ETW + sysinfo thread_map（不用 ToolHelp）

**否决理由**：
- sysinfo 0.34.2 在 Windows 上 `Process::tasks()` 实测不稳定（经常返回 None）
- ToolHelp `TH32CS_SNAPTHREAD` 是项目已有模式（src/collect.rs 用同款枚举进程）
- 5s 全量刷新够用

### F. 新增列（不覆盖 sysinfo）

**否决理由**：
- 用户不需要看"sysinfo 数据"和"ETW 数据"两列对比
- 接口越简单越好（surgical）

## Consequences

### 正面

- **精度提升**：管理员下 ETW 是 ground truth
- **零新增 UI/接口**：复用 v0.6 列，用户感知是"数据更准了"
- **零新增依赖**：windows-rs 已是项目依赖，只加一个 feature flag
- **WorkerManager 扩展**：与 v0.6 NetFlowWorker 同款模式，运维一致
- **`proc diag` 完整**：worker metrics 含 disk_io_etw 行（v0.7 阶段 1 TD-5 同款）

### 负面

- **schema 硬编码**：DiskIo_TypeGroup1 在未来 Windows 版本扩展字段时，需要测试验证。当前偏移覆盖 Win8 - Win11 x64
- **管理员权限**：非 admin 模式降级到 sysinfo（精度回落到 v0.6）
- **NT Kernel Logger 单实例**：资源监视器 / 另一个 proc 实例占用时降级
- **Linux 用户没此能力**（cfg-gate 降级）
- **thread→pid map 5s 刷新**：极少数情况下短命线程的 IO 可能丢失（5s 内起止）

### 缓解

- 跨版本测试：Win10 / Win11 都跑 spawn_collects_self_io_when_admin 测试
- 降级路径明确：catch + warn + fallback sysinfo
- 5s 全量刷新 + ETW 事件 PID 找不到时丢弃（不污染 accum）

## Implementation Notes

- 入口：`src/disk_io_etw/{mod.rs,provider.rs,thread_map.rs}`
- WorkerManager 集成：`src/workers/manager.rs::disk_io_etw_worker`
- App 集成：`src/app.rs::App::overlay_disk_speeds_etw`（在 `update_disk_speeds` 之后调用，覆盖 sysinfo delta）
- CLI / UI / Inspector 不动（接口不变）
- 测试：`tests/test_disk_io_etw.rs`（Windows cfg-gate；非管理员走 SKIP 路径不 fail）

## References

- [StartTraceW - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-starttracew)
- [OpenTraceW - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-opentracew)
- [ProcessTrace - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/evntrace/nf-evntrace-processtrace)
- [EVENT_TRACE_FLAG_DISK_IO - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/etw/event-tracing-mof)
- [DiskIo_TypeGroup1 - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/etw/diskio-typelongtype1)
- [System Informer etwmon.c 参考](https://github.com/SystemInformer/SystemInformer)
- proc v0.6.0 `src/estats.rs`（NetFlowWorker 同款模式参考）
- proc v0.6.0 `src/workers/manager.rs::NetFlowWorker`（worker 集成参考）
- proc v0.6.0 `src/throttle.rs`（手写 windows-rs 先例）
- proc v0.6.0 `src/collect.rs::collect_missing_processes`（ToolHelp snapshot 同款模式）
