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
    #[must_use]
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
        // Temperature / throttle rules
        ThresholdRule {
            id: "cpu-temp-80".into(),
            metric: MetricName::CpuTemperature,
            op: ComparisonOp::GT,
            threshold: 80.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Warning,
            description: "CPU temperature > 80°C".into(),
        },
        ThresholdRule {
            id: "cpu-temp-90".into(),
            metric: MetricName::CpuTemperature,
            op: ComparisonOp::GT,
            threshold: 90.0,
            consecutive_hits: 2,
            severity: AlertSeverity::Critical,
            description: "CPU temperature > 90°C".into(),
        },
        ThresholdRule {
            id: "gpu-temp-85".into(),
            metric: MetricName::GpuTemperature,
            op: ComparisonOp::GT,
            threshold: 85.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Warning,
            description: "GPU temperature > 85°C".into(),
        },
        ThresholdRule {
            id: "cpu-throttle-30".into(),
            metric: MetricName::CpuThrottlePercent,
            op: ComparisonOp::GT,
            threshold: 30.0,
            consecutive_hits: 5,
            severity: AlertSeverity::Warning,
            description: "CPU throttle > 30%".into(),
        },
        // v0.7 阶段 6：Linux PSI 规则（ADR-0013）。avg10 滑动平均百分比。
        // 这些规则在非 Linux 平台 metric.extract 返回空 Vec，自然不触发。
        ThresholdRule {
            id: "psi-cpu-some-50".into(),
            metric: MetricName::CpuPressureSome,
            op: ComparisonOp::GT,
            threshold: 50.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Critical,
            description: "CPU pressure (some) avg10 > 50%".into(),
        },
        ThresholdRule {
            id: "psi-mem-some-20".into(),
            metric: MetricName::MemPressureSome,
            op: ComparisonOp::GT,
            threshold: 20.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Warning,
            description: "Memory pressure (some) avg10 > 20%".into(),
        },
        ThresholdRule {
            id: "psi-mem-full-20".into(),
            metric: MetricName::MemPressureFull,
            op: ComparisonOp::GT,
            threshold: 20.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Critical,
            description: "Memory pressure (full) avg10 > 20%".into(),
        },
        ThresholdRule {
            id: "psi-io-some-50".into(),
            metric: MetricName::IoPressureSome,
            op: ComparisonOp::GT,
            threshold: 50.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Warning,
            description: "IO pressure (some) avg10 > 50%".into(),
        },
        ThresholdRule {
            id: "psi-io-full-20".into(),
            metric: MetricName::IoPressureFull,
            op: ComparisonOp::GT,
            threshold: 20.0,
            consecutive_hits: 3,
            severity: AlertSeverity::Critical,
            description: "IO pressure (full) avg10 > 20%".into(),
        },
    ]
}
