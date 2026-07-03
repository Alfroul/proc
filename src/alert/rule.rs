use serde::{Deserialize, Serialize};

use crate::collect::{ProcessInfo, SystemSnapshot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricName {
    CpuUsage,
    MemoryUsage,
    DiskUsagePercent,
    NetworkConnections,
    ProcessCpu(u32),
    ProcessMemory(u32),
    ProcessCount,
    CpuTemperature,
    GpuTemperature,
    CpuThrottlePercent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOp {
    GT,
    GTE,
    LT,
    LTE,
    EQ,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl ComparisonOp {
    #[must_use]
    pub fn compare(&self, value: f64, threshold: f64) -> bool {
        match self {
            ComparisonOp::GT => value > threshold,
            ComparisonOp::GTE => value >= threshold,
            ComparisonOp::LT => value < threshold,
            ComparisonOp::LTE => value <= threshold,
            ComparisonOp::EQ => (value - threshold).abs() < f64::EPSILON,
        }
    }
}

impl MetricName {
    /// Extract metric values from system data. Returns (pid, value) pairs.
    /// Global metrics use pid=0; process-level metrics use actual PIDs.
    #[must_use]
    pub fn extract(&self, snapshot: &SystemSnapshot, procs: &[ProcessInfo]) -> Vec<(u32, f64)> {
        match self {
            MetricName::CpuUsage => {
                vec![(0, snapshot.cpu_usage() as f64)]
            }
            MetricName::MemoryUsage => {
                let (used, total) = snapshot.memory_usage();
                let pct = if total > 0 {
                    used as f64 / total as f64 * 100.0
                } else {
                    0.0
                };
                vec![(0, pct)]
            }
            MetricName::DiskUsagePercent => {
                let (used, total) = snapshot.disk_usage();
                let pct = if total > 0 {
                    used as f64 / total as f64 * 100.0
                } else {
                    0.0
                };
                vec![(0, pct)]
            }
            MetricName::NetworkConnections => {
                let stats = SystemSnapshot::tcp_stats();
                let count = stats.established + stats.time_wait + stats.close_wait + stats.listen;
                vec![(0, count as f64)]
            }
            MetricName::ProcessCpu(target_pid) => {
                if *target_pid == 0 {
                    procs.iter().map(|p| (p.pid, p.cpu_usage as f64)).collect()
                } else {
                    procs
                        .iter()
                        .filter(|p| p.pid == *target_pid)
                        .map(|p| (p.pid, p.cpu_usage as f64))
                        .collect()
                }
            }
            MetricName::ProcessMemory(target_pid) => {
                let (mem_total, _) = snapshot.memory_usage();
                if *target_pid == 0 {
                    procs
                        .iter()
                        .map(|p| {
                            let pct = if mem_total > 0 {
                                p.memory as f64 / mem_total as f64 * 100.0
                            } else {
                                0.0
                            };
                            (p.pid, pct)
                        })
                        .collect()
                } else {
                    procs
                        .iter()
                        .filter(|p| p.pid == *target_pid)
                        .map(|p| {
                            let pct = if mem_total > 0 {
                                p.memory as f64 / mem_total as f64 * 100.0
                            } else {
                                0.0
                            };
                            (p.pid, pct)
                        })
                        .collect()
                }
            }
            MetricName::ProcessCount => {
                vec![(0, procs.len() as f64)]
            }
            MetricName::CpuTemperature => {
                let (cpu_temp, _) = snapshot.temperatures();
                match cpu_temp {
                    Some(t) => vec![(0, t as f64)],
                    None => Vec::new(),
                }
            }
            MetricName::GpuTemperature => {
                let (_, gpu_temp_sys) = snapshot.temperatures();
                let gpu_nvml: Option<f32> = snapshot.gpu_info().iter().find_map(|g| g.temperature);
                match gpu_nvml.or(gpu_temp_sys) {
                    Some(t) => vec![(0, t as f64)],
                    None => Vec::new(),
                }
            }
            MetricName::CpuThrottlePercent => match snapshot.throttle_info() {
                Some(ti) if ti.is_throttled => vec![(0, ti.throttle_pct as f64)],
                _ => Vec::new(),
            },
        }
    }
}
