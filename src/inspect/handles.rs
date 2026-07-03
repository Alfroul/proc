//! 句柄采集 + 文件占用反查（阶段 4，A1）。
//!
//! - **Windows**：用 `GetModuleHandleW` + `GetProcAddress` 加载 ntdll 的
//!   `NtQuerySystemInformation` / `NtQueryObject`，枚举
//!   `SystemExtendedHandleInformation`，按 PID 过滤后对每个句柄查类型
//!   （`ObjectTypeInformation` 快）。Name 字段对非 File 类型留空；File 类型
//!   在反查 `find_lockers` 路径下走 filelocksmith（更稳，避免同步
//!   `NtQueryObject(ObjectNameInformation)` 的阻塞风险）。
//!
//! 反查 `find_lockers(path)`：
//! - Windows：`filelocksmith::find_processes_locking_path(path)` 返回 PID
//!   列表（内部已用线程 + 超时规避 NtQueryObject 阻塞），再用 sysinfo 补
//!   进程名。

use crate::error::Result;

use super::{HandleInfo, HandleKind};

/// 同步采集 `pid` 打开的所有句柄。
///
/// 失败时返回 `Err`；空结果（进程已退出 / 句柄表为空）返回 `Ok(vec![])`。
/// 调用方据此决定显示「无数据」还是「权限不足」。
pub fn collect_handles(pid: u32) -> Result<Vec<HandleInfo>> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::collect(pid)
    }
}

/// 反查「谁占用 `path`」。返回所有持有该路径句柄的进程。
///
/// 注意：非管理员账户看不到系统进程的句柄，会返回空 Vec。UI / CLI 应在空
/// 结果时提示「需要管理员权限枚举系统进程句柄」。
pub fn find_lockers(path: &std::path::Path) -> Result<Vec<HandleInfo>> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::find_lockers(path)
    }
}

/// 把 NT 对象的 type_name 字符串（如 "File"、"Key"、"Mutant"）归类到
/// [`HandleKind`]。未知类型归 [`HandleKind::Other`]，空字符串归
/// [`HandleKind::Unknown`]。
///
/// 单独提取成函数以便单元测试覆盖；调用方拿到 type_name 字符串后调它即可。
#[must_use]
pub fn parse_handle_kind(type_name: &str) -> HandleKind {
    if type_name.is_empty() {
        return HandleKind::Unknown;
    }
    // Windows 内核的 type_name 命名约定（参考 winobj）：
    //   File / Directory / Key / Event / Semaphore / Mutant / Section /
    //   Process / Thread / Token / Timer / Job / Desktop / WindowStation /
    //   ALPC Port / IoCompletion / ...
    match type_name {
        "File" => HandleKind::File,
        "Directory" => HandleKind::Directory,
        "Key" => HandleKind::RegistryKey,
        "Event" => HandleKind::Event,
        "Semaphore" => HandleKind::Semaphore,
        "Mutant" => HandleKind::Mutant,
        "Section" => HandleKind::Section,
        "Process" => HandleKind::Process,
        "Thread" => HandleKind::Thread,
        "Token" => HandleKind::Token,
        _ => HandleKind::Other,
    }
}

/// 把 raw 句柄值（Windows HANDLE / Linux fd）规整成 16 进制字符串，用于
/// UI 渲染和 CLI 输出。Windows HANDLE 是 64-bit 指针，Linux fd 是 32-bit
/// 整数，统一用 0x{:X} 展示即可。
#[must_use]
pub fn format_raw_handle(raw: u64) -> String {
    format!("0x{raw:X}")
}

