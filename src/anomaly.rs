//! 连接异常检测引擎
//!
//! 自动检测网络连接异常模式，生成告警。在 UI 中醒目提示，严重时触发系统通知。

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;

use crate::collect::TcpStats;
use crate::dns_log::DnsQuery;
use crate::port_map::{
    ConnectionDiff, PortEntry, ProcessNetGroup, Protocol, RemoteGroup, service_name,
};

/// 异常严重级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AnomalySeverity {
    Info,
    Warning,
    Critical,
}

/// 单条异常告警
#[derive(Debug, Clone)]
pub struct Anomaly {
    pub rule_id: String,
    pub severity: AnomalySeverity,
    pub title: String,
    pub detail: String,
    pub affected_pid: Option<u32>,
    pub affected_ip: Option<IpAddr>,
}

/// 唯一标识，用于去重和忽略
impl Anomaly {
    #[must_use]
    pub fn id(&self) -> String {
        // rule_id + 附加标识（PID 或 IP）构成唯一键
        match (self.affected_pid, self.affected_ip) {
            (Some(pid), _) => format!("{}:{}", self.rule_id, pid),
            (_, Some(ip)) => format!("{}:{}", self.rule_id, ip),
            _ => self.rule_id.clone(),
        }
    }
}

/// 异常检测引擎
pub struct AnomalyDetector {
    /// 历史连接数（用于检测突增）
    connection_history: VecDeque<usize>,
    /// 已知 LISTEN 端口基线（首次扫描时建立）
    listen_baseline: HashSet<u16>,
    baseline_established: bool,
    /// 上一次检测到的异常 ID（用于 Toast 去重）
    prev_anomaly_ids: HashSet<String>,
    /// 最大历史长度
    history_len: usize,
}

