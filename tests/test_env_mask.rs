//! v0.6.0 阶段 2 — env_mask 集成测试。
//!
//! 三组：
//! 1. `is_secret_key` 覆盖常见 key 名（CI / cloud / DB 客户端）+ false positive
//! 2. `mask_value` 边界（空 / 单字符 / 多字节 / 长 token 不泄漏）
//! 3. `EnvVar::render_value_owned` 在 reveal/mask 下的行为 + 录屏强制 mask 联动

use proc::inspect::EnvVar;
use proc::inspect::env_mask::{is_secret_key, mask_value};

#[test]
fn detects_ci_cloud_db_keys() {
    for k in [
        "AWS_SECRET_ACCESS_KEY",
        "AWS_ACCESS_KEY_ID", // 含 KEY
        "AZURE_CLIENT_SECRET",
        "GITHUB_TOKEN",
        "GITLAB_PRIVATE_TOKEN",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "DB_PASSWORD",
        "POSTGRES_PASSWORD",
        "MYSQL_ROOT_PASSWORD",
        "REDIS_PASSWORD",
        "DATABASE_URL", // 特例：含 password@ 连接串
        "DSN",
        "PRIMARY_DSN",
        "PG_CONNECTION_STRING",
        "STRIPE_API_KEY",
        "SLACK_BOT_TOKEN",
        "NPM_AUTH_TOKEN",
        "DOCKER_HUB_PASSWORD",
        "KUBECONFIG_TOKEN",
        "SERVICE_ACCOUNT_CREDENTIAL",
        "JWT_PRIVATE_KEY",
    ] {
        assert!(is_secret_key(k), "expected secret: {k}");
    }
}

#[test]
fn does_not_flag_common_non_secret_keys() {
    for k in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "LANG",
        "LC_ALL",
        "TERM",
        "EDITOR",
        "SHELL",
        "TMP",
        "TEMP",
        "COLORTERM",
        "PROGRAMFILES",
        "COMSPEC",
        "APPDATA",
        "XDG_CONFIG_HOME",
    ] {
        assert!(!is_secret_key(k), "false positive: {k}");
    }
}

#[test]
fn case_insensitive() {
    assert!(is_secret_key("password"));
    assert!(is_secret_key("Password"));
    assert!(is_secret_key("PASSWORD"));
    assert!(is_secret_key("Api-Key"));
    assert!(!is_secret_key("pathname")); // 含 name 不应触发（无 secret 关键字）
}

#[test]
fn mask_value_empty_and_short() {
    assert_eq!(mask_value(""), "");
    assert_eq!(mask_value("a"), "a***(1 B)");
    assert_eq!(mask_value("ab"), "ab***(2 B)");
}

#[test]
fn mask_value_long_token() {
    let val = "AKIAIOSFODNN7EXAMPLE";
    let masked = mask_value(val);
    assert_eq!(masked, "AK***(20 B)");
    // 关键：不泄漏 3 字符之后的任何内容
    assert!(!masked.contains("IAIO"));
    assert!(!masked.contains("EXAMPLE"));
}

#[test]
fn mask_value_multibyte_chars() {
    // 中文前 2 字符 = 6 字节，总长 = 3*3 + 3 = 12
    assert_eq!(mask_value("密码值123"), "密码***(12 B)");
    assert_eq!(mask_value("单"), "单***(3 B)");
    // emoji（UTF-8 4 字节）+ ASCII；总字节数由 Rust len() 决定
    let emoji_val = "🔑secret";
    let masked = mask_value(emoji_val);
    let expected_len = emoji_val.len();
    assert_eq!(masked, format!("🔑s***({expected_len} B)"));
}

#[test]
fn env_var_render_masks_when_not_reveal() {
    let v = EnvVar {
        key: "AWS_SECRET_ACCESS_KEY".into(),
        value: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
        is_secret: true,
    };
    let masked = v.render_value_owned(false);
    assert!(masked.starts_with("wJ***("));
    // 不泄漏真值
    assert!(!masked.contains("rXUtnFEMI"));
    assert!(!masked.contains("bPxRfiCY"));
    // reveal=true 直接给原值
    assert_eq!(v.render_value_owned(true), v.value);
}

#[test]
fn env_var_render_passes_non_secret_through() {
    let v = EnvVar {
        key: "PATH".into(),
        value: "/usr/bin:/bin".into(),
        is_secret: false,
    };
    assert_eq!(v.render_value_owned(false), v.value);
    assert_eq!(v.render_value_owned(true), v.value);
}

#[test]
fn env_var_render_secret_with_empty_value() {
    let v = EnvVar {
        key: "EMPTY_PASSWORD".into(),
        value: "".into(),
        is_secret: true,
    };
    assert_eq!(v.render_value_owned(false), "");
    assert_eq!(v.render_value_owned(true), "");
}

#[test]
fn env_var_render_secret_with_multibyte_value() {
    let v = EnvVar {
        key: "PRIVATE_NOTE".into(),
        value: "密钥内容123".into(),
        is_secret: true,
    };
    let masked = v.render_value_owned(false);
    assert_eq!(masked, "密钥***(15 B)"); // 5 中文 = 15B
}

#[test]
fn collected_env_marks_real_secrets() {
    // 真正跑一次 collect_env(self)，确认 is_secret 字段被正确填充。
    // 不是所有 CI 环境都有 secret，但本测试进程通常会有 PATH（非 secret）；
    // 如果碰巧有 secret 类（如 GITHUB_TOKEN），is_secret 应为 true。
    let vars = proc::inspect::env::collect_env(std::process::id()).expect("collect self env");
    for v in &vars {
        // 不变量：is_secret 与 is_secret_key(key) 必须一致
        assert_eq!(
            v.is_secret,
            is_secret_key(&v.key),
            "key={} is_secret={} but is_secret_key={}",
            v.key,
            v.is_secret,
            is_secret_key(&v.key),
        );
    }
}
