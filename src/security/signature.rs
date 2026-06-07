use super::score::{RiskCategory, RiskFactor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureStatus {
    Signed,
    Trusted,
    Unsigned,
    Revoked,
    Unknown,
}

impl std::fmt::Display for SignatureStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trusted => write!(f, "受信签名"),
            Self::Signed => write!(f, "已签名"),
            Self::Unsigned => write!(f, "无签名"),
            Self::Revoked => write!(f, "签名已吊销"),
            Self::Unknown => write!(f, "未知"),
        }
    }
}

const TRUSTED_SIGNERS: &[&str] = &[
    "Microsoft Corporation",
    "Microsoft Windows",
    "Google LLC",
    "Google Inc",
    "Mozilla Corporation",
    "Apple Inc.",
    "Intel Corporation",
    "NVIDIA Corporation",
];

pub fn is_trusted_signer(subject: &str) -> bool {
    let subject_lower = subject.to_lowercase();
    TRUSTED_SIGNERS
        .iter()
        .any(|s| subject_lower.contains(&s.to_lowercase()))
}

pub fn verify_signature(exe_path: &str) -> SignatureStatus {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Security::WinTrust::{
            WinVerifyTrust, WINTRUST_DATA, WINTRUST_DATA_UNION_CHOICE,
            WINTRUST_FILE_INFO, WINTRUST_ACTION_GENERIC_VERIFY_V2,
        };
        use windows::core::PCWSTR;

        if !crate::collect::is_elevated() {
            return SignatureStatus::Unknown;
        }

        let path_wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            let mut file_info = WINTRUST_FILE_INFO::default();
            file_info.cbStruct = std::mem::size_of::<WINTRUST_FILE_INFO>() as u32;
            file_info.pcwszFilePath = PCWSTR(path_wide.as_ptr());

            let mut trust_data = WINTRUST_DATA::default();
            trust_data.cbStruct = std::mem::size_of::<WINTRUST_DATA>() as u32;
            trust_data.dwUnionChoice = WINTRUST_DATA_UNION_CHOICE(1);
            trust_data.Anonymous.pFile = &mut file_info;

            let mut action_id = WINTRUST_ACTION_GENERIC_VERIFY_V2;
            let result = WinVerifyTrust(
                None,
                &mut action_id,
                &mut trust_data as *mut _ as *mut _,
            );

            if result == 0 {
                // Signed — check company name from version info for trusted matching
                if let Some(company) = get_file_company_name(exe_path) {
                    if is_trusted_signer(&company) {
                        return SignatureStatus::Trusted;
                    }
                }
                SignatureStatus::Signed
            } else if result == (0x800B0100_u32 as i32) {
                // TRUST_E_SUBJECT_NOT_SIGNED
                SignatureStatus::Unsigned
            } else if result == (0x80092010_u32 as i32) {
                // CRYPT_E_REVOKED
                SignatureStatus::Revoked
            } else {
                SignatureStatus::Signed
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = exe_path;
        SignatureStatus::Unknown
    }
}

/// Extract the CompanyName from file version info resource.
/// Much simpler than extracting the certificate subject from the PKCS#7 blob.
#[cfg(target_os = "windows")]
fn get_file_company_name(exe_path: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    use windows::core::PCWSTR;

    let path_wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
    let pcwsz_path = PCWSTR(path_wide.as_ptr());

    unsafe {
        let mut handle = 0u32;
        let size = GetFileVersionInfoSizeW(pcwsz_path, Some(&mut handle));
        if size == 0 {
            return None;
        }

        let mut buf = vec![0u8; size as usize];
        GetFileVersionInfoW(pcwsz_path, 0, size, buf.as_mut_ptr() as *mut std::ffi::c_void).ok()?;

        // Query CompanyName
        let sub_block: Vec<u16> = "\\CompanyName".encode_utf16().chain(std::iter::once(0)).collect();
        let mut value_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut value_len: u32 = 0;

        let ok = VerQueryValueW(
            buf.as_ptr() as *const std::ffi::c_void,
            PCWSTR(sub_block.as_ptr()),
            &mut value_ptr,
            &mut value_len,
        );

        if ok.as_bool() && !value_ptr.is_null() && value_len > 0 {
            let wchar_ptr = value_ptr as *const u16;
            let slice = std::slice::from_raw_parts(wchar_ptr, value_len as usize);
            // value_len includes the null terminator
            let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
            return Some(String::from_utf16_lossy(&slice[..end]));
        }

        None
    }
}

pub fn signature_risk_factor(status: SignatureStatus) -> Option<RiskFactor> {
    match status {
        SignatureStatus::Unsigned => Some(RiskFactor {
            category: RiskCategory::Signature,
            name: "unsigned".to_string(),
            weight: 20,
            description: "无数字签名".to_string(),
        }),
        SignatureStatus::Revoked => Some(RiskFactor {
            category: RiskCategory::Signature,
            name: "revoked".to_string(),
            weight: 35,
            description: "签名已被吊销".to_string(),
        }),
        SignatureStatus::Signed => Some(RiskFactor {
            category: RiskCategory::Signature,
            name: "untrusted_sig".to_string(),
            weight: 10,
            description: "签名但不受信".to_string(),
        }),
        _ => None,
    }
}
