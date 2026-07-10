//! Windows USB / 可移除设备物理弹出（PowerShell Shell.Application COM）。
//!
//! v0.17 stage 6 落地 `proc_usb_release` tool 的第三步（kill → flush → eject）
//! 中 eject 步所需 API。与 [`super::cache::flush_write_cache`] 同款 PowerShell
//! 路径（reduced-privileges spawn），避免引入 unsafe windows-sys 调用 +
//! IOCTL_STORAGE_EJECT_MEDIA 的复杂句柄管理。
//!
//! PowerShell 脚本调用 shell32 Shell.Application COM：
//! - `Namespace(17)` → ssfDRIVES（我的电脑 / This PC 下的驱动器集合）
//! - `ParseName('E:')` → 找到指定驱动器的 FolderItem
//! - `InvokeVerb('Eject')` → 触发 shell 弹出动词（与右键菜单「弹出」同款路径）
//!
//! 该调用阻塞直到设备弹出或失败（用户拒绝 / 设备占用 / 系统忙）。失败时
//! PowerShell 退出码非零，本函数返 Err。

use crate::error::{ProcError, Result};
use crate::security::restricted_spawn::run_with_reduced_privileges;

/// 弹出指定驱动器的可移除设备。
///
/// 与 [`super::cache::flush_write_cache`] 同款路径（reduced-privileges spawn
/// PowerShell），失败返 Err 让 [`crate::mcp::handler::record::make_usb_release_json`]
/// 在 JSON 顶层透出错误。
///
/// 阻塞行为：Shell.Application InvokeVerb('Eject') 内部等待系统弹出完成
/// （通常 < 5s，但 Windows 在锁占用 / 缓存刷盘未完成时可能等到 30s+）。
pub fn eject_device(drive_letter: char) -> Result<()> {
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $shell = New-Object -ComObject Shell.Application; \
         $drive = $shell.Namespace(17).ParseName('{letter}:'); \
         if ($null -eq $drive) {{ \
             Write-Error \"drive {letter}: not found in Shell namespace\"; \
             exit 1; \
         }}; \
         $drive.InvokeVerb('Eject'); \
         Start-Sleep -Milliseconds 500",
        letter = drive_letter
    );

    let output = run_with_reduced_privileges(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )
    .map_err(|e| ProcError::usb_detect_with("执行 eject_device PowerShell 失败", e))?;

    if !output.status.success() {
        return Err(ProcError::usb_detect(format!(
            "eject_device({drive_letter}:) PowerShell 退出码非零 (exit={:?})",
            output.status.code
        )));
    }
    Ok(())
}
