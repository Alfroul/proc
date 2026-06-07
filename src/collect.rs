use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::error::Result;

const CPU_EMA_ALPHA: f32 = 0.3;

/// Windows API fallback: sysinfo returns WorkingSetSize which is 0 for vmmem/Hyper-V processes.
/// Uses PROCESS_MEMORY_COUNTERS_EX.PrivateUsage (committed private memory) instead.
#[cfg(target_os = "windows")]
fn query_process_memory_winapi(pid: u32) -> Option<u64> {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::Foundation::CloseHandle;

    unsafe {
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(e) => {
                tracing::trace!("OpenProcess({}) failed: {:?}", pid, e);
                return None;
            }
        };

        let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        let result = GetProcessMemoryInfo(
            handle,
            &mut counters as *mut _ as *mut _,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        );

        let _ = CloseHandle(handle);

        if let Err(e) = result {
            tracing::warn!("GetProcessMemoryInfo({}) failed: {:?}", pid, e);
            return None;
        }

        let private = counters.PrivateUsage as u64;
        let pagefile = counters.PagefileUsage as u64;
        let ws = counters.WorkingSetSize as u64;
        tracing::debug!(
            "query_process_memory_winapi({}): ws={} private={} pagefile={}",
            pid, ws, private, pagefile
        );
        Some(private.max(pagefile).max(ws))
    }
}

#[cfg(not(target_os = "windows"))]
fn query_process_memory_winapi(_pid: u32) -> Option<u64> {
    None
}

/// Collect processes that sysinfo misses (e.g., vmmemWSL in Session 0).
/// Uses CreateToolhelp32Snapshot to enumerate all PIDs, then for any PID
/// not in sysinfo's list, queries name + memory via Windows API.
#[cfg(target_os = "windows")]
fn collect_missing_processes(
    existing_pids: &std::collections::HashSet<u32>,
    tasklist_memory: &HashMap<u32, u64>,
) -> Vec<ProcessInfo> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32First, Process32Next,
        TH32CS_SNAPPROCESS, PROCESSENTRY32,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::Foundation::CloseHandle;

    let mut result = Vec::new();

    unsafe {
        let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(s) => s,
            Err(_) => return result,
        };

        let mut entry: PROCESSENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;

        if Process32First(snapshot, &mut entry).is_err() {
            let _ = CloseHandle(snapshot).ok();
            return result;
        }

        loop {
            let pid = entry.th32ProcessID;
            if pid != 0 && !existing_pids.contains(&pid) {
                let name_bytes: Vec<u8> = entry.szExeFile.iter()
                    .take_while(|&&b| b != 0)
                    .map(|&b| b as u8)
                    .collect();
                let name = String::from_utf8_lossy(&name_bytes).to_string();

                let memory = if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let mut counters: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
                    let mem = if GetProcessMemoryInfo(
                        handle,
                        &mut counters as *mut _ as *mut _,
                        std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                    ).is_ok() {
                        let private = counters.PrivateUsage as u64;
                        let pagefile = counters.PagefileUsage as u64;
                        let ws = counters.WorkingSetSize as u64;
                        private.max(pagefile).max(ws)
                    } else {
                        0
                    };
                    let _ = CloseHandle(handle).ok();
                    mem
                } else {
                    0
                };

                let memory = memory.max(tasklist_memory.get(&pid).copied().unwrap_or(0));

                if memory > 0 {
                    let parent_pid = if entry.th32ParentProcessID != 0 {
                        Some(entry.th32ParentProcessID)
                    } else {
                        None
                    };

                    result.push(ProcessInfo {
                        pid,
                        name,
                        cpu_usage: 0.0,
                        memory,
                        virtual_memory: 0,
                        disk_usage: (0, 0),
                        status: "Run".to_string(),
                        exe: None,
                        cmd: Vec::new(),
                        cwd: None,
                        parent_pid,
                        session_id: None,
                        user_id: None,
                        start_time: 0,
                        run_time: 0,
                    });
                }
            }

            if Process32Next(snapshot, &mut entry).is_err() {
                break;
            }
        }

        let _ = CloseHandle(snapshot).ok();
    }

    result
}