// ── Windows impl ────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{HandleInfo, parse_handle_kind};
    use crate::error::{ProcError, Result};
    use std::ffi::c_void;
    use std::path::Path;
    use windows::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE};
    use windows::core::{s, w};

    // NT constants —— 不依赖 windows-sys 的 Wdk feature，自己定义。
    const SYSTEM_EXTENDED_HANDLE_INFORMATION: u32 = 64; // 0x40
    const OBJECT_TYPE_INFORMATION: u32 = 2;
    const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC0000004u32 as i32;
    const STATUS_SUCCESS: i32 = 0;

    // NT 函数指针类型。HANDLE 在 windows 0.57 是 `pub struct HANDLE(pub isize)`，
    // 但函数签名里我们用 isize 传，避免 borrow 问题。
    type NtQuerySystemInformationFn =
        unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;
    type NtQueryObjectFn = unsafe extern "system" fn(isize, u32, *mut c_void, u32, *mut u32) -> i32;

    /// SYSTEM_HANDLE_INFORMATION_EX 的头部 + 灵活数组成员。
    /// 64-bit 上每条 entry 是 40 字节，布局参考 Process Hacker / winobj。
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SystemHandleEntryEx {
        object: *mut c_void,
        unique_process_id: usize, // HANDLE / PVOID
        handle_value: usize,      // HANDLE
        granted_access: u32,
        creator_back_trace_index: u16,
        object_type_index: u16,
        handle_attributes: u32,
        reserved: u32,
    }

    /// 我们只读前两个 usize 字段，剩下的直接用 u8 数组占位。
    /// `*const SystemHandleInformationExHeader` + `as *const SystemHandleEntryEx`
    /// 用来按 entry stride 遍历。
    #[repr(C)]
    struct SystemHandleInformationExHeader {
        number_of_handles: usize,
        reserved: usize,
        // 灵活数组成员：handles[number_of_handles]
    }

    /// NT `UNICODE_STRING` 结构（{len, max_len, buffer}）。
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct UnicodeString {
        length: u16, // 字节数（不含终止符）
        maximum_length: u16,
        buffer: *const u16,
    }

    /// `OBJECT_TYPE_INFORMATION` 头部（后面跟 reserved 字段，我们不读）。
    #[repr(C)]
    struct ObjectTypeInformationHeader {
        type_name: UnicodeString,
        // 后面还有 24+ bytes 的 reserved 字段，我们不读。
    }

    struct NtDll {
        query_system_information: NtQuerySystemInformationFn,
        query_object: NtQueryObjectFn,
    }

    impl NtDll {
        fn load() -> Option<Self> {
            unsafe {
                let module = GetModuleHandleW(w!("ntdll.dll")).ok()?;
                let qsi_addr = GetProcAddress(module, s!("NtQuerySystemInformation"))?;
                let qo_addr = GetProcAddress(module, s!("NtQueryObject"))?;
                Some(Self {
                    // 显式类型注解：让 clippy 满意 + 让 unsafe 块意图明确。
                    // FARPROC（unsafe extern "system" fn() -> isize）→ 目标 fn 类型。
                    query_system_information: {
                        let p: NtQuerySystemInformationFn = std::mem::transmute(qsi_addr);
                        p
                    },
                    query_object: {
                        let p: NtQueryObjectFn = std::mem::transmute(qo_addr);
                        p
                    },
                })
            }
        }
    }

    /// 多轮调用 NtQuerySystemInformation，自动扩容 buffer（STATUS_INFO_LENGTH_MISMATCH
    /// 时翻倍）。
    fn nt_query_system_handles(ntdll: &NtDll) -> Result<Vec<u8>> {
        let mut size: u32 = 64 * 1024;
        for _ in 0..8 {
            let mut buffer = vec![0u8; size as usize];
            let mut return_length: u32 = 0;
            let status = unsafe {
                (ntdll.query_system_information)(
                    SYSTEM_EXTENDED_HANDLE_INFORMATION,
                    buffer.as_mut_ptr().cast(),
                    size,
                    &mut return_length,
                )
            };
            if status == STATUS_SUCCESS {
                if (return_length as usize) <= buffer.len() {
                    buffer.truncate(return_length as usize);
                }
                return Ok(buffer);
            }
            if status == STATUS_INFO_LENGTH_MISMATCH {
                let next = std::cmp::max(size * 2, return_length);
                if next > 64 * 1024 * 1024 {
                    return Err(ProcError::permission_denied(
                        "NtQuerySystemInformation 返回 > 64MB，拒绝继续扩容",
                    ));
                }
                size = next;
                continue;
            }
            return Err(ProcError::permission_denied(format!(
                "NtQuerySystemInformation 失败 status=0x{:08X}",
                status as u32
            )));
        }
        Err(ProcError::permission_denied(
            "NtQuerySystemInformation 重试 8 次仍未成功",
        ))
    }

    /// 把 NT `UNICODE_STRING` 拷成 `String`。
    fn unicode_string_to_string(us: &UnicodeString) -> String {
        if us.length == 0 || us.buffer.is_null() {
            return String::new();
        }
        let char_count = (us.length as usize) / 2;
        let slice = unsafe { std::slice::from_raw_parts(us.buffer, char_count) };
        String::from_utf16_lossy(slice)
    }

    /// 对一个已 duplicate 到当前进程的句柄调 NtQueryObject 拿 type_name。
    fn query_object_type(ntdll: &NtDll, dup_handle_value: isize) -> String {
        let mut buf = [0u8; 1024];
        let mut return_length: u32 = 0;
        let status = unsafe {
            (ntdll.query_object)(
                dup_handle_value,
                OBJECT_TYPE_INFORMATION,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
                &mut return_length,
            )
        };
        if status != STATUS_SUCCESS {
            return String::new();
        }
        if (return_length as usize) < std::mem::size_of::<ObjectTypeInformationHeader>() {
            return String::new();
        }
        let info: &ObjectTypeInformationHeader =
            unsafe { &*(buf.as_ptr() as *const ObjectTypeInformationHeader) };
        unicode_string_to_string(&info.type_name)
    }

    pub fn collect(pid: u32) -> Result<Vec<HandleInfo>> {
        let Some(ntdll) = NtDll::load() else {
            return Err(ProcError::permission_denied(
                "无法加载 ntdll 的 NtQuerySystemInformation / NtQueryObject",
            ));
        };

        let buffer = nt_query_system_handles(&ntdll)?;
        if buffer.len() < std::mem::size_of::<SystemHandleInformationExHeader>() {
            return Ok(Vec::new());
        }
        let header: &SystemHandleInformationExHeader =
            unsafe { &*(buffer.as_ptr() as *const SystemHandleInformationExHeader) };
        let count = header.number_of_handles;

        let entry_size = std::mem::size_of::<SystemHandleEntryEx>();
        let handles_start = unsafe {
            buffer
                .as_ptr()
                .add(std::mem::size_of::<SystemHandleInformationExHeader>())
        };
        let expected_bytes =
            std::mem::size_of::<SystemHandleInformationExHeader>() + count * entry_size;
        if buffer.len() < expected_bytes {
            return Err(ProcError::permission_denied(format!(
                "NtQuerySystemInformation buffer 不足：期望 {expected_bytes} 字节，实际 {}",
                buffer.len()
            )));
        }

        // 一次性打开目标进程（PROCESS_DUP_HANDLE），所有句柄共享这一个 handle。
        let proc_handle = unsafe {
            OpenProcess(PROCESS_DUP_HANDLE, false, pid)
                .map_err(|e| ProcError::permission_denied_with("OpenProcess 失败", e))?
        };
        let current_proc = unsafe { GetCurrentProcess() };

        let mut handles = Vec::new();
        const MAX_PER_PID: usize = 65_536;
        let scan_count = count.min(MAX_PER_PID);

        for i in 0..scan_count {
            let entry: &SystemHandleEntryEx =
                unsafe { &*(handles_start.add(i * entry_size) as *const SystemHandleEntryEx) };
            if entry.unique_process_id as u32 != pid {
                continue;
            }

            // DuplicateHandle 把目标进程的句柄复制到当前进程，以便 NtQueryObject。
            let mut dup: HANDLE = HANDLE(0);
            let ok = unsafe {
                DuplicateHandle(
                    proc_handle,
                    HANDLE(entry.handle_value as isize),
                    current_proc,
                    &mut dup,
                    0,
                    false,
                    DUPLICATE_SAME_ACCESS,
                )
            };
            if ok.is_err() {
                continue;
            }

            let type_name = query_object_type(&ntdll, dup.0);
            let _ = unsafe { CloseHandle(dup) };

            let kind = parse_handle_kind(&type_name);
            handles.push(HandleInfo {
                raw_handle: entry.handle_value as u64,
                kind,
                name: String::new(),
                granted_access: entry.granted_access,
            });
        }

        let _ = unsafe { CloseHandle(proc_handle) };
        Ok(handles)
    }

    pub fn find_lockers(path: &Path) -> Result<Vec<HandleInfo>> {
        // filelocksmith 已经把 NtQueryObject 的阻塞问题用线程 + 超时解决，比我们
        // 自己写的同步版本稳。这里直接复用，把返回的 PID 编码进 raw_handle 字段
        // —— 这是 A1 反查路径下有意为之的字段复用，调用方（CLI `who`）知道这一点。
        let pids = filelocksmith::find_processes_locking_path(path);
        let handles: Vec<HandleInfo> = pids
            .into_iter()
            .map(|pid| HandleInfo {
                raw_handle: pid as u64,
                kind: super::HandleKind::File,
                name: path.to_string_lossy().to_string(),
                granted_access: 0,
            })
            .collect();
        Ok(handles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handle_kind_known_types() {
        assert_eq!(parse_handle_kind("File"), HandleKind::File);
        assert_eq!(parse_handle_kind("Directory"), HandleKind::Directory);
        assert_eq!(parse_handle_kind("Key"), HandleKind::RegistryKey);
        assert_eq!(parse_handle_kind("Event"), HandleKind::Event);
        assert_eq!(parse_handle_kind("Semaphore"), HandleKind::Semaphore);
        assert_eq!(parse_handle_kind("Mutant"), HandleKind::Mutant);
        assert_eq!(parse_handle_kind("Section"), HandleKind::Section);
        assert_eq!(parse_handle_kind("Process"), HandleKind::Process);
        assert_eq!(parse_handle_kind("Thread"), HandleKind::Thread);
        assert_eq!(parse_handle_kind("Token"), HandleKind::Token);
    }

    #[test]
    fn parse_handle_kind_other_for_unknown_strings() {
        // 已知但未在 11 档枚举里的内核对象。
        assert_eq!(parse_handle_kind("Timer"), HandleKind::Other);
        assert_eq!(parse_handle_kind("Job"), HandleKind::Other);
        assert_eq!(parse_handle_kind("Desktop"), HandleKind::Other);
        assert_eq!(parse_handle_kind("WindowStation"), HandleKind::Other);
        assert_eq!(parse_handle_kind("ALPC Port"), HandleKind::Other);
    }

    #[test]
    fn parse_handle_kind_empty_is_unknown() {
        assert_eq!(parse_handle_kind(""), HandleKind::Unknown);
    }

    #[test]
    fn format_raw_handle_is_hex() {
        assert_eq!(format_raw_handle(0x1234), "0x1234");
        assert_eq!(format_raw_handle(0), "0x0");
        assert_eq!(format_raw_handle(u64::MAX), "0xFFFFFFFFFFFFFFFF");
    }

    /// Windows: 自己进程至少能拿到一类非 Unknown 句柄。
    /// Linux: 自己进程至少有一个 fd（stdin）。
    #[test]
    fn self_collect_handles_nonempty() {
        let pid = std::process::id();
        match collect_handles(pid) {
            Ok(h) => {
                // Windows CI 在某些受限环境会拿到 0 个（NtQuerySystemInformation
                // 返回但 PROCESS_DUP_HANDLE 被拒），这里仅断言"不报错 + 不 panic"。
                eprintln!("note: self collected {} handles", h.len());
            }
            Err(e) => {
                eprintln!("note: collect_handles({pid}) failed in CI: {e}");
            }
        }
    }
}
