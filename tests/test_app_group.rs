use std::collections::HashMap;

use proc::app_group::{self, AppGroup, AppGroupItem, VersionInfo, build_visual_items};
use proc::collect::{ProcessInfo, ProcessViewMode};

fn make_proc(pid: u32, name: &str, exe: &str, cpu: f32, mem: u64) -> ProcessInfo {
    let name_arc: std::sync::Arc<str> = std::sync::Arc::from(name);
    ProcessInfo {
        pid,
        name: std::sync::Arc::clone(&name_arc),
        cpu_usage: cpu,
        memory: mem,
        virtual_memory: mem,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: proc::collect::ProcessStatus::Run,
        exe: Some(std::sync::Arc::from(exe)),
        cmd: std::sync::Arc::from(vec![name.to_string()]),
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
        name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
        throttled: proc::throttle::EcoQoSState::default(),
    }
}

#[allow(dead_code)]
fn make_proc_with_parent(
    pid: u32,
    name: &str,
    exe: &str,
    cpu: f32,
    mem: u64,
    ppid: u32,
) -> ProcessInfo {
    let mut p = make_proc(pid, name, exe, cpu, mem);
    p.parent_pid = Some(ppid);
    p
}

fn make_proc_with_cmd(
    pid: u32,
    name: &str,
    exe: &str,
    cpu: f32,
    mem: u64,
    cmd: Vec<&str>,
) -> ProcessInfo {
    let name_arc: std::sync::Arc<str> = std::sync::Arc::from(name);
    ProcessInfo {
        pid,
        name: std::sync::Arc::clone(&name_arc),
        cpu_usage: cpu,
        memory: mem,
        virtual_memory: mem,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: proc::collect::ProcessStatus::Run,
        exe: Some(std::sync::Arc::from(exe)),
        cmd: std::sync::Arc::from(cmd.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
        name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
        throttled: proc::throttle::EcoQoSState::default(),
    }
}

#[test]
fn test_same_directory_grouped() {
    let procs = vec![
        make_proc(
            1,
            "chrome.exe",
            r"C:\Program Files\Google\Chrome\chrome.exe",
            5.0,
            500_000_000,
        ),
        make_proc(
            2,
            "chrome.exe",
            r"C:\Program Files\Google\Chrome\chrome.exe",
            3.0,
            300_000_000,
        ),
        make_proc(3, "notepad.exe", r"C:\Windows\notepad.exe", 0.1, 10_000_000),
    ];

    let mut cache = HashMap::new();
    let groups = app_group::compute_groups(&procs, &mut cache);

    assert_eq!(groups.len(), 2, "Should have 2 groups");

    // First group should be chrome (higher total_cpu)
    let chrome_group = groups
        .iter()
        .find(|g| g.processes.len() == 2)
        .expect("should find 2-process group");
    assert!(chrome_group.processes.iter().any(|p| p.pid == 1));
    assert!(chrome_group.processes.iter().any(|p| p.pid == 2));
    assert!((chrome_group.total_cpu - 8.0).abs() < 0.01);
}

#[test]
fn test_svchost_grouped_by_k_param() {
    let procs = vec![
        make_proc_with_cmd(
            10,
            "svchost.exe",
            r"C:\Windows\System32\svchost.exe",
            1.0,
            50_000_000,
            vec!["svchost.exe", "-k", "netsvcs"],
        ),
        make_proc_with_cmd(
            11,
            "svchost.exe",
            r"C:\Windows\System32\svchost.exe",
            0.5,
            30_000_000,
            vec!["svchost.exe", "-k", "DcomLaunch"],
        ),
        make_proc_with_cmd(
            12,
            "svchost.exe",
            r"C:\Windows\System32\svchost.exe",
            0.2,
            20_000_000,
            vec!["svchost.exe"],
        ), // no -k param
    ];

    let mut cache = HashMap::new();
    let groups = app_group::compute_groups(&procs, &mut cache);

    // svchost processes should be split into separate groups by -k parameter
    assert!(
        groups.len() >= 3,
        "svchost should be split by -k param, got {} groups",
        groups.len()
    );

    let netsvcs = groups.iter().find(|g| g.display_name.contains("netsvcs"));
    assert!(netsvcs.is_some(), "Should have 'netsvcs' group");

    let dcom = groups
        .iter()
        .find(|g| g.display_name.contains("DcomLaunch"));
    assert!(dcom.is_some(), "Should have 'DcomLaunch' group");
}

#[test]
fn test_no_version_info_fallback_to_dir_name() {
    let procs = vec![make_proc(
        100,
        "myapp.exe",
        r"D:\MyApp\myapp.exe",
        2.0,
        100_000_000,
    )];

    let mut cache = HashMap::new();
    let groups = app_group::compute_groups(&procs, &mut cache);

    assert_eq!(groups.len(), 1);
    // Should fall back to directory name "MyApp" since no version info
    assert_eq!(groups[0].display_name, "MyApp");
}

#[test]
fn test_groups_sorted_by_cpu_desc() {
    let procs = vec![
        make_proc(1, "low.exe", r"C:\low\low.exe", 1.0, 100_000_000),
        make_proc(2, "high.exe", r"C:\high\high.exe", 50.0, 100_000_000),
        make_proc(3, "mid.exe", r"C:\mid\mid.exe", 10.0, 100_000_000),
    ];

    let mut cache = HashMap::new();
    let groups = app_group::compute_groups(&procs, &mut cache);

    assert!(groups[0].total_cpu >= groups[1].total_cpu);
    assert!(groups[1].total_cpu >= groups[2].total_cpu);
}

#[test]
fn test_empty_process_list() {
    let procs: Vec<ProcessInfo> = vec![];
    let mut cache = HashMap::new();
    let groups = app_group::compute_groups(&procs, &mut cache);
    assert!(groups.is_empty());
}

#[test]
fn test_process_view_mode_toggle() {
    assert_eq!(ProcessViewMode::List.toggle(), ProcessViewMode::AppGroup);
    assert_eq!(ProcessViewMode::AppGroup.toggle(), ProcessViewMode::List);
    assert_eq!(ProcessViewMode::Tree.toggle(), ProcessViewMode::List);
}

#[test]
fn test_visual_items_header_only_when_collapsed() {
    let groups = vec![AppGroup {
        display_name: "Chrome".to_string(),
        exe_dir: r"C:\Chrome".to_string(),
        processes: vec![
            app_group::AppGroupProcess {
                pid: 1,
                name: "chrome.exe".to_string(),
                cpu_usage: 5.0,
                memory: 100,
                role_hint: None,
            },
            app_group::AppGroupProcess {
                pid: 2,
                name: "chrome.exe".to_string(),
                cpu_usage: 3.0,
                memory: 100,
                role_hint: None,
            },
        ],
        total_cpu: 8.0,
        total_memory: 200,
    }];

    // Collapsed: only header
    let items = build_visual_items(&groups, None);
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], AppGroupItem::Header { group_idx: 0 }));

    // Expanded: header + 2 children
    let items = build_visual_items(&groups, Some(0));
    assert_eq!(items.len(), 3);
    assert!(matches!(items[0], AppGroupItem::Header { group_idx: 0 }));
    assert!(matches!(
        items[1],
        AppGroupItem::Child {
            group_idx: 0,
            child_idx: 0
        }
    ));
    assert!(matches!(
        items[2],
        AppGroupItem::Child {
            group_idx: 0,
            child_idx: 1
        }
    ));
}