#[cfg(not(target_os = "windows"))]
fn collect_missing_processes(
    _existing_pids: &std::collections::HashSet<u32>,
    _tasklist_memory: &HashMap<u32, u64>,
) -> Vec<ProcessInfo> {
    Vec::new()
}

/// 磁盘信息（用于侧边栏展示）
#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub used: u64,
    pub total: u64,
    pub is_removable: bool,
}

/// 网卡信息（名称 + IPv4）
#[derive(Debug, Clone)]
pub struct NetAdapterInfo {
    pub name: String,
    pub ipv4: Option<String>,
}

/// TCP 连接统计
#[derive(Debug, Clone, Default)]
pub struct TcpStats {
    pub established: usize,
    pub time_wait: usize,
    pub close_wait: usize,
    pub listen: usize,
}

/// 查询所有网卡的 IPv4 地址（Windows API GetAdaptersAddresses）
#[cfg(target_os = "windows")]
fn query_adapter_ipv4_addresses() -> Vec<NetAdapterInfo> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST,
        GAA_FLAG_SKIP_DNS_SERVER, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, ADDRESS_FAMILY, SOCKADDR_IN};
    use windows::core::PCWSTR;
    use windows::core::HRESULT;

    let mut adapters: Vec<NetAdapterInfo> = Vec::new();

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let family: u32 = AF_INET.0 as u32;

    let mut buf_len: u32 = 0;
    let _ = unsafe {
        GetAdaptersAddresses(family, flags, None, None, &mut buf_len)
    };

    if buf_len == 0 {
        return adapters;
    }

    let mut buffer: Vec<u8> = vec![0u8; buf_len as usize];
    let head = buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;

    let result = unsafe {
        GetAdaptersAddresses(family, flags, None, Some(head), &mut buf_len)
    };

    if HRESULT(result as i32).is_err() {
        return adapters;
    }

    let mut current = head;
    while !current.is_null() {
        let adapter = unsafe { &*current };
        let name = unsafe {
            let ptr = adapter.FriendlyName;
            if ptr.is_null() {
                String::new()
            } else {
                PCWSTR(ptr.as_ptr()).to_string().unwrap_or_default()
            }
        };

        if name.is_empty() {
            current = adapter.Next;
            continue;
        }

        let mut ipv4: Option<String> = None;
        let mut ua = adapter.FirstUnicastAddress;
        while !ua.is_null() {
            let addr = unsafe { &*ua };
            let sa = unsafe { &*addr.Address.lpSockaddr };
            if sa.sa_family == ADDRESS_FAMILY(AF_INET.0 as u16) {
                let sin = unsafe { &*(addr.Address.lpSockaddr as *const SOCKADDR_IN) };
                let addr_bytes = unsafe { sin.sin_addr.S_un.S_addr.to_ne_bytes() };
                ipv4 = Some(format!("{}.{}.{}.{}",
                    addr_bytes[0], addr_bytes[1], addr_bytes[2], addr_bytes[3]));
                break;
            }
            ua = addr.Next;
        }

        adapters.push(NetAdapterInfo { name, ipv4 });
        current = adapter.Next;
    }

    adapters
}

#[cfg(not(target_os = "windows"))]
fn query_adapter_ipv4_addresses() -> Vec<NetAdapterInfo> {
    Vec::new()
}

/// 查询 TCP 连接状态统计（轻量版，只计数不关联进程）
#[cfg(target_os = "windows")]
fn query_tcp_stats() -> TcpStats {
    let af_flags = netstat2::AddressFamilyFlags::IPV4 | netstat2::AddressFamilyFlags::IPV6;
    let proto_flags = netstat2::ProtocolFlags::TCP;

    let mut stats = TcpStats::default();

    let sockets_info = match netstat2::get_sockets_info(af_flags, proto_flags) {
        Ok(s) => s,
        Err(_) => return stats,
    };

    for si in &sockets_info {
        if let netstat2::ProtocolSocketInfo::Tcp(tcp) = &si.protocol_socket_info {
            let state_str = format!("{}", tcp.state);
            if state_str.contains("Established") {
                stats.established += 1;
            } else if state_str.contains("TimeWait") || state_str.contains("TIME_WAIT") {
                stats.time_wait += 1;
            } else if state_str.contains("CloseWait") || state_str.contains("CLOSE_WAIT") {
                stats.close_wait += 1;
            } else if state_str.contains("Listen") {
                stats.listen += 1;
            }
        }
    }

    stats
}

