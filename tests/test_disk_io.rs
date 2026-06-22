use proc::collect::{DiskIoInfo, ProcessInfo, SortField};

#[test]
fn test_disk_io_info_instantiation() {
    let info = DiskIoInfo {
        name: "NVMe".to_string(),
        mount_point: "C:\\".to_string(),
        read_speed: 1024 * 1024 * 50,
        write_speed: 1024 * 1024 * 25,
    };
    assert_eq!(info.name, "NVMe");
    assert_eq!(info.mount_point, "C:\\");
    assert_eq!(info.read_speed, 1024 * 1024 * 50);
    assert_eq!(info.write_speed, 1024 * 1024 * 25);
}

#[test]
fn test_per_disk_speed_calculation() {
    // Simulate: first snapshot has no prev, speed = 0
    let prev: std::collections::HashMap<String, (u64, u64)> = std::collections::HashMap::new();

    let mount = "C:\\".to_string();
    let curr_read: u64 = 1_000_000_000;
    let curr_write: u64 = 500_000_000;
    let elapsed = 2.0;

    let read_speed = match prev.get(&mount) {
        Some(&(pr, _)) => ((curr_read.saturating_sub(pr)) as f64 / elapsed) as u64,
        None => 0,
    };
    let write_speed = match prev.get(&mount) {
        Some(&(_, pw)) => ((curr_write.saturating_sub(pw)) as f64 / elapsed) as u64,
        None => 0,
    };
    assert_eq!(read_speed, 0);
    assert_eq!(write_speed, 0);

    // Second snapshot with prev
    let mut prev2 = std::collections::HashMap::new();
    prev2.insert(mount.clone(), (800_000_000u64, 400_000_000u64));
    let curr_read2: u64 = 1_000_000_000;
    let curr_write2: u64 = 500_000_000;

    let read_speed2 = match prev2.get(&mount) {
        Some(&(pr, _)) => ((curr_read2.saturating_sub(pr)) as f64 / elapsed) as u64,
        None => 0,
    };
    let write_speed2 = match prev2.get(&mount) {
        Some(&(_, pw)) => ((curr_write2.saturating_sub(pw)) as f64 / elapsed) as u64,
        None => 0,
    };
    assert_eq!(read_speed2, 100_000_000); // 200MB / 2s = 100MB/s
    assert_eq!(write_speed2, 50_000_000); // 100MB / 2s = 50MB/s
}

#[test]
fn test_per_process_speed_calculation() {
    let prev_r: u64 = 1_000_000;
    let prev_w: u64 = 500_000;
    let curr_usage = (2_000_000u64, 1_000_000u64);
    let elapsed = 2.0;

    let read_speed = ((curr_usage.0.saturating_sub(prev_r)) as f64 / elapsed) as u64;
    let write_speed = ((curr_usage.1.saturating_sub(prev_w)) as f64 / elapsed) as u64;

    assert_eq!(read_speed, 500_000); // 1MB / 2s = 500KB/s
    assert_eq!(write_speed, 250_000); // 500KB / 2s = 250KB/s
}

#[test]
fn test_sort_field_cycle() {
    // next() cycle
    let mut f = SortField::Cpu;
    f = f.next();
    assert_eq!(f, SortField::Memory);
    f = f.next();
    assert_eq!(f, SortField::Pid);
    f = f.next();
    assert_eq!(f, SortField::Name);
    f = f.next();
    assert_eq!(f, SortField::Security);
    f = f.next();
    assert_eq!(f, SortField::DiskRead);
    f = f.next();
    assert_eq!(f, SortField::DiskWrite);
    f = f.next();
    assert_eq!(f, SortField::NetSent);
    f = f.next();
    assert_eq!(f, SortField::NetRecv);
    f = f.next();
    assert_eq!(f, SortField::Cpu);

    // prev() cycle
    let mut f = SortField::Cpu;
    f = f.prev();
    assert_eq!(f, SortField::NetRecv);
    f = f.prev();
    assert_eq!(f, SortField::NetSent);
    f = f.prev();
    assert_eq!(f, SortField::DiskWrite);
    f = f.prev();
    assert_eq!(f, SortField::DiskRead);
    f = f.prev();
    assert_eq!(f, SortField::Security);
    f = f.prev();
    assert_eq!(f, SortField::Name);
    f = f.prev();
    assert_eq!(f, SortField::Pid);
    f = f.prev();
    assert_eq!(f, SortField::Memory);
    f = f.prev();
    assert_eq!(f, SortField::Cpu);
}

#[test]
fn test_sort_field_labels() {
    assert_eq!(SortField::DiskRead.label(), "磁盘R");
    assert_eq!(SortField::DiskWrite.label(), "磁盘W");
    assert_eq!(SortField::Cpu.label(), "CPU%");
    assert_eq!(SortField::Memory.label(), "MEM%");
}

#[test]
fn test_format_bytes() {
    assert_eq!(proc::format::format_bytes(0), "0B");
    assert_eq!(proc::format::format_bytes(512), "512B");
    assert_eq!(proc::format::format_bytes(1024), "1KB");
    assert_eq!(proc::format::format_bytes(1024 * 1024), "1MB");
    assert_eq!(proc::format::format_bytes(1024 * 1024 * 1024), "1.0GB");
    assert_eq!(proc::format::format_bytes(1536 * 1024 * 1024), "1.5GB");
}

#[test]
fn test_process_info_disk_fields() {
    let proc = ProcessInfo {
        pid: 1234,
        name: "test.exe".to_string(),
        cpu_usage: 5.0,
        memory: 1024 * 1024 * 100,
        virtual_memory: 1024 * 1024 * 200,
        disk_usage: (1_000_000, 500_000),
        disk_read_speed: 500_000,
        disk_write_speed: 250_000,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: "Run".to_string(),
        exe: Some("C:\\test.exe".to_string()),
        cmd: vec![],
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
    };
    assert_eq!(proc.disk_usage.0, 1_000_000);
    assert_eq!(proc.disk_usage.1, 500_000);
    assert_eq!(proc.disk_read_speed, 500_000);
    assert_eq!(proc.disk_write_speed, 250_000);
}
