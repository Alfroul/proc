use proc::alert::state::Alert;
use proc::alert::{
    AlertEventType, AlertManager, AlertSeverity, AlertState, ComparisonOp, MetricName,
    ThresholdConfig, ThresholdRule,
};

fn make_rule(
    id: &str,
    metric: MetricName,
    op: ComparisonOp,
    threshold: f64,
    hits: u32,
    severity: AlertSeverity,
) -> ThresholdRule {
    ThresholdRule {
        id: id.to_string(),
        metric,
        op,
        threshold,
        consecutive_hits: hits,
        severity,
        description: String::new(),
    }
}

#[test]
fn test_rule_evaluate_gt() {
    let rule = make_rule(
        "test",
        MetricName::CpuUsage,
        ComparisonOp::GT,
        90.0,
        1,
        AlertSeverity::Warning,
    );
    assert!(rule.evaluate(91.0));
    assert!(rule.evaluate(100.0));
    assert!(!rule.evaluate(90.0));
    assert!(!rule.evaluate(50.0));
}

#[test]
fn test_rule_evaluate_lte() {
    let rule = make_rule(
        "test",
        MetricName::CpuUsage,
        ComparisonOp::LTE,
        90.0,
        1,
        AlertSeverity::Info,
    );
    assert!(rule.evaluate(90.0));
    assert!(rule.evaluate(50.0));
    assert!(!rule.evaluate(91.0));
}

#[test]
fn test_rule_evaluate_gte() {
    let rule = make_rule(
        "test",
        MetricName::CpuUsage,
        ComparisonOp::GTE,
        90.0,
        1,
        AlertSeverity::Warning,
    );
    assert!(rule.evaluate(90.0));
    assert!(rule.evaluate(91.0));
    assert!(!rule.evaluate(89.9));
}

#[test]
fn test_rule_evaluate_lt() {
    let rule = make_rule(
        "test",
        MetricName::CpuUsage,
        ComparisonOp::LT,
        5.0,
        3,
        AlertSeverity::Warning,
    );
    assert!(rule.evaluate(4.9));
    assert!(!rule.evaluate(5.0));
}

#[test]
fn test_rule_evaluate_eq() {
    let rule = make_rule(
        "test",
        MetricName::CpuUsage,
        ComparisonOp::EQ,
        42.0,
        1,
        AlertSeverity::Info,
    );
    assert!(rule.evaluate(42.0));
    assert!(!rule.evaluate(42.1));
}

#[test]
fn test_metric_extract_cpu() {
    let mut snapshot = proc::collect::SystemSnapshot::new().expect("snapshot");
    let _ = snapshot.refresh_heavy_incremental();
    let procs = snapshot.cached_processes_vec();
    let values = MetricName::CpuUsage.extract(&snapshot, &procs);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].0, 0); // global metric
    assert!(values[0].1 >= 0.0);
    assert!(values[0].1 <= 100.0);
}

#[test]
fn test_metric_extract_memory() {
    let mut snapshot = proc::collect::SystemSnapshot::new().expect("snapshot");
    let _ = snapshot.refresh_heavy_incremental();
    let procs = snapshot.cached_processes_vec();
    let values = MetricName::MemoryUsage.extract(&snapshot, &procs);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].0, 0);
    assert!(values[0].1 >= 0.0);
    assert!(values[0].1 <= 100.0);
}

#[test]
fn test_metric_extract_process_cpu() {
    let mut snapshot = proc::collect::SystemSnapshot::new().expect("snapshot");
    let _ = snapshot.refresh_heavy_incremental();
    let procs = snapshot.cached_processes_vec();
    // ProcessCpu(0) should return all processes
    let values = MetricName::ProcessCpu(0).extract(&snapshot, &procs);
    assert!(!values.is_empty());
    // ProcessCpu with specific PID
    if let Some(first) = procs.first() {
        let values = MetricName::ProcessCpu(first.pid).extract(&snapshot, &procs);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].0, first.pid);
    }
}

#[test]
fn test_metric_extract_process_count() {
    let mut snapshot = proc::collect::SystemSnapshot::new().expect("snapshot");
    let _ = snapshot.refresh_heavy_incremental();
    let procs = snapshot.cached_processes_vec();
    let values = MetricName::ProcessCount.extract(&snapshot, &procs);
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].0, 0);
    assert!(values[0].1 > 0.0);
}

#[test]
fn test_alert_debounce() {
    let mut alert = Alert::new(
        "test-rule".into(),
        AlertSeverity::Warning,
        90.0,
        3, // need 3 consecutive hits
        None,
    );

    // Tick 1: triggered, hit_count=1, no event
    let e = alert.tick(true, 95.0);
    assert!(e.is_none());
    assert_eq!(alert.state, AlertState::Pending);
    assert_eq!(alert.hit_count, 1);

    // Tick 2: triggered, hit_count=2, no event
    let e = alert.tick(true, 96.0);
    assert!(e.is_none());
    assert_eq!(alert.hit_count, 2);

    // Tick 3: triggered, hit_count=3, FIRES
    let e = alert.tick(true, 97.0);
    assert!(e.is_some());
    let event = e.unwrap();
    assert!(matches!(event.event_type, AlertEventType::Fired));
    assert_eq!(alert.state, AlertState::Firing);
}

#[test]
fn test_alert_debounce_reset() {
    let mut alert = Alert::new("test-rule".into(), AlertSeverity::Warning, 90.0, 3, None);

    alert.tick(true, 95.0);
    assert_eq!(alert.hit_count, 1);

    // Not triggered: reset
    alert.tick(false, 50.0);
    assert_eq!(alert.hit_count, 0);

    // Start over
    alert.tick(true, 95.0);
    assert_eq!(alert.hit_count, 1);
    assert_eq!(alert.state, AlertState::Pending);
}

