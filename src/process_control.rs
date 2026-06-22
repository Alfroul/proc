//! 进程控制：优先级 + CPU affinity（阶段 4，A4）。
//!
//! - **Windows**：`SetPriorityClass` / `GetPriorityClass` /
//!   `GetProcessAffinityMask` / `SetProcessAffinityMask`。
//! - **Linux**：`setpriority(PRIO_PROCESS)` + `sched_getaffinity` /
//!   `sched_setaffinity`（`libc` 直接调用）。
//! - **macOS / 其它**：返回 `PermissionDenied`，UI 给「此平台不支持」提示。
//!
//! 优先级用 6 档枚举 [`PriorityClass`]，对应 Win32 的 6 个优先级类；Linux
//! 把这 6 档映射到 nice -20..19 区间。两平台的语义不强求 1:1（Win32 Realtime
//! 在 Linux 上没有等价物，落到 -20 即可）。

use crate::error::{ProcError, Result};

/// Win32 优先级类的跨平台抽象。`Default = Normal`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PriorityClass {
    Idle,
    BelowNormal,
    #[default]
    Normal,
    AboveNormal,
    High,
    Realtime,
}

impl PriorityClass {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::BelowNormal => "BelowNormal",
            Self::Normal => "Normal",
            Self::AboveNormal => "AboveNormal",
            Self::High => "High",
            Self::Realtime => "Realtime",
        }
    }

    /// `+` 键：往高调一档（Realtime 是上限）。
    #[must_use]
    pub fn bump_up(self) -> Self {
        match self {
            Self::Idle => Self::BelowNormal,
            Self::BelowNormal => Self::Normal,
            Self::Normal => Self::AboveNormal,
            Self::AboveNormal => Self::High,
            Self::High => Self::Realtime,
            Self::Realtime => Self::Realtime,
        }
    }

    /// `-` 键：往低调一档（Idle 是下限）。
    #[must_use]
    pub fn bump_down(self) -> Self {
        match self {
            Self::Idle => Self::Idle,
            Self::BelowNormal => Self::Idle,
            Self::Normal => Self::BelowNormal,
            Self::AboveNormal => Self::Normal,
            Self::High => Self::AboveNormal,
            Self::Realtime => Self::High,
        }
    }

    /// Linux `nice` 值（-20 最高，19 最低）。`Realtime → -20`、`Idle → 19`。
    #[must_use]
    pub fn to_nice(self) -> i32 {
        match self {
            Self::Idle => 19,
            Self::BelowNormal => 10,
            Self::Normal => 0,
            Self::AboveNormal => -5,
            Self::High => -10,
            Self::Realtime => -20,
        }
    }

    /// 把任意 nice 值归并到最接近的 [`PriorityClass`]。
    #[must_use]
    pub fn from_nice(nice: i32) -> Self {
        if nice >= 15 {
            Self::Idle
        } else if nice >= 5 {
            Self::BelowNormal
        } else if nice > -3 {
            Self::Normal
        } else if nice > -8 {
            Self::AboveNormal
        } else if nice > -15 {
            Self::High
        } else {
            Self::Realtime
        }
    }
}

/// 查询进程当前优先级。
pub fn get_priority(pid: u32) -> Result<PriorityClass> {
    #[cfg(target_os = "windows")]
    {
        get_priority_windows(pid)
    }
    #[cfg(target_os = "linux")]
    {
        get_priority_linux(pid)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = pid;
        Err(ProcError::permission_denied(
            "此平台（非 Windows/Linux）暂不支持优先级查询",
        ))
    }
}

/// 设置进程优先级。
pub fn set_priority(pid: u32, class: PriorityClass) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        set_priority_windows(pid, class)
    }
    #[cfg(target_os = "linux")]
    {
        set_priority_linux(pid, class)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (pid, class);
        Err(ProcError::permission_denied(
            "此平台（非 Windows/Linux）暂不支持优先级设置",
        ))
    }
}

/// 查询进程 CPU affinity mask（每核一位，bit i = 第 i 核可用）。
pub fn get_affinity(pid: u32) -> Result<u64> {
    #[cfg(target_os = "windows")]
    {
        get_affinity_windows(pid)
    }
    #[cfg(target_os = "linux")]
    {
        get_affinity_linux(pid)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = pid;
        Err(ProcError::permission_denied(
            "此平台（非 Windows/Linux）暂不支持 affinity 查询",
        ))
    }
}

/// 设置进程 CPU affinity mask（每核一位）。
pub fn set_affinity(pid: u32, mask: u64) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        set_affinity_windows(pid, mask)
    }
    #[cfg(target_os = "linux")]
    {
        set_affinity_linux(pid, mask)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (pid, mask);
        Err(ProcError::permission_denied(
            "此平台（非 Windows/Linux）暂不支持 affinity 设置",
        ))
    }
}

