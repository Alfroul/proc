//! v0.7 阶段 8：SecurityRule R15 — 外联行为评分（基于 ProcessFlow，ADR-0016 §8）。
//!
//! R15 是 v0.7 安全评分的第 15 项（v0.6 共 14 项）。命中**任一**条件即扣 30 分：
//!
//! 1. **SNI 不在白名单**：进程外联到 dns_name 不在 `~/.config/proc/sni_whitelist.txt`。
//! 2. **端口扫描特征**：同一进程 10s 内连接 ≥ 50 个不同 remote_addr。
//!
//! 白名单文件默认不存在 → R15 整体不启用（避免误报）。用户显式 touch 该文件
//! 才激活；空文件 = "所有 SNI 都不在白名单" = 所有有 dns_name 的外联都命中
//! 条件 1（用户自负）。
//!
//! **MVP（Part B）**：2 个条件。第三个「DNS 与 connect 不一致」需要更复杂的
//! dns_log 关联（fast-flux / DNS hijack 检测），留 tech-debt TD-17 与 SNI /
//! JA4 一起做（v0.8+）。

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::{RiskCategory, RiskFactor};
use crate::ebpf::flow::ProcessFlow;

/// R15 命中权重。一击扣 30（与 v0.6 已知 C2 端口 R5 同档）。
pub const R15_WEIGHT: u32 = 30;

/// 端口扫描特征阈值：10s 内多少个不同 remote_addr 才算扫描。
/// 50 取自 ADR-0016 §8（典型浏览器 10s 内 ≤ 20 连接，正常服务进程 ≤ 10）。
pub const R15_PORT_SCAN_THRESHOLD: usize = 50;

/// 端口扫描滚动窗口。
pub const R15_PORT_SCAN_WINDOW: Duration = Duration::from_secs(10);

/// SNI 白名单文件路径：`~/.config/proc/sni_whitelist.txt`。
///
/// 文件不存在 → 返回 `None`，R15 跳过。文件存在但空 → 返回 `Some(空集)`，
/// 所有 dns_name 都视为「不在白名单」。
#[must_use]
pub fn default_whitelist_path() -> PathBuf {
    crate::dirs_config_dir().join("sni_whitelist.txt")
}

/// SNI 白名单。_domains 全小写；匹配时也按小写比较。
///
/// 不 derive Clone：内容只读；如需共享用 `&SniWhitelist`。
pub struct SniWhitelist {
    domains: HashSet<String>,
}

impl SniWhitelist {
    /// 从文件加载。每行一个域名（忽略空行 + `#` 开头注释 + trailing dot）。
    /// 文件不存在 / 读失败 → `None`（调用方据此跳过 R15）。
    #[must_use]
    pub fn load() -> Option<Self> {
        Self::load_from(&default_whitelist_path())
    }

    /// 测试 / 自定义路径入口。文件不存在 → `None`；存在但解析失败 → `Some(空集)`。
    /// 设计：用户显式创建文件 = 想启用 R15；解析失败不应静默回 None。
    #[must_use]
    pub fn load_from(path: &std::path::Path) -> Option<Self> {
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(path).ok()?;
        let domains = parse_whitelist(&content);
        Some(Self { domains })
    }

    /// 域名是否在白名单。`domain` 自动 lowercase，去 trailing dot。
    #[must_use]
    pub fn contains(&self, domain: &str) -> bool {
        let key = domain.trim().trim_end_matches('.').to_lowercase();
        if key.is_empty() {
            return false;
        }
        self.domains.contains(&key)
    }

    /// 白名单条数（测试用）。
    #[must_use]
    pub fn len(&self) -> usize {
        self.domains.len()
    }

    /// 是否为空（测试用）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.domains.is_empty()
    }
}

/// 把文件内容解析为小写域名集合。空行 / `#` 开头注释 / trailing dot / 多余空白自动忽略。
fn parse_whitelist(content: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for line in content.lines() {
        let s = line.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        // 多次 trim：先去 trailing dot，再去因 dot 移除后暴露的空白，最后 lowercase。
        // 例如 "  indented.org  ." → trim → "indented.org  ." → trim_end '.' →
        // "indented.org  " → trim → "indented.org"。
        let cleaned = s.trim_end_matches('.').trim().to_lowercase();
        if !cleaned.is_empty() {
            set.insert(cleaned);
        }
    }
    set
}

