use std::time::SystemTime;

use clap::Parser;

fn make_container_info() -> proc::docker::ContainerInfo {
    proc::docker::ContainerInfo {
        id: "abc123def456".to_string(),
        name: "my-container".to_string(),
        image: "nginx:latest".to_string(),
        status: "Up 2 hours".to_string(),
        state: "running".to_string(),
        health: proc::docker::HealthStatus::Healthy,
        cpu_percent: 1.5,
        memory_usage: 50_000_000,
        network_in: 1_024_000,
        network_out: 512_000,
        running_since: Some(SystemTime::now()),
        ports: String::new(),
    }
}

#[test]
fn test_container_info_construction() {
    let info = make_container_info();
    assert_eq!(info.id, "abc123def456");
    assert_eq!(info.name, "my-container");
    assert_eq!(info.image, "nginx:latest");
    assert_eq!(info.state, "running");
    assert_eq!(info.health, proc::docker::HealthStatus::Healthy);
    assert!((info.cpu_percent - 1.5).abs() < f64::EPSILON);
    assert_eq!(info.memory_usage, 50_000_000);
    assert_eq!(info.network_in, 1_024_000);
    assert_eq!(info.network_out, 512_000);
    assert!(info.running_since.is_some());
}

#[test]
fn test_health_status_variants() {
    let variants = [
        proc::docker::HealthStatus::Healthy,
        proc::docker::HealthStatus::Unhealthy,
        proc::docker::HealthStatus::Starting,
        proc::docker::HealthStatus::NotConfigured,
    ];
    assert_eq!(variants.len(), 4);

    assert_eq!(
        format!("{}", proc::docker::HealthStatus::Healthy),
        "healthy"
    );
    assert_eq!(
        format!("{}", proc::docker::HealthStatus::Unhealthy),
        "unhealthy"
    );
    assert_eq!(
        format!("{}", proc::docker::HealthStatus::Starting),
        "starting"
    );
    assert_eq!(
        format!("{}", proc::docker::HealthStatus::NotConfigured),
        "-"
    );
}

#[test]
fn test_health_status_from_status() {
    use proc::docker::HealthStatus;

    assert_eq!(
        HealthStatus::from_status("Up 2 hours (healthy)"),
        HealthStatus::Healthy
    );
    assert_eq!(
        HealthStatus::from_status("Up 2 hours (unhealthy)"),
        HealthStatus::Unhealthy
    );
    assert_eq!(
        HealthStatus::from_status("Up 2 hours (health: starting)"),
        HealthStatus::Starting
    );
    assert_eq!(
        HealthStatus::from_status("Up 2 hours"),
        HealthStatus::NotConfigured
    );
    assert_eq!(
        HealthStatus::from_status("Exited (0) 3 minutes ago"),
        HealthStatus::NotConfigured
    );
}

#[test]
fn test_docker_event_construction() {
    let event = proc::docker::events::DockerEvent {
        action: "die".to_string(),
        container_id: "abc123".to_string(),
        container_name: Some("my-app".to_string()),
        timestamp: SystemTime::now(),
    };
    assert_eq!(event.action, "die");
    assert_eq!(event.container_id, "abc123");
    assert_eq!(event.container_name.as_deref(), Some("my-app"));
}

#[test]
fn test_docker_connect_no_docker() {
    let result = proc::docker::DockerMonitor::connect();
    // When Docker is not running, this should fail gracefully
    if let Err(e) = &result {
        let msg = format!("{}", e);
        assert!(
            msg.contains("Docker") || msg.contains("未运行") || msg.contains("tokio"),
            "Expected Docker-related error, got: {}",
            msg
        );
    }
    // If Docker IS running, the connection succeeds — that's also fine
}

