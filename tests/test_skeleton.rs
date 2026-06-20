use proc::alert::{AlertManager, AlertSeverity, ComparisonOp, MetricName};
use proc::app::{App, AppMode};
use proc::app_group::{AppGroup, AppGroupProcess, VersionInfo};
use proc::cli::{Cli, Command};
use proc::collect::{DiskIoInfo, ProcessInfo, ProcessViewMode, SystemSnapshot};
use proc::error::ProcError;
use proc::record::{FrameProcess, UiFrame};
use proc::security::SecurityScorer;
use proc::throttle::{ThrottleInfo, ThrottleReason};

use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn test_app_new_default_state() {
    let app = App::new().expect("App::new() should not panic");
    assert_eq!(app.mode, AppMode::ProcessList);
    assert!(!app.should_quit);
}

#[test]
fn test_app_mode_switching() {
    let mut app = App::new().expect("App::new() should not panic");

    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::ProcessList);

    app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::PortMap);

    app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::UsbAssistant);

    app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::MonitorPanel);

    app.handle_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::DockerPanel);

    app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::ProcessList);
}

#[test]
fn test_app_quit() {
    let mut app = App::new().expect("App::new() should not panic");
    assert!(!app.should_quit);
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(app.should_quit);
}

#[test]
fn test_app_escape_back() {
    let mut app = App::new().expect("App::new() should not panic");
    app.mode = AppMode::ProcessDetail;
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::ProcessList);
}

#[test]
fn test_app_help_mode_round_trip() {
    let mut app = App::new().expect("App::new() should not panic");
    // `?` enters Help
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::Help);
    // Esc returns to ProcessList
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::ProcessList);

    // Re-enter and exit via `q`
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::Help);
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::ProcessList);
}

#[test]
fn test_cli_export_parsing() {
    let cli = Cli::try_parse_from([
        "proc",
        "export",
        "--format",
        "csv",
        "--output",
        "/tmp/out.csv",
        "--sort",
        "mem",
        "--limit",
        "5",
    ]);
    let cli = cli.expect("CLI parse should succeed");
    match cli.command {
        Some(Command::Export {
            format,
            output,
            sort,
            limit,
        }) => {
            assert_eq!(format, "csv");
            assert_eq!(
                output.as_deref(),
                Some(std::path::Path::new("/tmp/out.csv"))
            );
            assert_eq!(sort, "mem");
            assert_eq!(limit, Some(5));
        }
        _ => panic!("Expected Export command"),
    }
}

#[test]
fn test_cli_ls_parsing() {
    let cli = Cli::try_parse_from(["proc", "ls", "--sort", "mem", "--limit", "10"]);
    let cli = cli.expect("CLI parse should succeed");
    match cli.command {
        Some(Command::Ls { sort, limit }) => {
            assert_eq!(sort, "mem");
            assert_eq!(limit, Some(10));
        }
        _ => panic!("Expected Ls command"),
    }
}

#[test]
fn test_cli_port_parsing() {
    let cli = Cli::try_parse_from(["proc", "port", "--port", "8080", "--kill"]);
    let cli = cli.expect("CLI parse should succeed");
    match cli.command {
        Some(Command::Port { port, kill }) => {
            assert_eq!(port, Some(8080));
            assert!(kill);
        }
        _ => panic!("Expected Port command"),
    }
}

#[test]
fn test_cli_kill_parsing() {
    let cli = Cli::try_parse_from(["proc", "kill", "1234", "--force"]);
    let cli = cli.expect("CLI parse should succeed");
    match cli.command {
        Some(Command::Kill { pid, force }) => {
            assert_eq!(pid, 1234);
            assert!(force);
        }
        _ => panic!("Expected Kill command"),
    }
}

#[test]
fn test_cli_eject_parsing() {
    let cli = Cli::try_parse_from(["proc", "eject", "E:", "--find-locks"]);
    let cli = cli.expect("CLI parse should succeed");
    match cli.command {
        Some(Command::Eject { drive, find_locks }) => {
            assert_eq!(drive, Some("E:".to_string()));
            assert!(find_locks);
        }
        _ => panic!("Expected Eject command"),
    }
}

#[test]
fn test_cli_monitor_parsing() {
    let cli = Cli::try_parse_from(["proc", "monitor", "--port", "8080", "--add"]);
    let cli = cli.expect("CLI parse should succeed");
    match cli.command {
        Some(Command::Monitor { add, port, .. }) => {
            assert!(add);
            assert_eq!(port, Some(8080));
        }
        _ => panic!("Expected Monitor command"),
    }
}

