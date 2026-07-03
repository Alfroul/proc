use super::score::{RiskCategory, RiskFactor};
use super::trusted_signers::TrustedSignersRule;

/// v0.11.0 阶段 1：加 `Pending` 变体作为 `#[default]`，用于 `ProcessInfo::signature_status`
/// 字段骨架。阶段 4 由 `BackgroundScorer` 调 `verify_signature` 异步填实为其他变体。
/// `Unknown` 保留为「verify_signature 因非 elevated 等原因无法判定」的真实结果，
/// 与 `Pending`（尚未触发验证）语义不同。
///
/// v0.12.0 阶段 3：扩 3 个新变体（TD-26）覆盖更细粒度的 HRESULT 状态机——
/// - `Expired`：证书过期（CERT_E_EXPIRED）—— 曾经受信但已过期
/// - `UntrustedRoot`：不受信根（CERT_E_UNTRUSTEDROOT）—— 自签名 / 缺失中间证书
/// - `ChainError`：证书链断裂（CERT_E_CHAINING / CERT_E_WRONG_NAME /
///   TRUST_E_CERT_SIGNATURE）—— 验证不完整
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SignatureStatus {
    #[default]
    Pending,
    Signed,
    Trusted,
    Unsigned,
    Revoked,
    Expired,
    UntrustedRoot,
    ChainError,
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
            Self::Expired => write!(f, "证书过期"),
            Self::UntrustedRoot => write!(f, "不受信根"),
            Self::ChainError => write!(f, "证书链断裂"),
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
    /// - `Expired` / `UntrustedRoot` → ⚠（v0.12 阶段 3：有签名但有问题，与
    ///   Unsigned/Revoked 同档让用户立即看到红旗）
    /// - `ChainError` → ❓（与 Unknown 同档——验证不完整，没有足够信息判定）
    /// - `Unknown` → ❓
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Trusted => " \u{1F512}", // 🔒
            Self::Unsigned | Self::Revoked | Self::Expired | Self::UntrustedRoot => {
                " \u{26A0}\u{FE0F}"
            } // ⚠️
            Self::Unknown | Self::ChainError => " \u{2753}", // ❓
            Self::Pending | Self::Signed => "",
        }
    }
}

/// 内置受信签名 vendor 列表（v0.12 阶段 3 扩，TD-27）。
///
/// 子串大小写不敏感匹配 `CompanyName` 版本资源字段。命中即升级 Signed → Trusted。
/// 用户在 `~/.config/proc/trusted_signers.toml` 配置的 rules 会**追加**到这个列表
/// （不替换），让用户能标记自家应用 vendor。
const TRUSTED_SIGNERS: &[&str] = &[
    // 操作系统 / Hypervisor 厂商
    "Microsoft Corporation",
    "Microsoft Windows",
    "Red Hat, Inc.",
    "Canonical Ltd.",
    // 浏览器 / 搜索厂商
    "Google LLC",
    "Google Inc",
    "Mozilla Corporation",
    "Apple Inc.",
    // 硬件厂商
    "Intel Corporation",
    "NVIDIA Corporation",
    "Advanced Micro Devices, Inc.",
    "VMware, Inc.",
    "Cisco Systems, Inc.",
    "Oracle Corporation",
    // 应用平台
    "Adobe Inc.",
    "Adobe Systems Incorporated",
    "Docker Inc.",
    "GitHub, Inc.",
    "Python Software Foundation",
    "Apache Software Foundation",
    "OpenJS Foundation",
    "Electron.js",
];

#[must_use]
pub fn is_trusted_signer(subject: &str) -> bool {
    let subject_lower = subject.to_lowercase();
    TRUSTED_SIGNERS
        .iter()
        .any(|s| subject_lower.contains(&s.to_lowercase()))
}