#[test]
fn test_alert_resolve() {
    let mut alert = Alert::new(
        "test-rule".into(),
        AlertSeverity::Warning,
        90.0,
        1, // fires on first hit
        None,
    );

    // Fire
    let e = alert.tick(true, 95.0);
    assert!(e.is_some());
    assert_eq!(alert.state, AlertState::Firing);
    alert.silence();

    // Simulate silence period ending (manually set state back)
    alert.state = AlertState::Firing;

    // Resolve: value returns below threshold
    let e = alert.tick(false, 50.0);
    assert!(e.is_some());
    let event = e.unwrap();
    assert!(matches!(event.event_type, AlertEventType::Resolved));
    assert_eq!(alert.state, AlertState::Resolved);
}

#[test]
fn test_alert_silence() {
    let mut alert = Alert::new("test-rule".into(), AlertSeverity::Critical, 90.0, 1, None);

    // Fire
    alert.tick(true, 95.0);
    assert_eq!(alert.state, AlertState::Firing);

    // Silence it
    alert.silence();
    assert_eq!(alert.state, AlertState::Silenced);

    // During silence period: no new events even if still triggered
    let e = alert.tick(true, 96.0);
    assert!(e.is_none());
    assert_eq!(alert.state, AlertState::Silenced);
}

#[test]
fn test_config_deserialize() {
    let toml_str = r#"
silence_secs = 600

[[rules]]
id = "test-cpu"
metric = "CpuUsage"
op = "GT"
threshold = 90.0
consecutive_hits = 3
severity = "Warning"
description = "CPU too high"

[[rules]]
id = "test-mem"
metric = "MemoryUsage"
op = "GT"
threshold = 85.0
consecutive_hits = 2
severity = "Critical"
"#;
    let config: ThresholdConfig = toml::from_str(toml_str).expect("TOML parse");
    assert_eq!(config.silence_secs, 600);
    assert_eq!(config.rules.len(), 2);
    assert_eq!(config.rules[0].id, "test-cpu");
    assert_eq!(config.rules[0].metric, MetricName::CpuUsage);
    assert_eq!(config.rules[0].op, ComparisonOp::GT);
    assert!((config.rules[0].threshold - 90.0).abs() < f64::EPSILON);
    assert_eq!(config.rules[0].consecutive_hits, 3);
    assert_eq!(config.rules[0].severity, AlertSeverity::Warning);

    assert_eq!(config.rules[1].id, "test-mem");
    assert_eq!(config.rules[1].severity, AlertSeverity::Critical);
}

#[test]
fn test_config_process_metric() {
    let toml_str = r#"
[[rules]]
id = "proc-cpu"
metric = { ProcessCpu = 0 }
op = "GT"
threshold = 80.0
consecutive_hits = 5
severity = "Info"
"#;
    let config: ThresholdConfig = toml::from_str(toml_str).expect("TOML parse");
    assert_eq!(config.rules.len(), 1);
    assert_eq!(config.rules[0].metric, MetricName::ProcessCpu(0));
}

#[test]
fn test_default_rules() {
    let mgr = AlertManager::load_or_default();
    let rules = mgr.rules();
    assert!(!rules.is_empty(), "Should have default rules");
    // Check the 4 default rules
    let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"sys-mem-90"));
    assert!(ids.contains(&"sys-mem-95"));
    assert!(ids.contains(&"proc-cpu-95"));
    assert!(ids.contains(&"sys-disk-95"));
}

#[test]
fn test_manager_evaluate() {
    let mgr = AlertManager::load_or_default();
    // Verify manager can be created and has active_alerts initially empty
    assert!(mgr.active_alerts().is_empty());
}

#[test]
fn test_manager_full_flow() {
    let mut mgr = AlertManager::load_or_default();
    let mut snapshot = proc::collect::SystemSnapshot::new().expect("snapshot");
    let _ = snapshot.refresh_heavy_incremental();
    let procs = snapshot.cached_processes_vec();

    // First evaluation should not crash and may produce events
    let events = mgr.evaluate(&snapshot, &procs);
    // No events expected on first tick since hit_count starts at 0
    assert!(events.is_empty());

    // After multiple evaluations, alerts may or may not fire depending on system state
    for _ in 0..10 {
        let _ = mgr.evaluate(&snapshot, &procs);
    }

    // Verify active_alerts() doesn't panic
    let _ = mgr.active_alerts();
    let _ = mgr.firing_counts();
}

#[test]
fn test_alert_severity_ordering() {
    // Verify severity enums exist and compare correctly
    assert_ne!(AlertSeverity::Info, AlertSeverity::Warning);
    assert_ne!(AlertSeverity::Warning, AlertSeverity::Critical);
    assert_ne!(AlertSeverity::Info, AlertSeverity::Critical);
}

#[test]
fn test_comparison_ops() {
    assert!(ComparisonOp::GT.compare(10.0, 5.0));
    assert!(!ComparisonOp::GT.compare(5.0, 10.0));

    assert!(ComparisonOp::GTE.compare(5.0, 5.0));
    assert!(!ComparisonOp::GTE.compare(4.9, 5.0));

    assert!(ComparisonOp::LT.compare(4.0, 5.0));
    assert!(!ComparisonOp::LT.compare(5.0, 5.0));

    assert!(ComparisonOp::LTE.compare(5.0, 5.0));
    assert!(!ComparisonOp::LTE.compare(5.1, 5.0));

    assert!(ComparisonOp::EQ.compare(5.0, 5.0));
    assert!(!ComparisonOp::EQ.compare(5.1, 5.0));
}
