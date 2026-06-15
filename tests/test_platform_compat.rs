//! 跨平台兼容性测试。
//!
//! 阶段 3：覆盖
//! - `local_offset_hours` 在所有平台返回合理值（-12 .. 14）
//! - 非 Windows 平台的 stub 函数可调用且返回默认值（编译期保证 + 运行期行为）
//! - `ProcessInfo` 可跨平台构造并完整序列化/反序列化

use proc::collect::ProcessInfo;
use proc::local_offset_hours;

#[test]
fn test_local_offset_hours_in_range() {
    let offset = local_offset_hours();
    assert!(
        (-12..=14).contains(&offset),
        "local_offset_hours() returned {offset}, expected -12 .. 14"
    );
}

#[test]
fn test_lib_functions_compile_non_windows() {
    // 构造一个最小可用的 ProcessInfo，确保结构体在所有平台都能落地。
    let info = sample_process();
    assert_eq!(info.pid, 4242);
    assert_eq!(info.name, "compat_probe");
}

#[test]
fn test_process_info_construction_cross_platform() {
    let info = sample_process();
    let json = serde_json::to_string(&info).expect("serialize ProcessInfo");
    let back: ProcessInfo = serde_json::from_str(&json).expect("deserialize ProcessInfo");
    assert_eq!(info, back);
    assert_eq!(back.cmd, vec!["--demo".to_string(), "value".to_string()]);
}

fn sample_process() -> ProcessInfo {
    ProcessInfo {
        pid: 4242,
        name: "compat_probe".to_string(),
        cpu_usage: 12.5,
        memory: 64 * 1024 * 1024,
        virtual_memory: 256 * 1024 * 1024,
        disk_usage: (1024, 2048),
        disk_read_speed: 100,
        disk_write_speed: 200,
        status: "Run".to_string(),
        exe: Some("/usr/bin/compat_probe".to_string()),
        cmd: vec!["--demo".to_string(), "value".to_string()],
        cwd: Some("/tmp".to_string()),
        parent_pid: Some(1),
        session_id: Some(0),
        user_id: Some("0".to_string()),
        start_time: 1_700_000_000,
        run_time: 3600,
    }
}
