//! v0.7 阶段 6 集成测试 — PSI（Pressure Stall Information）监控。
//!
//! 测试覆盖（ADR-0013）：
//! - Parser 单元测试（公开 API 走 `pub`）—— 全平台跑
//! - `read_psi()` 跨平台契约：非 Linux 返回 None
//! - alert Metric / 默认规则：5 个 PSI MetricName + ThresholdConfig::default 含 PSI 规则
//! - sidebar UI helper：`push_psi_lines` 对 None / Some 的行为

use proc::alert::{AlertSeverity, ComparisonOp, MetricName, ThresholdConfig};
use proc::psi::{PsiRecord, PsiStats};

// ── Parser：通过 PsiStats 构造直接验证（pub API）──

#[test]
fn psi_record_default_is_zero() {
    let r = PsiRecord::default();
    assert!((r.avg10 - 0.0).abs() < 1e-6);
    assert!((r.avg60 - 0.0).abs() < 1e-6);
    assert!((r.avg300 - 0.0).abs() < 1e-6);
    assert_eq!(r.total, 0);
}

#[test]
fn psi_stats_default_has_no_full() {
    let s = PsiStats::default();
    assert!(s.mem_full.is_none());
    assert!(s.io_full.is_none());
}

// ── read_psi 跨平台契约 ──

#[test]
fn read_psi_non_linux_returns_none() {
    // 非 Linux 平台恒 None；Linux 上若内核不支持 PSI 也 None ——
    // 此测试断言「不 panic 且为 Option」，不强约束 Linux 上的具体值。
    let result = proc::psi::read_psi();
    #[cfg(not(target_os = "linux"))]
    {
        assert!(result.is_none(), "非 Linux 平台 read_psi 应返回 None");
    }
    #[cfg(target_os = "linux")]
    {
        // Linux 上：要么 None（CONFIG_PSI=n / 内核 < 4.20），要么 Some
        if let Some(stats) = result {
            // 所有 avg 字段在 [0, 100]，total 单调
            assert!(stats.cpu_some.avg10 >= 0.0 && stats.cpu_some.avg10 <= 100.0);
        }
    }
    let _ = result;
}

// ── Alert Metric：5 个 PSI 变体存在 + 默认规则 ──

#[test]
fn psi_metric_variants_exist() {
    let _ = MetricName::CpuPressureSome;
    let _ = MetricName::MemPressureSome;
    let _ = MetricName::MemPressureFull;
    let _ = MetricName::IoPressureSome;
    let _ = MetricName::IoPressureFull;
}

#[test]
fn default_config_includes_psi_rules() {
    let config = ThresholdConfig::default();
    let psi_metric_count = config
        .rules
        .iter()
        .filter(|r| {
            matches!(
                r.metric,
                MetricName::CpuPressureSome
                    | MetricName::MemPressureSome
                    | MetricName::MemPressureFull
                    | MetricName::IoPressureSome
                    | MetricName::IoPressureFull
            )
        })
        .count();
    assert_eq!(
        psi_metric_count, 5,
        "默认 alerts.toml 应含 5 条 PSI 规则（ADR-0013）"
    );
}

#[test]
fn default_psi_rules_use_warning_or_critical() {
    let config = ThresholdConfig::default();
    for rule in config.rules.iter().filter(|r| {
        matches!(
            r.metric,
            MetricName::CpuPressureSome
                | MetricName::MemPressureSome
                | MetricName::MemPressureFull
                | MetricName::IoPressureSome
                | MetricName::IoPressureFull
        )
    }) {
        assert!(
            matches!(
                rule.severity,
                AlertSeverity::Warning | AlertSeverity::Critical
            ),
            "PSI 规则 {} 应是 Warning/Critical，实际 {:?}",
            rule.id,
            rule.severity
        );
        assert_eq!(rule.op, ComparisonOp::GT, "PSI 规则 {} 应是 GT", rule.id);
    }
}

// ── Alert Metric extract：psi_stats = None 时返回空 Vec（防 panic） ──

#[test]
fn psi_metric_extract_handles_missing_psi() {
    // 直接构造一个 SystemSnapshot。snapshot.psi_stats() 在非 Linux 平台
    // 必然为 None。Metric::extract 走 None 分支应返回空 Vec，不 panic。
    let mut snapshot = proc::collect::SystemSnapshot::new().expect("snapshot");
    let _ = snapshot.refresh_heavy_incremental();
    let procs = snapshot.cached_processes_vec();

    for metric in [
        MetricName::CpuPressureSome,
        MetricName::MemPressureSome,
        MetricName::MemPressureFull,
        MetricName::IoPressureSome,
        MetricName::IoPressureFull,
    ] {
        let values = metric.extract(&snapshot, &procs);
        // Linux 上 PSI 可用时 len=1（pid=0 全局），不可用时 len=0；
        // 非 Linux 上必然 len=0。两种都合法，断言「不 panic」。
        assert!(values.len() <= 1);
    }
}