#[cfg(not(target_os = "windows"))]
fn query_tcp_stats() -> TcpStats {
    TcpStats::default()
}

/// 进程信息
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub memory: u64,
    pub virtual_memory: u64,
    pub disk_usage: (u64, u64),
    pub status: String,
    pub exe: Option<String>,
    pub cmd: Vec<String>,
    pub cwd: Option<String>,
    pub parent_pid: Option<u32>,
    pub session_id: Option<u32>,
    pub user_id: Option<String>,
    pub start_time: u64,
    pub run_time: u64,
}

/// 排序字段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortField {
    #[default]
    Cpu,
    Memory,
    Pid,
    Name,
    Security,
}

impl SortField {
    pub fn label(&self) -> &str {
        match self {
            Self::Cpu => "CPU%",
            Self::Memory => "MEM%",
            Self::Pid => "PID",
            Self::Name => "名称",
            Self::Security => "安全分",
        }
    }

    /// 循环切换到下一个排序字段
    pub fn next(&self) -> Self {
        match self {
            Self::Cpu => Self::Memory,
            Self::Memory => Self::Pid,
            Self::Pid => Self::Name,
            Self::Name => Self::Security,
            Self::Security => Self::Cpu,
        }
    }

    /// 循环切换到上一个排序字段
    pub fn prev(&self) -> Self {
        match self {
            Self::Cpu => Self::Security,
            Self::Memory => Self::Cpu,
            Self::Pid => Self::Memory,
            Self::Name => Self::Pid,
            Self::Security => Self::Name,
        }
    }
}

/// 系统快照
pub struct SystemSnapshot {
    sys: sysinfo::System,
    networks: sysinfo::Networks,
    refresh_time: Instant,
    num_cpus: f32,
    prev_cpu: HashMap<u32, f32>,
    tasklist_memory: HashMap<u32, u64>,
    tasklist_rx: Option<std::sync::mpsc::Receiver<HashMap<u32, u64>>>,
    tasklist_last_refresh: Instant,
    prev_net_received: u64,
    prev_net_transmitted: u64,
    prev_net_time: Instant,
    pub net_down_speed: u64,
    pub net_up_speed: u64,
    disks: sysinfo::Disks,
    components: sysinfo::Components,
    pub net_total_rx: u64,
    pub net_total_tx: u64,
    gpu_collector: crate::gpu::GpuCollector,
    gpu_info: Vec<crate::gpu::GpuInfo>,
    // Replay mode overrides
    replay_cpu: Option<f32>,
    replay_memory: Option<(u64, u64)>,
    replay_net: Option<(u64, u64)>,
}

/// 调用 Windows 内置的 tasklist 命令获取所有进程的内存。
/// tasklist 底层使用 NtQuerySystemInformation，能读到 OpenProcess 无权访问的进程（如 vmmemWSL）。
#[cfg(target_os = "windows")]
fn query_all_process_memories_tasklist() -> HashMap<u32, u64> {
    let output = match std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return HashMap::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut map = HashMap::new();

    for line in stdout.lines() {
        // CSV格式: "name","pid","session","sess#","mem K"
        // 内存值含千位逗号如 "1,245,580 K"，不能按 , 分割
        let parts: Vec<&str> = line.split("\",\"").collect();
        if parts.len() >= 5 {
            let pid_str = parts[1].trim();
            let mem_str = parts[4].trim_matches('"').replace(",", "").replace(' ', "");
            let mem_str = mem_str.trim_end_matches('K').trim_end_matches('k').trim();
            if let (Ok(pid), Ok(kb)) = (pid_str.parse::<u32>(), mem_str.parse::<u64>()) {
                map.insert(pid, kb * 1024);
            }
        }
    }

    tracing::debug!("tasklist parsed {} processes", map.len());

    map
}

