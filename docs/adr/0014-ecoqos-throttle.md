# ADR-0014: Windows 11 EcoQoS / Efficiency Mode via SetProcessInformation

## Status

**Accepted** — v0.7.0 阶段 6 引入

## Context

Windows 11 引入 EcoQoS（也称 Efficiency Mode），让进程显式标记为"低优先级后台工作"，系统会：

- 选择最低 CPU 频率
- 调度到效率核（P-core / E-core 异构 CPU 上）
- 降低功耗 / 散热 / 风扇噪音

Win11 25H2 起系统会更激进地自动 throttle 后台进程（任务管理器有"绿叶"图标）。用户被自动 throttle 了都不知道，看到性能异常时找不到原因。

proc 已有 `proc priority` / `proc affinity` 子命令，但缺 EcoQoS 切换。Mission Center / Win11 任务管理器都有显示。

## Decision

**用 `windows-rs 0.57` 的 `SetProcessInformation(ProcessPowerThrottling)` + `PROCESS_POWER_THROTTLING_STATE` 直接切换，不引 `win32-ecoqos` crate。Windows only cfg-gate。**

具体决策：

1. **库选 windows-rs**（不引 win32-ecoqos）：
   - windows-rs 已是 proc 依赖
   - EcoQoS FFI 只有 3 行（OpenProcess + SetProcessInformation + CloseHandle）
   - win32-ecoqos 是个 100 行的薄封装，引依赖不如直接写

2. **set + query 双向支持**：
   - **set**：用户主动 `proc throttle <pid> on|off`
   - **query**：进程列表显示当前 throttled 状态（包括系统自动 throttle 的）

3. **query 走 NtQueryInformationProcess**（undocumented 但 System Informer 在用）：
   - 用 `PROCESS_POWER_THROTTLING_STATE { Version, ControlMask=0, StateMask=0 }` 调 `NtQueryInformationProcess(ProcessPowerThrottling)`
   - 返回的 StateMask 含 `PROCESS_POWER_THROTTLING_EXECUTION_SPEED` → 当前是 Eco
   - 替代方案：Win32 `SetProcessInformation` 用 `PROCESS_POWER_THROTTLING_CURRENT_VERSION` 也能 query，但语义不直观

4. **CLI 子命令 `proc throttle <pid> on|off`**：
   - 与 v0.6 `proc priority` / `proc affinity` 同款风格
   - on → 启用 EcoQoS（StateMask = EXECUTION_SPEED）
   - off → 禁用 EcoQoS（StateMask = 0）

5. **进程列表 🍃 标记**：
   - `ProcessInfo` 加 `throttled: EcoQoSState` 字段（cfg-gate）
   - HeavyWorker 1.5s 周期内批量 query 当前 PID 的 throttle 状态
   - UI：name 列后追加 🍃 emoji（如果 throttled == Eco）
   - 与 Mission Center / Win11 TM 视觉一致

6. **Inspector Summary Tab 显示**：
   - 详情页概要 Tab 加 "EcoQoS: Normal / Eco" 行
   - 详情页内 `T` 键切换（不需退出详情页去 CLI）

7. **Windows only cfg-gate**：
   ```rust
   #[cfg(windows)]
   pub fn set_throttle(pid: u32, eco: bool) -> anyhow::Result<()> { /* Win32 FFI */ }

   #[cfg(not(windows))]
   pub fn set_throttle(_pid: u32, _eco: bool) -> anyhow::Result<()> {
       Err(anyhow::anyhow!("EcoQoS only supported on Windows 11+"))
   }
   ```

8. **Windows 11 vs Windows 10 兼容性**：
   - EcoQoS 是 Win11 21H2 (build 22000) 引入
   - Win10 上 `SetProcessInformation(ProcessPowerThrottling)` 也能调用，但效果是 LowQoS（退化版）
   - 不做版本检测，让 Win32 API 自己处理；用户在 Win10 上调用不报错但效果弱

## Alternatives Considered

### A. 引 `win32-ecoqos` crate