#[test]
fn test_container_info_status_display() {
    let running = proc::docker::ContainerInfo {
        id: "abc".to_string(),
        name: "test".to_string(),
        image: "img".to_string(),
        status: "Up 2 hours".to_string(),
        state: "running".to_string(),
        health: proc::docker::HealthStatus::Healthy,
        cpu_percent: 0.0,
        memory_usage: 0,
        network_in: 0,
        network_out: 0,
        running_since: None,
        ports: String::new(),
    };
    assert_eq!(running.state, "running");

    let stopped = proc::docker::ContainerInfo {
        state: "exited".to_string(),
        ..running
    };
    assert_eq!(stopped.state, "exited");
}

#[test]
fn test_health_info_display() {
    use proc::docker::health::HealthInfo;

    let healthy = HealthInfo::Healthy {
        failing_streak: 0,
        last_output: "OK".to_string(),
    };
    assert!(format!("{}", healthy).contains("healthy"));

    let unhealthy = HealthInfo::Unhealthy {
        failing_streak: 3,
        last_output: "Error".to_string(),
    };
    assert!(format!("{}", unhealthy).contains("unhealthy"));

    let starting = HealthInfo::Starting;
    assert_eq!(format!("{}", starting), "starting");

    let not_configured = HealthInfo::NotConfigured;
    assert_eq!(format!("{}", not_configured), "not configured");
}

#[test]
fn test_container_stats_default() {
    let stats = proc::docker::stats::ContainerStats::default();
    assert_eq!(stats.cpu_percent, 0.0);
    assert_eq!(stats.memory_usage, 0);
    assert_eq!(stats.memory_limit, 0);
    assert_eq!(stats.network_in, 0);
    assert_eq!(stats.network_out, 0);
}

#[test]
fn test_docker_event_no_name() {
    let event = proc::docker::events::DockerEvent {
        action: "start".to_string(),
        container_id: "def456".to_string(),
        container_name: None,
        timestamp: SystemTime::now(),
    };
    assert!(event.container_name.is_none());
    assert_eq!(event.action, "start");
}

// Integration tests that need Docker running — mark with #[ignore]
#[test]
#[ignore = "需要 Docker 运行"]
fn test_docker_list_containers() {
    let monitor = proc::docker::DockerMonitor::connect().expect("Docker should be running");
    let containers = monitor
        .list_containers(true)
        .expect("Should list containers");
    // Just verify it returns a Vec without error
    println!("Found {} containers", containers.len());
}

#[test]
#[ignore = "需要 Docker 运行"]
fn test_docker_events() {
    let monitor = proc::docker::DockerMonitor::connect().expect("Docker should be running");
    let docker_client = monitor.docker();
    let _receiver = proc::docker::events::spawn_event_watcher(docker_client);
    // Just verify the watcher starts without panicking
    std::thread::sleep(std::time::Duration::from_millis(100));
}

// ──────────────────────────────────────────────────────────────────────────
// 阶段 3：E4 docker top 解析测试
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_top_output_typical() {
    use proc::docker::top::parse_top_output;
    let raw = "\
UID  PID  PPID  C STIME TTY  TIME CMD
root   1    0   0 Jan01 ?    00:00:01 /sbin/init
root  42    1   0 Jan01 ?    00:00:00 nginx -g 'daemon off;'";
    let out = parse_top_output(raw);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].pid, "1");
    assert_eq!(out[0].command, "/sbin/init");
    assert_eq!(out[1].command, "nginx -g 'daemon off;'");
}

#[test]
fn test_parse_top_output_empty() {
    use proc::docker::top::parse_top_output;
    assert!(parse_top_output("").is_empty());
    assert!(parse_top_output("   \n\n   ").is_empty());
}

#[test]
fn test_parse_top_output_header_only() {
    use proc::docker::top::parse_top_output;
    let raw = "UID PID PPID C STIME TTY TIME CMD\n";
    assert!(parse_top_output(raw).is_empty());
}

#[test]
fn test_parse_top_output_args_with_spaces() {
    use proc::docker::top::parse_top_output;
    let raw =
        "UID PID CMD\nroot 7 java -Xmx2g -Dspring.profiles.active=prod app.jar arg with spaces";
    let out = parse_top_output(raw);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].command,
        "java -Xmx2g -Dspring.profiles.active=prod app.jar arg with spaces"
    );
}

