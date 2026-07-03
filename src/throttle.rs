use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThrottleInfo {
    pub max_mhz: u32,
    pub current_mhz: u32,
    pub mhz_limit: u32,
    pub is_throttled: bool,
    pub throttle_pct: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThrottleReason {
    None,
    Thermal,
    PowerPolicy,
    Idle,
    Unknown,
}

#[cfg(target_os = "windows")]
pub fn query_processor_power_info()
-> Option<Vec<windows::Win32::System::Power::PROCESSOR_POWER_INFORMATION>> {
    use windows::Win32::System::Power::{
        CallNtPowerInformation, POWER_INFORMATION_LEVEL, PROCESSOR_POWER_INFORMATION,
    };

    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let buf_size = std::mem::size_of::<PROCESSOR_POWER_INFORMATION>() * num_cpus;
    let mut buffer: Vec<PROCESSOR_POWER_INFORMATION> =
        vec![unsafe { std::mem::zeroed() }; num_cpus];

    let status = unsafe {
        CallNtPowerInformation(
            POWER_INFORMATION_LEVEL(11), // ProcessorInformation
            None,
            0,
            Some(buffer.as_mut_ptr() as *mut _),
            buf_size as u32,
        )
    };

    if status.is_ok() {
        Some(buffer)
    } else {
        tracing::debug!("CallNtPowerInformation failed: {:?}", status);
        None
    }
}

/// Compute throttle info from raw per-core data (max_mhz, current_mhz, mhz_limit).
#[must_use]
pub fn detect_throttle_from_raw(cores: &[(u32, u32, u32)]) -> Option<ThrottleInfo> {
    if cores.is_empty() {
        return None;
    }

    let max_mhz = cores.iter().map(|c| c.0).max().unwrap_or(0);
    let current_mhz: u32 = {
        let sum: u32 = cores.iter().map(|c| c.1).sum();
        sum / cores.len() as u32
    };
    let mhz_limit = cores.iter().map(|c| c.2).min().unwrap_or(0);

    let is_throttled = mhz_limit > 0 && mhz_limit < max_mhz;
    let throttle_pct = if is_throttled && max_mhz > 0 {
        (1.0 - mhz_limit as f32 / max_mhz as f32) * 100.0
    } else {
        0.0
    };

    Some(ThrottleInfo {
        max_mhz,
        current_mhz,
        mhz_limit,
        is_throttled,
        throttle_pct,
    })
}

pub fn detect_throttle(info: &[impl CorePowerInfo]) -> Option<ThrottleInfo> {
    if info.is_empty() {
        return None;
    }

    let max_mhz = info.iter().map(|c| c.max_mhz()).max().unwrap_or(0);
    let current_mhz: u32 = {
        let sum: u32 = info.iter().map(|c| c.current_mhz()).sum();
        sum / info.len() as u32
    };
    let mhz_limit = info.iter().map(|c| c.mhz_limit()).min().unwrap_or(0);

    let is_throttled = mhz_limit > 0 && mhz_limit < max_mhz;
    let throttle_pct = if is_throttled && max_mhz > 0 {
        (1.0 - mhz_limit as f32 / max_mhz as f32) * 100.0
    } else {
        0.0
    };

    Some(ThrottleInfo {
        max_mhz,
        current_mhz,
        mhz_limit,
        is_throttled,
        throttle_pct,
    })
}

#[must_use]
pub fn classify_throttle(
    throttle: &ThrottleInfo,
    cpu_usage: f32,
    cpu_temp: Option<f32>,
) -> ThrottleReason {
    if !throttle.is_throttled {
        return ThrottleReason::None;
    }

    if cpu_usage < 20.0 {
        return ThrottleReason::Idle;
    }

    if let Some(temp) = cpu_temp
        && temp >= 80.0
    {
        return ThrottleReason::Thermal;
    }

    if cpu_usage >= 50.0 {
        return ThrottleReason::PowerPolicy;
    }

    ThrottleReason::Unknown
}

/// Abstraction over PROCESSOR_POWER_INFORMATION for testability.
pub trait CorePowerInfo {
    fn max_mhz(&self) -> u32;
    fn current_mhz(&self) -> u32;
    fn mhz_limit(&self) -> u32;
}

#[cfg(target_os = "windows")]
impl CorePowerInfo for windows::Win32::System::Power::PROCESSOR_POWER_INFORMATION {
    fn max_mhz(&self) -> u32 {
        self.MaxMhz
    }
    fn current_mhz(&self) -> u32 {
        self.CurrentMhz
    }
    fn mhz_limit(&self) -> u32 {
        self.MhzLimit
    }
}

// ──────────────────────────────────────────────────────────────────────────
// v0.7 阶段 6：Windows 11 EcoQoS / Efficiency Mode（ADR-0014）。
//
// 与上面「CPU 频率节流检测」(ThrottleInfo / ThrottleReason) 是两件事：
// - ThrottleInfo 检测 CPU 频率被降到 MaxMhz 以下（thermal / power policy）
// - EcoQoSState 是 Win11 Efficiency Mode（绿叶🍃）的 on/off/unknown 状态
// 两者共享 `src/throttle.rs` 文件但语义独立。
// ──────────────────────────────────────────────────────────────────────────

/// Windows 11 EcoQoS / Efficiency Mode 状态（ADR-0014）。
///
/// - `Normal`：进程未启用 EcoQoS（默认状态）
/// - `Eco`：进程已被切到 Efficiency Mode（用户主动 / 系统自动 throttle）
/// - `Unknown`：查询失败 / 平台不支持 / 权限不足
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EcoQoSState {
    #[default]
    Normal,
    Eco,
    Unknown,
}

impl EcoQoSState {
    /// TUI 标记。Eco 模式返回 🍃，其它返回空串（不渲染占位）。
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Eco => " \u{1F343}", // 🍃
            Self::Normal | Self::Unknown => "",
        }
    }

    /// Inspector Summary 行的文本。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Eco => "Eco",
            Self::Unknown => "Unknown",
        }
    }
}

