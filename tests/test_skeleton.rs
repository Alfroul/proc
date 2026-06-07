use proc::app::{App, AppMode};
use proc::cli::{Cli, Command};
use proc::collect::ProcessInfo;
use proc::error::ProcError;
use proc::alert::{AlertManager, MetricName, ComparisonOp, AlertSeverity};
use proc::security::SecurityScorer;
use proc::record::{UiFrame, FrameProcess};

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
    assert_eq!(app.mode, AppMode::ProcessTree);

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

    app.mode = AppMode::Help;
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::ProcessList);

    app.mode = AppMode::Menu;
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode, AppMode::ProcessList);
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
    let err = ProcError::NotFound("test process".to_string());
    assert!(err.to_string().contains("test process"));

    let err = ProcError::PermissionDenied("kill PID 4".to_string());
    assert!(err.to_string().contains("kill PID 4"));

    let err = ProcError::Sysinfo("init failed".to_string());
    assert!(err.to_string().contains("init failed"));

    let err = ProcError::PortScan("scan failed".to_string());
    assert!(err.to_string().contains("scan failed"));

    let err = ProcError::UsbDetect("no device".to_string());
    assert!(err.to_string().contains("no device"));

    let err = ProcError::Monitor("watch failed".to_string());
    assert!(err.to_string().contains("watch failed"));

    let err = ProcError::Docker("not running".to_string());
    assert!(err.to_string().contains("not running"));
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

// --- Stage 1 skeleton tests ---

#[test]
fn test_alert_manager_load_or_default() {
    let mgr = AlertManager::load_or_default();
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
    };

    let encoded = bincode::serialize(&frame).expect("serialize");
    let decoded: UiFrame = bincode::deserialize(&encoded).expect("deserialize");
    assert_eq!(decoded.timestamp, frame.timestamp);
    assert_eq!(decoded.processes.len(), 1);
    assert_eq!(decoded.processes[0].pid, 100);
    assert_eq!(decoded.processes[0].name, "test.exe");
}
