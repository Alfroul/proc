use std::collections::HashMap;

use crate::collect::ProcessInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskCategory {
    Signature,
    ParentChain,
    FilePath,
    CommandLine,
    NetworkBehavior,
    Privilege,
}

impl std::fmt::Display for RiskCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Signature => write!(f, "签名"),
            Self::ParentChain => write!(f, "父子链"),
            Self::FilePath => write!(f, "文件路径"),
            Self::CommandLine => write!(f, "命令行"),
            Self::NetworkBehavior => write!(f, "网络行为"),
            Self::Privilege => write!(f, "权限"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RiskFactor {
    pub category: RiskCategory,
    pub name: String,
    pub weight: u32,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct SecurityScore {
    pub score: u32,
    pub factors: Vec<RiskFactor>,
    pub signature: super::signature::SignatureStatus,
}

struct CacheEntry {
    score: SecurityScore,
    created_at: std::time::Instant,
    signature_cached: bool,
}

pub struct CachedScore {
    entries: HashMap<String, CacheEntry>,
    access_order: Vec<String>,
    max_entries: usize,
}

impl CachedScore {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            access_order: Vec::new(),
            max_entries: 500,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<&SecurityScore> {
        if let Some(entry) = self.entries.get_mut(key) {
            // Promote in LRU
            if let Some(pos) = self.access_order.iter().position(|k| k == key) {
                self.access_order.remove(pos);
                self.access_order.push(key.to_string());
            }
            Some(&entry.score)
        } else {
            None
        }
    }

    pub fn insert(&mut self, key: String, score: SecurityScore, signature_cached: bool) {
        if self.entries.len() >= self.max_entries && !self.entries.contains_key(&key) {
            // LRU eviction
            if let Some(old_key) = self.access_order.first().cloned() {
                self.entries.remove(&old_key);
                self.access_order.remove(0);
            }
        }
        if !self.entries.contains_key(&key) {
            self.access_order.push(key.clone());
        }
        self.entries.insert(key, CacheEntry {
            score,
            created_at: std::time::Instant::now(),
            signature_cached,
        });
    }

    pub fn invalidate_pid(&mut self, pid: u32) {
        self.entries.retain(|_, _| true);
        self.access_order.retain(|k| self.entries.contains_key(k));
        let _ = pid; // PID-based invalidation handled at scorer level
    }

    pub fn invalidate_dead_pids(&mut self, alive_pids: &std::collections::HashSet<u32>) {
        // We use (pid, exe_hash) keys — but cache is keyed by string
        // This is handled externally
        let _ = alive_pids;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.access_order.clear();
    }

    fn evict_expired(&mut self, max_age: std::time::Duration) {
        let now = std::time::Instant::now();
        let expired: Vec<String> = self.entries
            .iter()
            .filter(|(_, e)| !e.signature_cached && now.duration_since(e.created_at) > max_age)
            .map(|(k, _)| k.clone())
            .collect();

        for key in &expired {
            self.entries.remove(key);
            self.access_order.retain(|k| k != key);
        }
    }
}

const VERIFY_BUDGET_PER_CYCLE: usize = 3;

pub struct SecurityScorer {
    cache: CachedScore,
    verify_budget: usize,
}

impl SecurityScorer {
    pub fn new() -> Self {
        Self {
            cache: CachedScore::new(),
            verify_budget: VERIFY_BUDGET_PER_CYCLE,
        }
    }

    pub fn reset_budget(&mut self) {
        self.verify_budget = VERIFY_BUDGET_PER_CYCLE;
    }

    pub fn score(
        &mut self,
        proc: &ProcessInfo,
        all_procs: &[ProcessInfo],
        port_entries: &[crate::port_map::PortEntry],
    ) -> SecurityScore {
        let exe_path = proc.exe.as_deref().unwrap_or("");
        let cache_key = format!("{}:{}", proc.pid, hash_path(exe_path));

        // Check cache
        if let Some(cached) = self.cache.get(&cache_key) {
            return cached.clone();
        }

        let mut factors = Vec::new();

        // 1. Signature verification (budget-limited to avoid UI freezes)
        let sig_status = if self.verify_budget > 0 {
            self.verify_budget -= 1;
            super::signature::verify_signature(exe_path)
        } else {
            super::signature::SignatureStatus::Unknown
        };
        if let Some(risk) = super::signature::signature_risk_factor(sig_status) {
            factors.push(risk);
        }

        // 2. Parent chain analysis
        factors.extend(super::parent_chain::analyze_parent_chain(proc, all_procs));

        // 3. Path risk
        factors.extend(super::path_check::check_path_risk(proc.exe.as_deref()));

        // 4. Command line risk
        factors.extend(super::command_line::check_command_line(&proc.cmd));

        // 5. Network behavior
        factors.extend(check_network_behavior(proc.pid, &sig_status, port_entries));

        // Calculate score
        let total_deduction: u32 = factors.iter().map(|f| f.weight).sum();
        let score = 100u32.saturating_sub(total_deduction);

        let result = SecurityScore {
            score,
            factors,
            signature: sig_status,
        };

        let signature_cached = matches!(sig_status, super::signature::SignatureStatus::Trusted)
            || matches!(sig_status, super::signature::SignatureStatus::Signed)
            || matches!(sig_status, super::signature::SignatureStatus::Unsigned)
            || matches!(sig_status, super::signature::SignatureStatus::Revoked);

        // Only cache if signature was actually verified (not budget-exceeded)
        if signature_cached {
            self.cache.insert(cache_key, result.clone(), true);
        }

        result
    }

    pub fn evict_expired(&mut self) {
        self.cache.evict_expired(std::time::Duration::from_secs(30));
    }

    pub fn invalidate_dead(&mut self, alive_pids: &std::collections::HashSet<u32>) {
        self.cache.entries.retain(|key, _| {
            // Key format: "pid:hash"
            if let Some(pid_str) = key.split(':').next() {
                if let Ok(pid) = pid_str.parse::<u32>() {
                    return alive_pids.contains(&pid);
                }
            }
            true
        });
        self.cache.access_order.retain(|k| self.cache.entries.contains_key(k));
    }
}

fn check_network_behavior(
    pid: u32,
    sig_status: &super::signature::SignatureStatus,
    port_entries: &[crate::port_map::PortEntry],
) -> Vec<RiskFactor> {
    let mut factors = Vec::new();
    let mut has_listen = false;
    let mut has_nonstandard_remote = false;

    for entry in port_entries {
        if entry.pid != pid {
            continue;
        }
        if let Some(ref state) = entry.state {
            if state == "LISTEN" || state == "LISTENING" {
                has_listen = true;
            }
        }
        if let Some(port) = entry.remote_port {
            // Non-standard port (>49152) and not common service ports
            if port > 49152 && ![80, 443, 8080, 8443, 53, 25, 110, 143, 993, 995, 587, 3389, 22].contains(&port) {
                has_nonstandard_remote = true;
            }
        }
    }

    if has_listen && matches!(sig_status, super::signature::SignatureStatus::Unsigned) {
        factors.push(RiskFactor {
            category: RiskCategory::NetworkBehavior,
            name: "unsigned_listen".to_string(),
            weight: 15,
            description: "无签名进程监听端口".to_string(),
        });
    }

    if has_nonstandard_remote && matches!(sig_status, super::signature::SignatureStatus::Unsigned) {
        factors.push(RiskFactor {
            category: RiskCategory::NetworkBehavior,
            name: "unsigned_nonstandard".to_string(),
            weight: 5,
            description: "无签名进程连接非标端口".to_string(),
        });
    }

    factors
}

fn hash_path(path: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)[..16].to_string()
}
