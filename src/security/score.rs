use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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

impl Default for CachedScore {
    fn default() -> Self {
        Self::new()
    }
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
        if self.entries.len() >= self.max_entries
            && !self.entries.contains_key(&key)
            && let Some(old_key) = self.access_order.first().cloned()
        {
            self.entries.remove(&old_key);
            self.access_order.remove(0);
        }
        if !self.entries.contains_key(&key) {
            self.access_order.push(key.clone());
        }
        self.entries.insert(
            key,
            CacheEntry {
                score,
                created_at: std::time::Instant::now(),
                signature_cached,
            },
        );
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.access_order.clear();
    }

    fn evict_expired(&mut self, max_age: std::time::Duration) {
        let now = std::time::Instant::now();
        let expired: Vec<String> = self
            .entries
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

/// Signature verification budget per full scoring pass (background thread).
/// WinVerifyTrust can involve network OCSP/CRL checks, so we cap it.
const VERIFY_BUDGET_PER_PASS: usize = 50;

/// Internal scorer — used by BackgroundScorer and tests.
pub struct SecurityScorer {
    cache: CachedScore,
    verify_budget: usize,
    hash_reputation: super::hash_cache::HashReputation,
}

impl Default for SecurityScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityScorer {
    pub fn new() -> Self {
        Self {
            cache: CachedScore::new(),
            verify_budget: VERIFY_BUDGET_PER_PASS,
            hash_reputation: super::hash_cache::HashReputation::new(),
        }
    }

    pub fn reset_budget(&mut self) {
        self.verify_budget = VERIFY_BUDGET_PER_PASS;
    }

    pub fn flush(&mut self) {
        self.hash_reputation.flush();
    }

    pub fn score(
        &mut self,
        proc: &ProcessInfo,
        all_procs: &[ProcessInfo],
        port_entries: &[crate::port_map::PortEntry],
    ) -> SecurityScore {
        let exe_path = proc.exe.as_deref().unwrap_or("");
        let cache_key = format!("{}:{}", proc.pid, exe_path.to_lowercase());

        if let Some(cached) = self.cache.get(&cache_key) {
            return cached.clone();
        }

        let mut factors = Vec::new();

        // 1. Signature verification (budget-limited — WinVerifyTrust is slow)
        // Skip system paths entirely — they're whitelisted by hash_reputation
        let is_sys = super::hash_cache::HashReputation::is_whitelisted(exe_path);
        let cached_sig = if !exe_path.is_empty() && !is_sys {
            self.hash_reputation.get_cached_sig(exe_path)
        } else {
            None
        };
        let sig_status = if is_sys {
            super::signature::SignatureStatus::Trusted
        } else if let Some(cached) = cached_sig {
            cached
        } else if self.verify_budget > 0 {
            self.verify_budget -= 1;
            super::signature::verify_signature(exe_path)
        } else {
            super::signature::SignatureStatus::Unknown
        };
        if let Some(risk) = super::signature::signature_risk_factor(sig_status) {
            factors.push(risk);
        }

        // 2-4. Lightweight checks (pure string matching, no I/O)
        factors.extend(super::parent_chain::analyze_parent_chain(proc, all_procs));
        factors.extend(super::path_check::check_path_risk(proc.exe.as_deref()));
        factors.extend(super::command_line::check_command_line(&proc.cmd));

        // 5. Network behavior
        factors.extend(check_network_behavior(proc.pid, &sig_status, port_entries));

        // 6-9. Behavior checks (pure computation, no I/O)
        if let Some(risk) = super::behavior::check_name_spoofing(&proc.name) {
            factors.push(risk);
        }
        if let Some(risk) = super::behavior::check_resource_anomaly(proc) {
            factors.push(risk);
        }
        if let Some(risk) = super::behavior::check_child_explosion(proc, all_procs) {
            factors.push(risk);
        }
        if let Some(risk) = super::behavior::check_privilege_escalation(proc) {
            factors.push(risk);
        }

        // 10-11. Integrity checks
        if let Some(risk) = super::behavior::check_svchost_integrity(proc, all_procs) {
            factors.push(risk);
        }
        if let Some(risk) = super::behavior::check_name_path_mismatch(proc) {
            factors.push(risk);
        }

        // 12. DLL load check — no budget limit in background, but cap at 20 per pass
        factors.extend(super::dll_check::check_loaded_dlls(proc.pid));

        // 13. Token privilege check — fast API call, no budget needed
        if let Some(risk) =
            super::privilege::check_privilege_tokens(proc.pid, proc.user_id.as_deref())
        {
            factors.push(risk);
        }

        // 14. Hash reputation (path-based cache, no file I/O)
        if !exe_path.is_empty() {
            if matches!(
                sig_status,
                super::signature::SignatureStatus::Trusted
                    | super::signature::SignatureStatus::Signed
                    | super::signature::SignatureStatus::Unsigned
            ) {
                self.hash_reputation.record(exe_path, sig_status);
            }
            if let Some(risk) = self.hash_reputation.check_hash(exe_path) {
                factors.push(risk);
            }
        }

        let total_deduction: u32 = factors.iter().map(|f| f.weight).sum();
        let score = 100u32.saturating_sub(total_deduction);

        let result = SecurityScore {
            score,
            factors,
            signature: sig_status,
        };

        let signature_cached = matches!(
            sig_status,
            super::signature::SignatureStatus::Trusted
                | super::signature::SignatureStatus::Signed
                | super::signature::SignatureStatus::Unsigned
                | super::signature::SignatureStatus::Revoked
        );

        if signature_cached {
            self.cache.insert(cache_key, result.clone(), true);
        }

        result
    }

    pub fn evict_expired(&mut self) {
        self.cache.evict_expired(std::time::Duration::from_secs(30));
    }

    pub fn invalidate_dead(&mut self, alive_pids: &HashSet<u32>) {
        self.cache.entries.retain(|key, _| {
            if let Some(pid_str) = key.split(':').next()
                && let Ok(pid) = pid_str.parse::<u32>()
            {
                return alive_pids.contains(&pid);
            }
            true
        });
        self.cache
            .access_order
            .retain(|k| self.cache.entries.contains_key(k));
    }
}

// ---------------------------------------------------------------------------
// Background scorer — runs security scoring in a dedicated thread
// ---------------------------------------------------------------------------

#[allow(dead_code)]
enum ScoringRequest {
    Score {
        processes: Arc<Vec<ProcessInfo>>,
        ports: Arc<Vec<crate::port_map::PortEntry>>,
    },
    Shutdown,
}

/// Security scoring in a background thread.
/// Main thread sends data via `request()`, receives results via `poll_results()`.
pub struct BackgroundScorer {
    request_tx: std::sync::mpsc::SyncSender<ScoringRequest>,
    result_rx: std::sync::mpsc::Receiver<HashMap<u32, SecurityScore>>,
}

impl Default for BackgroundScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BackgroundScorer {
    fn drop(&mut self) {
        // Best-effort shutdown signal. If the worker is mid-scoring, the
        // queued Shutdown waits in the channel; the thread will process it
        // after the current batch and exit. try_send avoids blocking Drop
        // when the channel is already full.
        let _ = self.request_tx.try_send(ScoringRequest::Shutdown);
    }
}

impl BackgroundScorer {
    pub fn new() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::sync_channel(1);
        let (res_tx, res_rx) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("security-scorer".into())
            .spawn(move || {
                let mut scorer = SecurityScorer::new();

                while let Ok(mut req) = req_rx.recv() {
                    // Drain any queued requests, keep only the latest
                    while let Ok(r) = req_rx.try_recv() {
                        req = r;
                    }

                    match req {
                        ScoringRequest::Score { processes, ports } => {
                            let started = std::time::Instant::now();
                            let alive_pids: HashSet<u32> =
                                processes.iter().map(|p| p.pid).collect();
                            scorer.invalidate_dead(&alive_pids);
                            scorer.evict_expired();
                            scorer.reset_budget();

                            let procs_slice: &[ProcessInfo] = processes.as_ref();
                            let ports_slice: &[crate::port_map::PortEntry] = ports.as_ref();
                            let mut scores = HashMap::new();
                            for proc in procs_slice {
                                let score = scorer.score(proc, procs_slice, ports_slice);
                                scores.insert(proc.pid, score);
                            }
                            scorer.flush();
                            tracing::debug!(
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                procs = procs_slice.len(),
                                "BackgroundScorer 评分完成",
                            );
                            let _ = res_tx.send(scores);
                        }
                        ScoringRequest::Shutdown => break,
                    }
                }
            })
            .expect("failed to spawn security scorer thread");

        BackgroundScorer {
            request_tx: req_tx,
            result_rx: res_rx,
        }
    }

    /// Send new data for scoring. Non-blocking — drops the request if the
    /// worker is still busy with a previous batch.
    pub fn request(
        &self,
        processes: Arc<Vec<ProcessInfo>>,
        ports: Arc<Vec<crate::port_map::PortEntry>>,
    ) {
        let _ = self
            .request_tx
            .try_send(ScoringRequest::Score { processes, ports });
    }

    /// Non-blocking poll for completed scoring results.
    pub fn poll_results(&self) -> Option<HashMap<u32, SecurityScore>> {
        self.result_rx.try_recv().ok()
    }
}

