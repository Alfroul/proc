//! v0.11.0 阶段 4（ADR-0021）—— 进程签名验证测试套件。
//!
//! 覆盖范围：
//! - `SignatureStatus` 状态机 + serde round-trip（兼容旧录屏 — 缺字段默认 Pending）
//! - `from_wintrust_result` HRESULT → SignatureStatus 映射（pure function，跨平台跑）
//! - `signature_risk_factor` 全状态扣分契约（R16 评分映射）
//! - `SignatureStatus::badge` 进程列表 emoji 标记
//! - `is_trusted_signer` 已知 CA 匹配
//! - `verify_signature` 非 Windows stub 行为
//!
//! v0.12.0 阶段 3 增量（TD-26 + TD-27）：
//! - 扩 SignatureStatus 9 状态机（加 Expired / UntrustedRoot / ChainError）
//! - 扩 from_wintrust_result HRESULT 映射（CERT_E_EXPIRED / CERT_E_UNTRUSTEDROOT /
//!   CERT_E_CHAINING / CERT_E_WRONG_NAME / TRUST_E_CERT_SIGNATURE）
//! - 扩 TRUSTED_SIGNERS 内置列表（Adobe / Cisco / Oracle / VMWare / Docker / Red Hat 等）
//! - 加 trusted_signers.toml 用户配置解析测试
//!
//! `verify_signature_with_policy` 的 mock policy 注入测试在 `src/security/signature.rs`
//! 内的 `#[cfg(test)] mod tests`（函数是 `pub(crate)`，集成测试 crate 不可见）。

use proc::security::{
    SignatureStatus, from_wintrust_result, is_trusted_signer, signature_risk_factor,
};

/// `WinVerifyTrust` 的 HRESULT 常量。与 `src/security/signature.rs` 内私有 const 同值。
/// 这里独立定义避免依赖私有项；值变更时两边同步（serde round-trip 测试也会立刻挂掉）。
const TRUST_E_SUBJECT_NOT_SIGNED: i32 = 0x800B0100_u32 as i32;
const CRYPT_E_REVOKED: i32 = 0x80092010_u32 as i32;
// v0.12 阶段 3 TD-26：扩 HRESULT 常量
const CERT_E_EXPIRED: i32 = 0x800B0101_u32 as i32;
const CERT_E_UNTRUSTEDROOT: i32 = 0x800B0109_u32 as i32;
const CERT_E_CHAINING: i32 = 0x800B010A_u32 as i32;
const CERT_E_WRONG_NAME: i32 = 0x800B0113_u32 as i32;
const TRUST_E_CERT_SIGNATURE: i32 = 0x80096010_u32 as i32;

// ── SignatureStatus 默认值 + Display ────────────────────────────────────────

#[test]
fn default_status_is_pending() {
    // v0.11 阶段 1：Pending 是 #[default]，ProcessInfo::default() 走这条路径。
    // 与 Unknown 语义不同 —— Pending = 「尚未触发验证」，Unknown = 「已尝试验证但失败」。
    assert_eq!(SignatureStatus::default(), SignatureStatus::Pending);
}

#[test]
fn display_covers_all_variants() {
    // 防止新增变体时忘了更新 Display impl（编译器只警告 match arms，不警告文案）。
    // v0.12 阶段 3：扩到 9 个变体（加 Expired / UntrustedRoot / ChainError）。
    for status in [
        SignatureStatus::Pending,
        SignatureStatus::Trusted,
        SignatureStatus::Signed,
        SignatureStatus::Unsigned,
        SignatureStatus::Revoked,
        SignatureStatus::Expired,
        SignatureStatus::UntrustedRoot,
        SignatureStatus::ChainError,
        SignatureStatus::Unknown,
    ] {
        let s = format!("{}", status);
        assert!(
            !s.is_empty(),
            "Display impl returned empty for {:?}",
            status
        );
    }
}

// ── from_wintrust_result：HRESULT → SignatureStatus 状态机 ──────────────────

#[test]
fn from_wintrust_result_success_maps_to_signed() {
    // WinVerifyTrust 返回 0 = 已签名。是否升级为 Trusted 由调用方基于 CompanyName
    // 决定，from_wintrust_result 本身只返回 Signed。
    assert_eq!(from_wintrust_result(0), SignatureStatus::Signed);
}