impl Default for AnomalyDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl AnomalyDetector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            connection_history: VecDeque::new(),
            listen_baseline: HashSet::new(),
            baseline_established: false,
            prev_anomaly_ids: HashSet::new(),
            history_len: 30,
        }
    }

    /// 执行所有检测规则，返回当前活跃异常列表
    pub fn detect(
        &mut self,
        entries: &[PortEntry],
        diff: &ConnectionDiff,
        process_groups: &[ProcessNetGroup],
        remote_groups: &[RemoteGroup],
    ) -> Vec<Anomaly> {
        self.detect_with_tcp_stats(entries, diff, process_groups, remote_groups, None)
    }

    /// 同 [`detect`],但额外接收 TCP 传输质量计数。质量检测规则
    /// (R7/R8)只有在拿到非 None 的 `tcp_stats` 时才会触发,否则
    /// 退化为纯连接异常检测 —— 保证阶段 5 之前的调用方继续可用。
    pub fn detect_with_tcp_stats(
        &mut self,
        entries: &[PortEntry],
        diff: &ConnectionDiff,
        process_groups: &[ProcessNetGroup],
        remote_groups: &[RemoteGroup],
        tcp_stats: Option<&TcpStats>,
    ) -> Vec<Anomaly> {
        let mut anomalies = Vec::new();

        // 维护连接数历史
        if self.connection_history.len() >= self.history_len {
            self.connection_history.pop_front();
        }
        self.connection_history.push_back(diff.active_count);

        // R1: CLOSE_WAIT 堆积（单进程 > 5）
        self.detect_close_wait_stack(process_groups, &mut anomalies);

        // R2: 单 IP 高连接数（> 20 ESTABLISHED）
        self.detect_single_ip_high_connections(remote_groups, &mut anomalies);

        // R3: 连接数突增（> 2x 平均值）
        self.detect_connection_spike(&mut anomalies);

        // R4: 新 LISTEN 端口
        self.detect_new_listen_port(entries, &mut anomalies);

        // R5: TIME_WAIT 洪水（> 100）
        self.detect_time_wait_flood(diff, &mut anomalies);

        // R6: 全局 CLOSE_WAIT 过高（> 50）
        self.detect_global_close_wait(diff, &mut anomalies);

        // R7 / R8: 传输质量（阶段 5 D2 新增）
        if let Some(stats) = tcp_stats {
            self.detect_high_retransmit_rate(stats, &mut anomalies);
            self.detect_high_reset_rate(stats, &mut anomalies);
        }

        // 更新上一次异常 ID 集合
        self.prev_anomaly_ids = anomalies.iter().map(|a| a.id()).collect();

        anomalies
    }

    /// 返回本次新增的 Critical 异常（用于 Toast 通知）
    #[must_use]
    pub fn new_critical<'a>(&self, current: &'a [Anomaly]) -> Vec<&'a Anomaly> {
        current
            .iter()
            .filter(|a| a.severity == AnomalySeverity::Critical)
            .filter(|a| !self.prev_anomaly_ids.contains(&a.id()))
            .collect()
    }

    /// 阶段 8 D3 R9：`NewDnsQueryFromNewProcess`。
    ///
    /// 一个进程首次发起 DNS 查询，且其名称不在 `whitelist`（小写进程名集合）
    /// → 生成一条 Warning，每个 PID 仅触发一次。
    ///
    /// - `seen_pids`：跨调用维护，记录已触发过此规则的 PID。第一次见到新 PID
    ///   且不在白名单 → 触发；触发后立刻加入 `seen_pids` 避免重复。
    /// - `whitelist`：常见会做 DNS 的系统 / 浏览器进程名（如 `svchost.exe`、
    ///   `chrome.exe`），用于减少噪声。
    /// - `queries`：本 tick 新到的 DNS 查询日志。
    /// - `pid_to_name`：辅助函数，按 PID 拿进程名（collector 已填 `process_name`，
    ///   但可能为空 / `"?"`，调用方可覆盖）。
    ///
    /// 设计选择：作为独立方法（不复用 [`Self::detect`] 的多规则流水线），因为
    /// DNS 异常是事件驱动而非连接快照驱动，签名与现有规则簇不兼容。
    pub fn detect_new_dns_from_new_process(
        &mut self,
        seen_pids: &mut HashSet<u32>,
        queries: &[DnsQuery],
        whitelist: &HashSet<String>,
    ) -> Vec<Anomaly> {
        let mut out = Vec::new();
        for q in queries {
            if seen_pids.contains(&q.pid) {
                continue;
            }
            seen_pids.insert(q.pid);
            let name_lower = q.process_name.to_lowercase();
            if name_lower.is_empty() || name_lower == "?" {
                // 进程名未知 → 触发（更值得警觉）
            } else if whitelist.contains(&name_lower) {
                continue;
            }
            out.push(Anomaly {
                rule_id: "R9".to_string(),
                severity: AnomalySeverity::Warning,
                title: "新进程发起 DNS 查询".into(),
                detail: format!(
                    "PID {} ({}) 首次查询 DNS: {} ({})",
                    q.pid, q.process_name, q.query_name, q.query_type
                ),
                affected_pid: Some(q.pid),
                affected_ip: None,
            });
        }
        out
    }

    /// R1: 单进程 CLOSE_WAIT 堆积
    fn detect_close_wait_stack(
        &self,
        process_groups: &[ProcessNetGroup],
        anomalies: &mut Vec<Anomaly>,
    ) {
        for group in process_groups {
            if group.close_wait > 5 {
                anomalies.push(Anomaly {
                    rule_id: "R1".to_string(),
                    severity: AnomalySeverity::Warning,
                    title: format!(
                        "进程 {} (PID:{}) 有 {} 个 CLOSE_WAIT",
                        group.process_name, group.pid, group.close_wait
                    ),
                    detail: format!(
                        "进程 {} (PID:{}) 有 {} 个 CLOSE_WAIT 连接，可能存在连接泄漏",
                        group.process_name, group.pid, group.close_wait
                    ),
                    affected_pid: Some(group.pid),
                    affected_ip: None,
                });
            }
        }
    }

    /// R2: 单 IP 高连接数
    fn detect_single_ip_high_connections(
        &self,
        remote_groups: &[RemoteGroup],
        anomalies: &mut Vec<Anomaly>,
    ) {
        for group in remote_groups {
            if group.established > 20 {
                anomalies.push(Anomaly {
                    rule_id: "R2".to_string(),
                    severity: AnomalySeverity::Warning,
                    title: format!(
                        "远程 IP {} 建立了 {} 个连接",
                        group.remote_addr, group.established
                    ),
                    detail: format!(
                        "远程 IP {} 建立了 {} 个 ESTABLISHED 连接，可能异常",
                        group.remote_addr, group.established
                    ),
                    affected_pid: None,
                    affected_ip: Some(group.remote_addr),
                });
            }
        }
    }

    /// R3: 连接数突增
    fn detect_connection_spike(&self, anomalies: &mut Vec<Anomaly>) {
        if self.connection_history.len() < 5 {
            return; // 历史数据不足，跳过
        }
        let current = *self.connection_history.back().unwrap_or(&0);
        // 计算不含最后一个元素的平均值
        let history: Vec<&usize> = self
            .connection_history
            .iter()
            .rev()
            .skip(1)
            .take(10)
            .collect();
        if history.is_empty() {
            return;
        }
        let avg: usize = history.iter().map(|&&v| v).sum::<usize>() / history.len();
        if avg > 0 && current > avg * 2 {
            anomalies.push(Anomaly {
                rule_id: "R3".to_string(),
                severity: AnomalySeverity::Warning,
                title: format!("连接数突增至 {}（平均 {}）", current, avg),
                detail: format!("连接数突增至 {}（最近平均 {}），可能异常", current, avg),
                affected_pid: None,
                affected_ip: None,
            });
        }
    }

    /// R4: 新 LISTEN 端口
    fn detect_new_listen_port(&mut self, entries: &[PortEntry], anomalies: &mut Vec<Anomaly>) {
        let current_listen: HashSet<u16> = entries
            .iter()
            .filter(|e| {
                crate::port_map::TcpState::from_state_str(e.state.as_deref())
                    == crate::port_map::TcpState::Listen
            })
            .map(|e| e.local_port)
            .collect();

        if !self.baseline_established {
            // 首次扫描，建立基线
            self.listen_baseline = current_listen;
            self.baseline_established = true;
            return;
        }

        for &port in &current_listen {
            if !self.listen_baseline.contains(&port) {
                // 找到新 LISTEN 端口，查找对应进程
                let process_info = entries
                    .iter()
                    .find(|e| {
                        e.local_port == port
                            && crate::port_map::TcpState::from_state_str(e.state.as_deref())
                                == crate::port_map::TcpState::Listen
                    })
                    .map(|e| (e.process_name.clone(), e.pid));

                let svc = service_name(port, Protocol::Tcp)
                    .map(|name| format!(" ({})", name))
                    .unwrap_or_default();

                let (proc_name, pid) = process_info
                    .map(|(n, p)| (n, Some(p)))
                    .unwrap_or_else(|| ("未知".to_string(), None));

                anomalies.push(Anomaly {
                    rule_id: format!("R4:{}", port),
                    severity: AnomalySeverity::Info,
                    title: format!("新监听端口: {}{}，进程: {}", port, svc, proc_name),
                    detail: format!(
                        "新监听端口: {}{}，进程: {} (PID:{})",
                        port,
                        svc,
                        proc_name,
                        pid.map(|p| p.to_string()).unwrap_or_default()
                    ),
                    affected_pid: pid,
                    affected_ip: None,
                });
            }
        }

        // 更新基线
        self.listen_baseline = current_listen;
    }

    /// R5: TIME_WAIT 洪水
    fn detect_time_wait_flood(&self, diff: &ConnectionDiff, anomalies: &mut Vec<Anomaly>) {
        if diff.time_wait_count > 100 {
            anomalies.push(Anomaly {
                rule_id: "R5".to_string(),
                severity: AnomalySeverity::Warning,
                title: format!(
                    "TIME_WAIT 连接数 {}，可能耗尽端口资源",
                    diff.time_wait_count
                ),
                detail: format!(
                    "TIME_WAIT 连接数 {}，可能耗尽端口资源",
                    diff.time_wait_count
                ),
                affected_pid: None,
                affected_ip: None,
            });
        }
    }

    /// R6: 全局 CLOSE_WAIT 过高
    fn detect_global_close_wait(&self, diff: &ConnectionDiff, anomalies: &mut Vec<Anomaly>) {
        if diff.close_wait_count > 50 {
            anomalies.push(Anomaly {
                rule_id: "R6".to_string(),
                severity: AnomalySeverity::Critical,
                title: format!(
                    "全局 CLOSE_WAIT 达到 {}，严重连接泄漏",
                    diff.close_wait_count
                ),
                detail: format!(
                    "全局 CLOSE_WAIT 达到 {}，系统可能存在严重连接泄漏",
                    diff.close_wait_count
                ),
                affected_pid: None,
                affected_ip: None,
            });
        }
    }

    /// R7: 高重传率（retransmit / out_segs > 5%）。
    ///
    /// 这是 TCP 传输质量告警,触发条件由阶段 5 D2 定义。计算用累计值
    /// (`TcpStats.retransmitted_segs` / `TcpStats.out_segs`)而非瞬时,
    /// 但比例足够稳定 —— 短时间内的抖动不会触发 5% 阈值。低于
    /// `MIN_OUT_SEGS_FOR_QUALITY` 时(分母太小,统计噪音大)跳过。
    fn detect_high_retransmit_rate(&self, stats: &TcpStats, anomalies: &mut Vec<Anomaly>) {
        const THRESHOLD: f64 = 0.05;
        const MIN_OUT_SEGS_FOR_QUALITY: u64 = 1000;
        if stats.out_segs < MIN_OUT_SEGS_FOR_QUALITY || stats.retransmitted_segs == 0 {
            return;
        }
        let rate = stats.retransmitted_segs as f64 / stats.out_segs as f64;
        if rate > THRESHOLD {
            anomalies.push(Anomaly {
                rule_id: "R7".to_string(),
                severity: AnomalySeverity::Warning,
                title: format!(
                    "TCP 重传率 {:.1}%（{} / {}）",
                    rate * 100.0,
                    stats.retransmitted_segs,
                    stats.out_segs
                ),
                detail: format!(
                    "TCP 重传率 {:.1}%（重传 {} / 总输出 {}），网络可能拥塞或丢包",
                    rate * 100.0,
                    stats.retransmitted_segs,
                    stats.out_segs
                ),
                affected_pid: None,
                affected_ip: None,
            });
        }
    }

    /// R8: 高 RST 率（rst / out_segs > 2%）。
    ///
    /// RST 暴涨通常意味着对端拒绝连接或防火墙切断。阈值低(2%)是
    /// 因为正常系统 RST 量本就少,任何明显比例都值得提示。
    fn detect_high_reset_rate(&self, stats: &TcpStats, anomalies: &mut Vec<Anomaly>) {
        const THRESHOLD: f64 = 0.02;
        const MIN_OUT_SEGS_FOR_QUALITY: u64 = 1000;
        if stats.out_segs < MIN_OUT_SEGS_FOR_QUALITY || stats.reset_segs == 0 {
            return;
        }
        let rate = stats.reset_segs as f64 / stats.out_segs as f64;
        if rate > THRESHOLD {
            anomalies.push(Anomaly {
                rule_id: "R8".to_string(),
                severity: AnomalySeverity::Warning,
                title: format!(
                    "TCP RST 率 {:.1}%（{} / {}）",
                    rate * 100.0,
                    stats.reset_segs,
                    stats.out_segs
                ),
                detail: format!(
                    "TCP RST 率 {:.1}%（RST {} / 总输出 {}），可能存在端口扫描或防火墙切断",
                    rate * 100.0,
                    stats.reset_segs,
                    stats.out_segs
                ),
                affected_pid: None,
                affected_ip: None,
            });
        }
    }
}