#[cfg(not(target_os = "windows"))]
fn query_all_process_memories_tasklist() -> HashMap<u32, u64> {
    HashMap::new()
}

impl SystemSnapshot {
    /// 初始化 sysinfo
    pub fn new() -> Result<Self> {
        let sys = sysinfo::System::new_all();
        let num_cpus = sys.cpus().len().max(1) as f32;
        let networks = sysinfo::Networks::new_with_refreshed_list();
        let (total_rx, total_tx) = sum_network_totals(&networks);
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let components = sysinfo::Components::new_with_refreshed_list();
        let now = Instant::now();
        Ok(Self {
            sys,
            networks,
            refresh_time: now,
            num_cpus,
            prev_cpu: HashMap::new(),
            tasklist_memory: HashMap::new(),
            tasklist_rx: None,
            tasklist_last_refresh: Instant::now() - Duration::from_secs(60),
            prev_net_received: total_rx,
            prev_net_transmitted: total_tx,
            prev_net_time: now,
            net_down_speed: 0,
            net_up_speed: 0,
            disks,
            components,
            net_total_rx: total_rx,
            net_total_tx: total_tx,
            gpu_collector: crate::gpu::GpuCollector::new(),
            gpu_info: Vec::new(),
            replay_cpu: None,
            replay_memory: None,
            replay_net: None,
        })
    }

    /// 轻量刷新（每 1 秒）：CPU/内存/GPU/网络/温度
    pub fn refresh_light(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.networks.refresh(true);
        self.components.refresh(true);
        self.gpu_info = self.gpu_collector.refresh();

        let (total_rx, total_tx) = sum_network_totals(&self.networks);
        let now = Instant::now();
        let elapsed = now.duration_since(self.prev_net_time).as_secs_f64().max(0.001);
        self.net_down_speed = ((total_rx.saturating_sub(self.prev_net_received)) as f64 / elapsed) as u64;
        self.net_up_speed = ((total_tx.saturating_sub(self.prev_net_transmitted)) as f64 / elapsed) as u64;
        self.prev_net_received = total_rx;
        self.prev_net_transmitted = total_tx;
        self.prev_net_time = now;
        self.net_total_rx = total_rx;
        self.net_total_tx = total_tx;
    }

    /// 重量刷新（每 2 秒）：进程列表、磁盘、EMA 平滑、GPU
    pub fn refresh_heavy(&mut self) -> Result<()> {
        self.sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_disk_usage()
                .with_exe(sysinfo::UpdateKind::OnlyIfNotSet)
                .with_cwd(sysinfo::UpdateKind::OnlyIfNotSet)
                .with_cmd(sysinfo::UpdateKind::OnlyIfNotSet),
        );

        self.disks.refresh(true);

        let alive_pids: std::collections::HashSet<u32> = self
            .sys
            .processes()
            .keys()
            .map(|p| p.as_u32())
            .collect();
        self.prev_cpu.retain(|pid, _| alive_pids.contains(pid));

        for (pid, proc) in self.sys.processes() {
            let normalized = proc.cpu_usage() / self.num_cpus;
            let pid_u32 = pid.as_u32();
            let smoothed = match self.prev_cpu.get(&pid_u32) {
                Some(prev) => CPU_EMA_ALPHA * normalized + (1.0 - CPU_EMA_ALPHA) * prev,
                None => normalized,
            };
            self.prev_cpu.insert(pid_u32, smoothed);
        }

