//! 跨平台兼容性测试。
//!
//! 阶段 2：覆盖 ADR-0002 cfg-gate 后的 stub 行为
//! - 非 Windows 平台下 `eject::*` 公共 API 全部返回 `ProcError::UsbDetect`
//! - 非 Windows 平台下 `classify_process` 走路径启发式（无 Service Cache）
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

// ===========================================================================
// 阶段 2 — ADR-0002 cfg-gate stub 行为测试（仅非 Windows 编译/运行）
// ===========================================================================

#[cfg(not(target_os = "windows"))]
mod non_windows_stubs {
    use proc::classify::{ProcessClass, classify_process};
    use proc::collect::ProcessInfo;
    use proc::eject;
    use proc::error::ProcError;

    fn fake_proc(pid: u32, exe: Option<&str>) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: "stub".into(),
            cpu_usage: 0.0,
            memory: 0,
            virtual_memory: 0,
            disk_usage: (0, 0),
            disk_read_speed: 0,
            disk_write_speed: 0,
            status: String::new(),
            exe: exe.map(str::to_string),
            cmd: Vec::new(),
            cwd: None,
            parent_pid: None,
            session_id: None,
            user_id: None,
            start_time: 0,
            run_time: 0,
        }
    }

    #[test]
    fn scan_all_devices_unsupported() {
        let err = eject::scan_all_devices().unwrap_err();
        assert!(matches!(err, ProcError::UsbDetect { .. }), "got {:?}", err);
    }

    #[test]
    fn scan_device_locks_unsupported() {
        let err = eject::scan_device_locks('E').unwrap_err();
        assert!(matches!(err, ProcError::UsbDetect { .. }), "got {:?}", err);
    }

    #[test]
    fn scan_device_locks_with_processes_unsupported() {
        let err = eject::scan_device_locks_with_processes('E', &[]).unwrap_err();
        assert!(matches!(err, ProcError::UsbDetect { .. }), "got {:?}", err);
    }

    #[test]
    fn kill_safe_processes_unsupported() {
        let err = eject::kill_safe_processes('E').unwrap_err();
        assert!(matches!(err, ProcError::UsbDetect { .. }), "got {:?}", err);
    }

    #[test]
    fn cli_check_drive_unsupported() {
        let err = eject::cli_check_drive("E:", false).unwrap_err();
        assert!(matches!(err, ProcError::UsbDetect { .. }), "got {:?}", err);
    }

    #[test]
    fn classify_process_heuristic() {
        assert_eq!(classify_process(&fake_proc(0, None)), ProcessClass::Kernel);
        assert_eq!(
            classify_process(&fake_proc(1, Some("/init"))),
            ProcessClass::SystemProcess
        );
        assert_eq!(
            classify_process(&fake_proc(100, Some("/usr/bin/foo"))),
            ProcessClass::SystemProcess
        );
        assert_eq!(
            classify_process(&fake_proc(101, Some("/sbin/init"))),
            ProcessClass::SystemProcess
        );
        assert_eq!(
            classify_process(&fake_proc(200, Some("/home/alice/app"))),
            ProcessClass::UserApp
        );
        assert_eq!(
            classify_process(&fake_proc(201, Some("/root/.cache/x"))),
            ProcessClass::UserApp
        );
        // 无 exe → Unknown
        assert_eq!(
            classify_process(&fake_proc(300, None)),
            ProcessClass::Unknown
        );
        // 未识别路径 → Unknown
        assert_eq!(
            classify_process(&fake_proc(301, Some("/opt/strange"))),
            ProcessClass::Unknown
        );
    }
}
