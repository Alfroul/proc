use std::process::Command as StdCommand;

#[cfg(target_os = "windows")]
use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, LUID};
#[cfg(target_os = "windows")]
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_DEBUG_NAME,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_PRIVILEGES_ATTRIBUTES,
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

/// 按进程名匹配到的进程条目（kill_by_name 返回）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillByNameMatch {
    pub pid: u32,
    pub name: String,
}

/// kill_by_name 的单条结果
#[derive(Debug, Clone)]
pub struct KillByNameResult {
    pub pid: u32,
    pub name: String,
    /// `None` 表示 dry_run（未实际终止）；`Some(r)` 是真实 kill 结果
    pub outcome: Option<KillResult>,
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
                Attributes: TOKEN_PRIVILEGES_ATTRIBUTES(SE_PRIVILEGE_ENABLED.0),
            }],
        };

        AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None).ok();
        CloseHandle(token).ok();
    }
}

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
                return Ok(if code == ERROR_INVALID_PARAMETER.0 {
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

/// 枚举所有进程，返回 `name`（大小写不敏感，精确匹配）相同的进程列表。
/// 不支持通配符 — 仅精确匹配，避免误终止。
#[must_use]
pub fn find_processes_by_name(name: &str) -> Vec<KillByNameMatch> {
    let target = name.to_lowercase();
    crate::collect::sysinfo_with(|sys| {
        sys.processes()
            .iter()
            .filter(|(_, proc)| {
                let proc_name = proc.name().to_string_lossy().to_lowercase();
                proc_name == target
            })
            .map(|(pid, proc)| KillByNameMatch {
                pid: pid.as_u32(),
                name: proc.name().to_string_lossy().to_string(),
            })
            .collect()
    })
}

/// 按进程名终止。`dry_run = true` 时只返回匹配列表，不调用 `kill_process`。
/// `force = true` 时对每个匹配调用 `kill_process_tree`，否则 `kill_single`。
pub fn kill_by_name(name: &str, force: bool, dry_run: bool) -> Result<Vec<KillByNameResult>> {
    // 只调用一次：enable_debug_privilege 内部对每个 kill_process 都会再调用，
    // 但开销极小（一次 OpenProcessToken + LookupPrivilegeValueW）。
    let matches = find_processes_by_name(name);

    Ok(matches
        .into_iter()
        .map(|m| {
            let outcome = if dry_run {
                None
            } else {
                match kill_process(m.pid, force) {
                    Ok(r) => Some(r),
                    Err(e) => Some(KillResult::Failed(e.to_string())),
                }
            };
            KillByNameResult {
                pid: m.pid,
                name: m.name,
                outcome,
            }
        })
        .collect())
}
