use std::process::Command as StdCommand;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, LUID};
#[cfg(target_os = "windows")]
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, SE_DEBUG_NAME, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_PRIVILEGES_ATTRIBUTES,
    LUID_AND_ATTRIBUTES,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_TERMINATE, TerminateProcess,
};

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillResult {
    Killed,
    AlreadyGone,
    AccessDenied,
    Failed(String),
}

/// 启用 SeDebugPrivilege。管理员令牌默认拥有但禁用此特权，需显式启用才能终止系统/服务进程。
/// 非管理员调用静默失败（特权不在令牌中），不影响后续逻辑。
#[cfg(target_os = "windows")]
fn enable_debug_privilege() {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token).is_err() {
            return;
        }

        let mut luid = LUID::default();
        if LookupPrivilegeValueW(None, SE_DEBUG_NAME, &mut luid).is_err() {
            CloseHandle(token).ok();
            return;
        }

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: TOKEN_PRIVILEGES_ATTRIBUTES(SE_PRIVILEGE_ENABLED.0 as u32),
            }],
        };

        AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None).ok();
        CloseHandle(token).ok();
    }
}

#[cfg(not(target_os = "windows"))]
fn enable_debug_privilege() {}

pub fn kill_process(pid: u32, force: bool) -> Result<KillResult> {
    enable_debug_privilege();

    if force {
        kill_process_tree(pid)
    } else {
        kill_single(pid)
    }
}

#[cfg(target_os = "windows")]
fn kill_single(pid: u32) -> Result<KillResult> {
    unsafe {
        let handle = match OpenProcess(PROCESS_TERMINATE, false, pid) {
            Ok(h) => h,
            Err(e) => {
                let code = e.code().0 as u32;
                return Ok(if code == ERROR_INVALID_PARAMETER.0 as u32 {
                    KillResult::AlreadyGone
                } else {
                    tracing::warn!("OpenProcess({pid}) failed: {code}");
                    KillResult::AccessDenied
                });
            }
        };

        let terminated = TerminateProcess(handle, 1);
        CloseHandle(handle).ok();

        if terminated.is_ok() {
            Ok(KillResult::Killed)
        } else {
            Ok(KillResult::AccessDenied)
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn kill_single(pid: u32) -> Result<KillResult> {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let pid_obj = sysinfo::Pid::from_u32(pid);
    match sys.process(pid_obj) {
        Some(proc) => {
            if proc.kill() {
                Ok(KillResult::Killed)
            } else {
                Ok(KillResult::AccessDenied)
            }
        }
        None => Ok(KillResult::AlreadyGone),
    }
}

fn kill_process_tree(pid: u32) -> Result<KillResult> {
    let output = StdCommand::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output()?;

    if output.status.success() {
        return Ok(KillResult::Killed);
    }

    let exit_code = output.status.code().unwrap_or(0);
    if exit_code == 128 {
        return Ok(KillResult::AlreadyGone);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_lower = stderr.to_lowercase();
    if stderr_lower.contains("denied") || stderr_lower.contains("拒绝") {
        Ok(KillResult::AccessDenied)
    } else {
        Ok(KillResult::Failed(stderr.to_string()))
    }
}
