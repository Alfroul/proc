use super::score::{RiskCategory, RiskFactor};

const SYSTEM_USER_MARKERS: &[&str] = &["system", "localsystem", "network service", "local service"];

/// Dangerous privilege LUID values (low, high) → description.
/// Using numeric LUIDs instead of string names avoids AV false positives.
const DANGEROUS_LUIDS: &[(u32, u32, &str)] = &[
    (20, 0, "高危调试特权"),
    (10, 0, "高危驱动加载特权"),
    (9, 0, "高危所有权特权"),
];

/// Check if a process holds dangerous privilege tokens.
/// Only meaningful for non-system processes — system processes naturally hold these.
#[must_use]
pub fn check_privilege_tokens(pid: u32, user_id: Option<&str>) -> Option<RiskFactor> {
    // If running as SYSTEM/service, these privileges are expected
    if let Some(user) = user_id {
        let user_lower = user.to_lowercase();
        if SYSTEM_USER_MARKERS.iter().any(|m| user_lower.contains(m)) {
            return None;
        }
    }

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TokenPrivileges};
        use windows::Win32::System::Threading::{
            OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) };
        let Ok(process) = process else { return None };

        let mut token = HANDLE::default();
        let result = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
        if result.is_err() {
            unsafe {
                let _ = CloseHandle(process);
            }
            return None;
        }

        // Get required buffer size first
        let mut buf_len = 0u32;
        unsafe {
            GetTokenInformation(token, TokenPrivileges, None, 0, &mut buf_len)
                .ok()
                .unwrap_or(());
        }

        if buf_len == 0 || buf_len > 65536 {
            unsafe {
                let _ = CloseHandle(token);
                let _ = CloseHandle(process);
            }
            return None;
        }

        let mut buf = vec![0u8; buf_len as usize];
        let result = unsafe {
            GetTokenInformation(
                token,
                TokenPrivileges,
                Some(buf.as_mut_ptr() as *mut _),
                buf_len,
                &mut buf_len,
            )
        };

        unsafe {
            let _ = CloseHandle(token);
        }
        unsafe {
            let _ = CloseHandle(process);
        }

        if result.is_err() {
            return None;
        }

        // Parse TOKEN_PRIVILEGES
        let priv_count = u32::from_le_bytes(buf[0..4].try_into().unwrap_or([0; 4])) as usize;
        let mut found_dangerous = Vec::new();

        // TOKEN_PRIVILEGES: { DWORD PrivilegeCount; LUID_AND_ATTRIBUTES Privileges[] }
        // LUID_AND_ATTRIBUTES: { LUID (LowPart u32 + HighPart i32 = 8 bytes); DWORD Attributes = 4 bytes }
        // → 每个 entry 12 字节，数组从 offset 4 起。早期实现误用 8 字节步长，
        //   导致除首项外全部读错位，特权检测近乎失效。
        const LUID_AND_ATTRIBUTES_SIZE: usize = 12;
        for i in 0..priv_count.min(100) {
            let offset = 4 + i * LUID_AND_ATTRIBUTES_SIZE;
            if offset + LUID_AND_ATTRIBUTES_SIZE > buf.len() {
                break;
            }

            let low = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap_or([0; 4]));
            let high = u32::from_le_bytes(buf[offset + 4..offset + 8].try_into().unwrap_or([0; 4]));

            // Direct LUID match — no privilege name strings needed
            if let Some((_, _, desc)) = DANGEROUS_LUIDS
                .iter()
                .find(|(l, h, _)| *l == low && *h == high)
            {
                found_dangerous.push(*desc);
            }
        }

        if found_dangerous.is_empty() {
            return None;
        }

        Some(RiskFactor {
            category: RiskCategory::Privilege,
            name: "dangerous_privilege".to_string(),
            weight: 15,
            description: format!("持有高危特权: {}", found_dangerous.join(", ")),
        })
    }
}
