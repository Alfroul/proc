// 跨平台降级策略见 ADR-0002：device / locks / cache 仅 Windows；
// classify 跨平台（HandleRisk 枚举 + 风险权重）。非 Windows 下公共 API
// 全部返回 Err(ProcError::UsbDetect)，类型本身保持可见以便消费者编译。
pub mod classify;

#[cfg(target_os = "windows")]
pub mod cache;
#[cfg(target_os = "windows")]
pub mod device;
#[cfg(target_os = "windows")]
pub mod locks;

use crate::error::{ProcError, Result};

/// U 盘扫描结果（跨平台类型，便于录制/重放）
#[derive(Debug, Clone)]
pub struct UsbScanResult {
    pub devices: Vec<RemovableDevice>,
    pub selected_index: usize,
    pub locks: Vec<(HandleLock, classify::HandleRisk)>,
    pub status_message: Option<String>,
}

/// 可移除设备信息（跨平台类型；仅 Windows 上 `scan_all_devices` 会真正返回数据）
#[derive(Debug, Clone)]
pub struct RemovableDevice {
    pub drive_letter: char,
    pub label: String,
    pub total_size: u64,
    pub used_size: u64,
    pub file_system: String,
    pub is_occupied: bool,
    pub device_path: String,
}

/// 句柄占用信息（跨平台类型；非 Windows 永远拿不到实例）
#[derive(Debug, Clone)]
pub struct HandleLock {
    pub pid: u32,
    pub process_name: String,
    pub exe_path: Option<String>,
    pub process_class: crate::classify::ProcessClass,
    pub port_info: Vec<String>,
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{HandleLock, RemovableDevice};
    use crate::error::Result;

    /// 扫描所有可移除设备
    pub fn scan_all_devices() -> Result<Vec<RemovableDevice>> {
        super::device::detect_removable_devices()
    }

    /// 扫描指定设备的占用进程，按风险分级排序
    pub fn scan_device_locks(
        drive_letter: char,
    ) -> Result<Vec<(HandleLock, super::classify::HandleRisk)>> {
        scan_device_locks_with_processes(drive_letter, &[])
    }

    pub fn scan_device_locks_with_processes(
        drive_letter: char,
        processes: &[crate::collect::ProcessInfo],
    ) -> Result<Vec<(HandleLock, super::classify::HandleRisk)>> {
        let locks = super::locks::find_volume_lockers_with_processes(drive_letter, processes)?;
        let mut classified: Vec<(HandleLock, super::classify::HandleRisk)> = locks
            .into_iter()
            .map(|l| {
                let risk = super::classify::classify_handle(&l);
                (l, risk)
            })
            .collect();

        classified.sort_by_key(|b| std::cmp::Reverse(super::classify::risk_weight(b.1)));

        Ok(classified)
    }

    /// 终止指定设备上所有安全可终止的进程
    ///
    /// 只终止 Safe 级别的进程，跳过 Warning 和 Critical。
    /// 返回 (终止成功数, 跳过数, 失败信息)
    pub fn kill_safe_processes(drive_letter: char) -> Result<(u32, u32, Vec<String>)> {
        let classified = scan_device_locks(drive_letter)?;

        let mut killed = 0u32;
        let mut skipped = 0u32;
        let mut errors = Vec::new();

        for (lock, risk) in &classified {
            match risk {
                super::classify::HandleRisk::Safe => {
                    match crate::kill::kill_process(lock.pid, false) {
                        Ok(crate::kill::KillResult::Killed) => killed += 1,
                        Ok(crate::kill::KillResult::AlreadyGone) => {}
                        Ok(crate::kill::KillResult::AccessDenied) => {
                            errors
                                .push(format!("PID {} ({}) 权限不足", lock.pid, lock.process_name));
                        }
                        Ok(crate::kill::KillResult::Failed(e)) => {
                            errors.push(format!(
                                "PID {} ({}) 失败: {}",
                                lock.pid, lock.process_name, e
                            ));
                        }
                        Err(e) => {
                            errors.push(format!(
                                "PID {} ({}) 错误: {}",
                                lock.pid, lock.process_name, e
                            ));
                        }
                    }
                }
                _ => skipped += 1,
            }
        }

        if killed > 0 {
            let _ = super::cache::flush_write_cache(drive_letter);
        }

        Ok((killed, skipped, errors))
    }

    /// CLI: 列出所有可移除设备
    pub fn cli_list_devices() -> Result<()> {
        let devices = scan_all_devices()?;

        if devices.is_empty() {
            println!("未检测到可移除设备");
            return Ok(());
        }

        for dev in &devices {
            let size_info = format!(
                "{} / {} ({})",
                crate::format::format_bytes(dev.used_size),
                crate::format::format_bytes(dev.total_size),
                dev.file_system
            );
            let status = if dev.is_occupied {
                "⚠ 占用中"
            } else {
                "✓ 可弹出"
            };
            println!(
                "{}: {} [{}] - {}",
                dev.drive_letter, dev.label, size_info, status
            );
        }

        Ok(())
    }