        // Non-blocking tasklist refresh: spawn background thread, receive when ready
        if let Some(ref rx) = self.tasklist_rx {
            if let Ok(map) = rx.try_recv() {
                self.tasklist_memory = map;
                self.tasklist_rx = None;
                self.tasklist_last_refresh = Instant::now();
            }
        }
        if self.tasklist_last_refresh.elapsed() >= Duration::from_secs(30) && self.tasklist_rx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.tasklist_rx = Some(rx);
            self.tasklist_last_refresh = Instant::now();
            std::thread::spawn(move || {
                let map = query_all_process_memories_tasklist();
                let _ = tx.send(map);
            });
        }

        self.refresh_time = Instant::now();
        Ok(())
    }

    /// 完整刷新（首次启动用）
    pub fn refresh(&mut self) -> Result<()> {
        self.refresh_light();
        self.refresh_heavy()
    }

    /// 获取进程列表（sysinfo + 补充缺失的系统进程如 vmmemWSL）
    pub fn processes(&self) -> Vec<ProcessInfo> {
        let mut procs: Vec<ProcessInfo> = self.sys
            .processes()
            .iter()
            .map(|(pid, proc)| {
                let disk = proc.disk_usage();
                let raw_cpu = proc.cpu_usage() / self.num_cpus;
                let smoothed = match self.prev_cpu.get(&pid.as_u32()) {
                    Some(prev) => CPU_EMA_ALPHA * raw_cpu + (1.0 - CPU_EMA_ALPHA) * prev,
                    None => raw_cpu,
                };
                ProcessInfo {
                    pid: pid.as_u32(),
                    name: proc.name().to_string_lossy().to_string(),
                    cpu_usage: smoothed,
                    memory: {
                        let ws = proc.memory();
                        let vm = proc.virtual_memory();
                        let tasklist = self.tasklist_memory.get(&pid.as_u32()).copied().unwrap_or(0);
                        let winapi = if ws == 0 && tasklist == 0 {
                            query_process_memory_winapi(pid.as_u32())
                        } else {
                            None
                        };
                        let result = ws.max(winapi.unwrap_or(0)).max(tasklist).max(vm);
                        result
                    },
                    virtual_memory: proc.virtual_memory(),
                    disk_usage: (disk.read_bytes, disk.written_bytes),
                    status: format!("{:?}", proc.status()),
                    exe: proc.exe().map(|p| p.to_string_lossy().to_string()),
                    cmd: proc.cmd().iter().map(|s| s.to_string_lossy().to_string()).collect(),
                    cwd: proc.cwd().map(|p| p.to_string_lossy().to_string()),
                    parent_pid: proc.parent().map(|p| p.as_u32()),
                    session_id: None,
                    user_id: proc.user_id().map(|uid| uid.to_string()),
                    start_time: proc.start_time(),
                    run_time: proc.run_time(),
                }
            })
            .collect();

        let existing_pids: std::collections::HashSet<u32> =
            procs.iter().map(|p| p.pid).collect();
        procs.extend(collect_missing_processes(&existing_pids, &self.tasklist_memory));

        procs
    }

    /// 获取全局 CPU 使用率 (0-100)
    pub fn cpu_usage(&self) -> f32 {
        if let Some(cpu) = self.replay_cpu {
            return cpu;
        }
        let cpus = self.sys.cpus();
        if cpus.is_empty() {
            return 0.0;
        }
        let total: f32 = cpus.iter().map(|c| c.cpu_usage()).sum();
        total / cpus.len() as f32
    }

    /// 获取内存使用情况 (已用 bytes, 总量 bytes)
    pub fn memory_usage(&self) -> (u64, u64) {
        if let Some(mem) = self.replay_memory {
            return mem;
        }
        let total = self.sys.total_memory();
        let used = self.sys.used_memory();
        (used, total)
    }

    pub fn set_replay_metrics(&mut self, cpu: f32, mem_used: u64, mem_total: u64, net_down: u64, net_up: u64) {
        self.replay_cpu = Some(cpu);
        self.replay_memory = Some((mem_used, mem_total));
        self.replay_net = Some((net_down, net_up));
        self.net_down_speed = net_down;
        self.net_up_speed = net_up;
    }

    pub fn clear_replay(&mut self) {
        self.replay_cpu = None;
        self.replay_memory = None;
        self.replay_net = None;
    }

    /// 获取 Swap 使用情况 (已用 bytes, 总量 bytes)
    pub fn swap_usage(&self) -> (u64, u64) {
        (self.sys.used_swap(), self.sys.total_swap())
    }

    /// 获取系统盘（最大非可移动磁盘）使用情况 (已用 bytes, 总量 bytes)
    pub fn disk_usage(&self) -> (u64, u64) {
        self.disks
            .list()
            .iter()
            .filter(|d| !d.is_removable())
            .max_by_key(|d| d.total_space())
            .map(|d| (d.total_space() - d.available_space(), d.total_space()))
            .unwrap_or((0, 0))
    }

    /// 获取系统运行时间（秒）
    pub fn uptime_secs() -> u64 {
        sysinfo::System::uptime()
    }

    /// 获取 CPU 和 GPU 温度 (cpu_temp, gpu_temp)，None 表示未检测到
    pub fn temperatures(&self) -> (Option<f32>, Option<f32>) {
        let mut cpu_temp: Option<f32> = None;
        let mut gpu_temp: Option<f32> = None;
        for component in self.components.iter() {
            let label = component.label().to_lowercase();
            let temp = match component.temperature() {
                Some(t) if t >= 0.0 => t,
                _ => continue,
            };
            if cpu_temp.is_none() && (label.contains("cpu") || label.contains("core")) {
                cpu_temp = Some(temp);
            }
            if gpu_temp.is_none() && (label.contains("gpu") || label.contains("render")) {
                gpu_temp = Some(temp);
            }
        }
        (cpu_temp, gpu_temp)
    }

    pub fn process_name_map(&self) -> std::collections::HashMap<u32, String> {
        self.sys
            .processes()
            .iter()
            .map(|(pid, proc)| (pid.as_u32(), proc.name().to_string_lossy().to_string()))
            .collect()
    }

    pub fn gpu_info(&self) -> &[crate::gpu::GpuInfo] {
        &self.gpu_info
    }

    /// 获取上次刷新时间
    pub fn last_refresh(&self) -> Instant {
        self.refresh_time
    }

    /// 获取进程数量
    pub fn process_count(&self) -> usize {
        self.sys.processes().len()
    }

    pub fn disk_io_speed(&self, procs: &[ProcessInfo]) -> (u64, u64) {
        let mut read: u64 = 0;
        let mut write: u64 = 0;
        for p in procs {
            read += p.disk_usage.0;
            write += p.disk_usage.1;
        }
        (read, write)
    }

    /// 获取所有磁盘信息列表
    pub fn all_disks(&self) -> Vec<DiskInfo> {
        self.disks
            .list()
            .iter()
            .map(|d| DiskInfo {
                name: d.name().to_string_lossy().to_string(),
                mount_point: d.mount_point().to_string_lossy().to_string(),
                used: d.total_space() - d.available_space(),
                total: d.total_space(),
                is_removable: d.is_removable(),
            })
            .collect()
    }

    /// 获取活跃网卡信息（过滤 APIPA 169.254.x.x 和回环 127.0.0.1）
    pub fn net_adapters(&self) -> Vec<NetAdapterInfo> {
        let ipv4_map = query_adapter_ipv4_addresses();
        ipv4_map
            .into_iter()
            .filter(|a| match &a.ipv4 {
                Some(ip) => !ip.starts_with("169.254.") && ip != "127.0.0.1",
                None => false,
            })
            .collect()
    }

    /// 获取 TCP 连接状态统计
    pub fn tcp_stats() -> TcpStats {
        query_tcp_stats()
    }
}

/// 检测当前进程是否以管理员权限运行
#[cfg(target_os = "windows")]
pub fn is_elevated() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut size = 0u32;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        let _ = windows::Win32::Foundation::CloseHandle(token);
        result.is_ok() && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_elevated() -> bool {
    false
}

/// 轻量刷新间隔（CPU/内存/GPU/网络等指标）
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
/// 重量刷新间隔（进程列表、端口扫描等）
pub const HEAVY_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

fn sum_network_totals(networks: &sysinfo::Networks) -> (u64, u64) {
    let mut total_rx: u64 = 0;
    let mut total_tx: u64 = 0;
    for (_name, data) in networks {
        total_rx += data.received();
        total_tx += data.transmitted();
    }
    (total_rx, total_tx)
}