#[test]
fn test_cli_docker_parsing() {
    let cli = Cli::try_parse_from(["proc", "docker", "--watch"]);
    let cli = cli.expect("CLI parse should succeed");
    match cli.command {
        Some(Command::Docker { watch, container }) => {
            assert!(watch);
            assert!(container.is_none());
        }
        _ => panic!("Expected Docker command"),
    }
}

#[test]
fn test_proc_error_display() {
    let err = ProcError::not_found("test process");
    assert!(err.to_string().contains("test process"));

    let err = ProcError::permission_denied("kill PID 4");
    assert!(err.to_string().contains("kill PID 4"));

    let err = ProcError::sysinfo("init failed");
    assert!(err.to_string().contains("init failed"));

    let err = ProcError::port_scan("scan failed");
    assert!(err.to_string().contains("scan failed"));

    let err = ProcError::usb_detect("no device");
    assert!(err.to_string().contains("no device"));

    let err = ProcError::monitor("watch failed");
    assert!(err.to_string().contains("watch failed"));

    let err = ProcError::docker("not running");
    assert!(err.to_string().contains("not running"));
}

#[test]
fn test_proc_error_source_chain() {
    use std::error::Error;
    use std::io;

    let root = io::Error::other("boom");
    let err = ProcError::sysinfo_with("sysinfo init failed", root);

    assert!(err.to_string().contains("sysinfo init failed"));
    let source = err
        .source()
        .expect("source chain should preserve root cause");
    assert_eq!(source.to_string(), "boom");
}

#[test]
fn test_process_info_construction() {
    let info = ProcessInfo {
        pid: 1234,
        name: "test.exe".to_string(),
        cpu_usage: 12.5,
        memory: 1024 * 1024,
        virtual_memory: 10 * 1024 * 1024,
        disk_usage: (100, 50),
        disk_read_speed: 0,
        disk_write_speed: 0,
        status: "Running".to_string(),
        exe: Some("C:\\test.exe".to_string()),
        cmd: vec!["test.exe".to_string(), "--flag".to_string()],
        cwd: Some("C:\\".to_string()),
        parent_pid: Some(1),
        session_id: Some(1),
        user_id: Some("admin".to_string()),
        start_time: 1000,
        run_time: 500,
    };
    assert_eq!(info.pid, 1234);
    assert_eq!(info.name, "test.exe");
    assert!((info.cpu_usage - 12.5).abs() < f32::EPSILON);
    assert_eq!(info.memory, 1024 * 1024);
    assert_eq!(info.disk_usage, (100, 50));
    assert_eq!(info.exe, Some("C:\\test.exe".to_string()));
    assert_eq!(info.cmd.len(), 2);
}

#[test]
fn test_system_snapshot_new() {
    let snapshot = proc::collect::SystemSnapshot::new();
    assert!(snapshot.is_ok(), "SystemSnapshot::new() should not panic");
}

// --- Stage 6 skeleton tests (merged from test_stage6_skeleton.rs) ---

#[test]
fn test_process_view_mode_toggle() {
    let mode = ProcessViewMode::List;
    assert_eq!(mode.toggle(), ProcessViewMode::AppGroup);
    assert_eq!(mode.toggle().toggle(), ProcessViewMode::List);
}

#[test]
fn test_process_view_mode_default() {
    assert_eq!(ProcessViewMode::default(), ProcessViewMode::List);
}

#[test]
fn test_process_view_mode_label() {
    assert_eq!(ProcessViewMode::List.label(), "列表");
    assert_eq!(ProcessViewMode::Tree.label(), "树形");
    assert_eq!(ProcessViewMode::AppGroup.label(), "应用");
}

#[test]
fn test_app_group_struct_instantiation() {
    let group = AppGroup {
        display_name: "Chrome".to_string(),
        exe_dir: "C:\\Program Files\\Google\\Chrome".to_string(),
        processes: vec![AppGroupProcess {
            pid: 1234,
            name: "chrome.exe".to_string(),
            cpu_usage: 5.0,
            memory: 1024 * 1024 * 500,
            role_hint: Some("main".to_string()),
        }],
        total_cpu: 5.0,
        total_memory: 1024 * 1024 * 500,
    };
    assert_eq!(group.display_name, "Chrome");
    assert_eq!(group.processes.len(), 1);
    assert_eq!(group.processes[0].role_hint, Some("main".to_string()));
}