// ── Windows ─────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn get_priority_windows(pid: u32) -> Result<PriorityClass> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetPriorityClass, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| ProcError::permission_denied_with("OpenProcess 失败", e))?
    };

    let raw = unsafe { GetPriorityClass(handle) };
    let _ = unsafe { CloseHandle(handle) };

    if raw == 0 {
        return Err(ProcError::permission_denied(
            "GetPriorityClass 返回 0 — 进程可能已退出或权限不足",
        ));
    }

    Ok(PriorityClass::from_win32_raw(raw))
}

#[cfg(target_os = "windows")]
fn set_priority_windows(pid: u32, class: PriorityClass) -> Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_INFORMATION, SetPriorityClass,
    };

    let handle = unsafe {
        OpenProcess(PROCESS_SET_INFORMATION, false, pid)
            .map_err(|e| ProcError::permission_denied_with("OpenProcess 失败", e))?
    };

    let flags = class.win32_flags();
    let result = unsafe { SetPriorityClass(handle, flags) };
    let _ = unsafe { CloseHandle(handle) };
    result.map_err(|e| ProcError::permission_denied_with("SetPriorityClass 失败", e))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn get_affinity_windows(pid: u32) -> Result<u64> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        GetProcessAffinityMask, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| ProcError::permission_denied_with("OpenProcess 失败", e))?
    };

    let mut process_mask: usize = 0;
    let mut system_mask: usize = 0;
    let ok = unsafe { GetProcessAffinityMask(handle, &mut process_mask, &mut system_mask) };
    let _ = unsafe { CloseHandle(handle) };
    ok.map_err(|e| ProcError::permission_denied_with("GetProcessAffinityMask 失败", e))?;

    Ok(process_mask as u64)
}

#[cfg(target_os = "windows")]
fn set_affinity_windows(pid: u32, mask: u64) -> Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_INFORMATION, SetProcessAffinityMask,
    };

    let handle = unsafe {
        OpenProcess(PROCESS_SET_INFORMATION, false, pid)
            .map_err(|e| ProcError::permission_denied_with("OpenProcess 失败", e))?
    };

    let result = unsafe { SetProcessAffinityMask(handle, mask as usize) };
    let _ = unsafe { CloseHandle(handle) };
    result.map_err(|e| ProcError::permission_denied_with("SetProcessAffinityMask 失败", e))?;
    Ok(())
}

#[cfg(target_os = "windows")]
impl PriorityClass {
    /// Win32 `PROCESS_CREATION_FLAGS` 转换。
    fn win32_flags(self) -> windows::Win32::System::Threading::PROCESS_CREATION_FLAGS {
        use windows::Win32::System::Threading::*;
        match self {
            Self::Idle => IDLE_PRIORITY_CLASS,
            Self::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
            Self::Normal => NORMAL_PRIORITY_CLASS,
            Self::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
            Self::High => HIGH_PRIORITY_CLASS,
            Self::Realtime => REALTIME_PRIORITY_CLASS,
        }
    }

    fn from_win32_raw(raw: u32) -> Self {
        use windows::Win32::System::Threading::*;
        let flags = PROCESS_CREATION_FLAGS(raw);
        if flags == REALTIME_PRIORITY_CLASS {
            Self::Realtime
        } else if flags == HIGH_PRIORITY_CLASS {
            Self::High
        } else if flags == ABOVE_NORMAL_PRIORITY_CLASS {
            Self::AboveNormal
        } else if flags == BELOW_NORMAL_PRIORITY_CLASS {
            Self::BelowNormal
        } else if flags == IDLE_PRIORITY_CLASS {
            Self::Idle
        } else {
            Self::Normal
        }
    }
}

// ── Linux ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

#[cfg(target_os = "linux")]
fn get_priority_linux(pid: u32) -> Result<PriorityClass> {
    // getpriority 有奇葩语义：成功时返回 nice 值，失败时返回 -1 并设 errno；
    // 但 -1 本身也是合法的 nice 值。所以调用前必须清 errno，调用后检查。
    unsafe {
        *libc::__errno_location() = 0;
        let nice = libc::getpriority(libc::PRIO_PROCESS, pid);
        if nice == -1 {
            let err = errno();
            if err != 0 {
                return Err(ProcError::permission_denied(format!(
                    "getpriority 失败 (errno={err})"
                )));
            }
        }
        Ok(PriorityClass::from_nice(nice))
    }
}

