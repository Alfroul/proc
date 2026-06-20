//! 连接异常检测引擎
//!
//! 自动检测网络连接异常模式，生成告警。在 UI 中醒目提示，严重时触发系统通知。

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;

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
}