    /// CLI: 检测指定驱动器的占用进程
    pub fn cli_check_drive(drive_str: &str, find_locks_only: bool) -> Result<()> {
        let drive_letter = super::parse_drive_letter(drive_str)?;

        let classified = scan_device_locks(drive_letter)?;

        if classified.is_empty() {
            println!("✅ {}:\\ 无占用进程，可以安全弹出", drive_letter);
            return Ok(());
        }

        println!("{}:\\ 占用进程:", drive_letter);
        println!();

        for (lock, risk) in &classified {
            let ports = if lock.port_info.is_empty() {
                String::new()
            } else {
                format!(" [{}]", lock.port_info.join(", "))
            };
            let exe = lock.exe_path.as_deref().unwrap_or("-");
            println!(
                "  {} PID {:>6}  {:<25} {} {}{}",
                risk.label(),
                lock.pid,
                lock.process_name,
                risk.description(),
                exe,
                ports
            );
        }

        println!();

        if find_locks_only {
            return Ok(());
        }

        let safe_count = classified
            .iter()
            .filter(|(_, r)| *r == super::classify::HandleRisk::Safe)
            .count();
        let warn_count = classified
            .iter()
            .filter(|(_, r)| *r == super::classify::HandleRisk::Warning)
            .count();
        let crit_count = classified
            .iter()
            .filter(|(_, r)| *r == super::classify::HandleRisk::Critical)
            .count();

        if safe_count > 0 {
            println!(
                "  提示: {} 个用户进程可安全终止 (使用 TUI 界面按 k 键终止)",
                safe_count
            );
        }
        if warn_count > 0 {
            println!(
                "  提示: {} 个系统后台进程占用，建议等待或关闭相关窗口",
                warn_count
            );
        }
        if crit_count > 0 {
            println!(
                "  提示: {} 个关键系统进程占用（可能是写入缓存），尝试刷新缓存...",
                crit_count
            );
            let _ = super::cache::flush_write_cache(drive_letter);
        }

        Ok(())
    }
}

#[cfg(target_os = "windows")]
pub use windows_impl::{
    cli_check_drive, cli_list_devices, kill_safe_processes, scan_all_devices, scan_device_locks,
    scan_device_locks_with_processes,
};

#[cfg(not(target_os = "windows"))]
mod stub_impl {
    use super::{HandleLock, RemovableDevice};
    use crate::error::{ProcError, Result};

    const UNSUPPORTED: &str = "Linux/macOS 不支持 USB 助手，详见 README 平台支持表";

    pub fn scan_all_devices() -> Result<Vec<RemovableDevice>> {
        Err(ProcError::usb_detect(UNSUPPORTED))
    }

    pub fn scan_device_locks(
        _drive_letter: char,
    ) -> Result<Vec<(HandleLock, super::classify::HandleRisk)>> {
        Err(ProcError::usb_detect(UNSUPPORTED))
    }

    pub fn scan_device_locks_with_processes(
        _drive_letter: char,
        _processes: &[crate::collect::ProcessInfo],
    ) -> Result<Vec<(HandleLock, super::classify::HandleRisk)>> {
        Err(ProcError::usb_detect(UNSUPPORTED))
    }

    pub fn kill_safe_processes(_drive_letter: char) -> Result<(u32, u32, Vec<String>)> {
        Err(ProcError::usb_detect(UNSUPPORTED))
    }

    pub fn cli_list_devices() -> Result<()> {
        eprintln!("Linux/macOS 不支持 USB 助手，详见 README 平台支持表");
        Ok(())
    }

    pub fn cli_check_drive(_drive_str: &str, _find_locks_only: bool) -> Result<()> {
        Err(ProcError::usb_detect(UNSUPPORTED))
    }
}

#[cfg(not(target_os = "windows"))]
pub use stub_impl::{
    cli_check_drive, cli_list_devices, kill_safe_processes, scan_all_devices, scan_device_locks,
    scan_device_locks_with_processes,
};

fn parse_drive_letter(drive_str: &str) -> Result<char> {
    let cleaned = drive_str
        .trim()
        .trim_end_matches(':')
        .trim_end_matches('\\');
    if cleaned.len() == 1 {
        let c = cleaned.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return Ok(c.to_ascii_uppercase());
        }
    }
    Err(ProcError::usb_detect(format!(
        "无效的驱动器号: '{drive_str}'，请使用如 'E:' 格式"
    )))
}
