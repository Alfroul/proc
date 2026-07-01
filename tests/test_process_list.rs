use proc::classify::{self, ProcessClass};
use proc::collect::{ProcessInfo, SortField, SystemSnapshot};
use proc::kill;

fn make_process(pid: u32, name: &str, cpu: f32, memory: u64) -> ProcessInfo {
    let name_arc: std::sync::Arc<str> = std::sync::Arc::from(name);
    ProcessInfo {
        pid,
        name: std::sync::Arc::clone(&name_arc),
        cpu_usage: cpu,
        memory,
        virtual_memory: memory * 2,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: proc::collect::ProcessStatus::Run,
        exe: Some(std::sync::Arc::from(format!("C:\\{}", name).as_str())),
        cmd: std::sync::Arc::from(vec![name.to_string()]),
        cwd: Some(std::sync::Arc::from("C:\\")),
        parent_pid: None,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
        name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
        throttled: proc::throttle::EcoQoSState::default(),
        signature_status: proc::security::SignatureStatus::default(),
        parent_chain: Vec::new(),
    }
}

#[test]
fn test_process_list_snapshot_refresh() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    let result = snapshot.refresh();
    assert!(result.is_ok(), "refresh() should not panic or error");
}

#[test]
fn test_process_list_processes_not_empty() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    let _ = snapshot.refresh_heavy_incremental();
    let processes = snapshot.cached_processes_vec();
    assert!(!processes.is_empty(), "Should have at least some processes");
}

#[test]
fn test_process_list_cpu_usage() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    snapshot.refresh().expect("refresh() should succeed");
    let cpu = snapshot.cpu_usage();
    assert!(cpu >= 0.0, "CPU usage should be non-negative");
}

#[test]
fn test_process_list_memory_usage() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    snapshot.refresh().expect("refresh() should succeed");
    let (used, total) = snapshot.memory_usage();
    assert!(total > 0, "Total memory should be positive");
    assert!(used <= total, "Used memory should not exceed total");
}

#[test]
fn test_classify_kernel_process() {
    let proc = make_process(4, "System", 0.0, 1000);
    assert_eq!(classify::classify_process(&proc), ProcessClass::Kernel);
}

#[test]
fn test_classify_system_process() {
    let names = [
        "csrss.exe",
        "smss.exe",
        "wininit.exe",
        "svchost.exe",
        "lsass.exe",
    ];
    for name in names {
        let proc = make_process(100, name, 0.0, 1000);
        assert_eq!(
            classify::classify_process(&proc),
            ProcessClass::SystemProcess,
            "{} should be classified as SystemProcess",
            name
        );
    }
}

#[test]
fn test_classify_user_process() {
    let proc = make_process(9999, "chrome.exe", 15.0, 500_000_000);
    assert_eq!(classify::classify_process(&proc), ProcessClass::UserApp);
}

#[test]
fn test_classify_case_insensitive() {
    let proc = make_process(100, "CSRSS.EXE", 0.0, 1000);
    assert_eq!(
        classify::classify_process(&proc),
        ProcessClass::SystemProcess
    );
}

#[test]
fn test_classify_count() {
    let processes = vec![
        make_process(4, "System", 0.0, 1000),
        make_process(100, "csrss.exe", 0.0, 1000),
        make_process(9999, "chrome.exe", 15.0, 500_000_000),
    ];
    let count = classify::classify_count(&processes);
    assert_eq!(count.kernel, 1);
    assert_eq!(count.system, 1);
    assert_eq!(count.user, 1);
}

#[test]
fn test_classify_label() {
    assert_eq!(ProcessClass::UserApp.label(), "用户");
    assert_eq!(ProcessClass::SystemProcess.label(), "系统");
    assert_eq!(ProcessClass::WindowsService.label(), "服务");
    assert_eq!(ProcessClass::Kernel.label(), "内核");
    assert_eq!(ProcessClass::Unknown.label(), "未知");
}

#[test]
fn test_kill_already_gone() {
    let result = kill::kill_process(99999999, false).expect("kill_process should not error");
    assert!(
        matches!(
            result,
            kill::KillResult::AlreadyGone | kill::KillResult::AccessDenied
        ),
        "expected AlreadyGone or AccessDenied for non-existent PID, got {:?}",
        result
    );
}

#[test]
fn test_kill_force_already_gone() {
    let result = kill::kill_process(99999999, true).expect("kill_process should not error");
    assert!(
        matches!(
            result,
            kill::KillResult::AlreadyGone | kill::KillResult::AccessDenied
        ),
        "expected AlreadyGone or AccessDenied for non-existent PID, got {:?}",
        result
    );
}

#[test]
fn test_sort_field_cycle() {
    let field = SortField::Cpu;
    assert_eq!(field.next(), SortField::Memory);
    assert_eq!(field.next().next(), SortField::Pid);
    assert_eq!(field.next().next().next(), SortField::Name);
    assert_eq!(field.next().next().next().next(), SortField::Security);
    assert_eq!(
        field.next().next().next().next().next(),
        SortField::DiskRead
    );
    assert_eq!(
        field.next().next().next().next().next().next(),
        SortField::DiskWrite
    );
    assert_eq!(
        field.next().next().next().next().next().next().next(),
        SortField::NetSent
    );
    assert_eq!(
        field
            .next()
            .next()
            .next()
            .next()
            .next()
            .next()
            .next()
            .next(),
        SortField::NetRecv
    );
    assert_eq!(
        field
            .next()
            .next()
            .next()
            .next()
            .next()
            .next()
            .next()
            .next()
            .next(),
        SortField::Cpu
    );

    assert_eq!(field.prev(), SortField::NetRecv);
    assert_eq!(field.prev().prev(), SortField::NetSent);
    assert_eq!(field.prev().prev().prev(), SortField::DiskWrite);
    assert_eq!(field.prev().prev().prev().prev(), SortField::DiskRead);
    assert_eq!(
        field.prev().prev().prev().prev().prev(),
        SortField::Security
    );
    assert_eq!(
        field.prev().prev().prev().prev().prev().prev(),
        SortField::Name
    );
    assert_eq!(
        field.prev().prev().prev().prev().prev().prev().prev(),
        SortField::Pid
    );
}

