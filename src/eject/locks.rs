//! Windows 卷句柄占用扫描。整个模块 cfg-gate 到 Windows（见 ADR-0002）。

use super::HandleLock;
use crate::error::Result;

pub fn find_volume_lockers(drive_letter: char) -> Result<Vec<HandleLock>> {
    find_volume_lockers_with_processes(drive_letter, &[])
}

pub fn find_volume_lockers_with_processes(
    drive_letter: char,
    processes: &[crate::collect::ProcessInfo],
) -> Result<Vec<HandleLock>> {
    let started = std::time::Instant::now();
    let drive_root = format!("{}:\\", drive_letter);

    let pids = filelocksmith::find_processes_locking_path(&drive_root);
    let raw_pid_count = pids.len();

    // 缓存里命不中的 PID 才需要查 sysinfo；一次性在循环外构造 fallback map，
    // 避免每个未命中 PID 都触发一次 sysinfo::System::new_all()（50-200ms）。
    let cached_pids: std::collections::HashSet<u32> = processes.iter().map(|p| p.pid).collect();
    let missing_pids: Vec<u32> = pids
        .iter()
        .map(|&p| p as u32)
        .filter(|pid| !cached_pids.contains(pid))
        .collect();

    let fallback_map: std::collections::HashMap<u32, (String, Option<String>)> =
        if missing_pids.is_empty() {
            std::collections::HashMap::new()
        } else {
            crate::collect::sysinfo_with(|sys| {
                missing_pids
                    .iter()
                    .filter_map(|&pid| {
                        let pid_sys = sysinfo::Pid::from_u32(pid);
                        sys.process(pid_sys).map(|proc| {
                            (
                                pid,
                                (
                                    proc.name().to_string_lossy().to_string(),
                                    proc.exe().map(|p| p.to_string_lossy().to_string()),
                                ),
                            )
                        })
                    })
                    .collect()
            })
        };

    let mut locks = Vec::new();

    for pid_usize in pids {
        let pid = pid_usize as u32;

        let (name, exe) = processes
            .iter()
            .find(|p| p.pid == pid)
            .map(|p| (p.name.clone(), p.exe.clone()))
            .or_else(|| fallback_map.get(&pid).cloned())
            .unwrap_or_else(|| (format!("PID {}", pid), None));

        let proc_info = crate::collect::ProcessInfo {
            pid,
            name: name.clone(),
            cpu_usage: 0.0,
            memory: 0,
            virtual_memory: 0,
            disk_usage: (0, 0),
            disk_read_speed: 0,
            disk_write_speed: 0,
            status: String::new(),
            exe: exe.clone(),
            cmd: Vec::new(),
            cwd: None,
            parent_pid: None,
            session_id: None,
            user_id: None,
            start_time: 0,
            run_time: 0,
        };
        let process_class = crate::classify::classify_process(&proc_info);

        let port_info = crate::port_map::find_ports_by_pid(pid)
            .unwrap_or_default()
            .iter()
            .map(|e| format!("{}:{}", e.protocol, e.local_port))
            .collect();

        locks.push(HandleLock {
            pid,
            process_name: name,
            exe_path: exe,
            process_class,
            port_info,
        });
    }

    locks.sort_by_key(|l| l.pid);
    locks.dedup_by_key(|l| l.pid);

    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        raw_pids = raw_pid_count,
        locks = locks.len(),
        "find_volume_lockers 完成",
    );

    Ok(locks)
}
