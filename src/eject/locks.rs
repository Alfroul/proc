use crate::classify;
use crate::collect::ProcessInfo;
use crate::error::Result;
use crate::port_map;

/// 句柄占用信息
#[derive(Debug, Clone)]
pub struct HandleLock {
    pub pid: u32,
    pub process_name: String,
    pub exe_path: Option<String>,
    pub process_class: classify::ProcessClass,
    pub port_info: Vec<String>,
}

pub fn find_volume_lockers(drive_letter: char) -> Result<Vec<HandleLock>> {
    find_volume_lockers_with_processes(drive_letter, &[])
}

pub fn find_volume_lockers_with_processes(drive_letter: char, processes: &[crate::collect::ProcessInfo]) -> Result<Vec<HandleLock>> {
    let drive_root = format!("{}:\\", drive_letter);

    let pids = filelocksmith::find_processes_locking_path(&drive_root);

    let mut locks = Vec::new();

    for pid_usize in pids {
        let pid = pid_usize as u32;
        let pid_sys = sysinfo::Pid::from_u32(pid);

        let (name, exe) = processes.iter()
            .find(|p| p.pid == pid)
            .map(|p| (p.name.clone(), p.exe.clone()))
            .unwrap_or_else(|| {
                let sys = sysinfo::System::new_all();
                if let Some(proc) = sys.process(pid_sys) {
                    (proc.name().to_string_lossy().to_string(), proc.exe().map(|p| p.to_string_lossy().to_string()))
                } else {
                    (format!("PID {}", pid), None)
                }
            });

        let proc_info = ProcessInfo {
            pid,
            name: name.clone(),
            cpu_usage: 0.0,
            memory: 0,
            virtual_memory: 0,
            disk_usage: (0, 0),
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
        let process_class = classify::classify_process(&proc_info);

        let port_info = port_map::find_ports_by_pid(pid)
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

    Ok(locks)
}
