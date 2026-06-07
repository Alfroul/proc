use serde::{Deserialize, Serialize};

use super::rule::{AlertSeverity, ComparisonOp, MetricName};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdRule {
    pub id: String,
    pub metric: MetricName,
    pub op: ComparisonOp,
    pub threshold: f64,
    pub consecutive_hits: u32,
    pub severity: AlertSeverity,
    #[serde(default)]
    pub description: String,
}

impl ThresholdRule {
    pub fn evaluate(&self, value: f64) -> bool {
        self.op.compare(value, self.threshold)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdConfig {
    #[serde(default = "default_silence_secs")]
    pub silence_secs: u64,
    #[serde(default)]
    pub rules: Vec<ThresholdRule>,
}

fn default_silence_secs() -> u64 {
    300
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            silence_secs: 300,
            rules: default_rules(),
        }
    }
}

fn default_rules() -> Vec<ThresholdRule> {
    vec![
        ThresholdRule {
            id: "sys-mem-90".into(),
            metric: MetricName::MemoryUsage,
            op: ComparisonOp::GT,
            threshold: 90.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Warning,
            description: "System memory > 90%".into(),
        },
        ThresholdRule {
            id: "sys-mem-95".into(),
            metric: MetricName::MemoryUsage,
            op: ComparisonOp::GT,
            threshold: 95.0,
            consecutive_hits: 2,
            severity: AlertSeverity::Critical,
            description: "System memory > 95%".into(),
        },
        ThresholdRule {
            id: "proc-cpu-95".into(),
            metric: MetricName::ProcessCpu(0),
            op: ComparisonOp::GT,
            threshold: 95.0,
            consecutive_hits: 5,
            severity: AlertSeverity::Warning,
            description: "Single process CPU > 95%".into(),
        },
        ThresholdRule {
            id: "sys-disk-95".into(),
            metric: MetricName::DiskUsagePercent,
            op: ComparisonOp::GT,
            threshold: 95.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Warning,
            description: "Disk usage > 95%".into(),
        },
    ]
}
