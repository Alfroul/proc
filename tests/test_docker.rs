use std::time::SystemTime;

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