**否决理由**：
- crate 总共 100 行薄封装，引依赖不划算
- crate 维护活跃度低（最近 commit 2 年前）
- 直接用 windows-rs FFI 3 行解决

### B. 不做 query，只做 set

**否决理由**：
- Win11 25H2 自动 throttle 后台进程，用户不知道
- 没有 query 就不知道哪些进程被自动 throttle 了
- query 是关键差异化（v0.6 没有这个能力）

### C. 用 PowerShell `Get-Process | throttling` 间接实现

**否决理由**：
- PowerShell 没有 throttle cmdlet
- 走 WMI 也无对应字段
- 必须 FFI 直调

### D. 跨平台抽象（Linux cgroup v2 freezer / `nice -n 19`）

**否决理由**：
- Linux 的 nice / cgroup 语义与 EcoQoS 不同（nice 是优先级 / cgroup 是隔离 / EcoQoS 是 power throttling）
- 跨平台抽象会让用户误以为语义一致
- 第一版 Windows only，Linux 等价物留 v0.8.0+ 评估（如 systemd transient scope + cpu.weight）

### E. 不做（让用户用 Win11 TM）

**否决理由**：
- proc 主打"任务管理器替代"，不能缺关键能力
- 用户在 proc 内查 + 改 throttle，不需要切到 TM

## Consequences

### 正面

- **Win11 用户痛点解决**：能看到哪些进程被自动 throttle
- **CLI 一键切换**：`proc throttle <pid> on` 比 TM 右键菜单快
- **进程列表视觉提示**：🍃 一眼可见
- **复用 v0.6 priority/affinity 子命令模式**

### 负面

- **Windows only**：Linux/macOS 用户没此功能（降级提示）
- **Win11 build 差异**：旧 Win11 build 22000 之前不支持（用户报错）
- **NtQueryInformationProcess 是 undocumented**：未来 Win32 API 变更可能破坏 query 路径
- **进程列表每 PID 一次 OpenProcess**：500 进程下 500 次 syscall，需缓存

### 缓解

- HeavyWorker 1.5s 周期内批量 query，不在每帧 OpenProcess
- query 失败（如权限不足）→ EcoQoSState::Unknown，UI 显示 `-` 而非 panic
- NtQueryInformationProcess fallback：如果 query 失败，用 set 时的 state 推断（known sets 自己跟踪）

## Implementation Notes

- 入口：`src/throttle.rs::{set_throttle, query_throttle, EcoQoSState}`
- CLI：`src/cli/throttle.rs::run_throttle`
- UI 标记：`src/tui/process_table.rs` name 列后加 🍃
- Inspector：`src/tui/detail_view.rs` Summary Tab 加 EcoQoS 行 + `T` 键
- 测试：`tests/test_throttle.rs`（Windows cfg-gate，含 set+query 往返）

## Code Sketch

```rust
#[cfg(windows)]
pub fn set_throttle(pid: u32, eco: bool) -> anyhow::Result<()> {
    use windows::Win32::System::Threading::*;
    use windows::Win32::Foundation::CloseHandle;

    unsafe {
        let h = OpenProcess(
            PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )?;

        let mut state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: if eco { PROCESS_POWER_THROTTLING_EXECUTION_SPEED } else { 0 },
        };

        SetProcessInformation(
            h,
            ProcessPowerThrottling,
            &mut state as *mut _ as *mut _,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )?;

        let _ = CloseHandle(h);
    }
    Ok(())
}
```

## References

- [SetProcessInformation - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation)
- [Quality of Service - Win32 docs](https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service)
- [PROCESS_POWER_THROTTLING_STATE - windows-sys](https://docs.rs/windows-sys/latest/windows_sys/Win32/System/Threading/struct.PROCESS_POWER_THROTTLING_STATE.html)
- [win32-ecoqos crate](https://docs.rs/win32-ecoqos)（参考否定）
- [System Informer（NtQueryInformationProcess 参考）](https://github.com/SystemInformer/SystemInformer)
- proc v0.6.0 `src/cli/priority.rs`（同款 subcommand 模式参考）
- proc v0.6.0 `src/cli/affinity.rs`（同款 subcommand 模式参考）