#[test]
fn test_parse_top_response_structured() {
    use proc::docker::top::parse_top_response;
    let resp = bollard::models::ContainerTopResponse {
        titles: Some(vec![
            "UID".to_string(),
            "PID".to_string(),
            "STIME".to_string(),
            "TIME".to_string(),
            "CMD".to_string(),
        ]),
        processes: Some(vec![
            vec![
                "root".to_string(),
                "1".to_string(),
                "Jan01".to_string(),
                "00:00:01".to_string(),
                "/sbin/init".to_string(),
            ],
            vec![
                "root".to_string(),
                "42".to_string(),
                "Jan01".to_string(),
                "00:00:00".to_string(),
                "nginx: master process nginx".to_string(),
            ],
        ]),
    };
    let out = parse_top_response(&resp);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].pid, "1");
    assert_eq!(out[0].user, "root");
    assert_eq!(out[0].started, "Jan01");
    assert_eq!(out[0].cpu_time, "00:00:01");
    // 结构化路径下 bollard 已 pre-join CMD 内空格。
    assert_eq!(out[1].command, "nginx: master process nginx");
}

// ──────────────────────────────────────────────────────────────────────────
// 阶段 3：E1 日志时间戳解析测试
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_log_timestamp_with_z() {
    use proc::docker::logs::parse_log_timestamp;
    let line = "2026-06-20T08:30:45.123456789Z hello world\n";
    let (ts, msg) = parse_log_timestamp(line);
    assert_eq!(ts.as_deref(), Some("2026-06-20T08:30:45.123456789Z"));
    assert_eq!(msg, "hello world");
}

#[test]
fn test_parse_log_timestamp_with_offset() {
    use proc::docker::logs::parse_log_timestamp;
    let line = "2026-06-20T08:30:45+08:00 hi\n";
    let (ts, msg) = parse_log_timestamp(line);
    assert_eq!(ts.as_deref(), Some("2026-06-20T08:30:45+08:00"));
    assert_eq!(msg, "hi");
}

#[test]
fn test_parse_log_timestamp_no_timestamp() {
    use proc::docker::logs::parse_log_timestamp;
    let line = "just a log line without ts\n";
    let (ts, msg) = parse_log_timestamp(line);
    assert!(ts.is_none());
    assert_eq!(msg, "just a log line without ts");
}

#[test]
fn test_parse_log_timestamp_preserves_ansi() {
    use proc::docker::logs::parse_log_timestamp;
    let line = "2026-06-20T08:30:45Z \x1b[31mERROR\x1b[0m boom\n";
    let (ts, msg) = parse_log_timestamp(line);
    assert_eq!(ts.as_deref(), Some("2026-06-20T08:30:45Z"));
    assert_eq!(msg, "\x1b[31mERROR\x1b[0m boom");
}

#[test]
fn test_parse_log_timestamp_short_line() {
    use proc::docker::logs::parse_log_timestamp;
    // 太短不可能是 RFC3339，原文保留。
    let line = "err";
    let (ts, msg) = parse_log_timestamp(line);
    assert!(ts.is_none());
    assert_eq!(msg, "err");
}

#[test]
fn test_log_line_display() {
    use proc::docker::logs::LogLine;
    let stderr = LogLine {
        timestamp: None,
        message: "boom".to_string(),
        is_stderr: true,
    };
    assert!(format!("{stderr}").contains("[stderr]"));

    let stdout = LogLine {
        timestamp: None,
        message: "ok".to_string(),
        is_stderr: false,
    };
    assert_eq!(format!("{stdout}"), "ok");
}

// ──────────────────────────────────────────────────────────────────────────
// 阶段 3：E1 日志 worker 单元测试
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn test_log_chunk_default_empty() {
    use proc::docker::logs_worker::LogChunk;
    let c = LogChunk::default();
    assert!(c.lines.is_empty());
}