// ── WinVerifyTrust HRESULT 常量（v0.12 阶段 3 扩 TD-26） ────────────────────
/// TRUST_E_SUBJECT_NOT_SIGNED — the subject has no signature
const TRUST_E_SUBJECT_NOT_SIGNED: i32 = 0x800B0100_u32 as i32;
/// CRYPT_E_REVOKED — the signature certificate has been revoked
const CRYPT_E_REVOKED: i32 = 0x80092010_u32 as i32;
/// CERT_E_EXPIRED — the required certificate is not within its validity period
const CERT_E_EXPIRED: i32 = 0x800B0101_u32 as i32;
/// CERT_E_UNTRUSTEDROOT — a certification chain processed correctly but terminated
/// in a root certificate not trusted by the trust provider
const CERT_E_UNTRUSTEDROOT: i32 = 0x800B0109_u32 as i32;
/// CERT_E_CHAINING — a chain of certificates was not correctly created
const CERT_E_CHAINING: i32 = 0x800B010A_u32 as i32;
/// CERT_E_WRONG_NAME — the CN in the certificate does not match the subject
const CERT_E_WRONG_NAME: i32 = 0x800B0113_u32 as i32;
/// TRUST_E_CERT_SIGNATURE — the signature of the certificate cannot be verified
const TRUST_E_CERT_SIGNATURE: i32 = 0x80096010_u32 as i32;

