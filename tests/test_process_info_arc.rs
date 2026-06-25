//! v0.6.0 阶段 4 — ProcessInfo Arc 化 + ProcessStatus 枚举单元测试。
//!
//! 验证：
//! - `name / exe / cmd / cwd / user_id` 字段是 `Arc` 化的（Clone 不分配堆）
//! - `name_lower` 预计算字段
//! - `ProcessStatus` 的 `From<sysinfo::ProcessStatus>` 映射 + badge/tooltip 完整性
//! - serde round-trip：序列化 `Arc<str>` 字段与 `String` 等价，`#[serde(skip)] name_lower` 不持久化

use proc::collect::{ProcessInfo, ProcessStatus};
use std::sync::Arc;

#[test]
fn arc_str_clone_is_refcount_only() {
    // Arc<str> clone 是原子计数，不应触发堆分配。这里只验证类型可 Clone
    // 并共享底层指针（通过 Arc::ptr_eq 验证）。
    let info = ProcessInfo {
        pid: 100,
        name: Arc::from("test.exe"),
        name_lower: Arc::from("test.exe"),
        ..ProcessInfo::default()
    };
    let cloned = info.clone();
    assert!(Arc::ptr_eq(&info.name, &cloned.name));
    assert!(Arc::ptr_eq(&info.name_lower, &cloned.name_lower));
}

#[test]
fn default_has_empty_name_and_unknown_status() {
    let info = ProcessInfo::default();
    assert_eq!(&*info.name, "");
    assert_eq!(&*info.name_lower, "");
    assert_eq!(info.status, ProcessStatus::Unknown);
    assert_eq!(info.cmd.len(), 0);
    assert!(info.exe.is_none());
}

#[test]
fn process_status_from_sysinfo_run() {
    let s = sysinfo::ProcessStatus::Run;
    let p = ProcessStatus::from(s);
    assert_eq!(p, ProcessStatus::Run);
}

#[test]
fn process_status_from_sysinfo_sleep() {
    let s = sysinfo::ProcessStatus::Sleep;
    assert_eq!(ProcessStatus::from(s), ProcessStatus::Sleep);
}

#[test]
fn process_status_from_sysinfo_zombie() {
    let s = sysinfo::ProcessStatus::Zombie;
    assert_eq!(ProcessStatus::from(s), ProcessStatus::Zombie);
}

#[test]
fn process_status_from_sysinfo_unknown_with_code() {
    // sysinfo 0.34 的 Unknown 带 u32 参数；映射到我们的 Unknown 无参变体。
    let s = sysinfo::ProcessStatus::Unknown(42);
    assert_eq!(ProcessStatus::from(s), ProcessStatus::Unknown);
}

#[test]
fn process_status_from_sysinfo_uninterruptible_disk_sleep() {
    let s = sysinfo::ProcessStatus::UninterruptibleDiskSleep;
    assert_eq!(
        ProcessStatus::from(s),
        ProcessStatus::UninterruptibleDiskSleep
    );
}

#[test]
fn process_status_from_sysinfo_lockblocked() {
    let s = sysinfo::ProcessStatus::LockBlocked;
    assert_eq!(ProcessStatus::from(s), ProcessStatus::LockBlocked);
}

#[test]
fn process_status_badge_and_tooltip_cover_all_variants() {
    // 每个变体都有非空 badge + tooltip，且 tooltip 至少和 badge 一样长。
    for variant in [
        ProcessStatus::Idle,
        ProcessStatus::Run,
        ProcessStatus::Sleep,
        ProcessStatus::Stop,
        ProcessStatus::Zombie,
        ProcessStatus::Tracing,
        ProcessStatus::Dead,
        ProcessStatus::Wakekill,
        ProcessStatus::Waking,
        ProcessStatus::Parked,
        ProcessStatus::LockBlocked,
        ProcessStatus::UninterruptibleDiskSleep,
        ProcessStatus::Unknown,
    ] {
        let badge = variant.badge();
        let tooltip = variant.tooltip();
        let as_str = variant.as_str();
        assert!(!badge.is_empty(), "{:?} badge empty", variant);
        assert!(!tooltip.is_empty(), "{:?} tooltip empty", variant);
        assert!(!as_str.is_empty(), "{:?} as_str empty", variant);
        assert!(tooltip.len() >= badge.len());
    }
}

