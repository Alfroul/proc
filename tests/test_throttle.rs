use proc::throttle::{ThrottleInfo, ThrottleReason, classify_throttle, detect_throttle_from_raw};

// ── classify_throttle ──

#[test]
fn test_classify_high_load_high_temp_thermal() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 1800,
        mhz_limit: 1800,
        is_throttled: true,
        throttle_pct: 60.0,
    };
    assert_eq!(
        classify_throttle(&throttle, 80.0, Some(85.0)),
        ThrottleReason::Thermal
    );
}

#[test]
fn test_classify_high_load_high_temp_boundary() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 1800,
        mhz_limit: 1800,
        is_throttled: true,
        throttle_pct: 60.0,
    };
    assert_eq!(
        classify_throttle(&throttle, 80.0, Some(80.0)),
        ThrottleReason::Thermal
    );
}

#[test]
fn test_classify_high_load_low_temp_power_policy() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 3000,
        mhz_limit: 3000,
        is_throttled: true,
        throttle_pct: 33.3,
    };
    assert_eq!(
        classify_throttle(&throttle, 50.0, Some(60.0)),
        ThrottleReason::PowerPolicy
    );
}

#[test]
fn test_classify_high_load_no_temp_power_policy() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 3000,
        mhz_limit: 3000,
        is_throttled: true,
        throttle_pct: 33.3,
    };
    assert_eq!(
        classify_throttle(&throttle, 50.0, None),
        ThrottleReason::PowerPolicy
    );
}

#[test]
fn test_classify_low_load_idle() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 800,
        mhz_limit: 800,
        is_throttled: true,
        throttle_pct: 82.2,
    };
    assert_eq!(
        classify_throttle(&throttle, 10.0, Some(85.0)),
        ThrottleReason::Idle
    );
}

#[test]
fn test_classify_load_boundary_at_20() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 1000,
        mhz_limit: 1000,
        is_throttled: true,
        throttle_pct: 77.8,
    };
    // Exactly 20% — not idle (>= 20), but below PowerPolicy threshold (< 50)
    assert_eq!(
        classify_throttle(&throttle, 20.0, Some(60.0)),
        ThrottleReason::Unknown
    );
}

#[test]
fn test_classify_medium_load_power_policy() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 2500,
        mhz_limit: 2500,
        is_throttled: true,
        throttle_pct: 44.4,
    };
    // 50% load, no thermal issue
    assert_eq!(
        classify_throttle(&throttle, 50.0, Some(60.0)),
        ThrottleReason::PowerPolicy
    );
}

#[test]
fn test_classify_no_throttle_none() {
    let throttle = ThrottleInfo {
        max_mhz: 4500,
        current_mhz: 4500,
        mhz_limit: 4500,
        is_throttled: false,
        throttle_pct: 0.0,
    };
    assert_eq!(
        classify_throttle(&throttle, 90.0, Some(95.0)),
        ThrottleReason::None
    );
}

// ── detect_throttle_from_raw ──

#[test]
fn test_detect_throttle_empty() {
    let cores: Vec<(u32, u32, u32)> = vec![];
    assert!(detect_throttle_from_raw(&cores).is_none());
}

#[test]
fn test_detect_throttle_no_throttle() {
    let cores = vec![(4500, 4500, 4500), (4500, 4500, 4500)];
    let result = detect_throttle_from_raw(&cores).unwrap();
    assert!(!result.is_throttled);
    assert_eq!(result.max_mhz, 4500);
    assert_eq!(result.current_mhz, 4500);
    assert_eq!(result.mhz_limit, 4500);
    assert!((result.throttle_pct - 0.0).abs() < 0.01);
}

#[test]
fn test_detect_throttle_active() {
    let cores = vec![(4500, 2000, 2000), (4500, 1800, 1800)];
    let result = detect_throttle_from_raw(&cores).unwrap();
    assert!(result.is_throttled);
    assert_eq!(result.max_mhz, 4500);
    assert_eq!(result.current_mhz, 1900); // (2000 + 1800) / 2
    assert_eq!(result.mhz_limit, 1800); // min limit
    assert!((result.throttle_pct - 60.0).abs() < 0.1); // (1 - 1800/4500) * 100
}

#[test]
fn test_detect_throttle_single_core() {
    let cores = vec![(3600, 1200, 1200)];
    let result = detect_throttle_from_raw(&cores).unwrap();
    assert!(result.is_throttled);
    assert_eq!(result.max_mhz, 3600);
    assert_eq!(result.current_mhz, 1200);
    assert_eq!(result.mhz_limit, 1200);
    let expected_pct = (1.0 - 1200.0 / 3600.0) * 100.0;
    assert!((result.throttle_pct - expected_pct).abs() < 0.1);
}

// ── Alert metric types exist and compile ──

#[test]
fn test_metric_name_temperature_variants_exist() {
    use proc::alert::MetricName;
    let _ = MetricName::CpuTemperature;
    let _ = MetricName::GpuTemperature;
    let _ = MetricName::CpuThrottlePercent;
}

// ── Alert config default rules include temperature/throttle ──

#[test]
fn test_default_config_includes_temp_rules() {
    use proc::alert::{MetricName, ThresholdConfig};
    let config = ThresholdConfig::default();
    let metrics: Vec<&str> = config
        .rules
        .iter()
        .filter_map(|r| match r.metric {
            MetricName::CpuTemperature => Some("cpu-temp"),
            MetricName::GpuTemperature => Some("gpu-temp"),
            MetricName::CpuThrottlePercent => Some("cpu-throttle"),
            _ => None,
        })
        .collect();
    assert!(
        metrics.contains(&"cpu-temp"),
        "Missing CPU temperature rule"
    );
    assert!(
        metrics.contains(&"gpu-temp"),
        "Missing GPU temperature rule"
    );
    assert!(
        metrics.contains(&"cpu-throttle"),
        "Missing CPU throttle rule"
    );
}