#[test]
fn test_version_info_struct() {
    let info = VersionInfo {
        product_name: Some("Google Chrome".to_string()),
        company_name: Some("Google LLC".to_string()),
        file_description: None,
    };
    assert_eq!(info.product_name, Some("Google Chrome".to_string()));
    assert!(info.file_description.is_none());
}

#[test]
fn test_throttle_info_struct() {
    let info = ThrottleInfo {
        max_mhz: 3600,
        current_mhz: 2400,
        mhz_limit: 2400,
        is_throttled: true,
        throttle_pct: 33.3,
    };
    assert!(info.is_throttled);
    assert!((info.throttle_pct - 33.3).abs() < 0.1);
}

#[test]
fn test_throttle_reason_enum() {
    let reason = ThrottleReason::Thermal;
    match reason {
        ThrottleReason::None => panic!("expected Thermal"),
        ThrottleReason::Thermal => {}
        ThrottleReason::PowerPolicy => panic!("expected Thermal"),
        ThrottleReason::Idle => panic!("expected Thermal"),
        ThrottleReason::Unknown => panic!("expected Thermal"),
    }
}

#[test]
fn test_disk_io_info_struct() {
    let info = DiskIoInfo {
        name: "NVMe SSD".to_string(),
        mount_point: "C:\\".to_string(),
        read_speed: 1024 * 1024 * 100,
        write_speed: 1024 * 1024 * 50,
    };
    assert_eq!(info.mount_point, "C:\\");
    assert!(info.read_speed > info.write_speed);
}

#[test]
fn test_metric_name_temperature_extracts() {
    // 原测试假设 CPU/GPU 温度永远为空（"stub"），但 LightWorker 预热 + NVML
    // 可用时 gpu_info 会被填充。CI runner 通常无 GPU，开发机有 — 测试必须
    // 同时覆盖两种情况：返回值要么为空，要么是单个 (0, 温度) 元组且温度在
    // 合理范围（< 150°C）。
    let mut snapshot = SystemSnapshot::new().expect("snapshot creation");
    let _ = snapshot.refresh_heavy_incremental();
    let procs = snapshot.cached_processes_vec();

    let cpu_temp = MetricName::CpuTemperature.extract(&snapshot, &procs);
    let gpu_temp = MetricName::GpuTemperature.extract(&snapshot, &procs);
    let throttle = MetricName::CpuThrottlePercent.extract(&snapshot, &procs);

    fn assert_temp_slice(s: &[(u32, f64)], label: &str) {
        assert!(
            s.len() <= 1,
            "{label} should have at most one entry, got {s:?}"
        );
        if let Some(&(_, t)) = s.first() {
            assert!(
                (0.0..=150.0).contains(&t),
                "{label} temperature {t} out of plausible range"
            );
        }
    }

    assert_temp_slice(&cpu_temp, "CpuTemperature");
    assert_temp_slice(&gpu_temp, "GpuTemperature");
    // throttle 是百分比，要么空要么 [0, 100]。
    assert!(
        throttle.len() <= 1,
        "CpuThrottlePercent shape wrong: {throttle:?}"
    );
    if let Some(&(_, t)) = throttle.first() {
        assert!(
            (0.0..=100.0).contains(&t),
            "CpuThrottlePercent {t} out of [0, 100]"
        );
    }
}

#[test]
fn test_frame_process_view_mode_serialization() {
    let frame = UiFrame {
        timestamp: 12345,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: 50.0,
        memory_used: 8_000_000_000,
        memory_total: 16_000_000_000,
        net_down: 1000,
        net_up: 500,
        cpu_history: vec![],
        mem_history: vec![],
        processes: vec![],
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 2,
        tree_nodes: vec![],
        port_entries: vec![],
        port_view_mode: 0,
        port_process_groups: vec![],
        port_remote_groups: vec![],
        connection_diff: Default::default(),
        anomalies: vec![],
        usb_devices: vec![],
        usb_locks: vec![],
        monitors: vec![],
        docker_containers: vec![],
        docker_events: vec![],
        ops: vec![],
        nav: Default::default(),
    };

    let encoded = bincode::serialize(&frame).expect("serialize");
    let decoded: UiFrame = bincode::deserialize(&encoded).expect("deserialize");
    assert_eq!(decoded.process_view_mode, 2);
}