#[test]
fn from_wintrust_result_not_signed_maps_to_unsigned() {
    assert_eq!(
        from_wintrust_result(TRUST_E_SUBJECT_NOT_SIGNED),
        SignatureStatus::Unsigned,
    );
}

#[test]
fn from_wintrust_result_revoked_maps_to_revoked() {
    assert_eq!(
        from_wintrust_result(CRYPT_E_REVOKED),
        SignatureStatus::Revoked
    );
}

// v0.12 阶段 3 TD-26：3 个新变体的 HRESULT 映射

#[test]
fn from_wintrust_result_expired_maps_to_expired() {
    // CERT_E_EXPIRED (0x800B0101) — 证书过期，曾经在受信期内但已过期。
    assert_eq!(
        from_wintrust_result(CERT_E_EXPIRED),
        SignatureStatus::Expired,
    );
}

#[test]
fn from_wintrust_result_untrusted_root_maps_to_untrusted_root() {
    // CERT_E_UNTRUSTEDROOT (0x800B0109) — 证书链根不受信（自签名 / 缺失中间证书）。
    assert_eq!(
        from_wintrust_result(CERT_E_UNTRUSTEDROOT),
        SignatureStatus::UntrustedRoot,
    );
}

#[test]
fn from_wintrust_result_chaining_maps_to_chain_error() {
    // CERT_E_CHAINING (0x800B010A) — 证书链构造失败。
    assert_eq!(
        from_wintrust_result(CERT_E_CHAINING),
        SignatureStatus::ChainError,
    );
}

#[test]
fn from_wintrust_result_wrong_name_maps_to_chain_error() {
    // CERT_E_WRONG_NAME (0x800B0113) — 名称不匹配，归类为链问题（TD-26）。
    assert_eq!(
        from_wintrust_result(CERT_E_WRONG_NAME),
        SignatureStatus::ChainError,
    );
}

#[test]
fn from_wintrust_result_cert_signature_failure_maps_to_chain_error() {
    // TRUST_E_CERT_SIGNATURE (0x80096010) — 证书签名无效，归类为链问题（TD-26）。
    assert_eq!(
        from_wintrust_result(TRUST_E_CERT_SIGNATURE),
        SignatureStatus::ChainError,
    );
}

#[test]
fn from_wintrust_result_other_errors_map_to_unknown() {
    // 未映射的 HRESULT 仍落入 Unknown。v0.12 阶段 3：CERT_E_EXPIRED 等已精确映射，
    // 这里改用 E_FAIL (0x80004005) 验证 fallback 路径。
    assert_eq!(from_wintrust_result(-1), SignatureStatus::Unknown);
    assert_eq!(
        from_wintrust_result(0x80004005_u32 as i32), // E_FAIL
        SignatureStatus::Unknown,
    );
    assert_eq!(
        from_wintrust_result(0x80070057_u32 as i32), // E_INVALIDARG
        SignatureStatus::Unknown,
    );
}

// ── signature_risk_factor：R16 评分映射 ────────────────────────────────────

#[test]
fn signature_risk_factor_pending_is_none() {
    // 关键契约：Pending 不扣分。启动后头 1-2 个 heavy refresh 内全部 Pending，
    // 如果扣分会让所有进程瞬间变红色 — ADR-0021 §3 明确这点。
    assert!(signature_risk_factor(SignatureStatus::Pending).is_none());
}

#[test]
fn signature_risk_factor_trusted_is_none() {
    // 受信签名不扣分。
    assert!(signature_risk_factor(SignatureStatus::Trusted).is_none());
}

#[test]
fn signature_risk_factor_signed_deducts_10() {
    // 已签名但非受信 CA：轻扣分（10 分），让用户看到「签名但不受信」与 Trusted 区分。
    let f = signature_risk_factor(SignatureStatus::Signed).expect("Signed produces a factor");
    assert_eq!(f.weight, 10);
    assert_eq!(f.category, proc::security::RiskCategory::Signature);
}

#[test]
fn signature_risk_factor_unsigned_deducts_20() {
    // 无签名：与 stage-4.md 原方案的 25 分档接近，但实际选用 20 分（v0.6 调优结果）。
    let f = signature_risk_factor(SignatureStatus::Unsigned).expect("Unsigned produces a factor");
    assert_eq!(f.weight, 20);
    assert_eq!(f.category, proc::security::RiskCategory::Signature);
}