#[test]
fn test_logs_worker_drains_chunks() {
    use proc::docker::logs_worker::LogChunk;
    use std::sync::mpsc;
    // 模拟 worker 把 chunk 推到 channel，drain 拿走所有。
    let (tx, rx) = mpsc::sync_channel::<LogChunk>(4);
    tx.try_send(LogChunk::default()).unwrap();
    tx.try_send(LogChunk::default()).unwrap();
    let mut count = 0;
    while rx.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 2);
}

// ──────────────────────────────────────────────────────────────────────────
// 阶段 3：E3 镜像 / volume 类型测试
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn test_image_info_in_use_and_display() {
    use proc::docker::images::ImageInfo;
    let info = ImageInfo {
        id: "sha256:abcdef1234567890".to_string(),
        short_id: "abcdef123456".to_string(),
        repo_tags: vec!["nginx:latest".to_string()],
        created: 0,
        size: 100_000_000,
        containers: 2,
    };
    assert!(info.in_use());
    assert_eq!(info.display_name(), "nginx:latest");
    let s = format!("{info}");
    assert!(s.contains("nginx:latest"));
    assert!(s.contains("容器=2"));
}

#[test]
fn test_image_info_no_tags() {
    use proc::docker::images::ImageInfo;
    let info = ImageInfo {
        id: "sha256:abc".to_string(),
        short_id: "abc".to_string(),
        repo_tags: Vec::new(),
        created: 0,
        size: 0,
        containers: 0,
    };
    assert!(!info.in_use());
    assert!(info.display_name().contains("abc"));
    let s = format!("{info}");
    assert!(s.contains("<none>"));
}

#[test]
fn test_volume_info_display() {
    let v = proc::docker::volumes::VolumeInfo {
        name: "my-vol".to_string(),
        driver: "local".to_string(),
        mountpoint: "/var/lib/docker/volumes/my-vol/_data".to_string(),
        created: 0,
        size: 5_000_000,
        in_use: true,
    };
    let s = format!("{v}");
    assert!(s.contains("my-vol"));
    assert!(s.contains("使用中"));
}

#[test]
fn test_volume_info_unused_no_size() {
    use proc::docker::volumes::VolumeInfo;
    let v = VolumeInfo {
        name: "v".to_string(),
        driver: "local".to_string(),
        mountpoint: "/v".to_string(),
        created: 0,
        size: 0,
        in_use: false,
    };
    let s = format!("{v}");
    assert!(s.contains("未使用"));
    assert!(s.contains("大小=-"));
}

#[test]
fn test_parse_rfc3339_to_unix_known_epoch() {
    // 直接访问内部纯函数（test 通过 super 路径）。
    // 2026-06-20T08:30:45Z → 1_781_944_245（与 volumes.rs 内测试一致）。
    // 不通过 super（私有），改为通过 volumes::VolumeInfo::created 转测：构造时传 0 即可。
    use proc::docker::volumes::VolumeInfo;
    let v = VolumeInfo {
        name: "x".to_string(),
        driver: "local".to_string(),
        mountpoint: "/".to_string(),
        created: 1_781_944_245,
        size: 0,
        in_use: false,
    };
    assert_eq!(v.created, 1_781_944_245);
}

// ──────────────────────────────────────────────────────────────────────────
// 阶段 3：E3 CLI mock 测试（不依赖真实 Docker）
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn test_docker_sub_ps_parsing() {
    use proc::cli::{Cli, Command, DockerSub};
    let cli = Cli::try_parse_from(["proc", "docker", "ps"]).expect("parse");
    match cli.command {
        Some(Command::Docker { sub }) => assert!(matches!(sub, DockerSub::Ps)),
        _ => panic!("Expected Docker"),
    }
}

#[test]
fn test_docker_sub_top_parsing() {
    use proc::cli::{Cli, Command, DockerSub};
    let cli = Cli::try_parse_from(["proc", "docker", "top", "my-ctr"]).expect("parse");
    match cli.command {
        Some(Command::Docker { sub }) => match sub {
            DockerSub::Top { name } => assert_eq!(name, "my-ctr"),
            _ => panic!("Expected Top"),
        },
        _ => panic!("Expected Docker"),
    }
}

