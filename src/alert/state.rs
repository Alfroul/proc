use std::time::Instant;

use super::rule::AlertSeverity;

/// Silence period: 5 minutes
const SILENCE_DURATION_SECS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertState {
    Pending,
    Firing,
    Resolved,
    Silenced,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub rule_id: String,
    pub state: AlertState,
    pub severity: AlertSeverity,
    pub triggered_at: Instant,
    pub current_value: f64,
    pub threshold: f64,
    pub related_pid: Option<u32>,
    pub hit_count: u32,
    pub consecutive_hits_required: u32,
    pub silenced_at: Option<Instant>,
}

/// Events emitted by Alert::tick()
#[derive(Debug, Clone)]
pub enum AlertEventType {
    /// Transitioned to Firing state
    Fired,
    /// Transitioned back to normal
    Resolved,
}

#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub rule_id: String,
    pub severity: AlertSeverity,
    pub event_type: AlertEventType,
    pub value: f64,
    pub threshold: f64,
    pub related_pid: Option<u32>,
    pub message: String,
}

impl Alert {
    #[must_use]
    pub fn new(
        rule_id: String,
        severity: AlertSeverity,
        threshold: f64,
        consecutive_hits: u32,
        pid: Option<u32>,
    ) -> Self {
        Self {
            rule_id,
            state: AlertState::Pending,
            severity,
            triggered_at: Instant::now(),
            current_value: 0.0,
            threshold,
            related_pid: pid,
            hit_count: 0,
            consecutive_hits_required: consecutive_hits,
            silenced_at: None,
        }
    }

    /// Called each tick. Returns Some(AlertEvent) when a state transition occurs.
    pub fn tick(&mut self, is_triggered: bool, value: f64) -> Option<AlertEvent> {
        self.current_value = value;

        match self.state {
            AlertState::Pending => {
                if is_triggered {
                    self.hit_count += 1;
                    if self.hit_count >= self.consecutive_hits_required {
                        self.state = AlertState::Firing;
                        self.triggered_at = Instant::now();
                        return Some(self.make_event(AlertEventType::Fired));
                    }
                } else {
                    self.hit_count = 0;
                }
                None
            }
            AlertState::Firing => {
                if !is_triggered {
                    self.state = AlertState::Resolved;
                    return Some(self.make_event(AlertEventType::Resolved));
                }
                None
            }
            AlertState::Resolved => {
                if is_triggered {
                    // Re-trigger immediately if was recently firing
                    self.hit_count = 1;
                    self.state = AlertState::Pending;
                }
                None
            }
            AlertState::Silenced => {
                if let Some(silenced_at) = self.silenced_at {
                    if silenced_at.elapsed().as_secs() >= SILENCE_DURATION_SECS {
                        // Silence period over
                        if is_triggered {
                            self.state = AlertState::Firing;
                            self.triggered_at = Instant::now();
                            self.silenced_at = None;
                            return Some(self.make_event(AlertEventType::Fired));
                        } else {
                            self.state = AlertState::Resolved;
                            self.silenced_at = None;
                        }
                    }
                } else {
                    self.state = AlertState::Resolved;
                }
                None
            }
        }
    }

    /// After a Fired event is processed, caller can silence the alert.
    pub fn silence(&mut self) {
        if self.state == AlertState::Firing {
            self.state = AlertState::Silenced;
            self.silenced_at = Some(Instant::now());
        }
    }

    fn make_event(&self, event_type: AlertEventType) -> AlertEvent {
        let message = match event_type {
            AlertEventType::Fired => format!(
                "{}: {} threshold {} ({:.1})",
                self.rule_id,
                self.severity_label(),
                self.threshold,
                self.current_value
            ),
            AlertEventType::Resolved => {
                format!("{}: resolved (now {:.1})", self.rule_id, self.current_value)
            }
        };
        AlertEvent {
            rule_id: self.rule_id.clone(),
            severity: self.severity,
            event_type,
            value: self.current_value,
            threshold: self.threshold,
            related_pid: self.related_pid,
            message,
        }
    }

    #[must_use]
    pub fn severity_label(&self) -> &'static str {
        match self.severity {
            AlertSeverity::Info => "INFO",
            AlertSeverity::Warning => "WARN",
            AlertSeverity::Critical => "CRIT",
        }
    }

    /// Check if this alert is currently active (firing or silenced)
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self.state, AlertState::Firing | AlertState::Silenced)
    }
}
