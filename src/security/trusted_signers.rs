//! v0.12 阶段 3：用户配置的受信签名 vendor 列表（TD-27）。
//!
//! 与 `signature.rs::TRUSTED_SIGNERS` 内置列表**追加**（不替换），让用户能标记
//! 自家应用 vendor 升级 Signed → Trusted。配置文件 `~/.config/proc/trusted_signers.toml`
//! 默认不存在 → 空 Vec（只用内置 24 个 vendor）。
//!
//! **设计取舍**：内置列表用子串大小写不敏感匹配（覆盖 `CN=Microsoft Corporation,
//! O=Microsoft` 这类 X.509 subject 字段），用户 rules 走 regex（更灵活，能匹配
//! `(?i)^adobe` 这类前缀规则）。两条路径任一命中即升级——内部列表是 fast path
//! （避免编译 regex），用户 rules 是 slow path（编译一次缓存）。
//!
//! **加载时机**：`SecurityScorer::new` 构造时一次性读取，每次 score 调用复用
//! （避免每进程读文件）。文件不存在 / TOML 解析失败 / regex 编译失败 → 静默
//! 降级为空（`tracing::warn` 记录原因），与 `lineage_rules.rs` / `path_rules.rs`
//! 同款契约。

use std::path::PathBuf;

/// 用户自定义的受信签名 vendor 规则。
///
/// 从 `~/.config/proc/trusted_signers.toml` 加载。`vendor_pattern` 在加载时编译
/// 为 `regex::Regex` 缓存（避免每进程 score 调用重新编译）。
#[derive(Debug, Clone)]
pub struct TrustedSignersRule {
    /// 规则名（用于日志 / 调试）。
    pub name: String,
    /// 已编译的 vendor 匹配 regex。匹配对象：FileVersion Information 的
    /// `CompanyName` 字段（不是 X.509 certificate subject——后者取 Cert Subject CN）。
    pub vendor_regex: regex::Regex,
    /// 可选的备注说明（用户在 toml 里写为什么这条规则受信）。
    pub reason: Option<String>,
}

#[derive(serde::Deserialize)]
struct TrustedSignerRaw {
    name: String,
    vendor_pattern: String,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct TrustedSignersFile {
    #[serde(default)]
    signer: Vec<TrustedSignerRaw>,
}

/// 默认 `trusted_signers.toml` 路径：`~/.config/proc/trusted_signers.toml`。
#[must_use]
pub fn default_rules_path() -> PathBuf {
    crate::dirs_config_dir().join("trusted_signers.toml")
}

/// 从默认路径加载用户配置。文件不存在 / 解析失败 → 空 Vec。
#[must_use]
pub fn load_trusted_signers() -> Vec<TrustedSignersRule> {
    load_trusted_signers_from(&default_rules_path())
}

/// 测试 / 自定义路径入口。文件不存在 / 解析失败 / regex 编译失败 → 空 Vec。
#[must_use]
pub fn load_trusted_signers_from(path: &std::path::Path) -> Vec<TrustedSignersRule> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(parsed) = toml::from_str::<TrustedSignersFile>(&content) else {
        tracing::warn!("trusted_signers.toml 解析失败：{}", path.display());
        return Vec::new();
    };
    parsed
        .signer
        .into_iter()
        .filter_map(|raw| {
            let Ok(regex) = regex::Regex::new(&raw.vendor_pattern) else {
                tracing::warn!(
                    "trusted_signers rule「{}」vendor_pattern 正则编译失败：{}",
                    raw.name,
                    raw.vendor_pattern
                );
                return None;
            };
            Some(TrustedSignersRule {
                name: raw.name,
                vendor_regex: regex,
                reason: raw.reason,
            })
        })
        .collect()
}

/// 检查 `subject`（CompanyName）是否匹配任一用户 rule。
///
/// 命中任一 rule 即返回 `true`（与内置 `TRUSTED_SIGNERS` 子串匹配取或）。
/// 大小写敏感——用户需要在 toml 里用 `(?i)` 前缀显式声明大小写不敏感
/// （如 `(?i)^adobe`），与 `lineage_rules.rs` 一致的契约。
#[must_use]
pub fn matches_any_rule(subject: &str, rules: &[TrustedSignersRule]) -> bool {
    rules.iter().any(|rule| rule.vendor_regex.is_match(subject))
}

#[cfg(test)]
mod tests {
    //! v0.12 阶段 3 TD-27：trusted_signers.toml 解析 + 匹配逻辑单元测试。
    //! 集成测试（含 SecurityScorer 升级 Trusted）在 `tests/test_signature.rs`。
    use super::*;

    #[test]
    fn load_nonexistent_returns_empty() {
        let rules = load_trusted_signers_from(std::path::Path::new(
            "/nonexistent/path/trusted_signers.toml",
        ));
        assert!(rules.is_empty());
    }

    #[test]
    fn load_invalid_toml_returns_empty() {
        let tmp = std::env::temp_dir().join(format!(
            "proc-trustedsigners-test-{}.toml",
            std::process::id()
        ));
        std::fs::write(&tmp, "this is not toml {{{{").unwrap();
        let rules = load_trusted_signers_from(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert!(rules.is_empty());
    }

    #[test]
    fn load_valid_toml_with_3_rules() {
        let tmp = std::env::temp_dir().join(format!(
            "proc-trustedsigners-test-3-{}.toml",
            std::process::id()
        ));
        std::fs::write(
            &tmp,
            r#"
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
    }

    #[test]
    fn load_invalid_regex_skips_rule() {
        let tmp = std::env::temp_dir().join(format!(
            "proc-trustedsigners-test-bad-{}.toml",
            std::process::id()
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
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "good");
    }

    #[test]
    fn matches_any_rule_case_sensitive_default() {
        let rule = TrustedSignersRule {
            name: "test".to_string(),
            vendor_regex: regex::Regex::new("^Adobe Inc\\.$").unwrap(),
            reason: None,
        };
        // 大小写敏感（无 (?i) 前缀）
        assert!(matches_any_rule("Adobe Inc.", std::slice::from_ref(&rule)));
        assert!(!matches_any_rule("adobe inc.", std::slice::from_ref(&rule)));
    }

    #[test]
    fn matches_any_rule_case_insensitive_with_flag() {
        let rule = TrustedSignersRule {
            name: "test".to_string(),
            vendor_regex: regex::Regex::new("(?i)^adobe").unwrap(),
            reason: None,
        };
        assert!(matches_any_rule("Adobe Inc.", std::slice::from_ref(&rule)));
        assert!(matches_any_rule(
            "adobe systems",
            std::slice::from_ref(&rule)
        ));
    }

    #[test]
    fn matches_any_rule_empty_rules_returns_false() {
        assert!(!matches_any_rule("Adobe Inc.", &[]));
        assert!(!matches_any_rule("", &[]));
    }

    #[test]
    fn matches_any_rule_multiple_rules_any_match() {
        let r1 = TrustedSignersRule {
            name: "adobe".to_string(),
            vendor_regex: regex::Regex::new("(?i)adobe").unwrap(),
            reason: None,
        };
        let r2 = TrustedSignersRule {
            name: "cisco".to_string(),
            vendor_regex: regex::Regex::new("(?i)cisco").unwrap(),
            reason: None,
        };
        let rules = vec![r1, r2];
        assert!(matches_any_rule("Adobe Inc.", &rules));
        assert!(matches_any_rule("Cisco Systems", &rules));
        assert!(!matches_any_rule("Random Vendor", &rules));
    }
}
