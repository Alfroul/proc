//! Windows 可移除设备检测。整个模块 cfg-gate 到 Windows（见 ADR-0002）。

use super::RemovableDevice;
use crate::error::Result;

/// 使用 Windows API 检测所有可移除设备
pub fn detect_removable_devices() -> Result<Vec<RemovableDevice>> {
    let mut devices = Vec::new();

    for b in b'A'..=b'Z' {
        let letter = b as char;
        let root = format!("{}:\\", letter);
        let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

        // SAFETY: root_wide is a stack-allocated, null-terminated UTF-16 buffer. The pointer is valid for the duration of this call.
        let drive_type = unsafe {
            windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(
                root_wide.as_ptr(),
            ))
        };

        if drive_type != 2u32 {
            continue;
        }

        let mut free_available: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut free_bytes: u64 = 0;

        // SAFETY: root_wide is valid as above. Output parameters are stack-allocated u64 mut refs.
        let free_space_ok = unsafe {
            windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                windows::core::PCWSTR(root_wide.as_ptr()),
                Some(&mut free_available),
                Some(&mut total_bytes),
                Some(&mut free_bytes),
            )
        };

        let total_size = if free_space_ok.is_ok() {
            total_bytes
        } else {
            0
        };
        let used_size = total_size.saturating_sub(free_bytes);

        let mut volume_name_buf = [0u16; 256];
        let mut file_system_buf = [0u16; 32];
        // SAFETY: root_wide is valid. volume_name_buf and file_system_buf are stack-allocated fixed-size arrays.
        let _ = unsafe {
            windows::Win32::Storage::FileSystem::GetVolumeInformationW(
                windows::core::PCWSTR(root_wide.as_ptr()),
                Some(&mut volume_name_buf),
                None,
                None,
                None,
                Some(&mut file_system_buf),
            )
        };

        let label = wide_to_string(&volume_name_buf);
        let file_system = wide_to_string(&file_system_buf);

        devices.push(RemovableDevice {
            drive_letter: letter,
            label: if label.is_empty() {
                "可移动磁盘".to_string()
            } else {
                label
            },
            total_size,
            used_size,
            file_system,
            is_occupied: false,
            device_path: root,
        });
    }

    Ok(devices)
}

/// 从宽字符缓冲区转换到 String
fn wide_to_string(buf: &[u16]) -> String {
    use std::os::windows::ffi::OsStringExt;

    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    if len == 0 {
        return String::new();
    }
    std::ffi::OsString::from_wide(&buf[..len])
        .to_string_lossy()
        .into_owned()
}