// ---------------------------------------------------------------------------
// Network behavior check (kept private)
// ---------------------------------------------------------------------------

const KNOWN_C2_PORTS: &[u16] = &[
    4444,  // Metasploit default
    5555,  // Common reverse shell
    31337, // Back Orifice
    6666,  // Common C2
    6667,  // IRC C2
    9999,  // Common reverse shell
];

fn check_network_behavior(
    pid: u32,
    sig_status: &super::signature::SignatureStatus,
    port_entries: &[crate::port_map::PortEntry],
) -> Vec<RiskFactor> {
    let mut factors = Vec::new();
    let mut has_listen = false;
    let mut has_c2_port = false;

    for entry in port_entries {
        if entry.pid != pid {
            continue;
        }
        if let Some(ref state) = entry.state
            && (state == "LISTEN" || state == "LISTENING")
        {
            has_listen = true;
            if KNOWN_C2_PORTS.contains(&entry.local_port) {
                has_c2_port = true;
            }
        }
    }

    if has_c2_port {
        factors.push(RiskFactor {
            category: RiskCategory::NetworkBehavior,
            name: "c2_port".to_string(),
            weight: 30,
            description: "监听已知恶意端口".to_string(),
        });
    }

    if has_listen {
        match sig_status {
            super::signature::SignatureStatus::Unsigned => {
                factors.push(RiskFactor {
                    category: RiskCategory::NetworkBehavior,
                    name: "unsigned_listen".to_string(),
                    weight: 15,
                    description: "无签名进程监听端口".to_string(),
                });
            }
            super::signature::SignatureStatus::Unknown => {
                factors.push(RiskFactor {
                    category: RiskCategory::NetworkBehavior,
                    name: "unverified_listen".to_string(),
                    weight: 8,
                    description: "未验证签名进程监听端口".to_string(),
                });
            }
            _ => {}
        }
    }

    factors
}
