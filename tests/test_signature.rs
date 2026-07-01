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
//! `verify_signature_with_policy` 的 mock policy 注入测试在 `src/security/signature.rs`
//! 内的 `#[cfg(test)] mod tests`（函数是 `pub(crate)`，集成测试 crate 不可见）。

use proc::security::{
    SignatureStatus, from_wintrust_result, is_trusted_signer, signature_risk_factor,
};

/// `WinVerifyTrust` 的 HRESULT 常量。与 `src/security/signature.rs` 内私有 const 同值。
/// 这里独立定义避免依赖私有项；值变更时两边同步（serde round-trip 测试也会立刻挂掉）。
const TRUST_E_SUBJECT_NOT_SIGNED: i32 = 0x800B0100_u32 as i32;
const CRYPT_E_REVOKED: i32 = 0x80092010_u32 as i32;

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
    for status in [
        SignatureStatus::Pending,
        SignatureStatus::Trusted,
        SignatureStatus::Signed,
        SignatureStatus::Unsigned,
        SignatureStatus::Revoked,
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

#[test]
fn from_wintrust_result_other_errors_map_to_unknown() {
    // 链断裂 / 过期 / API 错误等任何非预期 HRESULT 都落入 Unknown。
    // 选两个具体值避免硬编码「任意非零」——0x800B0101 (TRUST_E_NOSIGNATURE 邻居)
    // 和一个明显不相关的负数。
    assert_eq!(
        from_wintrust_result(0x800B0101_u32 as i32),
        SignatureStatus::Unknown
    );
    assert_eq!(from_wintrust_result(-1), SignatureStatus::Unknown);
    assert_eq!(
        from_wintrust_result(0x80004005_u32 as i32), // E_FAIL
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

#[test]
fn signature_risk_factor_unknown_deducts_5_on_windows() {
    // v0.11 阶段 8 REVIEW-13 P1-3：Windows 上 Unknown 扣 5 分（非管理员降级行为，
    // ADR-0021 设计）；非 Windows 平台没有 WinVerifyTrust 概念，所有进程都返
    // Unknown，扣分无意义 → cfg-gate 不扣分（返回 None）。
    #[cfg(target_os = "windows")]
    {
        let f = signature_risk_factor(SignatureStatus::Unknown)
            .expect("Unknown produces a factor on Windows");
        assert_eq!(f.weight, 5);
        assert_eq!(f.name, "signature_unverified");
    }
    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            signature_risk_factor(SignatureStatus::Unknown).is_none(),
            "非 Windows 上 Unknown 不应扣分（无 WinVerifyTrust 概念）"
        );
    }
}

// ── SignatureStatus::badge：进程列表 emoji 标记 ────────────────────────────

#[test]
fn badge_trusted_returns_lock_emoji() {
    assert_eq!(SignatureStatus::Trusted.badge(), " \u{1F512}"); // " 🔒"
}

#[test]
fn badge_unsigned_and_revoked_return_warning_emoji() {
    // stage-4.md 原方案：Untrusted → ⚠。6 状态机里 Unsigned + Revoked 都映射到 ⚠。
    assert_eq!(SignatureStatus::Unsigned.badge(), " \u{26A0}\u{FE0F}"); // " ⚠️"
    assert_eq!(SignatureStatus::Revoked.badge(), " \u{26A0}\u{FE0F}");
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
    for status in [
        SignatureStatus::Pending,
        SignatureStatus::Trusted,
        SignatureStatus::Signed,
        SignatureStatus::Unsigned,
        SignatureStatus::Revoked,
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
    assert!(is_trusted_signer("Google LLC"));
    assert!(is_trusted_signer("Google Inc"));
    assert!(is_trusted_signer("Mozilla Corporation"));
    assert!(is_trusted_signer("Apple Inc."));
    assert!(is_trusted_signer("Intel Corporation"));
    assert!(is_trusted_signer("NVIDIA Corporation"));
}

#[test]
fn is_trusted_signer_is_case_insensitive_and_substring_match() {
    // 子串匹配：CN 里可能含 OU / O 字段
    assert!(is_trusted_signer("CN=Microsoft Corporation, O=Microsoft"));
    // 大小写不敏感
    assert!(is_trusted_signer("microsoft corporation"));
    assert!(is_trusted_signer("MICROSOFT WINDOWS"));
}

#[test]
fn is_trusted_signer_rejects_unknown_vendors() {
    assert!(!is_trusted_signer("Random Vendor LLC"));
    assert!(!is_trusted_signer(""));
    assert!(!is_trusted_signer("Acme Corp"));
}

// ── verify_signature 跨平台行为 ────────────────────────────────────────────

/// 非 Windows 上 `verify_signature` 是 stub，对任意路径返回 Unknown。
/// Windows 上跑会真实调 WinVerifyTrust（结果依赖测试机签名状态），
/// 因此这个测试仅 cfg(target_os != "windows") 跑。
#[cfg(not(target_os = "windows"))]
#[test]
fn verify_signature_stub_returns_unknown_on_non_windows() {
    use proc::security::verify_signature;
    assert_eq!(verify_signature("/bin/ls"), SignatureStatus::Unknown);
    assert_eq!(
        verify_signature("/nonexistent/path"),
        SignatureStatus::Unknown
    );
}

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