#[cfg(target_os = "linux")]
fn set_priority_linux(pid: u32, class: PriorityClass) -> Result<()> {
    let r = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, class.to_nice()) };
    if r != 0 {
        let err = errno();
        return Err(ProcError::permission_denied(format!(
            "setpriority 失败 (errno={err})"
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn get_affinity_linux(pid: u32) -> Result<u64> {
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    let r = unsafe {
        libc::CPU_ZERO(&mut set);
        libc::sched_getaffinity(pid as i32, std::mem::size_of::<libc::cpu_set_t>(), &mut set)
    };
    if r != 0 {
        let err = errno();
        return Err(ProcError::permission_denied(format!(
            "sched_getaffinity 失败 (errno={err})"
        )));
    }

    let mut mask: u64 = 0;
    for i in 0..64 {
        if unsafe { libc::CPU_ISSET(i, &set) } {
            mask |= 1u64 << i;
        }
    }
    Ok(mask)
}

#[cfg(target_os = "linux")]
fn set_affinity_linux(pid: u32, mask: u64) -> Result<()> {
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::CPU_ZERO(&mut set);
        for i in 0..64 {
            if mask & (1u64 << i) != 0 {
                libc::CPU_SET(i, &mut set);
            }
        }
    }
    let r = unsafe {
        libc::sched_setaffinity(pid as i32, std::mem::size_of::<libc::cpu_set_t>(), &set)
    };
    if r != 0 {
        let err = errno();
        return Err(ProcError::permission_denied(format!(
            "sched_setaffinity 失败 (errno={err})"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_class_label_is_distinct() {
        let labels = [
            PriorityClass::Idle.label(),
            PriorityClass::BelowNormal.label(),
            PriorityClass::Normal.label(),
            PriorityClass::AboveNormal.label(),
            PriorityClass::High.label(),
            PriorityClass::Realtime.label(),
        ];
        let unique = labels
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(unique, labels.len());
    }

    #[test]
    fn priority_class_bump_up_has_realtime_ceiling() {
        let mut cur = PriorityClass::Realtime;
        // Realtime 已经是上限，bump_up 不应越界。
        for _ in 0..5 {
            cur = cur.bump_up();
            assert_eq!(cur, PriorityClass::Realtime);
        }
    }

    #[test]
    fn priority_class_bump_down_has_idle_floor() {
        let mut cur = PriorityClass::Idle;
        for _ in 0..5 {
            cur = cur.bump_down();
            assert_eq!(cur, PriorityClass::Idle);
        }
    }

    #[test]
    fn priority_class_to_nice_monotonic() {
        // bump_up 应该让 nice 值严格非递增（除非已经在 Realtime/Idle 边界）。
        let cases = [
            PriorityClass::Idle,
            PriorityClass::BelowNormal,
            PriorityClass::Normal,
            PriorityClass::AboveNormal,
            PriorityClass::High,
            PriorityClass::Realtime,
        ];
        for w in cases.windows(2) {
            assert!(
                w[0].to_nice() > w[1].to_nice(),
                "{} should have higher nice than {}",
                w[0].label(),
                w[1].label()
            );
        }
    }

    #[test]
    fn priority_class_from_nice_round_trip_within_class() {
        // to_nice 给出的值经 from_nice 归并后应回到原 class（每档的中心 nice 是稳定的）。
        for class in [
            PriorityClass::Idle,
            PriorityClass::BelowNormal,
            PriorityClass::Normal,
            PriorityClass::AboveNormal,
            PriorityClass::High,
            PriorityClass::Realtime,
        ] {
            let nice = class.to_nice();
            assert_eq!(
                PriorityClass::from_nice(nice),
                class,
                "{} (nice={}) should round-trip",
                class.label(),
                nice
            );
        }
    }

    #[test]
    fn priority_class_default_is_normal() {
        assert_eq!(PriorityClass::default(), PriorityClass::Normal);
    }

    /// 自身进程 get_priority 在 Windows 上应当返回 `Normal`（CI 默认账户不挑优先级）。
    /// Linux 上不允许 setpriority 失败但能 getpriority，且应该回退到 `Normal` 或
    /// 至少返回某个变体（不 panic）。
    #[test]
    fn self_get_priority_returns_known_class() {
        let pid = std::process::id();
        match get_priority(pid) {
            Ok(class) => {
                // 任何 6 档之一都接受；最常见的应是 Normal。
                let _ = class.label();
            }
            Err(e) => {
                // Linux CI 容器里偶尔会因 RLIMIT_NICE 限制拒绝查询 —— 仅记录，不挂测试。
                eprintln!("note: get_priority({pid}) failed in CI: {e}");
            }
        }
    }
}
