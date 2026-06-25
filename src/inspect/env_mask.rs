//! 环境变量 secret 脱敏 — 见 CONTEXT.md / ADR-0008。
//!
//! 默认 mask，详情页按 `v` 切换 `App::env_reveal`；录屏时（`recording_wanted=true`）
//! 强制 mask，即便用户切到 reveal 也立即复位（[`crate::tui::detail_view`] 计算时
//! `reveal = env_reveal && !recording`）。
//!
//! 判定规则：env key 大写化后包含 [`SECRET_PATTERNS`] 任一关键字，或匹配
//! `DATABASE_URL` / `*_AUTHORIZATION` 这类连接串/授权头。
//!
//! 渲染：[`mask_value`] 把值截前 2 字符 + `***` + 原始字节数，例如
//! `wJalrXUtnFEMI/K7MDENG/...` → `wJ***(40 B)`。

/// 疑似 secret 的 env key 关键字集合（大小写不敏感 substring 匹配）。
///
/// 来源：常见 CI / cloud SDK / DB 客户端命名约定。漏检成本高于误检，
/// 因此宁可把 `apikey` / `api_key` / `X-API-KEY` 都判为 secret。
pub const SECRET_PATTERNS: &[&str] = &[
    "KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PWD",
    "CREDENTIAL",
    "PRIVATE",
    "AUTH",
    "API",
    "DSN",
    "CONNECTION_STRING",
];

/// 判断 env key 是否疑似 secret（大小写不敏感）。
///
/// 单独把 `DATABASE_URL` 与 `*_AUTHORIZATION` 拎出来是因为它们不含上列关键字但
/// 实际值常常是 `postgres://user:password@host/db` 或 `Bearer xxx`。
#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    if SECRET_PATTERNS.iter().any(|p| upper.contains(p)) {
        return true;
    }
    upper == "DATABASE_URL" || upper.ends_with("_AUTHORIZATION")
}

/// 把 secret 值脱敏为 `前2字符***(原长 B)`。
///
/// 多字节字符按 char（不是字节）截取前 2 个；长度仍按 UTF-8 字节数显示，
/// 这样用户能直观判断 token 大致长度，又不泄漏具体内容。
#[must_use]
pub fn mask_value(val: &str) -> String {
    if val.is_empty() {
        return String::new();
    }
    let prefix: String = val.chars().take(2).collect();
    let len = val.len();
    format!("{prefix}***({len} B)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_secret_keys() {
        assert!(is_secret_key("AWS_SECRET_ACCESS_KEY"));
        assert!(is_secret_key("GITHUB_TOKEN"));
        assert!(is_secret_key("DB_PASSWORD"));
        assert!(is_secret_key("OPENAI_API_KEY"));
        assert!(is_secret_key("password")); // 小写
        assert!(is_secret_key("Database_URL"));
        assert!(is_secret_key("DSN_PRIMARY"));
        assert!(is_secret_key("PG_CONNECTION_STRING"));
        assert!(is_secret_key("STRIPED_AUTHORIZATION")); // _AUTHORIZATION 后缀
    }

    #[test]
    fn does_not_false_positive_common_keys() {
        assert!(!is_secret_key("PATH"));
        assert!(!is_secret_key("HOME"));
        assert!(!is_secret_key("SYSTEMROOT"));
        assert!(!is_secret_key("LANG"));
        assert!(!is_secret_key("USERPROFILE"));
        assert!(!is_secret_key("TMP"));
        assert!(!is_secret_key("EDITOR"));
        assert!(!is_secret_key("SHELL"));
    }

    #[test]
    fn database_url_special_case() {
        assert!(is_secret_key("DATABASE_URL"));
        assert!(is_secret_key("database_url"));
        // 不要把含 database_url 子串的误判（理论上有，但极少见）
        assert!(!is_secret_key("DATABASE")); // 不含 _URL 后缀
    }

    #[test]
    fn mask_value_preserves_prefix() {
        assert_eq!(mask_value(""), "");
        assert_eq!(mask_value("ab"), "ab***(2 B)");
        assert_eq!(mask_value("wJalrXUt"), "wJ***(8 B)");
    }

    #[test]
    fn mask_value_handles_multibyte() {
        // 多字节字符按 char 取前 2 个；长度按字节算（3 中文 = 9 字节 + 3 数字 = 12）
        assert_eq!(mask_value("密码值123"), "密码***(12 B)");
        assert_eq!(mask_value("单"), "单***(3 B)");
    }

    #[test]
    fn mask_value_does_not_leak_full_value() {
        let secret = "AKIAIOSFODNN7EXAMPLE";
        let masked = mask_value(secret);
        assert!(masked.starts_with("AK***("));
        // 不应直接出现原值的 3 字符之后部分
        assert!(!masked.contains("IAIO"));
        assert!(!masked.contains("EXAMPLE"));
    }
}