/// 评估一组 flows 是否触发 R15。返回 `Some(RiskFactor)` 表示命中（任一条件）。
///
/// `flows` 应为已按 `(pid, start_time)` 过滤后的「同一进程」flows（调用方
/// [`super::score::SecurityScorer::score`] 完成）。`whitelist` 为 `None`
/// 时条件 1 整体跳过；`Some(空)` 时所有 dns_name 都视为不在白名单。
///
/// `now` 显式传入便于测试。返回的 `RiskFactor` 携带命中的具体描述，便于
/// UI / MCP 消费者展示。
#[must_use]
pub fn check_flow_risk(
    flows: &[&ProcessFlow],
    whitelist: Option<&SniWhitelist>,
    now: SystemTime,
) -> Option<RiskFactor> {
    // Condition 1：SNI 不在白名单（仅当白名单存在时启用）。
    if let Some(wl) = whitelist {
        for f in flows {
            let Some(name) = f.dns_name.as_deref() else {
                continue;
            };
            if !wl.contains(name) {
                return Some(RiskFactor {
                    category: RiskCategory::NetworkBehavior,
                    name: "r15_sni_not_whitelisted".to_string(),
                    weight: R15_WEIGHT,
                    description: format!("外联域名 {name} 未在 SNI 白名单"),
                });
            }
        }
    }

    // Condition 2：10s 内 ≥ R15_PORT_SCAN_THRESHOLD 个不同 remote_addr。
    let Some(cutoff) = now.checked_sub(R15_PORT_SCAN_WINDOW) else {
        // now 早于窗口（系统时间回退）→ 跳过 condition 2，避免 false positive。
        return None;
    };
    let distinct: HashSet<&str> = flows
        .iter()
        .filter(|f| f.last_seen >= cutoff)
        .map(|f| f.remote_addr.as_str())
        .collect();
    if distinct.len() >= R15_PORT_SCAN_THRESHOLD {
        return Some(RiskFactor {
            category: RiskCategory::NetworkBehavior,
            name: "r15_port_scan".to_string(),
            weight: R15_WEIGHT,
            description: format!(
                "{}s 内连接 {} 个不同 IP（端口扫描特征）",
                R15_PORT_SCAN_WINDOW.as_secs(),
                distinct.len()
            ),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mk_flow(
        pid: u32,
        start_time: u64,
        remote: &str,
        dns_name: Option<&str>,
        last_seen: SystemTime,
    ) -> ProcessFlow {
        ProcessFlow {
            pid,
            start_time,
            comm: String::new(),
            local_addr: String::new(),
            remote_addr: remote.into(),
            remote_port: 443,
            bytes_out: 0,
            bytes_in: 0,
            dns_name: dns_name.map(str::to_string),
            first_seen: last_seen,
            last_seen,
            exit_time: None,
        }
    }

    #[test]
    fn empty_flows_no_risk() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let wl = SniWhitelist {
            domains: HashSet::new(),
        };
        assert!(check_flow_risk(&[], Some(&wl), now).is_none());
        assert!(check_flow_risk(&[], None, now).is_none());
    }

    /// 条件 1 命中：白名单存在但 dns_name 不在其中。
    #[test]
    fn condition1_sni_not_whitelisted_hits() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let flows = [mk_flow(1, 100, "1.2.3.4", Some("evil.example.com"), now)];
        let refs: Vec<&ProcessFlow> = flows.iter().collect();
        let wl = SniWhitelist {
            domains: ["good.example.com".to_string()].into_iter().collect(),
        };
        let risk = check_flow_risk(&refs, Some(&wl), now).expect("应命中条件 1");
        assert_eq!(risk.weight, R15_WEIGHT);
        assert_eq!(risk.name, "r15_sni_not_whitelisted");
        assert!(risk.description.contains("evil.example.com"));
    }

    /// 条件 1 不命中：dns_name 在白名单中。
    #[test]
    fn condition1_whitelisted_sni_passes() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let flows = [mk_flow(1, 100, "1.2.3.4", Some("good.example.com"), now)];
        let refs: Vec<&ProcessFlow> = flows.iter().collect();
        let wl = SniWhitelist {
            domains: ["good.example.com".to_string()].into_iter().collect(),
        };
        assert!(check_flow_risk(&refs, Some(&wl), now).is_none());
    }

    /// 条件 1 跳过：白名单 = None（文件不存在），即便 dns_name 存在也不扣分。
    #[test]
    fn condition1_skipped_when_whitelist_none() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let flows = [mk_flow(1, 100, "1.2.3.4", Some("evil.example.com"), now)];
        let refs: Vec<&ProcessFlow> = flows.iter().collect();
        assert!(check_flow_risk(&refs, None, now).is_none());
    }

    /// 条件 1：白名单匹配大小写不敏感 + 容忍 trailing dot。
    #[test]
    fn condition1_whitelist_matching_is_case_insensitive_and_trailing_dot() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let flows = [mk_flow(1, 100, "1.2.3.4", Some("Good.Example.COM."), now)];
        let refs: Vec<&ProcessFlow> = flows.iter().collect();
        let wl = SniWhitelist {
            domains: ["good.example.com".to_string()].into_iter().collect(),
        };
        assert!(check_flow_risk(&refs, Some(&wl), now).is_none());
    }

    /// 条件 2 命中：10s 内 ≥ 50 个不同 IP。
    #[test]
    fn condition2_port_scan_hits() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let flows: Vec<ProcessFlow> = (0..60)
            .map(|i| mk_flow(1, 100, &format!("10.0.0.{i}"), None, now))
            .collect();
        let refs: Vec<&ProcessFlow> = flows.iter().collect();
        let risk = check_flow_risk(&refs, None, now).expect("应命中条件 2");
        assert_eq!(risk.weight, R15_WEIGHT);
        assert_eq!(risk.name, "r15_port_scan");
    }

    /// 条件 2 不命中：50 个 IP 但其中一些超出 10s 窗口。
    #[test]
    fn condition2_old_flows_outside_window_ignored() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let recent: Vec<ProcessFlow> = (0..30)
            .map(|i| mk_flow(1, 100, &format!("10.0.0.{i}"), None, now))
            .collect();
        let old: Vec<ProcessFlow> = (0..30)
            .map(|i| {
                mk_flow(
                    1,
                    100,
                    &format!("172.16.0.{i}"),
                    None,
                    now - Duration::from_secs(60),
                )
            })
            .collect();
        let all: Vec<&ProcessFlow> = recent.iter().chain(old.iter()).collect();
        assert_eq!(all.len(), 60);
        assert!(
            check_flow_risk(&all, None, now).is_none(),
            "窗口外 flows 应被忽略"
        );
    }

    /// 条件 2 不命中：49 个不同 IP（< 50 阈值）。
    #[test]
    fn condition2_below_threshold_passes() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let flows: Vec<ProcessFlow> = (0..49)
            .map(|i| mk_flow(1, 100, &format!("10.0.0.{i}"), None, now))
            .collect();
        let refs: Vec<&ProcessFlow> = flows.iter().collect();
        assert!(check_flow_risk(&refs, None, now).is_none());
    }

    /// 白名单解析：注释 / 空行 / trailing dot / 大小写规范化。
    #[test]
    fn whitelist_parsing_handles_comments_empty_and_case() {
        let content = "\
# 这是注释
example.com
GOOD.Example.COM.

# 中间又一空行
  indented.org  .
";
        let set = parse_whitelist(content);
        assert_eq!(set.len(), 3);
        assert!(set.contains("example.com"));
        assert!(set.contains("good.example.com"));
        assert!(set.contains("indented.org"));
    }

    /// 文件不存在 → None；存在 → Some。
    #[test]
    fn load_from_returns_none_when_file_missing() {
        let path = std::env::temp_dir().join("proc-test-sni-whitelist-doesnotexist.txt");
        let _ = std::fs::remove_file(&path);
        assert!(SniWhitelist::load_from(&path).is_none());
    }

    #[test]
    fn load_from_returns_empty_when_file_exists_but_blank() {
        let path = std::env::temp_dir().join("proc-test-sni-whitelist-blank.txt");
        std::fs::write(&path, "# only a comment\n\n").unwrap();
        let wl = SniWhitelist::load_from(&path).expect("存在应返回 Some");
        assert!(wl.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
