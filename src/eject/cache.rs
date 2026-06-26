//! Windows 卷写入缓存刷新（PowerShell Write-VolumeCache）。整个模块 cfg-gate 到 Windows（见 ADR-0002）。

use std::thread;
use std::time::Duration;

use crate::error::{ProcError, Result};
use crate::security::restricted_spawn::run_with_reduced_privileges;

/// 刷新指定驱动器的写入缓存
pub fn flush_write_cache(drive_letter: char) -> Result<()> {
    let script = format!("Write-VolumeCache {}:", drive_letter);

    let output = run_with_reduced_privileges(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .map_err(|e| ProcError::usb_detect_with("执行 Write-VolumeCache 失败", e))?;

    if !output.success() {
        tracing::warn!(
            "Write-VolumeCache 退出码非零（exit = {:?}）",
            output.status.code
        );
    }

    thread::sleep(Duration::from_secs(3));

    Ok(())
}

/// 持续监测驱动器占用状态
///
/// 每隔 `interval_secs` 秒扫描一次，直到无占用或达到最大尝试次数。
/// 返回 true 表示已无占用，false 表示仍有占用。
pub fn wait_and_monitor(drive_letter: char, interval_secs: u64, max_attempts: u32) -> Result<bool> {
    for _ in 0..max_attempts {
        thread::sleep(Duration::from_secs(interval_secs));

        let locks = crate::eject::locks::find_volume_lockers(drive_letter)?;
        if locks.is_empty() {
            return Ok(true);
        }
    }

    let locks = crate::eject::locks::find_volume_lockers(drive_letter)?;
    Ok(locks.is_empty())
}