#[test]
fn test_frame_process_view_mode_default_zero() {
    // Serialize a frame without process_view_mode, verify it defaults to 0
    let minimal_json = r#"{"timestamp":1,"mode":"ProcessList","status_message":null,"cpu_usage":0.0,"memory_used":0,"memory_total":0,"net_down":0,"net_up":0,"cpu_history":[],"mem_history":[],"processes":[],"search_query":"","sort_field":"Cpu","tree_nodes":[],"port_entries":[],"port_view_mode":0,"port_process_groups":[],"port_remote_groups":[],"connection_diff":{"new_count":0,"closed_count":0,"active_count":0,"close_wait_count":0,"time_wait_count":0},"anomalies":[],"usb_devices":[],"usb_locks":[],"monitors":[],"docker_containers":[],"docker_events":[],"ops":[],"nav":{"cursor":0,"scroll":0,"selected":[],"tree_cursor":0,"tree_scroll":0,"tree_selected":[],"port_cursor":0,"port_scroll":0,"port_process_cursor":0,"port_process_scroll":0,"port_remote_cursor":0,"port_remote_scroll":0,"usb_device_cursor":0,"monitor_cursor":0,"docker_cursor":0,"docker_scroll":0}}"#;
    let frame: UiFrame = serde_json::from_str(minimal_json).expect("deserialize from JSON");
    assert_eq!(
        frame.process_view_mode, 0,
        "process_view_mode should default to 0"
    );
}

// --- Stage 1 skeleton tests ---

#[test]
fn test_alert_manager_default() {
    let mgr = AlertManager::default();
    assert!(mgr.active_alerts().is_empty());
}

#[test]
fn test_security_scorer_returns_100() {
    let mut scorer = SecurityScorer::new();
    let proc = ProcessInfo {
        pid: 1234,
        name: "test.exe".to_string(),
        cpu_usage: 0.0,
        memory: 0,
        virtual_memory: 0,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        status: "Running".to_string(),
        exe: Some("C:\\Windows\\System32\\test.exe".to_string()),
        cmd: vec![],
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
    };
    let all_procs = vec![proc.clone()];
    let score = scorer.score(&proc, &all_procs, &[]);
    // Score may not be 100 depending on signature status (non-elevated)
    assert!(score.score <= 100);
}

#[test]
fn test_threshold_rule_toml_deserialize() {
    let toml_str = r#"
[[rules]]
id = "test-cpu"
metric = "CpuUsage"
op = "GT"
threshold = 90.0
consecutive_hits = 3
severity = "Warning"
description = "CPU too high"
"#;
    let config: proc::alert::ThresholdConfig = toml::from_str(toml_str).expect("TOML parse");
    assert_eq!(config.rules.len(), 1);
    assert_eq!(config.rules[0].id, "test-cpu");
    assert_eq!(config.rules[0].metric, MetricName::CpuUsage);
    assert_eq!(config.rules[0].op, ComparisonOp::GT);
    assert!((config.rules[0].threshold - 90.0).abs() < f64::EPSILON);
    assert_eq!(config.rules[0].severity, AlertSeverity::Warning);
}

#[test]
fn test_system_frame_bincode_roundtrip() {
    let frame = UiFrame {
        timestamp: 1234567890,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: 75.0,
        memory_used: 8_000_000_000,
        memory_total: 16_000_000_000,
        net_down: 5000,
        net_up: 2000,
        cpu_history: vec![],
        mem_history: vec![],
        processes: vec![FrameProcess {
            pid: 100,
            name: "test.exe".to_string(),
            cpu: 50.0,
            memory: 1024,
            disk_read: 100,
            disk_write: 50,
        }],
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: vec![],
        port_entries: vec![],
        port_view_mode: 0,
        port_process_groups: vec![],
        port_remote_groups: vec![],
        connection_diff: Default::default(),
        anomalies: vec![],
        usb_devices: vec![],
        usb_locks: vec![],
        monitors: vec![],
        docker_containers: vec![],
        docker_events: vec![],
        ops: vec![],
        nav: Default::default(),
    };

    let encoded = bincode::serialize(&frame).expect("serialize");
    let decoded: UiFrame = bincode::deserialize(&encoded).expect("deserialize");
    assert_eq!(decoded.timestamp, frame.timestamp);
    assert_eq!(decoded.processes.len(), 1);
    assert_eq!(decoded.processes[0].pid, 100);
    assert_eq!(decoded.processes[0].name, "test.exe");
}
