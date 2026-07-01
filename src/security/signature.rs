use super::score::{RiskCategory, RiskFactor};

/// v0.11.0 阶段 1：加 `Pending` 变体作为 `#[default]`，用于 `ProcessInfo::signature_status`
/// 字段骨架。阶段 4 由 `BackgroundScorer` 调 `verify_signature` 异步填实为其他变体。
/// `Unknown` 保留为「verify_signature 因非 elevated 等原因无法判定」的真实结果，
/// 与 `Pending`（尚未触发验证）语义不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SignatureStatus {
    #[default]
    Pending,
    Signed,
    Trusted,
    Unsigned,
    Revoked,
    Unknown,
}

impl std::fmt::Display for SignatureStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "未验证"),
            Self::Trusted => write!(f, "受信签名"),
            Self::Signed => write!(f, "已签名"),
            Self::Unsigned => write!(f, "无签名"),
            Self::Revoked => write!(f, "签名已吊销"),
            Self::Unknown => write!(f, "未知"),
        }
    }
}

impl SignatureStatus {
    /// v0.11 阶段 4（ADR-0021）：进程列表 name 后追加的签名状态 emoji。
    /// 与 v0.7 EcoQoS 🍃 同款规则——「无信息可显」的状态返回空串，避免列宽波动：
    /// - `Pending` → 空（默认值，启动后头 1-2 个 heavy refresh 内全部 Pending）
    /// - `Signed` → 空（已签名但非受信 CA，避免噪音；显示「已签名」反而让用户误以为安全）
    /// - `Trusted` → 🔒
    /// - `Unsigned` / `Revoked` → ⚠（与 stage-4.md 原方案 `Untrusted → ⚠` 对齐）
    /// - `Unknown` → ❓
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Trusted => " \u{1F512}",                         // 🔒
            Self::Unsigned | Self::Revoked => " \u{26A0}\u{FE0F}", // ⚠️
            Self::Unknown => " \u{2753}",                          // ❓
            Self::Pending | Self::Signed => "",
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

#[must_use]
pub fn is_trusted_signer(subject: &str) -> bool {
    let subject_lower = subject.to_lowercase();
    TRUSTED_SIGNERS
        .iter()
        .any(|s| subject_lower.contains(&s.to_lowercase()))
}

/// TRUST_E_SUBJECT_NOT_SIGNED — the subject has no signature
const TRUST_E_SUBJECT_NOT_SIGNED: i32 = 0x800B0100_u32 as i32;
/// CRYPT_E_REVOKED — the signature certificate has been revoked
const CRYPT_E_REVOKED: i32 = 0x80092010_u32 as i32;

/// 把 `WinVerifyTrust` 返回的 `HRESULT` 映射到 `SignatureStatus`。
///
/// v0.11 阶段 4 抽出的纯函数（ADR-0021）：原 `verify_signature` 内联了这块逻辑，
/// 单元测试无法注入 mock result code。返回值约定：
/// - `0` → `Signed`（调用方在此基础上再判定是否升级为 `Trusted`）
/// - `TRUST_E_SUBJECT_NOT_SIGNED` → `Unsigned`
/// - `CRYPT_E_REVOKED` → `Revoked`
/// - 其他非零 → `Unknown`（含链断裂 / 过期 / API 错误）
#[must_use]
pub fn from_wintrust_result(result: i32) -> SignatureStatus {
    if result == 0 {
        SignatureStatus::Signed
    } else if result == TRUST_E_SUBJECT_NOT_SIGNED {
        SignatureStatus::Unsigned
    } else if result == CRYPT_E_REVOKED {
        SignatureStatus::Revoked
    } else {
        SignatureStatus::Unknown
    }
}

/// 内部可注入入口：用 caller 提供的 result code（绕过真实 `WinVerifyTrust` 调用）
/// 推导签名状态。`policy_override = None` 走真实 WinVerifyTrust；`Some(hresult)`
/// 走 mock 路径，专供单元测试验证状态机映射。
///
/// 与 stage-4.md 原方案的「verify_signature_with_policy(path, policy_override)」对齐：
/// - 真实路径：调 WinVerifyTrust → `from_wintrust_result` → 必要时升级为 `Trusted`
/// - mock 路径：直接 `from_wintrust_result(policy_override)`，不读文件
///
/// Trusted 升级规则（仅真实路径触发）：返回 `Signed` 时读 `CompanyName` 版本信息
/// 资源，命中 `TRUSTED_SIGNERS` 列表则升级为 `Trusted`。mock 路径不模拟 company
/// 查询，因此 `policy_override = Some(0)` 严格返回 `Signed`。
pub(crate) fn verify_signature_with_policy(
    exe_path: &str,
    policy_override: Option<i32>,
) -> SignatureStatus {
    #[cfg(target_os = "windows")]
    {
        if let Some(result) = policy_override {
            return from_wintrust_result(result);
        }

        use windows::Win32::Security::WinTrust::{
            WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_UNION_CHOICE,
            WINTRUST_FILE_INFO, WinVerifyTrust,
        };
        use windows::core::PCWSTR;

        if !crate::collect::is_elevated() {
            return SignatureStatus::Unknown;
        }

        let path_wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            let mut file_info = WINTRUST_FILE_INFO {
                cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
                pcwszFilePath: PCWSTR(path_wide.as_ptr()),
                ..Default::default()
            };

            let mut trust_data = WINTRUST_DATA {
                cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
                dwUnionChoice: WINTRUST_DATA_UNION_CHOICE(1),
                ..Default::default()
            };
            trust_data.Anonymous.pFile = &mut file_info;

            let mut action_id = WINTRUST_ACTION_GENERIC_VERIFY_V2;
            let result = WinVerifyTrust(None, &mut action_id, &mut trust_data as *mut _ as *mut _);

            let status = from_wintrust_result(result);
            if matches!(status, SignatureStatus::Signed)
                && let Some(company) = get_file_company_name(exe_path)
                && is_trusted_signer(&company)
            {
                return SignatureStatus::Trusted;
            }
            if matches!(status, SignatureStatus::Unknown) {
                tracing::debug!(
                    "WinVerifyTrust unknown error 0x{:08X} for {}",
                    result as u32,
                    exe_path
                );
            }
            status
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // 非 Windows 上：真实路径无法调用 WinVerifyTrust，统一返回 Unknown；
        // mock 路径仍走 from_wintrust_result，让跨平台 CI 可以验证状态机。
        if let Some(result) = policy_override {
            return from_wintrust_result(result);
        }
        let _ = exe_path;
        SignatureStatus::Unknown
    }
}

pub fn verify_signature(exe_path: &str) -> SignatureStatus {
    verify_signature_with_policy(exe_path, None)
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
        GetFileVersionInfoW(
            pcwsz_path,
            0,
            size,
            buf.as_mut_ptr() as *mut std::ffi::c_void,
        )
        .ok()?;

        // Query CompanyName
        let sub_block: Vec<u16> = "\\CompanyName"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
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

#[must_use]
pub fn signature_risk_factor(status: SignatureStatus) -> Option<RiskFactor> {
    match status {
        // v0.11.0 阶段 1：Pending 表示尚未触发验证（ProcessInfo 默认值），
        // 不出 risk_factor —— 等 BackgroundScorer 阶段 4 真实填入结果再评分。
        SignatureStatus::Pending => None,
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
        SignatureStatus::Unknown => {
            // v0.11 阶段 8 REVIEW-13 P1-3：非 Windows 平台没有 WinVerifyTrust，
            // verify_signature_with_policy 永远返 Unknown，所有进程都扣 5 分会让
            // Linux/macOS 用户看到全部进程被标红。ADR-0021 §Consequences 明确
            // Windows 非 elevated 也返 Unknown 扣 5 分（设计行为），但 Linux/macOS
            // 根本没有 WinVerifyTrust 概念，扣分无意义 → cfg-gate 不扣分。
            // 同时 P2-15：把 _ 通配符改为显式 Trusted，让未来加新变体时编译器
            // 强制更新 match（避免新变体静默落入 _ 桶）。
            #[cfg(not(target_os = "windows"))]
            {
                None
            }
            #[cfg(target_os = "windows")]
            {
                Some(RiskFactor {
                    category: RiskCategory::Signature,
                    name: "signature_unverified".to_string(),
                    weight: 5,
                    description: "签名未验证（需管理员权限）".to_string(),
                })
            }
        }
        // P2-15：Trusted 显式列出（不扣分），避免未来加新 SignatureStatus 变体时
        // 静默落入 _ 通配符桶（编译器强制穷尽检查）。
        SignatureStatus::Trusted => None,
    }
}

#[cfg(test)]
mod tests {
    //! v0.11.0 阶段 4（ADR-0021）—— `verify_signature_with_policy` mock policy
    //! 注入路径测试。
    //!
    //! `verify_signature_with_policy` 是 `pub(crate)`，集成测试 crate 看不到，
    //! 因此 mock policy 行为在本模块内验证。`from_wintrust_result` 是 pure
    //! function 公开 API，状态机映射的细粒度断言在 `tests/test_signature.rs`。
    use super::*;

    const TRUST_E_SUBJECT_NOT_SIGNED_TEST: i32 = 0x800B0100_u32 as i32;
    const CRYPT_E_REVOKED_TEST: i32 = 0x80092010_u32 as i32;

    /// `policy_override = Some(0)` 在所有平台上返回 `Signed`（mock 路径不调
    /// WinVerifyTrust，不读 CompanyName，因此不升级 Trusted）。
    #[test]
    fn mock_policy_success_returns_signed() {
        assert_eq!(
            verify_signature_with_policy("C:\\ignore.exe", Some(0)),
            SignatureStatus::Signed,
        );
        assert_eq!(
            verify_signature_with_policy("/nonexistent/path", Some(0)),
            SignatureStatus::Signed,
        );
    }

    #[test]
    fn mock_policy_not_signed_returns_unsigned() {
        assert_eq!(
            verify_signature_with_policy("ignored", Some(TRUST_E_SUBJECT_NOT_SIGNED_TEST)),
            SignatureStatus::Unsigned,
        );
    }

    #[test]
    fn mock_policy_revoked_returns_revoked() {
        assert_eq!(
            verify_signature_with_policy("ignored", Some(CRYPT_E_REVOKED_TEST)),
            SignatureStatus::Revoked,
        );
    }

    #[test]
    fn mock_policy_unknown_hresult_returns_unknown() {
        assert_eq!(
            verify_signature_with_policy("ignored", Some(0x800B0101_u32 as i32)),
            SignatureStatus::Unknown,
        );
    }

    /// `policy_override = None` 是真实路径，跨平台行为：
    /// - 非 Windows：无法调 WinVerifyTrust，返回 Unknown。
    /// - Windows：依赖运行环境（admin / 文件存在），不强断言具体状态。
    #[test]
    fn real_path_returns_unknown_on_non_windows() {
        // 只在非 Windows 上断言 stub 行为；Windows 上仅确保不 panic。
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(
                verify_signature_with_policy("/bin/ls", None),
                SignatureStatus::Unknown,
            );
        }
        #[cfg(target_os = "windows")]
        {
            let _ = verify_signature_with_policy("C:\\nonexistent\\path.exe", None);
        }
    }
}
