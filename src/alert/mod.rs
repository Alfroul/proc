pub mod config;
pub mod rule;
pub mod state;

pub use config::{ThresholdConfig, ThresholdRule};
pub use rule::{AlertSeverity, ComparisonOp, MetricName};
pub use state::{Alert, AlertEvent, AlertEventType, AlertState};

use std::collections::HashMap;
use std::path::PathBuf;

use crate::collect::{ProcessInfo, SystemSnapshot};

pub struct AlertManager {
    config: ThresholdConfig,
    active_alerts: HashMap<String, Alert>,
}

impl AlertManager {
    pub fn load_or_default() -> Self {
        let config = Self::load_config().unwrap_or_default();
        Self {
            config,
            active_alerts: HashMap::new(),
        }
    }

    pub fn evaluate(&mut self, snapshot: &SystemSnapshot, procs: &[ProcessInfo]) -> Vec<AlertEvent> {
        let mut events = Vec::new();

        for rule in &self.config.rules {
            let values = rule.metric.extract(snapshot, procs);
            for (pid, value) in values {
                let is_triggered = rule.evaluate(value);
                let key = if pid == 0 {
                    rule.id.clone()
                } else {
                    format!("{}:{}", rule.id, pid)
                };

                let alert = self.active_alerts.entry(key.clone()).or_insert_with(|| {
                    Alert::new(
                        key.clone(),
                        rule.severity,
                        rule.threshold,
                        rule.consecutive_hits,
                        if pid == 0 { None } else { Some(pid) },
                    )
                });

                if let Some(event) = alert.tick(is_triggered, value) {
                    // After firing, silence the alert
                    if matches!(event.event_type, AlertEventType::Fired) {
                        alert.silence();
                    }
                    events.push(event);
                }
            }
        }

        // Clean up resolved alerts that have been resolved for a while
        self.active_alerts.retain(|_, alert| {
            alert.state != AlertState::Resolved || alert.hit_count > 0
        });

        events
    }

    pub fn active_alerts(&self) -> Vec<&Alert> {
        self.active_alerts.values().filter(|a| a.is_active()).collect()
    }

    pub fn all_alerts(&self) -> Vec<&Alert> {
        self.active_alerts.values().collect()
    }

    /// Count of firing (non-silenced) alerts by severity
    pub fn firing_counts(&self) -> (usize, usize, usize) {
        let mut info = 0;
        let mut warning = 0;
        let mut critical = 0;
        for alert in self.active_alerts.values() {
            if alert.state == AlertState::Firing {
                match alert.severity {
                    AlertSeverity::Info => info += 1,
                    AlertSeverity::Warning => warning += 1,
                    AlertSeverity::Critical => critical += 1,
                }
            }
        }
        (info, warning, critical)
    }

    fn load_config() -> anyhow::Result<ThresholdConfig> {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let path = PathBuf::from(home).join(".config").join("proc").join("alerts.toml");
        let content = std::fs::read_to_string(&path)?;
        let config: ThresholdConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn rules(&self) -> &[ThresholdRule] {
        &self.config.rules
    }
}