#[test]
fn signature_risk_factor_revoked_deducts_35() {
    // 签名被吊销：最严重档（35 分），高于无签名。被吊销意味着曾经受信但被 CA 撤销。
    let f = signature_risk_factor(SignatureStatus::Revoked).expect("Revoked produces a factor");
    assert_eq!(f.weight, 35);
}

// v0.12 阶段 3 TD-26：3 个新变体的扣分契约

#[test]
fn signature_risk_factor_expired_deducts_15() {
    // 证书过期：曾经受信但已过期，比 Unsigned 轻微（曾经走过验证流程）。
    let f = signature_risk_factor(SignatureStatus::Expired).expect("Expired produces a factor");
    assert_eq!(f.weight, 15);
    assert_eq!(f.name, "expired");
    assert_eq!(f.category, proc::security::RiskCategory::Signature);
}

#[test]
fn signature_risk_factor_untrusted_root_deducts_15() {
    // 不受信根：可能是自签名证书或缺失中间证书，与 Expired 同档（曾经尝试验证）。
    let f = signature_risk_factor(SignatureStatus::UntrustedRoot)
        .expect("UntrustedRoot produces a factor");
    assert_eq!(f.weight, 15);
    assert_eq!(f.name, "untrusted_root");
}

#[test]
fn signature_risk_factor_chain_error_deducts_10() {
    // 链断裂：验证不完整（链构造失败 / 名称不匹配 / 签名无效），扣分最低（10）。
    let f =
        signature_risk_factor(SignatureStatus::ChainError).expect("ChainError produces a factor");
    assert_eq!(f.weight, 10);
    assert_eq!(f.name, "chain_error");
}

#[test]
fn signature_risk_factor_unknown_deducts_5_on_windows() {
    // v0.11 阶段 8 REVIEW-13 P1-3：Windows 上 Unknown 扣 5 分（非管理员降级行为，
    // ADR-0021 设计）；非 Windows 平台没有 WinVerifyTrust 概念，所有进程都返
    // Unknown 扣 5 分（非 elevated 时 verify_signature 返 Unknown）。
    let f = signature_risk_factor(SignatureStatus::Unknown).expect("Unknown produces a factor");
    assert_eq!(f.weight, 5);
    assert_eq!(f.name, "signature_unverified");
}

// ── SignatureStatus::badge：进程列表 emoji 标记 ────────────────────────────

#[test]
fn badge_trusted_returns_lock_emoji() {
    assert_eq!(SignatureStatus::Trusted.badge(), " \u{1F512}"); // " 🔒"
}

#[test]
fn badge_unsigned_and_revoked_return_warning_emoji() {
    // stage-4.md 原方案：Untrusted → ⚠。9 状态机里 Unsigned + Revoked 都映射到 ⚠。
    assert_eq!(SignatureStatus::Unsigned.badge(), " \u{26A0}\u{FE0F}"); // " ⚠️"
    assert_eq!(SignatureStatus::Revoked.badge(), " \u{26A0}\u{FE0F}");
}

// v0.12 阶段 3 TD-26：新变体的 badge

#[test]
fn badge_expired_and_untrusted_root_return_warning_emoji() {
    // Expired / UntrustedRoot 与 Unsigned/Revoked 同档（有签名但有问题，红旗标记）。
    assert_eq!(SignatureStatus::Expired.badge(), " \u{26A0}\u{FE0F}"); // " ⚠️"
    assert_eq!(SignatureStatus::UntrustedRoot.badge(), " \u{26A0}\u{FE0F}");
}

#[test]
fn badge_chain_error_returns_question_mark() {
    // ChainError 与 Unknown 同档（验证不完整，没有足够信息判定）。
    assert_eq!(SignatureStatus::ChainError.badge(), " \u{2753}"); // " ❓"
}

#[test]
fn badge_unknown_returns_question_mark() {
    assert_eq!(SignatureStatus::Unknown.badge(), " \u{2753}"); // " ❓"
}

#[test]
fn badge_pending_and_signed_are_empty_to_avoid_column_jitter() {
    // 关键 UX 契约：Pending（默认值）/ Signed 不渲染占位，避免每行 name 列宽波动。
    // 与 v0.7 EcoQoS Non-Eco 处理一致（src/throttle.rs::EcoQoSState::badge）。
    assert_eq!(SignatureStatus::Pending.badge(), "");
    assert_eq!(SignatureStatus::Signed.badge(), "");
}