#[test]
fn test_sort_field_label() {
    assert_eq!(SortField::Cpu.label(), "CPU%");
    assert_eq!(SortField::Memory.label(), "MEM%");
    assert_eq!(SortField::Pid.label(), "PID");
    assert_eq!(SortField::Name.label(), "名称");
    assert_eq!(SortField::Security.label(), "安全分");
}

#[test]
fn test_sort_by_cpu() {
    let mut processes = [
        make_process(1, "low.exe", 1.0, 100),
        make_process(2, "high.exe", 50.0, 100),
        make_process(3, "mid.exe", 25.0, 100),
    ];

    processes.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    assert_eq!(processes[0].pid, 2);
    assert_eq!(processes[1].pid, 3);
    assert_eq!(processes[2].pid, 1);
}

#[test]
fn test_sort_by_memory() {
    let mut processes = [
        make_process(1, "low.exe", 1.0, 100),
        make_process(2, "high.exe", 1.0, 500),
        make_process(3, "mid.exe", 1.0, 300),
    ];

    processes.sort_by_key(|b| std::cmp::Reverse(b.memory));

    assert_eq!(processes[0].pid, 2);
    assert_eq!(processes[1].pid, 3);
    assert_eq!(processes[2].pid, 1);
}

#[test]
fn test_sort_by_name() {
    let mut processes = [
        make_process(1, "chrome.exe", 1.0, 100),
        make_process(2, "alg.exe", 1.0, 100),
        make_process(3, "zoom.exe", 1.0, 100),
    ];

    processes.sort_by_key(|a| a.name.to_lowercase());

    assert_eq!(processes[0].name.as_ref(), "alg.exe");
    assert_eq!(processes[1].name.as_ref(), "chrome.exe");
    assert_eq!(processes[2].name.as_ref(), "zoom.exe");
}

#[test]
fn test_process_info_fields() {
    let info = make_process(1234, "test.exe", 12.5, 1024 * 1024);
    assert_eq!(info.pid, 1234);
    assert_eq!(info.name.as_ref(), "test.exe");
    assert!((info.cpu_usage - 12.5).abs() < f32::EPSILON);
    assert_eq!(info.memory, 1024 * 1024);
    assert_eq!(info.virtual_memory, 2 * 1024 * 1024);
    assert_eq!(info.exe.as_deref(), Some("C:\\test.exe"));
}

#[test]
fn test_process_count() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    let _ = snapshot.refresh_heavy_incremental();
    let count = snapshot.process_count();
    assert!(count > 0, "Should have at least 1 process");
    // `process_count()` 走 SystemSnapshot 自身的 sysinfo（new() 时一次性 snapshot），
    // `cached_processes_vec()` 走 HeavyWorker 的 sysinfo（refresh_heavy_incremental
    // 后的更新帧）。两份 sysinfo 来自不同时刻，进程在期间 spawn/exit 完全可能，
    // 因此不能要求 cached >= count 严格成立；只要两者都在同一量级即可。
    let cached = snapshot.cached_processes_vec().len();
    let larger = count.max(cached);
    let smaller = count.min(cached);
    assert!(
        larger.saturating_sub(smaller) <= larger / 10,
        "process_count {count} vs cached {cached} diverged > 10%; snapshot race?"
    );
}

#[test]
fn test_incremental_refresh_populates_cache() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    snapshot.refresh().expect("refresh() should succeed");
    // After initial refresh, process_cache should be empty (not yet used incremental path)
    assert!(snapshot.process_cache().is_empty());

    // Now run incremental refresh
    snapshot
        .refresh_heavy_incremental()
        .expect("incremental refresh should succeed");
    let cache = snapshot.process_cache();
    assert!(
        !cache.is_empty(),
        "process_cache should be populated after incremental refresh"
    );

    // Every cached process should have a valid PID
    for (&pid, proc) in cache.iter() {
        assert_eq!(pid, proc.pid);
    }
}

#[test]
fn test_cached_processes_vec_matches_cache() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    snapshot.refresh().expect("refresh() should succeed");
    snapshot
        .refresh_heavy_incremental()
        .expect("incremental refresh should succeed");

    let vec = snapshot.cached_processes_vec();
    let cache = snapshot.process_cache();
    assert_eq!(vec.len(), cache.len());

    for proc in &vec {
        assert!(
            cache.contains_key(&proc.pid),
            "Vec entry PID {} should exist in cache",
            proc.pid
        );
    }
}

#[test]
fn test_incremental_refresh_idempotent() {
    let mut snapshot = SystemSnapshot::new().expect("SystemSnapshot::new() should not panic");
    snapshot.refresh().expect("refresh() should succeed");

    snapshot
        .refresh_heavy_incremental()
        .expect("first incremental should succeed");
    let count1 = snapshot.process_cache().len();

    snapshot
        .refresh_heavy_incremental()
        .expect("second incremental should succeed");
    let count2 = snapshot.process_cache().len();

    // Counts should be similar (processes may start/stop but not hundreds in 2s)
    assert!(
        (count1 as i64 - count2 as i64).unsigned_abs() < 50,
        "Process count should be stable between incremental refreshes: {} vs {}",
        count1,
        count2
    );
}