#[cfg(target_os = "windows")]
mod ecoqos_imp {
    use super::EcoQoSState;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, ProcessPowerThrottling,
        SetProcessInformation,
    };

    /// `PROCESSOR_POWER_THROTTLING_STATE` 的内存布局跨 Win11 build 兼容，
    /// 显式 zero-fill 避免未初始化内存被内核拒绝。
    fn make_state(control_mask: u32, state_mask: u32) -> PROCESS_POWER_THROTTLING_STATE {
        PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: control_mask,
            StateMask: state_mask,
        }
    }

    /// 设置进程的 EcoQoS 状态（ADR-0014）。
    ///
    /// - `eco = true` → StateMask = EXECUTION_SPEED（启用 Efficiency Mode）
    /// - `eco = false` → StateMask = 0（恢复 Normal）
    ///
    /// 失败原因：进程已退出 / 权限不足（需要 `PROCESS_SET_INFORMATION`）/
    /// Win11 build < 22000。
    pub fn set_throttle(pid: u32, eco: bool) -> anyhow::Result<()> {
        unsafe {
            let handle = OpenProcess(
                PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            )
            .map_err(|e| anyhow::anyhow!("OpenProcess({}) 失败: {}", pid, e))?;

            let mut state = make_state(
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                if eco {
                    PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                } else {
                    0
                },
            );

            let r = SetProcessInformation(
                handle,
                ProcessPowerThrottling,
                &mut state as *mut _ as *mut _,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            );

            let _ = CloseHandle(handle);

            r.map_err(|e| anyhow::anyhow!("SetProcessInformation 失败: {}", e))?;
        }
        Ok(())
    }

    /// 查询进程的 EcoQoS 状态（ADR-0014）。
    ///
    /// 实现走 `SetProcessInformation` 的隐藏查询模式：ControlMask = 0 +
    /// StateMask = 0 时，Win32 不修改状态，而是把当前 StateMask 填回结构。
    /// 这是 MSDN 文档化的「read current state」用法，比 undocumented 的
    /// `NtQueryInformationProcess(ProcessPowerThrottling)` 更稳定。
    ///
    /// **关键**：查询模式需要 `PROCESS_SET_INFORMATION` 权限（与 set 路径一致），
    /// 否则 `SetProcessInformation` 会拒绝。所以 OpenProcess 用 set + query 两个
    /// flag。
    pub fn query_throttle(pid: u32) -> EcoQoSState {
        unsafe {
            let Ok(handle) = OpenProcess(
                PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            ) else {
                return EcoQoSState::Unknown;
            };

            let mut state = make_state(0, 0);
            let r = SetProcessInformation(
                handle,
                ProcessPowerThrottling,
                &mut state as *mut _ as *mut _,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            );

            let _ = CloseHandle(handle);

            if r.is_err() {
                return EcoQoSState::Unknown;
            }

            if (state.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED) != 0 {
                EcoQoSState::Eco
            } else {
                EcoQoSState::Normal
            }
        }
    }

    /// 批量查询多个 PID 的 EcoQoS 状态，供 HeavyWorker 在一个 refresh 周期内
    /// 调一次（避免每帧 OpenProcess 风暴）。失败的 PID 返回 `Unknown`。
    pub fn query_throttle_batch(pids: &[u32]) -> std::collections::HashMap<u32, EcoQoSState> {
        pids.iter().map(|&p| (p, query_throttle(p))).collect()
    }
}

#[cfg(target_os = "windows")]
pub use ecoqos_imp::{query_throttle, query_throttle_batch, set_throttle};
