//! 已加载模块（Windows DLL）采集。
//!
//! - **Windows**：`CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)`
//!   遍历 `MODULEENTRY32W`，与 `security/dll_check.rs` 同源。

use crate::error::{ProcError, Result};

use super::DllInfo;

#[cfg(target_os = "windows")]
pub fn collect_dlls(pid: u32) -> Result<Vec<DllInfo>> {
    use std::collections::BTreeMap;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, TH32CS_SNAPMODULE,
        TH32CS_SNAPMODULE32,
    };

    let snapshot = unsafe {
        CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)
            .map_err(|e| ProcError::permission_denied_with("CreateToolhelp32Snapshot 失败", e))?
    };

    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };

    if unsafe { Module32FirstW(snapshot, &mut entry) }.is_err() {
        let _ = unsafe { CloseHandle(snapshot) };
        // 没有模块 = 进程可能已退出 / 权限不足；返回空 Vec 让上层显示「无数据」。
        return Ok(Vec::new());
    }

    // 同一 path 可能被多次枚举（rare but possible），用 BTreeMap 按 path 去重，
    // 保留首次出现的 base_addr；size 取 modBaseSize（字节数）。
    let mut by_path: BTreeMap<String, DllInfo> = BTreeMap::new();
    loop {
        let path = pcwstr_to_string(entry.szExePath.as_ptr());
        if !path.is_empty() {
            let base = entry.modBaseAddr as u64;
            let size = entry.modBaseSize as u64;
            by_path.entry(path.clone()).or_insert(DllInfo {
                path,
                base_addr: base,
                size,
            });
        }

        if unsafe { Module32NextW(snapshot, &mut entry) }.is_err() {
            break;
        }
    }

    let _ = unsafe { CloseHandle(snapshot) };
    Ok(by_path.into_values().collect())
}

#[cfg(target_os = "windows")]
fn pcwstr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn self_dlls_nonempty() {
        let dlls = super::collect_dlls(std::process::id()).expect("self dlls");
        // 自己至少加载一个模块（自身可执行文件 + kernel32 等）。
        assert!(!dlls.is_empty(), "expected ≥1 module, got {:?}", dlls);
    }
}
