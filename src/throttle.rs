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

#[cfg(not(target_os = "windows"))]
pub fn query_processor_power_info() -> Option<Vec<()>> {
    None
}

/// Compute throttle info from raw per-core data (max_mhz, current_mhz, mhz_limit).
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