#[test]
fn test_version_info_product_name_grouping() {
    // Two processes in different directories but same ProductName should merge
    let mut cache = HashMap::new();
    cache.insert(
        r"C:\Chrome1\chrome.exe".to_string(),
        Some(VersionInfo {
            product_name: Some("Google Chrome".to_string()),
            company_name: None,
            file_description: None,
        }),
    );
    cache.insert(
        r"C:\Chrome2\chrome.exe".to_string(),
        Some(VersionInfo {
            product_name: Some("Google Chrome".to_string()),
            company_name: None,
            file_description: None,
        }),
    );

    let procs = vec![
        make_proc(1, "chrome.exe", r"C:\Chrome1\chrome.exe", 5.0, 500_000_000),
        make_proc(2, "chrome.exe", r"C:\Chrome2\chrome.exe", 3.0, 300_000_000),
    ];

    let groups = app_group::compute_groups(&procs, &mut cache);

    // Should merge into single group
    assert_eq!(
        groups.len(),
        1,
        "Same ProductName should merge into 1 group, got {}",
        groups.len()
    );
    assert_eq!(groups[0].processes.len(), 2);
}

#[test]
fn test_vmmem_grouped_as_wsl() {
    let procs = vec![make_proc(
        100,
        "vmmem.exe",
        r"C:\Windows\System32\vmmem.exe",
        10.0,
        2_000_000_000,
    )];

    let mut cache = HashMap::new();
    let groups = app_group::compute_groups(&procs, &mut cache);

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].display_name, "WSL");
}
