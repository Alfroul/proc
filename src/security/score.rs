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

/// 解析 `cache_key`（`{pid}:{start_time}:{exe}`）的前两段为 `(pid, start_time)`。
/// 失败时返回 `None`（保留该条目不动），保证键格式漂移时不会误清整个缓存。
fn parse_alive_key(key: &str) -> Option<(u32, u64)> {
    let mut parts = key.splitn(3, ':');
    let pid = parts.next()?.parse::<u32>().ok()?;
    let start_time = parts.next()?.parse::<u64>().ok()?;
    Some((pid, start_time))
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
    #[must_use]
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

/// DLL load check budget per full scoring pass.
///
/// `check_loaded_dlls` calls `CreateToolhelp32Snapshot` which is expensive
/// (几十~几百毫秒/进程,且会临时持有目标进程的 loader lock)。1000+ 进程
/// 全检查会让 BackgroundScorer worker 跑数分钟,期间持续占用一个 CPU 核,
/// 间接拖累主线程帧率(用户感知为"每 ~4 秒一次卡顿")。
///
/// 每 pass 最多检查 20 个进程;多 pass 内会覆盖完整进程列表(进程顺序
/// 由 cached_processes 决定,随 PID/compute 顺序自然轮换)。
const DLL_CHECK_BUDGET_PER_PASS: usize = 20;

/// Internal scorer — used by BackgroundScorer and tests.
pub struct SecurityScorer {
    cache: CachedScore,
    verify_budget: usize,
    dll_check_budget: usize,
    hash_reputation: super::hash_cache::HashReputation,
    /// v0.7 阶段 8 R15：SNI 白名单。`None` = 文件不存在，R15 条件 1 跳过；
    /// `Some(空)` = 用户显式建空文件，所有 dns_name 都视为不在白名单。
    /// 加载自 `~/.config/proc/sni_whitelist.txt`，构造时一次性读取。
    sni_whitelist: Option<super::flow::SniWhitelist>,
    /// v0.11 阶段 5 R17：用户自定义父子链规则。加载自
    /// `~/.config/proc/lineage_rules.toml`，文件不存在 → 空 Vec（只用内置 3 种 pattern）。
    lineage_rules: Vec<super::lineage::LineageRule>,
    /// v0.11 阶段 6 R18：用户自定义可疑路径规则。加载自
    /// `~/.config/proc/path_rules.toml`，文件不存在 → 空 Vec（只用内置 4 种 SuspiciousPathKind）。
    path_rules: Vec<super::path_rules::PathRule>,
    /// v0.11 阶段 6 R18：当前进程环境变量展开后的用户目录缓存。SecurityScorer
    /// 构造时一次性展开，每次 score 调用复用（避免每进程读 env var）。
    user_dirs: super::path_rules::UserDirs,
}

impl Default for SecurityScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityScorer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: CachedScore::new(),
            verify_budget: VERIFY_BUDGET_PER_PASS,
            dll_check_budget: DLL_CHECK_BUDGET_PER_PASS,
            hash_reputation: super::hash_cache::HashReputation::new(),
            // 文件不存在 → None，R15 自动跳过；解析失败也降级为 None 避免误报。
            sni_whitelist: super::flow::SniWhitelist::load(),
            // v0.11 阶段 5 R17：文件不存在 → 空 Vec，只用内置 OfficeToShell /
            // BrowserToShell / ScriptInterpreter 三种 pattern。
            lineage_rules: super::lineage::load_lineage_rules(),
            // v0.11 阶段 6 R18：文件不存在 → 空 Vec，只用内置 4 种 SuspiciousPathKind
            // （Temp / AppData / LocalAppData / UserProfileDownloads）。
            path_rules: super::path_rules::load_path_rules(),
            user_dirs: super::path_rules::UserDirs::from_env(),
        }
    }

    pub fn reset_budget(&mut self) {
        self.verify_budget = VERIFY_BUDGET_PER_PASS;
        self.dll_check_budget = DLL_CHECK_BUDGET_PER_PASS;
    }

    pub fn flush(&mut self) {
        self.hash_reputation.flush();
    }

    pub fn score(
        &mut self,
        proc: &ProcessInfo,
        all_procs: &[ProcessInfo],
        port_entries: &[crate::port_map::PortEntry],
        flows: &[crate::ebpf::flow::ProcessFlow],
    ) -> SecurityScore {
        let exe_path = proc.exe.as_deref().unwrap_or("");
        // ADR-0003：键加 start_time，PID 复用后旧实例的签名缓存不会过继给新进程。
        let cache_key = format!(
            "{}:{}:{}",
            proc.pid,
            proc.start_time,
            exe_path.to_lowercase()
        );

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

        // 12. DLL load check — `CreateToolhelp32Snapshot` 慢且影响主线程
        // 帧率,每 pass 最多 20 个进程(见 `DLL_CHECK_BUDGET_PER_PASS`)。
        if self.dll_check_budget > 0 {
            self.dll_check_budget -= 1;
            factors.extend(super::dll_check::check_loaded_dlls(proc.pid));
        }

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

        // 15. R15：外联行为评分（v0.7 阶段 8，ADR-0016 §8）。基于 ProcessFlow
        // 的 2 条命中条件：SNI 不在白名单 / 端口扫描特征。flows 为空（非 Linux
        // / 无 ebpf feature / 内核不支持）→ 自动 no-op，与 v0.6 行为一致。
        // （PID 复用：按 (pid, start_time) 双字段过滤，与缓存键一致。）
        let flows_for_pid: Vec<&crate::ebpf::flow::ProcessFlow> = flows
            .iter()
            .filter(|f| f.pid == proc.pid && f.start_time == proc.start_time)
            .collect();
        let now = std::time::SystemTime::now();
        if let Some(risk) =
            super::flow::check_flow_risk(&flows_for_pid, self.sni_whitelist.as_ref(), now)
        {
            factors.push(risk);
        }

        // 17. R17：可疑父子链（v0.11 阶段 5）。基于 ProcessInfo.parent_chain
        // 字段（由 HeavyWorker collect 时填实）+ 当前进程名判定 Office/Browser
        // → Shell / ScriptInterpreter / 用户自定义规则。空 chain 或非 shell 名
        // → check_lineage_risk 返回空 Vec，no-op。stage-5.md 任务 3。
        //
        // 注：R16（v0.11 阶段 4 原方案）已合并到第 1 步 signature verification
        // ——见 ADR-0021。这里命名保留 R17（历史编号），实际是 score 函数第 16
        // 个被调用的检查（R15 后第 1 个新增）。
        factors.extend(super::lineage::check_lineage_risk(
            std::slice::from_ref(proc),
            &self.lineage_rules,
        ));

        // 18. R18：可疑启动路径（v0.11 阶段 6）。基于 `ProcessInfo.exe` + 用户目录
        // 缓存判定 Temp / AppData / LocalAppData / UserProfileDownloads / Custom。
        // 与 v0.6 path_check 第 3 步 temp_dir / downloads_dir **叠加扣分**
        // （同 R17 与 v0.7 office_spawning_shell 的 surgical 原则——安全评分偏向严格）。
        // stage-6.md 任务 2：协同扣分——R16（未签名 / 吊销，第 1 步 sig_status）
        // 同时命中 R18 时额外扣 10 分（双重特征强信号）。
        let r18_hit = super::path_rules::check_path_risk(
            std::slice::from_ref(proc),
            &self.user_dirs,
            &self.path_rules,
        );
        let r18_matched = !r18_hit.is_empty();
        factors.extend(r18_hit);
        if let Some(coop) = r18_cooperation_factor(sig_status, r18_matched) {
            factors.push(coop);
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

    /// 精确清理死亡进程的缓存条目。
    ///
    /// `alive` 以 `(pid, start_time)` 元组传入 —— 即使 PID 被复用（A 死亡 → B 接管同 PID），
    /// 仅 B 在 `alive` 中时，A 的陈旧 entry 也会被清掉。键格式 `{pid}:{start_time}:{exe}`
    /// 见 `score()` 的 `cache_key` 构造。
    pub fn invalidate_dead(&mut self, alive: &HashSet<(u32, u64)>) {
        self.cache
            .entries
            .retain(|key, _| parse_alive_key(key).is_none_or(|k| alive.contains(&k)));
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
        /// v0.7 阶段 8 R15：FlowAggregator drain 出的 ProcessFlow 快照。
        /// 非 Linux / 无 ebpf feature 时为空 Vec，R15 自动 no-op。
        flows: Arc<Vec<crate::ebpf::flow::ProcessFlow>>,
    },
    Shutdown,
}

/// Security scoring in a background thread.
/// Main thread sends data via `request()`, receives results via `poll_results()`.
pub struct BackgroundScorer {
    request_tx: Option<std::sync::mpsc::SyncSender<ScoringRequest>>,
    result_rx: std::sync::mpsc::Receiver<HashMap<u32, SecurityScore>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Default for BackgroundScorer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BackgroundScorer {
    fn drop(&mut self) {
        // Drop the sender BEFORE joining so the worker's `recv()` returns
        // Disconnected after draining the queue. If we kept request_tx
        // alive while joining, and the bounded channel was full (worker
        // busy + a queued request), the try_send Shutdown would fail with
        // Full and the worker would block on `recv()` forever → deadlock.
        if let Some(tx) = self.request_tx.take() {
            // Best-effort shutdown hint. Even if this fails (channel full),
            // dropping `tx` below disconnects the channel and the worker
            // exits on its next `recv()`.
            let _ = tx.try_send(ScoringRequest::Shutdown);
        }
        // Wait for the worker to exit so we never leak a thread. The worker
        // also polls `shutdown::requested()` between processes, so Ctrl+C
        // during a long pass unblocks this join promptly.
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl BackgroundScorer {
    #[must_use]
    pub fn new() -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::sync_channel(1);
        let (res_tx, res_rx) = std::sync::mpsc::channel();

        let handle = std::thread::Builder::new()
            .name("security-scorer".into())
            .spawn(move || {
                let mut scorer = SecurityScorer::new();

                while let Ok(mut req) = req_rx.recv() {
                    // Drain any queued requests, keep only the latest
                    while let Ok(r) = req_rx.try_recv() {
                        req = r;
                    }

                    match req {
                        ScoringRequest::Score {
                            processes,
                            ports,
                            flows,
                        } => {
                            let started = std::time::Instant::now();
                            let alive: HashSet<(u32, u64)> =
                                processes.iter().map(|p| (p.pid, p.start_time)).collect();
                            scorer.invalidate_dead(&alive);
                            scorer.evict_expired();
                            scorer.reset_budget();

                            let procs_slice: &[ProcessInfo] = processes.as_ref();
                            let ports_slice: &[crate::port_map::PortEntry] = ports.as_ref();
                            let flows_slice: &[crate::ebpf::flow::ProcessFlow] = flows.as_ref();
                            let mut scores = HashMap::new();
                            for proc in procs_slice {
                                // Honor global Ctrl+C so a long pass can be
                                // aborted between processes — keeps the Drop
                                // join bounded when the user is trying to quit.
                                if crate::shutdown::requested() {
                                    break;
                                }
                                let score =
                                    scorer.score(proc, procs_slice, ports_slice, flows_slice);
                                scores.insert(proc.pid, score);
                            }
                            scorer.flush();
                            tracing::debug!(
                                elapsed_ms = started.elapsed().as_millis() as u64,
                                procs = procs_slice.len(),
                                flows = flows_slice.len(),
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
            request_tx: Some(req_tx),
            result_rx: res_rx,
            thread: Some(handle),
        }
    }

    /// Send new data for scoring. Non-blocking — drops the request if the
    /// worker is still busy with a previous batch.
    pub fn request(
        &self,
        processes: Arc<Vec<ProcessInfo>>,
        ports: Arc<Vec<crate::port_map::PortEntry>>,
        flows: Arc<Vec<crate::ebpf::flow::ProcessFlow>>,
    ) {
        if let Some(tx) = &self.request_tx {
            let _ = tx.try_send(ScoringRequest::Score {
                processes,
                ports,
                flows,
            });
        }
    }

    /// Non-blocking poll for completed scoring results.
    #[must_use]
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

/// v0.11 阶段 6：R16（未签名 / 吊销）+ R18（可疑路径）协同扣分纯函数。
///
/// 抽出 free function 让单元测试能直接验证状态机（score 函数内 sig_status 由
/// verify_signature 实时算出，无法注入 mock；这里把决策逻辑分离出来）。
///
/// 命中条件（stage-6.md 任务 2）：`r18_matched` 且 `sig_status` 是 Unsigned /
/// Revoked → 返回额外 -10 分 RiskFactor。其他状态（Trusted / Signed / Pending /
/// Unknown）不触发协同（Pending / Unknown 不强信号；Signed 已正常扣分）。
#[must_use]
pub(crate) fn r18_cooperation_factor(
    sig_status: super::signature::SignatureStatus,
    r18_matched: bool,
) -> Option<RiskFactor> {
    if r18_matched
        && matches!(
            sig_status,
            super::signature::SignatureStatus::Unsigned
                | super::signature::SignatureStatus::Revoked
        )
    {
        Some(RiskFactor {
            category: RiskCategory::FilePath,
            name: "unsigned_in_suspicious_path".to_string(),
            weight: 10,
            description: "未签名 + 可疑路径协同命中（双重特征强信号）".to_string(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    //! v0.11 阶段 6：R16 + R18 协同扣分状态机单元测试。
    //! score 函数内 sig_status 由 verify_signature 实时算出无法注入 mock，
    //! 协同决策逻辑抽成 `r18_cooperation_factor` 纯函数在此覆盖。
    use super::*;

    #[test]
    fn coop_unsigned_r18_hit_returns_factor_10() {
        let f = r18_cooperation_factor(super::super::signature::SignatureStatus::Unsigned, true);
        assert!(f.is_some());
        let f = f.unwrap();
        assert_eq!(f.weight, 10);
        assert_eq!(f.name, "unsigned_in_suspicious_path");
        assert_eq!(f.category, RiskCategory::FilePath);
    }

    #[test]
    fn coop_revoked_r18_hit_returns_factor_10() {
        let f = r18_cooperation_factor(super::super::signature::SignatureStatus::Revoked, true);
        assert!(f.is_some());
        assert_eq!(f.unwrap().weight, 10);
    }

    #[test]
    fn coop_trusted_r18_hit_returns_none() {
        let f = r18_cooperation_factor(super::super::signature::SignatureStatus::Trusted, true);
        assert!(f.is_none());
    }

    #[test]
    fn coop_signed_r18_hit_returns_none() {
        // Signed（已签名但非受信 CA）已经 -10 分，不再加协同扣分。
        let f = r18_cooperation_factor(super::super::signature::SignatureStatus::Signed, true);
        assert!(f.is_none());
    }

    #[test]
    fn coop_pending_r18_hit_returns_none() {
        let f = r18_cooperation_factor(super::super::signature::SignatureStatus::Pending, true);
        assert!(f.is_none());
    }

    #[test]
    fn coop_unknown_r18_hit_returns_none() {
        let f = r18_cooperation_factor(super::super::signature::SignatureStatus::Unknown, true);
        assert!(f.is_none());
    }

    #[test]
    fn coop_unsigned_r18_not_hit_returns_none() {
        let f = r18_cooperation_factor(super::super::signature::SignatureStatus::Unsigned, false);
        assert!(f.is_none());
    }
}
