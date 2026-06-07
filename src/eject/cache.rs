use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::error::{ProcError, Result};

/// 刷新指定驱动器的写入缓存
pub fn flush_write_cache(drive_letter: char) -> Result<()> {
    let script = format!("Write-VolumeCache {}:", drive_letter);

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| ProcError::UsbDetect(format!("执行 Write-VolumeCache 失败: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("Write-VolumeCache 输出: {}", stderr);
    }

    thread::sleep(Duration::from_secs(3));

    Ok(())
}

/// 持续监测驱动器占用状态
///
/// 每隔 `interval_secs` 秒扫描一次，直到无占用或达到最大尝试次数。
/// 返回 true 表示已无占用，false 表示仍有占用。
pub fn wait_and_monitor(
    drive_letter: char,
    interval_secs: u64,
    max_attempts: u32,
) -> Result<bool> {
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
