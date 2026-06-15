use std::collections::HashSet;

use proc::monitor::{MonitorManager, MonitorStatus, MonitorTarget, RestartPolicy};

#[test]
fn test_monitor_manager_add_by_pid() {
    let mut mgr = MonitorManager::new();
    let id = mgr
        .add_monitor(
            MonitorTarget::ByPid { pid: 5678 },
            RestartPolicy::NotifyOnly,
        )
        .unwrap();
    assert_eq!(id, 1);
    assert_eq!(mgr.list_monitors().len(), 1);
    let entry = &mgr.list_monitors()[0];
    assert_eq!(entry.pid, Some(5678));
    assert_eq!(entry.status, MonitorStatus::Running);
}

#[test]
fn test_monitor_manager_add_by_port() {
    let mut mgr = MonitorManager::new();
    let id = mgr
        .add_monitor(
            MonitorTarget::ByPort { port: 8080 },
            RestartPolicy::NotifyOnly,
        )
        .unwrap();
    assert_eq!(id, 1);
    let entry = &mgr.list_monitors()[0];
    assert_eq!(entry.pid, None);
}

#[test]
fn test_monitor_manager_add_by_command() {
    let mut mgr = MonitorManager::new();
    let id = mgr
        .add_monitor(
            MonitorTarget::ByCommand {
                cmd: "cargo run".to_string(),
                args: vec!["--release".to_string()],
                cwd: None,
            },
            RestartPolicy::AutoRestart {
                max_retries: 5,
                base_backoff: 1,
                max_backoff: 30,
            },
        )
        .unwrap();
    assert_eq!(id, 1);
    let entry = &mgr.list_monitors()[0];
    assert_eq!(entry.pid, None);
}

#[test]
fn test_monitor_manager_remove() {
    let mut mgr = MonitorManager::new();
    let id = mgr
        .add_monitor(
            MonitorTarget::ByPid { pid: 1234 },
            RestartPolicy::NotifyOnly,
        )
        .unwrap();
    mgr.remove_monitor(id).unwrap();
    assert!(mgr.list_monitors().is_empty());
}

#[test]
fn test_monitor_manager_remove_nonexistent() {
    let mut mgr = MonitorManager::new();
    assert!(mgr.remove_monitor(999).is_err());
}

#[test]
fn test_monitor_manager_auto_increment_id() {
    let mut mgr = MonitorManager::new();
    let id1 = mgr
        .add_monitor(MonitorTarget::ByPid { pid: 1 }, RestartPolicy::NotifyOnly)
        .unwrap();
    let id2 = mgr
        .add_monitor(MonitorTarget::ByPid { pid: 2 }, RestartPolicy::NotifyOnly)
        .unwrap();
    let id3 = mgr
        .add_monitor(MonitorTarget::ByPid { pid: 3 }, RestartPolicy::NotifyOnly)
        .unwrap();
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);
}

#[test]
fn test_diff_snapshots_new_and_dead() {
    let old: HashSet<u32> = [100, 200, 300].into_iter().collect();
    let new: HashSet<u32> = [200, 300, 400].into_iter().collect();
    let (new_pids, dead_pids) = proc::monitor::snapshot::diff_snapshots(&old, &new);
    assert!(new_pids.contains(&400));
    assert!(dead_pids.contains(&100));
    assert_eq!(new_pids.len(), 1);
    assert_eq!(dead_pids.len(), 1);
}

#[test]
fn test_diff_snapshots_no_change() {
    let old: HashSet<u32> = [1, 2, 3].into_iter().collect();
    let new: HashSet<u32> = [1, 2, 3].into_iter().collect();
    let (new_pids, dead_pids) = proc::monitor::snapshot::diff_snapshots(&old, &new);
    assert!(new_pids.is_empty());
    assert!(dead_pids.is_empty());
}

#[test]
fn test_diff_snapshots_empty_old() {
    let old: HashSet<u32> = HashSet::new();
    let new: HashSet<u32> = [10, 20].into_iter().collect();
    let (new_pids, dead_pids) = proc::monitor::snapshot::diff_snapshots(&old, &new);
    assert_eq!(new_pids.len(), 2);
    assert!(dead_pids.is_empty());
}

#[test]
fn test_diff_snapshots_empty_new() {
    let old: HashSet<u32> = [10, 20].into_iter().collect();
    let new: HashSet<u32> = HashSet::new();
    let (new_pids, dead_pids) = proc::monitor::snapshot::diff_snapshots(&old, &new);
    assert!(new_pids.is_empty());
    assert_eq!(dead_pids.len(), 2);
}

#[test]
fn test_send_toast_no_panic() {
    let _ = proc::monitor::notify::send_toast("测试标题", "测试内容");
}

#[test]
fn test_notify_crash_no_panic() {
    let _ = proc::monitor::notify::notify_crash("test_proc", 1234, 1, 1, 5);
}

#[test]
fn test_notify_port_change_no_panic() {
    let _ = proc::monitor::notify::notify_port_change(8080, "released", "occupied");
}

#[test]
fn test_calc_backoff() {
    assert_eq!(MonitorManager::calc_backoff(1, 1, 30), 1);
    assert_eq!(MonitorManager::calc_backoff(1, 2, 30), 2);
    assert_eq!(MonitorManager::calc_backoff(1, 3, 30), 4);
    assert_eq!(MonitorManager::calc_backoff(1, 4, 30), 8);
    assert_eq!(MonitorManager::calc_backoff(1, 5, 30), 16);
    assert_eq!(MonitorManager::calc_backoff(1, 6, 30), 30);
    assert_eq!(MonitorManager::calc_backoff(2, 3, 30), 8);
    assert_eq!(MonitorManager::calc_backoff(1, 10, 30), 30);
}

#[test]
fn test_monitor_status_display() {
    assert_eq!(format!("{}", MonitorStatus::Running), "运行中");
    assert_eq!(format!("{}", MonitorStatus::Stopped), "已停止");
    assert_eq!(format!("{}", MonitorStatus::Crashed), "已崩溃");
    assert_eq!(format!("{}", MonitorStatus::Paused), "已暂停");
}

#[test]
fn test_notification_records() {
    let mut mgr = MonitorManager::new();
    mgr.add_notification("test event 1".to_string());
    mgr.add_notification("test event 2".to_string());
    assert_eq!(mgr.notifications().len(), 2);
}

#[test]
fn test_notification_cap_at_100() {
    let mut mgr = MonitorManager::new();
    for i in 0..105 {
        mgr.add_notification(format!("event {}", i));
    }
    assert_eq!(mgr.notifications().len(), 100);
}

#[test]
fn test_monitor_get_by_id() {
    let mut mgr = MonitorManager::new();
    let id = mgr
        .add_monitor(MonitorTarget::ByPid { pid: 42 }, RestartPolicy::NotifyOnly)
        .unwrap();
    let entry = mgr.get_monitor(id).unwrap();
    assert_eq!(entry.pid, Some(42));
    assert!(mgr.get_monitor(999).is_none());
}