#[test]
fn process_status_display_matches_as_str() {
    // Display 实现应等价于 as_str，TUI 表格 Cell::from(proc.status.to_string()) 行为与
    // 原 String 字段一致。
    assert_eq!(ProcessStatus::Run.to_string(), "Run");
    assert_eq!(ProcessStatus::Zombie.to_string(), "Zombie");
    assert_eq!(
        ProcessStatus::UninterruptibleDiskSleep.to_string(),
        "UninterruptibleDiskSleep"
    );
    assert_eq!(ProcessStatus::Unknown.to_string(), "Unknown");
}

#[test]
fn serde_round_trip_preserves_all_arc_fields() {
    // 启用 serde rc feature 后，Arc<str> 序列化等价于 str，Arc<[String]> 等价于 [String]。
    // round-trip 后字段值应严格等价。
    let info = ProcessInfo {
        pid: 4242,
        name: Arc::from("chrome.exe"),
        cpu_usage: 17.5,
        memory: 1_000_000,
        virtual_memory: 2_000_000,
        disk_usage: (100, 200),
        disk_read_speed: 10,
        disk_write_speed: 20,
        net_sent_rate: 30,
        net_recv_rate: 40,
        status: ProcessStatus::Run,
        exe: Some(Arc::from("C:\\Program Files\\chrome.exe")),
        cmd: Arc::from(vec!["chrome.exe".to_string(), "--flag".to_string()]),
        cwd: Some(Arc::from("C:\\Users")),
        parent_pid: Some(1),
        session_id: Some(0),
        user_id: Some(Arc::from("alice")),
        start_time: 1_700_000_000,
        run_time: 3600,
        // name_lower 不参与 round-trip（serde skip），由 heavy worker 重算。
        name_lower: Arc::from("chrome.exe"),
    };

    let json = serde_json::to_string(&info).expect("serialize");
    let back: ProcessInfo = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(back.pid, 4242);
    assert_eq!(&*back.name, "chrome.exe");
    assert!((back.cpu_usage - 17.5).abs() < f32::EPSILON);
    assert_eq!(back.memory, 1_000_000);
    assert_eq!(back.disk_usage, (100, 200));
    assert_eq!(back.status, ProcessStatus::Run);
    assert_eq!(&*back.exe.unwrap(), "C:\\Program Files\\chrome.exe");
    assert_eq!(back.cmd.len(), 2);
    assert_eq!(&*back.cmd[0], "chrome.exe");
    assert_eq!(&*back.cmd[1], "--flag");
    assert_eq!(back.cwd.as_deref(), Some("C:\\Users"));
    assert_eq!(back.user_id.as_deref(), Some("alice"));
}

#[test]
fn serde_skips_name_lower_field() {
    // 录屏文件不应包含 name_lower（重算成本低且能减小 .prec 体积）。
    let info = ProcessInfo {
        pid: 1,
        name: Arc::from("ABC.exe"),
        name_lower: Arc::from("abc.exe"),
        ..ProcessInfo::default()
    };
    let json = serde_json::to_string(&info).expect("serialize");
    assert!(
        !json.contains("name_lower"),
        "name_lower must not be serialized, got: {json}"
    );

    // 反序列化后 name_lower 应为空（Default）。
    let back: ProcessInfo = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(&*back.name, "ABC.exe");
    assert_eq!(&*back.name_lower, "");
}

#[test]
fn arc_cmd_iterates_and_derefs() {
    // Arc<[String]> 应通过 .iter() 或 * 解引用正常遍历。
    let cmd: Arc<[String]> = Arc::from(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let collected: Vec<String> = cmd.iter().cloned().collect();
    assert_eq!(
        collected,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}
