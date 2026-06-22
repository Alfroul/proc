//! 阶段 5 B3 — SMART 解析集成测试。
//!
//! 测试策略:
//! 1. 用真实 smartctl 输出 fixture(`tests/fixtures/smartctl_sample.json`)
//!    跑 `parse_smartctl_json`,断言关键属性 ≥ 10 条 + 健康 Ok。
//! 2. 手工构造 failing 样本,断言 `parse_smartctl_json` 返回 Failing。

use proc::smart::{self, SmartHealth};

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/smartctl_sample.json"
);

#[test]
fn parse_real_smartctl_fixture_returns_ok_with_attributes() {
    let content = std::fs::read_to_string(FIXTURE_PATH)
        .expect("fixture file should exist; check tests/fixtures/smartctl_sample.json");
    let data = smart::parse_smartctl_json(&content).expect("parse should succeed");
    assert_eq!(data.health, SmartHealth::Ok);
    assert_eq!(data.device, "/dev/sda");
    assert_eq!(data.model, "Samsung SSD 850 EVO 500GB");
    assert_eq!(data.serial, "S2RANX0J123456A");
    assert_eq!(data.temperature, Some(35.0));
    assert!(
        data.attributes.len() >= 10,
        "fixture should contain at least 10 attributes, got {}",
        data.attributes.len()
    );
    // 关键属性必须被正确解析
    let reallocated = data
        .attributes
        .iter()
        .find(|a| a.id == 5)
        .expect("Reallocated_Sector_Ct (id=5) should be in fixture");
    assert_eq!(reallocated.name, "Reallocated_Sector_Ct");
    assert_eq!(reallocated.value, 100);
    assert_eq!(reallocated.threshold, 10);
    assert_eq!(reallocated.raw_value, 0);
    assert!(!reallocated.failing);
}

#[test]
fn parse_failing_fixture_returns_failing() {
    let failing = r#"{
        "device": { "name": "/dev/sdb" },
        "model_name": "Old HDD",
        "smart_status": { "passed": false },
        "ata_smart_attributes": {
            "table": [
                { "id": 5, "name": "Reallocated_Sector_Ct",
                  "value": 1, "thresh": 5, "raw": { "value": 500 },
                  "when_failed": "now" },
                { "id": 197, "name": "Current_Pending_Sector",
                  "value": 100, "thresh": 0, "raw": { "value": 32 },
                  "when_failed": "now" }
            ]
        }
    }"#;
    let data = smart::parse_smartctl_json(failing).expect("parse should succeed");
    assert_eq!(data.health, SmartHealth::Failing);
    let reallocated = data
        .attributes
        .iter()
        .find(|a| a.id == 5)
        .expect("Reallocated_Sector_Ct should be present");
    assert!(reallocated.failing, "when_failed=now → failing=true");
    let pending = data
        .attributes
        .iter()
        .find(|a| a.id == 197)
        .expect("Current_Pending_Sector should be present");
    assert!(pending.failing);
}

#[test]
fn list_disks_returns_vec_without_panicking() {
    // 仅检查跨平台 list_disks 不 panic 且返回 Vec<String>。
    // 具体值依赖运行环境,只验证类型契约。
    let _disks = smart::list_disks();
}
