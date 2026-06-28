pub mod behavior;
pub mod command_line;
pub mod dll_check;
/// v0.7 阶段 8：SecurityRule R15 — 外联行为评分（基于 ProcessFlow）。
pub mod flow;
pub mod hash_cache;
pub mod parent_chain;
pub mod path_check;
pub mod privilege;
pub mod restricted_spawn;
pub mod score;
pub mod self_mitigation;
pub mod signature;

pub use flow::{SniWhitelist, check_flow_risk};
pub use score::{BackgroundScorer, RiskCategory, RiskFactor, SecurityScore, SecurityScorer};
pub use signature::{SignatureStatus, is_trusted_signer, signature_risk_factor, verify_signature};
