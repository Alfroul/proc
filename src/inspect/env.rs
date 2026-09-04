//! 进程环境变量采集。
//!
//! - **Windows**：`OpenProcess` + `NtQueryInformationProcess(ProcessBasicInformation)`
//!   拿到 PEB 地址，再用 `ReadProcessMemory` 走 PEB → ProcessParameters → Environment。
//!   PEB 偏移在 x64 上固定 0x20；RTL_USER_PROCESS_PARAMETERS 的 Environment 指针
//!   偏移 0x80、EnvironmentSize 偏移 0x3F0（Vista 起稳定）。
//! - **Linux**：直接读 `/proc/<pid>/environ`（NUL 分隔的 KEY=VALUE）。
//! - **macOS**：没有等价的 `/proc`，proc_pidinfo 需要 task_for_pid（通常无权限），
//!   v1 直接 stub。

use crate::error::{ProcError, Result};

use super::EnvVar;

/// 等价于 ntddk 的 PROCESS_BASIC_INFORMATION。windows 0.57 把它放在
/// `Win32::System::Threading` 但 cfg-gate 在 `Win32_System_Kernel` 后面，所以这里
/// 自己声明 `#[repr(C)]` —— ABI 不变，省一个 feature 开关。
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessBasicInformation {
    exit_status: i32,
    peb_base_address: usize,
    affinity_mask: usize,
    base_priority: i32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
}

#[cfg(target_os = "windows")]
pub fn collect_env(pid: u32) -> Result<Vec<EnvVar>> {
    // 32-bit Windows 的 PEB 偏移不同，本项目目前未发布 32-bit 版本。
    #[cfg(not(target_pointer_width = "64"))]
    {
        let _ = pid;
        return Err(ProcError::permission_denied(
            "Windows x86 target 暂不支持环境变量采集",
        ));
    }

    #[cfg(target_pointer_width = "64")]
    {
        use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
        };

        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
                .map_err(|e| {
                    ProcError::permission_denied_with(format!("OpenProcess({pid}) 失败"), e)
                })?;

            // 任何子步骤失败都先关 handle 再返回；用 macro 把样板收敛住。
            macro_rules! bail {
                ($msg:expr) => {{
                    let _ = CloseHandle(handle);
                    return Err(ProcError::permission_denied($msg));
                }};
            }

            // 1) NtQueryInformationProcess → PEB 地址
            let mut pbi = ProcessBasicInformation::default();
            let status = NtQueryInformationProcess(
                handle,
                ProcessBasicInformation,
                &mut pbi as *mut _ as *mut _,
                std::mem::size_of::<ProcessBasicInformation>() as u32,
                std::ptr::null_mut(),
            );
            if status.is_err() {
                bail!("NtQueryInformationProcess(ProcessBasicInformation) 失败");
            }
            let peb = pbi.peb_base_address as *const u8;
            if peb.is_null() {
                bail!("PEB 地址为空");
            }

            // 2) 读 PEB.ProcessParameters（offset 0x20 on x64）
            let params_addr = match read_usize(handle, peb.add(0x20)) {
                Ok(v) => v,
                Err(_) => bail!("读取 ProcessParameters 指针失败"),
            };
            if params_addr == 0 {
                bail!("ProcessParameters 指针为空");
            }
            let params = params_addr as *const u8;

            // 3) 读 RTL_USER_PROCESS_PARAMETERS.Environment（0x80）和 .EnvironmentSize（0x3F0）
            let env_addr = match read_usize(handle, params.add(0x80)) {
                Ok(v) => v,
                Err(_) => bail!("读取 Environment 指针失败"),
            };
            if env_addr == 0 {
                bail!("Environment 指针为空");
            }
            let env_size = read_u32(handle, params.add(0x3F0)).unwrap_or(0);

            // 4) 读 UTF-16 环境块（在关 handle 之前完成所有内存读取）
            let max_scan = 64 * 1024usize;
            let want = if env_size > 0 && (env_size as usize) <= max_scan {
                env_size as usize
            } else {
                max_scan
            };
            let mut raw = vec![0u8; want];
            let read_ok = ReadProcessMemory(
                handle,
                env_addr as *const _,
                raw.as_mut_ptr() as *mut _,
                want,
                None,
            );
            let _ = CloseHandle(handle);
            if read_ok.is_err() {
                return Err(ProcError::permission_denied("读取 Environment 块失败"));
            }

            Ok(parse_utf16_env(&raw))
        }
    }
}

#[cfg(target_os = "windows")]
#[cfg(target_pointer_width = "64")]
unsafe fn read_usize(
    handle: windows::Win32::Foundation::HANDLE,
    addr: *const u8,
) -> std::result::Result<usize, ()> {
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    let mut out: usize = 0;
    let ok = unsafe {
        ReadProcessMemory(
            handle,
            addr as *const _,
            &mut out as *mut _ as *mut _,
            std::mem::size_of::<usize>(),
            None,
        )
    };
    if ok.is_ok() { Ok(out) } else { Err(()) }
}

#[cfg(target_os = "windows")]
#[cfg(target_pointer_width = "64")]
unsafe fn read_u32(
    handle: windows::Win32::Foundation::HANDLE,
    addr: *const u8,
) -> std::result::Result<u32, ()> {
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    let mut out: u32 = 0;
    let ok = unsafe {
        ReadProcessMemory(
            handle,
            addr as *const _,
            &mut out as *mut _ as *mut _,
            std::mem::size_of::<u32>(),
            None,
        )
    };
    if ok.is_ok() { Ok(out) } else { Err(()) }
}

#[cfg(target_os = "windows")]
fn parse_utf16_env(raw: &[u8]) -> Vec<EnvVar> {
    // UTF-16 LE，NUL 分隔每条，结尾双 NUL。多余字节按 lossy 处理。
    let units: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_ne_bytes(*c))
        .collect();
    let s = String::from_utf16_lossy(&units);
    // 截到首个双 NUL（完整环境块结束）。
    let block = s.split("\u{0}\u{0}").next().unwrap_or(&s);
    block
        .split('\u{0}')
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(2, '=');
            let key = parts.next()?.to_string();
            let value = parts.next()?.to_string();
            if key.is_empty() {
                None
            } else {
                Some(EnvVar {
                    is_secret: super::env_mask::is_secret_key(&key),
                    key,
                    value,
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_env_has_path() {
        // 在自己进程上跑，应当总能拿到 PATH。
        let vars = collect_env(std::process::id()).expect("self env");
        let has_path = vars.iter().any(|v| v.key.eq_ignore_ascii_case("PATH"));
        // CI 上有时会清空环境；如确实没 PATH，至少应能拿到 1 条变量。
        assert!(
            has_path || !vars.is_empty(),
            "collect_env returned empty: {:?}",
            vars.iter().take(3).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unknown_pid_returns_err() {
        // PID 0xFFFF_FFFF 几乎肯定不存在。
        let res = collect_env(u32::MAX);
        assert!(res.is_err(), "expected err for bogus pid, got {:?}", res);
    }
}