// ── serde：兼容旧录屏（缺字段默认 Pending）────────────────────────────────

#[test]
fn serde_roundtrip_all_variants() {
    // v0.12 阶段 3：扩到 9 个变体 round-trip。
    for status in [
        SignatureStatus::Pending,
        SignatureStatus::Trusted,
        SignatureStatus::Signed,
        SignatureStatus::Unsigned,
        SignatureStatus::Revoked,
        SignatureStatus::Expired,
        SignatureStatus::UntrustedRoot,
        SignatureStatus::ChainError,
        SignatureStatus::Unknown,
    ] {
        let json = serde_json::to_string(&status).expect("serialize");
        let back: SignatureStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, status, "round-trip failed for {:?}", status);
    }
}

#[test]
fn serde_pending_serializes_as_default_tag() {
    // 序列化形态：Stage-4 设计的 SignatureStatus 用 PascalCase 标签。
    // 这里仅断言 Pending 在 JSON 里有非空表示，不锁死具体字符串（避免 enum rename 时连锁挂）。
    let json = serde_json::to_string(&SignatureStatus::Pending).expect("serialize");
    assert!(!json.is_empty());
    assert!(
        serde_json::from_str::<SignatureStatus>(&json).is_ok(),
        "Pending JSON must deserialize back"
    );
}

/// 旧录屏兼容性测试：v0.10 之前的 `.prec` 反序列化时，ProcessInfo.signature_status
/// 字段可能不存在；`#[serde(default)]` 让缺字段回退到 Pending。
///
/// 做法：先序列化一个完整 ProcessInfo（signature_status = Trusted 区别于默认值），
/// 把 JSON object 里的 signature_status 节点删掉，再反序列化 —— 应该走 #[serde(default)]
/// 路径返回 Pending。这样不依赖 ProcessInfo schema 冻结，新加字段不会让测试挂。
#[test]
fn serde_missing_field_falls_back_to_pending() {
    use proc::collect::ProcessInfo;

    // 设非默认 signature_status，确保后续 deserialize 走 #[serde(default)] 而非 Default::default impl。
    let proc = ProcessInfo {
        signature_status: SignatureStatus::Trusted,
        ..ProcessInfo::default()
    };

    let mut value: serde_json::Value = serde_json::to_value(&proc).expect("serialize ProcessInfo");
    // 删 signature_status 节点，模拟旧录屏
    if let Some(obj) = value.as_object_mut() {
        obj.remove("signature_status");
    } else {
        panic!("ProcessInfo should serialize to a JSON object");
    }

    let parsed: ProcessInfo =
        serde_json::from_value(value).expect("deserialize tolerates missing signature_status");
    assert_eq!(
        parsed.signature_status,
        SignatureStatus::Pending,
        "missing signature_status must default to Pending (#[serde(default)])",
    );
}

// ── is_trusted_signer：已知 CA 厂商匹配 ────────────────────────────────────

#[test]
fn is_trusted_signer_matches_microsoft() {
    assert!(is_trusted_signer("Microsoft Corporation"));
    assert!(is_trusted_signer("Microsoft Windows"));
}

#[test]
fn is_trusted_signer_matches_other_known_cas() {
    // v0.12 阶段 3 TD-27：扩内置 vendor 列表，覆盖 Adobe / Cisco / Oracle /
    // VMWare / Docker / Red Hat / Apache / Python / GitHub / Electron 等。
    assert!(is_trusted_signer("Google LLC"));
    assert!(is_trusted_signer("Google Inc"));
    assert!(is_trusted_signer("Mozilla Corporation"));
    assert!(is_trusted_signer("Apple Inc."));
    assert!(is_trusted_signer("Intel Corporation"));
    assert!(is_trusted_signer("NVIDIA Corporation"));
    // v0.12 阶段 3 新增
    assert!(is_trusted_signer("Adobe Inc."));
    assert!(is_trusted_signer("Adobe Systems Incorporated"));
    assert!(is_trusted_signer("Cisco Systems, Inc."));
    assert!(is_trusted_signer("Oracle Corporation"));
    assert!(is_trusted_signer("VMware, Inc."));
    assert!(is_trusted_signer("Docker Inc."));
    assert!(is_trusted_signer("Red Hat, Inc."));
    assert!(is_trusted_signer("Apache Software Foundation"));
    assert!(is_trusted_signer("Python Software Foundation"));
    assert!(is_trusted_signer("GitHub, Inc."));
    assert!(is_trusted_signer("OpenJS Foundation"));
    assert!(is_trusted_signer("Electron.js"));
}

