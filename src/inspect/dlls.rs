//! 已加载模块（Windows DLL / Linux .so）采集。
//!
//! - **Windows**：`CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid)`
//!   遍历 `MODULEENTRY32W`，与 `security/dll_check.rs` 同源。
//! - **Linux**：解析 `/proc/<pid>/maps`，按 path 合并多段映射（同一 .so 通常会有
//!   r-xp / r--p / rw-p 三段）。
//! - **macOS**：无 `/proc`，stub。

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

#[cfg(not(target_os = "windows"))]
pub fn collect_dlls(pid: u32) -> Result<Vec<DllInfo>> {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/maps");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ProcError::permission_denied_with(format!("读取 {path} 失败"), e))?;
        Ok(parse_proc_maps(&text))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Err(ProcError::permission_denied(
            "此平台（非 Linux/Windows）暂不支持模块列表采集",
        ))
    }
}

#[cfg(target_os = "linux")]
fn parse_proc_maps(text: &str) -> Vec<DllInfo> {
    use std::collections::BTreeMap;

    // 典型行：`7f8a1b2c3000-7f8a1b2e5000 r-xp 0000fe00 fd:00 12345  /usr/lib/libc.so.6`
    // 取首段（最低 base）作为代表，size 按 address-range 长度求和。
    let mut by_path: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for line in text.lines() {
        let (range, rest) = match line.split_once(char::is_whitespace) {
            Some(v) => v,
            None => continue,
        };
        let (start_s, end_s) = match range.split_once('-') {
            Some(v) => v,
            None => continue,
        };
        let (Ok(start), Ok(end)) = (
            u64::from_str_radix(start_s.trim(), 16),
            u64::from_str_radix(end_s.trim(), 16),
        ) else {
            continue;
        };

        // rest 形如 `r-xp 0000fe00 fd:00 12345  /usr/lib/libc.so.6`
        // 找到第一个 '/' —— 后面就是绝对路径（也覆盖 [heap]/[stack] 这种我们跳过的伪项）。
        let path_idx = match rest.find('/') {
            Some(i) => i,
            None => continue,
        };
        let module_path = rest[path_idx..].trim().to_string();
        if module_path.is_empty() {
            continue;
        }

        let span = end.saturating_sub(start);
        let entry = by_path.entry(module_path.clone()).or_insert((start, 0));
        if start < entry.0 {
            entry.0 = start;
        }
        entry.1 += span;
    }

    by_path
        .into_iter()
        .map(|(path, (base, size))| DllInfo {
            path,
            base_addr: base,
            size,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_dlls_nonempty() {
        let dlls = collect_dlls(std::process::id()).expect("self dlls");
        // 自己至少加载一个模块（自身可执行文件 + libc / kernel32 等）。
        assert!(!dlls.is_empty(), "expected ≥1 module, got {:?}", dlls);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_proc_maps_merges_segments() {
        let maps = "7f0000000000-7f0000001000 r-xp 00000000 fd:00 1  /usr/lib/libfoo.so\n\
                    7f0000001000-7f0000002000 r--p 00001000 fd:00 1  /usr/lib/libfoo.so\n\
                    7f0000002000-7f0000002100 rw-p 00002000 fd:00 1  /usr/lib/libfoo.so\n";
        let out = parse_proc_maps(maps);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].path, "/usr/lib/libfoo.so");
        assert_eq!(out[0].base_addr, 0x7f0000000000);
        assert_eq!(out[0].size, 0x2100);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_proc_maps_skips_anon() {
        // 匿名映射没有 path，应被跳过。
        let maps = "7f0000000000-7f0000001000 rw-p 00000000 00:00 0\n";
        let out = parse_proc_maps(maps);
        assert!(out.is_empty());
    }
}