/// 把 `WinVerifyTrust` 返回的 `HRESULT` 映射到 `SignatureStatus`。
///
/// v0.11 阶段 4 抽出的纯函数（ADR-0021）：原 `verify_signature` 内联了这块逻辑，
/// 单元测试无法注入 mock result code。返回值约定：
/// - `0` → `Signed`（调用方在此基础上再判定是否升级为 `Trusted`）
/// - `TRUST_E_SUBJECT_NOT_SIGNED` → `Unsigned`
/// - `CRYPT_E_REVOKED` → `Revoked`
/// - `CERT_E_EXPIRED` → `Expired`（v0.12 阶段 3 加）
/// - `CERT_E_UNTRUSTEDROOT` → `UntrustedRoot`（v0.12 阶段 3 加）
/// - `CERT_E_CHAINING` / `CERT_E_WRONG_NAME` / `TRUST_E_CERT_SIGNATURE` → `ChainError`
///   （v0.12 阶段 3 加；名称不匹配 / 签名无效归类为链问题）
/// - 其他非零 → `Unknown`（含 API 错误 / 未知 HRESULT）
#[must_use]
pub fn from_wintrust_result(result: i32) -> SignatureStatus {
    if result == 0 {
        SignatureStatus::Signed
    } else if result == TRUST_E_SUBJECT_NOT_SIGNED {
        SignatureStatus::Unsigned
    } else if result == CRYPT_E_REVOKED {
        SignatureStatus::Revoked
    } else if result == CERT_E_EXPIRED {
        SignatureStatus::Expired
    } else if result == CERT_E_UNTRUSTEDROOT {
        SignatureStatus::UntrustedRoot
    } else if matches!(
        result,
        CERT_E_CHAINING | CERT_E_WRONG_NAME | TRUST_E_CERT_SIGNATURE
    ) {
        SignatureStatus::ChainError
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
/// 资源，命中**内置 `TRUSTED_SIGNERS`** 或 **用户配置 `trusted_rules`** 任一规则
/// 则升级为 `Trusted`（v0.12 阶段 3 加用户 rules）。mock 路径不模拟 company
/// 查询，因此 `policy_override = Some(0)` 严格返回 `Signed`。
pub(crate) fn verify_signature_with_policy(
    exe_path: &str,
    policy_override: Option<i32>,
    trusted_rules: &[TrustedSignersRule],
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
                && (is_trusted_signer(&company)
                    || super::trusted_signers::matches_any_rule(&company, trusted_rules))
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
        // 非 Windows 无 WinVerifyTrust API，仅走 mock policy 路径。
        if let Some(result) = policy_override {
            return from_wintrust_result(result);
        }
        // 真实路径无 API 可调，返回 Unknown（非 elevated 等价语义）。
        let _ = (exe_path, trusted_rules);
        SignatureStatus::Unknown
    }
}

pub fn verify_signature(exe_path: &str) -> SignatureStatus {
    verify_signature_with_policy(exe_path, None, &[])
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
        // v0.12 阶段 3：3 个新变体（TD-26）—— 比 Unsigned 轻微（曾经受信），
        // 比 Signed 严重（有签名但状态有问题）。RiskFactor.name 用独立字符串
        // 让用户能在 Inspector / JSON 里看到具体状态。
        SignatureStatus::Expired => Some(RiskFactor {
            category: RiskCategory::Signature,
            name: "expired".to_string(),
            weight: 15,
            description: "证书已过期".to_string(),
        }),
        SignatureStatus::UntrustedRoot => Some(RiskFactor {
            category: RiskCategory::Signature,
            name: "untrusted_root".to_string(),
            weight: 15,
            description: "不受信根证书".to_string(),
        }),
        SignatureStatus::ChainError => Some(RiskFactor {
            category: RiskCategory::Signature,
            name: "chain_error".to_string(),
            weight: 10,
            description: "证书链断裂".to_string(),
        }),
        SignatureStatus::Unknown => {
            // 非 elevated 时 verify_signature 返 Unknown，扣 5 分（设计行为）。
            // P2-15：把 _ 通配符改为显式 Trusted，让未来加新变体时编译器
            // 强制更新 match（避免新变体静默落入 _ 桶）。
            Some(RiskFactor {
                category: RiskCategory::Signature,
                name: "signature_unverified".to_string(),
                weight: 5,
                description: "签名未验证（需管理员权限）".to_string(),
            })
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
    const CERT_E_EXPIRED_TEST: i32 = 0x800B0101_u32 as i32;
    const CERT_E_UNTRUSTEDROOT_TEST: i32 = 0x800B0109_u32 as i32;
    const CERT_E_CHAINING_TEST: i32 = 0x800B010A_u32 as i32;

    /// `policy_override = Some(0)` 在所有平台上返回 `Signed`（mock 路径不调
    /// WinVerifyTrust，不读 CompanyName，因此不升级 Trusted）。
    #[test]
    fn mock_policy_success_returns_signed() {
        assert_eq!(
            verify_signature_with_policy("C:\\ignore.exe", Some(0), &[]),
            SignatureStatus::Signed,
        );
        assert_eq!(
            verify_signature_with_policy("/nonexistent/path", Some(0), &[]),
            SignatureStatus::Signed,
        );
    }

    #[test]
    fn mock_policy_not_signed_returns_unsigned() {
        assert_eq!(
            verify_signature_with_policy("ignored", Some(TRUST_E_SUBJECT_NOT_SIGNED_TEST), &[]),
            SignatureStatus::Unsigned,
        );
    }

    #[test]
    fn mock_policy_revoked_returns_revoked() {
        assert_eq!(
            verify_signature_with_policy("ignored", Some(CRYPT_E_REVOKED_TEST), &[]),
            SignatureStatus::Revoked,
        );
    }

    #[test]
    fn mock_policy_expired_returns_expired() {
        // v0.12 阶段 3 TD-26：CERT_E_EXPIRED 现在精确映射到 Expired（之前落入 Unknown）。
        assert_eq!(
            verify_signature_with_policy("ignored", Some(CERT_E_EXPIRED_TEST), &[]),
            SignatureStatus::Expired,
        );
    }

    #[test]
    fn mock_policy_untrusted_root_returns_untrusted_root() {
        // v0.12 阶段 3 TD-26：CERT_E_UNTRUSTEDROOT 现在精确映射到 UntrustedRoot。
        assert_eq!(
            verify_signature_with_policy("ignored", Some(CERT_E_UNTRUSTEDROOT_TEST), &[]),
            SignatureStatus::UntrustedRoot,
        );
    }

    #[test]
    fn mock_policy_chaining_returns_chain_error() {
        // v0.12 阶段 3 TD-26：CERT_E_CHAINING 现在精确映射到 ChainError。
        assert_eq!(
            verify_signature_with_policy("ignored", Some(CERT_E_CHAINING_TEST), &[]),
            SignatureStatus::ChainError,
        );
    }

    #[test]
    fn mock_policy_unknown_hresult_returns_unknown() {
        // 0x80004005 = E_FAIL，未映射的 HRESULT 仍落入 Unknown。
        assert_eq!(
            verify_signature_with_policy("ignored", Some(0x80004005_u32 as i32), &[]),
            SignatureStatus::Unknown,
        );
    }

    /// `policy_override = None` 是真实路径，依赖运行环境（admin / 文件存在），
    /// 不强断言具体状态，仅确保不 panic。
    #[test]
    fn real_path_does_not_panic() {
        let _ = verify_signature_with_policy("C:\\nonexistent\\path.exe", None, &[]);
    }
}
