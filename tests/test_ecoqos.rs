//! v0.7 阶段 6 集成测试 — Windows 11 EcoQoS / Efficiency Mode（ADR-0014）。
//!
//! 与 `tests/test_throttle.rs`（CPU 频率节流检测，v0.6）共存：两者共享
//! `src/throttle.rs` 文件，但语义独立。
//!
//! 测试覆盖：
//! - `EcoQoSState` 枚举的 badge / label 跨平台
//! - `query_throttle_batch` 跨平台契约（空 PID / 多 PID）
//! - `set_throttle` 非 Windows 平台返回错误
//! - ProcessInfo 新增 `throttled` 字段，serde `#[serde(default)]` 兼容
//! - Windows 平台：对自身进程做 set + query 往返

use proc::collect::ProcessInfo;
use proc::throttle::EcoQoSState;

// ── EcoQoSState 枚举 ──

#[test]
fn ecoqos_default_is_normal() {
    assert_eq!(EcoQoSState::default(), EcoQoSState::Normal);
}

#[test]
fn ecoqos_badge_eco_has_leaf() {
    // 🍃 U+1F343。Eco 时返回带 🍃 的字符串；其它返回空。
    let badge = EcoQoSState::Eco.badge();
    assert!(badge.contains('\u{1F343}'), "Eco badge 应含 🍃: {badge:?}");

    assert_eq!(EcoQoSState::Normal.badge(), "");
    assert_eq!(EcoQoSState::Unknown.badge(), "");
}

#[test]
fn ecoqos_label_distinct() {
    assert_eq!(EcoQoSState::Normal.label(), "Normal");
    assert_eq!(EcoQoSState::Eco.label(), "Eco");
    assert_eq!(EcoQoSState::Unknown.label(), "Unknown");
}

// ── query_throttle_batch 跨平台契约 ──

#[test]
fn query_throttle_batch_empty_returns_empty_map() {
    let map = proc::throttle::query_throttle_batch(&[]);
    assert!(map.is_empty());
}

#[test]
fn query_throttle_batch_non_windows_all_unknown() {
    // 非 Windows 平台 stub：每个 PID 都返回 Unknown。
    // Windows 上 PID 0（System Idle Process）/ PID 4（System）通常无权 set，
    // query 返回 Unknown 也合法。本测试只验证「不 panic + 每个 PID 都有 entry」。
    let pids = vec![0, 4, 999_999]; // 999_999 几乎肯定不存在
    let map = proc::throttle::query_throttle_batch(&pids);
    assert_eq!(map.len(), 3);
    for &pid in &pids {
        assert!(map.contains_key(&pid), "missing entry for PID {pid}");
    }

    #[cfg(not(target_os = "windows"))]
    {
        for &state in map.values() {
            assert_eq!(state, EcoQoSState::Unknown);
        }
    }
}

// ── set_throttle 跨平台契约 ──

#[test]
fn set_throttle_non_windows_returns_error() {
    #[cfg(not(target_os = "windows"))]
    {
        let r = proc::throttle::set_throttle(1234, true);
        assert!(r.is_err(), "非 Windows 平台 set_throttle 应返回 Err");
    }
    #[cfg(target_os = "windows")]
    {
        // Windows 上对不存在的 PID set → OpenProcess 失败 → Err
        let r = proc::throttle::set_throttle(9_999_999, true);
        assert!(r.is_err(), "Windows 上对不存在的 PID 应返回 Err");
    }
}

// ── ProcessInfo.throttled 字段 ──

#[test]
fn process_info_default_has_normal_throttle() {
    let p = ProcessInfo::default();
    assert_eq!(p.throttled, EcoQoSState::Normal);
}

#[test]
fn process_info_throttled_serde_default_compat() {
    // 旧录屏文件不含 throttled 字段 → 反序列化时应走 #[serde(default)] → Normal
    let old_json = r#"{
        "pid": 1234,
        "name": "test.exe",
        "cpu_usage": 0.0,
        "memory": 0,
        "virtual_memory": 0,
        "disk_usage": [0, 0],
        "disk_read_speed": 0,
        "disk_write_speed": 0,
        "net_sent_rate": 0,
        "net_recv_rate": 0,
        "status": "Run",
        "exe": null,
        "cmd": [],
        "cwd": null,
        "parent_pid": null,
        "session_id": null,
        "user_id": null,
        "start_time": 0,
        "run_time": 0
    }"#;
    let p: ProcessInfo = serde_json::from_str(old_json).expect("deserialize");
    assert_eq!(
        p.throttled,
        EcoQoSState::Normal,
        "缺 throttled 字段应走 default"
    );
}

// ── Windows 实际 round-trip（仅 Windows 跑） ──

#[cfg(target_os = "windows")]
#[test]
fn ecoqos_roundtrip_on_self_process() {
    // 测试自身进程的 EcoQoS set + query 往返。
    // Win11 build < 22000：SetProcessInformation 不报错但可能不生效，query 可能
    // 返回 Normal 而非 Eco —— 这种环境用「set 不报错」作为弱断言。
    let r = proc::throttle::set_throttle(std::process::id(), true);
    assert!(r.is_ok(), "set_throttle(self, true) 应成功: {:?}", r.err());
    let state = proc::throttle::query_throttle(std::process::id());
    // 至少不应是 Unknown（query 失败）—— 自身进程有 PROCESS_QUERY_LIMITED_INFORMATION
    assert_ne!(
        state,
        EcoQoSState::Unknown,
        "query 自身进程 EcoQoS 不应失败"
    );
    // 还原 Normal，避免影响后续测试
    let _ = proc::throttle::set_throttle(std::process::id(), false);
}