#[cfg(test)]
mod tcp_quality_tests {
    use super::*;
    use crate::collect::TcpStats;

    #[test]
    fn r7_high_retransmit_rate_triggers_warning() {
        let det = AnomalyDetector::new();
        let stats = TcpStats {
            retransmitted_segs: 200, // 20%
            out_segs: 1000,
            ..Default::default()
        };
        let mut out = Vec::new();
        det.detect_high_retransmit_rate(&stats, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rule_id, "R7");
        assert_eq!(out[0].severity, AnomalySeverity::Warning);
    }

    #[test]
    fn r7_below_threshold_no_anomaly() {
        let det = AnomalyDetector::new();
        let stats = TcpStats {
            retransmitted_segs: 30, // 3%
            out_segs: 1000,
            ..Default::default()
        };
        let mut out = Vec::new();
        det.detect_high_retransmit_rate(&stats, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn r7_small_sample_skipped() {
        let det = AnomalyDetector::new();
        let stats = TcpStats {
            retransmitted_segs: 50,
            out_segs: 100, // < 1000
            ..Default::default()
        };
        let mut out = Vec::new();
        det.detect_high_retransmit_rate(&stats, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn r8_high_reset_rate_triggers_warning() {
        let det = AnomalyDetector::new();
        let stats = TcpStats {
            reset_segs: 100, // 5%
            out_segs: 2000,
            ..Default::default()
        };
        let mut out = Vec::new();
        det.detect_high_reset_rate(&stats, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rule_id, "R8");
    }

    #[test]
    fn r8_below_threshold_no_anomaly() {
        let det = AnomalyDetector::new();
        let stats = TcpStats {
            reset_segs: 50, // 1%
            out_segs: 5000,
            ..Default::default()
        };
        let mut out = Vec::new();
        det.detect_high_reset_rate(&stats, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn detect_with_tcp_stats_includes_quality_anomalies() {
        let mut det = AnomalyDetector::new();
        let entries: Vec<PortEntry> = Vec::new();
        let diff = ConnectionDiff::default();
        let groups: Vec<ProcessNetGroup> = Vec::new();
        let remotes: Vec<RemoteGroup> = Vec::new();
        let stats = TcpStats {
            retransmitted_segs: 300,
            out_segs: 1000,
            ..Default::default()
        };
        let out = det.detect_with_tcp_stats(&entries, &diff, &groups, &remotes, Some(&stats));
        assert!(
            out.iter().any(|a| a.rule_id == "R7"),
            "R7 should fire on 30% retransmit"
        );
    }

    #[test]
    fn detect_without_tcp_stats_does_not_panic() {
        let mut det = AnomalyDetector::new();
        let entries: Vec<PortEntry> = Vec::new();
        let diff = ConnectionDiff::default();
        let groups: Vec<ProcessNetGroup> = Vec::new();
        let remotes: Vec<RemoteGroup> = Vec::new();
        let out = det.detect_with_tcp_stats(&entries, &diff, &groups, &remotes, None);
        // R7/R8 都不应触发,因为 stats=None;基础规则在空输入下也不会触发。
        assert!(
            out.iter()
                .all(|a| !["R7", "R8"].contains(&a.rule_id.as_str()))
        );
    }
}

#[cfg(test)]
mod dns_anomaly_tests {
    use super::*;
    use crate::dns_log::{DnsQuery, DnsResult};
    use std::time::SystemTime;

    fn q(pid: u32, name: &str, query_name: &str) -> DnsQuery {
        DnsQuery {
            timestamp: SystemTime::UNIX_EPOCH,
            pid,
            start_time: 0,
            process_name: name.into(),
            query_name: query_name.into(),
            query_type: "A".into(),
            result: DnsResult::Success(vec![]),
        }
    }

    fn whitelist(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_lowercase()).collect()
    }

    #[test]
    fn r9_fires_for_new_non_whitelisted_pid() {
        let mut det = AnomalyDetector::new();
        let mut seen = HashSet::new();
        let wl = whitelist(&["svchost.exe", "chrome.exe"]);
        let queries = vec![q(1234, "suspicious.exe", "evil.example.com")];
        let out = det.detect_new_dns_from_new_process(&mut seen, &queries, &wl);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rule_id, "R9");
        assert_eq!(out[0].severity, AnomalySeverity::Warning);
        assert_eq!(out[0].affected_pid, Some(1234));
        assert!(seen.contains(&1234));
    }

    #[test]
    fn r9_no_fire_for_whitelisted_process() {
        let mut det = AnomalyDetector::new();
        let mut seen = HashSet::new();
        let wl = whitelist(&["svchost.exe", "chrome.exe"]);
        let queries = vec![q(500, "chrome.exe", "google.com")];
        let out = det.detect_new_dns_from_new_process(&mut seen, &queries, &wl);
        assert!(out.is_empty());
        // 白名单 PID 仍然加入 seen，避免后续重复判定
        assert!(seen.contains(&500));
    }

    #[test]
    fn r9_fires_only_once_per_pid() {
        let mut det = AnomalyDetector::new();
        let mut seen = HashSet::new();
        let wl = HashSet::new();
        let queries = vec![
            q(100, "a.exe", "first.example.com"),
            q(100, "a.exe", "second.example.com"),
        ];
        let out = det.detect_new_dns_from_new_process(&mut seen, &queries, &wl);
        assert_eq!(out.len(), 1, "同 PID 第二条查询不应再触发");
    }

    #[test]
    fn r9_fires_for_unknown_process_name() {
        // process_name = "?" → 视为未知，仍触发（更值得警觉）
        let mut det = AnomalyDetector::new();
        let mut seen = HashSet::new();
        let wl = whitelist(&["chrome.exe"]);
        let queries = vec![q(999, "?", "suspicious.example.com")];
        let out = det.detect_new_dns_from_new_process(&mut seen, &queries, &wl);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn r9_empty_queries_returns_empty() {
        let mut det = AnomalyDetector::new();
        let mut seen = HashSet::new();
        let wl = HashSet::new();
        let out = det.detect_new_dns_from_new_process(&mut seen, &[], &wl);
        assert!(out.is_empty());
    }

    #[test]
    fn r9_seen_pids_persists_across_calls() {
        let mut det = AnomalyDetector::new();
        let mut seen = HashSet::new();
        let wl = HashSet::new();
        // 第一批：PID 1 触发
        let q1 = vec![q(1, "a.exe", "x.com")];
        let out1 = det.detect_new_dns_from_new_process(&mut seen, &q1, &wl);
        assert_eq!(out1.len(), 1);
        // 第二批：PID 1 再次出现，不触发；PID 2 触发
        let q2 = vec![q(1, "a.exe", "y.com"), q(2, "b.exe", "z.com")];
        let out2 = det.detect_new_dns_from_new_process(&mut seen, &q2, &wl);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0].affected_pid, Some(2));
    }
}
