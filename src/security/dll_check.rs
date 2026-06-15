use super::score::{RiskCategory, RiskFactor};

#[derive(Debug, Clone)]
pub struct DllInfo {
    pub path: String,
    pub suspicious: Option<RiskFactor>,
}

/// Enumerate loaded DLLs for a process and check for suspicious modules.
/// Returns a list of suspicious DLL risk factors (empty if none found).
pub fn check_loaded_dlls(pid: u32) -> Vec<RiskFactor> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
            TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
        };
        let snapshot =
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };

        let Ok(snapshot) = snapshot else {
            return Vec::new();
        };

        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };

        let mut factors = Vec::new();
        let mut dll_count = 0u32;

        let ok = unsafe { Module32FirstW(snapshot, &mut entry) };
        if ok.is_err() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(snapshot);
            }
            return Vec::new();
        }

        loop {
            dll_count += 1;

            // Only check up to 200 DLLs per process
            if dll_count > 200 {
                break;
            }

            let dll_path = pcwstr_to_string(entry.szExePath.as_ptr());
            let dll_path_lower = dll_path.to_lowercase();

            // Skip Windows system DLLs — too many false positives
            if dll_path_lower.starts_with("c:\\windows\\")
                && (dll_path_lower.contains("\\system32\\")
                    || dll_path_lower.contains("\\syswow64\\")
                    || dll_path_lower.contains("\\winsxs\\"))
            {
                let ok = unsafe { Module32NextW(snapshot, &mut entry) };
                if ok.is_err() {
                    break;
                }
                continue;
            }

            // Check: DLL in user-writable path
            let user_writable = (dll_path_lower.contains("\\users\\")
                && !dll_path_lower.contains("\\program files"))
                || dll_path_lower.contains("\\temp\\")
                || dll_path_lower.contains("\\appdata\\local\\temp")
                || dll_path_lower.contains("\\downloads");

            if user_writable && dll_count <= 50 {
                // Only report first few suspicious DLLs to avoid noise
                factors.push(RiskFactor {
                    category: RiskCategory::FilePath,
                    name: "suspicious_dll".to_string(),
                    weight: 10,
                    description: format!("加载可疑 DLL: {}", truncate_path(&dll_path, 60)),
                });
            }

            let ok = unsafe { Module32NextW(snapshot, &mut entry) };
            if ok.is_err() {
                break;
            }
        }

        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        }

        // Cap at 3 DLL findings per process to avoid score explosion
        factors.truncate(3);
        factors
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = pid;
        Vec::new()
    }
}

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

fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        let start = path.floor_char_boundary(path.len().saturating_sub(max_len).saturating_sub(3));
        format!("...{}", &path[start..])
    }
}