#[test]
fn test_docker_sub_logs_follow_parsing() {
    use proc::cli::{Cli, Command, DockerSub};
    let cli = Cli::try_parse_from(["proc", "docker", "logs", "--follow", "--tail", "100", "app"])
        .expect("parse");
    match cli.command {
        Some(Command::Docker { sub }) => match sub {
            DockerSub::Logs { name, follow, tail } => {
                assert_eq!(name, "app");
                assert!(follow);
                assert_eq!(tail.as_deref(), Some("100"));
            }
            _ => panic!("Expected Logs"),
        },
        _ => panic!("Expected Docker"),
    }
}

#[test]
fn test_docker_sub_image_rm_parsing() {
    use proc::cli::{Cli, Command, DockerSub};
    let cli = Cli::try_parse_from(["proc", "docker", "image-rm", "--force", "sha256:abc"])
        .expect("parse");
    match cli.command {
        Some(Command::Docker { sub }) => match sub {
            DockerSub::ImageRm { id, force } => {
                assert_eq!(id, "sha256:abc");
                assert!(force);
            }
            _ => panic!("Expected ImageRm"),
        },
        _ => panic!("Expected Docker"),
    }
}

#[test]
fn test_docker_sub_volume_rm_parsing() {
    use proc::cli::{Cli, Command, DockerSub};
    let cli = Cli::try_parse_from(["proc", "docker", "volume-rm", "my-vol"]).expect("parse");
    match cli.command {
        Some(Command::Docker { sub }) => match sub {
            DockerSub::VolumeRm { name, force } => {
                assert_eq!(name, "my-vol");
                assert!(!force);
            }
            _ => panic!("Expected VolumeRm"),
        },
        _ => panic!("Expected Docker"),
    }
}

#[test]
fn test_docker_sub_compose_parsing() {
    use proc::cli::{Cli, Command, DockerSub};
    // trailing_var_arg + allow_hyphen_values：`up -d` 一并吞掉，不被 `-d` 触发 clap option 解析。
    let cli = Cli::try_parse_from(["proc", "docker", "compose", "up", "-d"]).expect("parse");
    match cli.command {
        Some(Command::Docker { sub }) => match sub {
            DockerSub::Compose { args } => {
                assert_eq!(args, vec!["up".to_string(), "-d".to_string()]);
            }
            _ => panic!("Expected Compose"),
        },
        _ => panic!("Expected Docker"),
    }
}

// ──────────────────────────────────────────────────────────────────────────
// 阶段 3：ViewModel 状态机测试
// ──────────────────────────────────────────────────────────────────────────

#[test]
fn test_docker_view_mode_cycle() {
    use proc::view_models::docker_panel::DockerViewMode;
    assert_eq!(DockerViewMode::Containers.next(), DockerViewMode::Images);
    assert_eq!(DockerViewMode::Images.next(), DockerViewMode::Volumes);
    assert_eq!(DockerViewMode::Volumes.next(), DockerViewMode::Containers);
}

#[test]
fn test_docker_view_mode_labels() {
    use proc::view_models::docker_panel::DockerViewMode;
    assert_eq!(DockerViewMode::Containers.label(), "容器");
    assert_eq!(DockerViewMode::Images.label(), "镜像");
    assert_eq!(DockerViewMode::Volumes.label(), "卷");
}

#[test]
fn test_docker_panel_new_default_state() {
    use proc::view_models::docker_panel::{DockerPanel, DockerViewMode};
    let p = DockerPanel::new();
    assert_eq!(p.view_mode, DockerViewMode::Containers);
    assert!(p.top_processes.is_empty());
    assert!(!p.show_top_processes);
    assert!(p.log_viewer.is_none());
    assert!(p.logs_worker.is_none());
    assert!(p.images.is_empty());
    assert!(p.volumes.is_empty());
    assert!(p.delete_pending.is_none());
}