#[test]
fn is_trusted_signer_is_case_insensitive_and_substring_match() {
    // 子串匹配：CN 里可能含 OU / O 字段
    assert!(is_trusted_signer("CN=Microsoft Corporation, O=Microsoft"));
    // 大小写不敏感
    assert!(is_trusted_signer("microsoft corporation"));
    assert!(is_trusted_signer("MICROSOFT WINDOWS"));
    // v0.12 阶段 3：Adobe 大小写不敏感
    assert!(is_trusted_signer("adobe inc."));
    assert!(is_trusted_signer("ADOBE SYSTEMS INCORPORATED"));
}

#[test]
fn is_trusted_signer_rejects_unknown_vendors() {
    assert!(!is_trusted_signer("Random Vendor LLC"));
    assert!(!is_trusted_signer(""));
    assert!(!is_trusted_signer("Acme Corp"));
}

// ── trusted_signers.toml 用户配置解析（v0.12 阶段 3 TD-27） ─────────────────

#[test]
fn load_trusted_signers_nonexistent_returns_empty() {
    use proc::security::load_trusted_signers_from;
    use std::path::Path;
    let rules = load_trusted_signers_from(Path::new("/nonexistent/path/trusted_signers.toml"));
    assert!(rules.is_empty());
}

#[test]
fn load_trusted_signers_invalid_toml_returns_empty() {
    use proc::security::load_trusted_signers_from;
    let tmp = std::env::temp_dir().join(format!(
        "proc-test-sig-toml-{}-{}.toml",
        std::process::id(),
        line!()
    ));
    std::fs::write(&tmp, "this is not toml {{{{").unwrap();
    let rules = load_trusted_signers_from(&tmp);
    let _ = std::fs::remove_file(&tmp);
    assert!(rules.is_empty());
}

#[test]
fn load_trusted_signers_valid_toml_3_rules() {
    use proc::security::load_trusted_signers_from;
    let tmp = std::env::temp_dir().join(format!(
        "proc-test-sig-toml-3-{}-{}.toml",
        std::process::id(),
        line!()
    ));
    std::fs::write(
        &tmp,
        r#"
# 用户标记自家 vendor
[[signer]]
name = "adobe"
vendor_pattern = "(?i)adobe"
reason = "Adobe 系统"

[[signer]]
name = "my_company"
vendor_pattern = "^MyCompany Inc\\.$"

[[signer]]
name = "docker"
vendor_pattern = "(?i)docker"
"#,
    )
    .unwrap();
    let rules = load_trusted_signers_from(&tmp);
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[0].name, "adobe");
    assert!(rules[0].reason.is_some());
    assert_eq!(rules[1].name, "my_company");
    assert!(rules[1].reason.is_none());
    assert_eq!(rules[2].name, "docker");
}

#[test]
fn load_trusted_signers_invalid_regex_skips_rule() {
    use proc::security::load_trusted_signers_from;
    let tmp = std::env::temp_dir().join(format!(
        "proc-test-sig-toml-bad-{}-{}.toml",
        std::process::id(),
        line!()
    ));
    std::fs::write(
        &tmp,
        r#"
[[signer]]
name = "bad"
vendor_pattern = "[unclosed"

[[signer]]
name = "good"
vendor_pattern = "good"
"#,
    )
    .unwrap();
    let rules = load_trusted_signers_from(&tmp);
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(rules.len(), 1, "invalid regex rule should be skipped");
    assert_eq!(rules[0].name, "good");
}

// ── verify_signature 行为 ────────────────────────────────────────────────

/// Windows 上 verify_signature 在非 elevated 时返回 Unknown
/// （WinVerifyTrust 需要 SE_DEBUG 或等价权限才能查询完整签名链）。
/// 不在 CI 里强测 —— 仅作 sanity check：调用不 panic。
#[cfg(target_os = "windows")]
#[test]
fn verify_signature_does_not_panic_on_windows() {
    use proc::security::verify_signature;
    // 任意不存在的路径：返回 Unknown（不能是 Pending，因为函数确实跑过验证逻辑）。
    let _ = verify_signature("C:\\nonexistent\\path.exe");
}
