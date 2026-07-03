pub mod behavior;
pub mod command_line;
pub mod dll_check;
/// v0.7 阶段 8：SecurityRule R15 — 外联行为评分（基于 ProcessFlow）。
pub mod flow;
pub mod hash_cache;
/// v0.11 阶段 5：父子链构建 + R17 可疑链检测（基于 ProcessInfo.parent_chain）。
pub mod lineage;
pub mod parent_chain;
pub mod path_check;
/// v0.11 阶段 6：R18 可疑启动路径（%TEMP% / %APPDATA% / %LOCALAPPDATA% /
/// %USERPROFILE%\Downloads + 用户自定义）。与 v0.6 `path_check.rs` 叠加扣分。
pub mod path_rules;
pub mod privilege;
pub mod restricted_spawn;
pub mod score;
pub mod self_mitigation;
pub mod signature;
/// v0.12 阶段 3：用户配置的受信签名 vendor 列表（TD-27）。
pub mod trusted_signers;

pub use flow::{SniWhitelist, check_flow_risk};
pub use lineage::{LineageRule, SuspiciousPattern, check_lineage_risk, load_lineage_rules};
pub use path_rules::{
    PathRule, SuspiciousPathKind, UserDirs, check_path_risk as check_suspicious_path_risk,
    expand_user_dir, is_in_suspicious_path, load_path_rules,
};

// 注：path_rules::check_path_risk 与 path_check::check_path_risk 同名，前者 re-export
// 加 alias `check_suspicious_path_risk` 避免歧义；score.rs 内全路径访问两者，无歧义。
pub use score::{BackgroundScorer, RiskCategory, RiskFactor, SecurityScore, SecurityScorer};
pub use signature::{
    SignatureStatus, from_wintrust_result, is_trusted_signer, signature_risk_factor,
    verify_signature,
};
pub use trusted_signers::{
    TrustedSignersRule, load_trusted_signers, load_trusted_signers_from, matches_any_rule,
};
