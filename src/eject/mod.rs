pub mod cache;
pub mod classify;
pub mod device;
pub mod locks;

use crate::eject::classify::{HandleRisk, classify_handle};
use crate::eject::device::{RemovableDevice, detect_removable_devices, format_size};
use crate::eject::locks::HandleLock;
use crate::error::{ProcError, Result};
use crate::kill;

/// U盘扫描结果
#[derive(Debug, Clone)]
pub struct UsbScanResult {
    pub devices: Vec<RemovableDevice>,
    pub selected_index: usize,
    pub locks: Vec<(HandleLock, HandleRisk)>,
    pub status_message: Option<String>,
}

/// 扫描所有可移除设备
pub fn scan_all_devices() -> Result<Vec<RemovableDevice>> {
    detect_removable_devices()
}

/// 扫描指定设备的占用进程，按风险分级排序
pub fn scan_device_locks(drive_letter: char) -> Result<Vec<(HandleLock, HandleRisk)>> {
    scan_device_locks_with_processes(drive_letter, &[])
}

pub fn scan_device_locks_with_processes(
    drive_letter: char,
    processes: &[crate::collect::ProcessInfo],
) -> Result<Vec<(HandleLock, HandleRisk)>> {
    let locks = locks::find_volume_lockers_with_processes(drive_letter, processes)?;
    let mut classified: Vec<(HandleLock, HandleRisk)> = locks
        .into_iter()
        .map(|l| {
            let risk = classify_handle(&l);
            (l, risk)
        })
        .collect();

    classified.sort_by_key(|b| std::cmp::Reverse(classify::risk_weight(b.1)));

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
            HandleRisk::Safe => match kill::kill_process(lock.pid, false) {
                Ok(kill::KillResult::Killed) => killed += 1,
                Ok(kill::KillResult::AlreadyGone) => {}
                Ok(kill::KillResult::AccessDenied) => {
                    errors.push(format!("PID {} ({}) 权限不足", lock.pid, lock.process_name));
                }
                Ok(kill::KillResult::Failed(e)) => {
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
            },
            _ => skipped += 1,
        }
    }

    if killed > 0 {
        let _ = cache::flush_write_cache(drive_letter);
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
            format_size(dev.used_size),
            format_size(dev.total_size),
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
    let drive_letter = parse_drive_letter(drive_str)?;

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
        .filter(|(_, r)| *r == HandleRisk::Safe)
        .count();
    let warn_count = classified
        .iter()
        .filter(|(_, r)| *r == HandleRisk::Warning)
        .count();
    let crit_count = classified
        .iter()
        .filter(|(_, r)| *r == HandleRisk::Critical)
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
        let _ = cache::flush_write_cache(drive_letter);
    }

    Ok(())
}

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
    Err(ProcError::UsbDetect(format!(
        "无效的驱动器号: '{}'，请使用如 'E:' 格式",
        drive_str
    )))
}
