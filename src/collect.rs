use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::Result;

const CPU_EMA_ALPHA: f32 = 0.3;

// ---------------------------------------------------------------------------
// SysinfoRegistry — 全局 sysinfo::System 单例
// 设计动机见 CHANGELOG.md「阶段 5 — 资源生命周期」(ADR SysinfoRegistry 收敛)。
// ---------------------------------------------------------------------------

static SYSINFO_REGISTRY: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();

/// 全局 sysinfo::System 访问入口。首次调用时 `new_all()` 初始化。
/// 之后**不再 refresh**——读者只用来按 PID 查 name / exe，refresh 由
/// HeavyWorker 在它自己的 System 上独占负责。
pub fn sysinfo_shared() -> &'static Mutex<sysinfo::System> {
    SYSINFO_REGISTRY.get_or_init(|| Mutex::new(sysinfo::System::new_all()))
}

/// 用法：`sysinfo_with(|sys| { /* 读 sys.processes() */ })`
///
/// 闭包内**禁止**：refresh_*、再次进入 sysinfo_with、长耗时操作。
/// 只做只读的 name / exe 查询。
///
/// 中毒时（worker 线程 panic）通过 `into_inner()` 取回内部 System 继续，
/// 让 TUI 不至于因为某次 sysinfo 调用炸了就整个挂掉。
pub fn sysinfo_with<F, R>(f: F) -> R
where
    F: FnOnce(&sysinfo::System) -> R,
{
    let guard = sysinfo_shared().lock().unwrap_or_else(|e| {
        tracing::warn!("sysinfo registry mutex poisoned, recovering: {:?}", e);
        e.into_inner()
    });
    f(&guard)
}

/// Windows API fallback: sysinfo returns WorkingSetSize which is 0 for vmmem/Hyper-V processes.
/// Uses PROCESS_MEMORY_COUNTERS_EX.PrivateUsage (committed private memory) instead.
#[cfg(target_os = "windows")]
fn query_process_memory_winapi(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

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
            pid,
            ws,
            private,
            pagefile
        );
        Some(private.max(pagefile).max(ws))
    }
}

/// Collect processes that sysinfo misses (e.g., vmmemWSL in Session 0).
/// Uses CreateToolhelp32Snapshot to enumerate all PIDs, then for any PID
/// not in sysinfo's list, queries name + memory via Windows API.
#[cfg(target_os = "windows")]
fn collect_missing_processes(
    existing_pids: &std::collections::HashSet<u32>,
    tasklist_memory: &HashMap<u32, u64>,
) -> Vec<ProcessInfo> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32, Process32First, Process32Next, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    fn get_process_start_time(handle: windows::Win32::Foundation::HANDLE) -> u64 {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::GetProcessTimes;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
            .is_ok()
        {
            let epoch_diff: u64 = 11644473600;
            let ft64 = ((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64);
            ft64 / 10_000_000 - epoch_diff
        } else {
            0
        }
    }

    fn get_process_exe_path(handle: windows::Win32::Foundation::HANDLE) -> Option<String> {
        use windows::core::PWSTR;
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        if unsafe {
            windows::Win32::System::Threading::QueryFullProcessImageNameW(
                handle,
                windows::Win32::System::Threading::PROCESS_NAME_FORMAT(0),
                PWSTR(buf.as_mut_ptr()),
                &mut size,
            )
        }
        .is_ok()
            && size > 0
        {
            let s = String::from_utf16_lossy(&buf[..size as usize]);
            Some(s)
        } else {
            None
        }
    }

    let mut result = Vec::new();

    // unsafe scope 1: CreateToolhelp32Snapshot
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(s) => s,
        Err(_) => return result,
    };

    let mut entry: PROCESSENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32>() as u32;

    if unsafe { Process32First(snapshot, &mut entry) }.is_err() {
        let _ = unsafe { CloseHandle(snapshot) }.ok();
        return result;
    }

    loop {
        let pid = entry.th32ProcessID;
        if pid != 0 && !existing_pids.contains(&pid) {
            // Read name bytes from entry — szExeFile is a fixed-size array, no unsafe needed
            let name_bytes: Vec<u8> = entry
                .szExeFile
                .iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as u8)
                .collect();
            let name = String::from_utf8_lossy(&name_bytes).to_string();
            let parent_pid_raw = entry.th32ParentProcessID;

            let (memory, start_time, exe) = match unsafe {
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            } {
                Ok(handle) => {
                    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
                    let mem = if unsafe {
                        GetProcessMemoryInfo(
                            handle,
                            &mut counters as *mut _ as *mut _,
                            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                        )
                    }
                    .is_ok()
                    {
                        let private = counters.PrivateUsage as u64;
                        let pagefile = counters.PagefileUsage as u64;
                        let ws = counters.WorkingSetSize as u64;
                        private.max(pagefile).max(ws)
                    } else {
                        0
                    };
                    let st = get_process_start_time(handle);
                    let exe = get_process_exe_path(handle);
                    let _ = unsafe { CloseHandle(handle) }.ok();
                    (mem, st, exe)
                }
                Err(_) => (0, 0, None),
            };

            let memory = memory.max(tasklist_memory.get(&pid).copied().unwrap_or(0));

            if memory > 0 {
                let parent_pid = if parent_pid_raw != 0 {
                    Some(parent_pid_raw)
                } else {
                    None
                };

                let run_time = if start_time > 0 {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    now.saturating_sub(start_time)
                } else {
                    0
                };

                result.push(ProcessInfo {
                    pid,
                    name: std::sync::Arc::from(name.as_str()),
                    cpu_usage: 0.0,
                    memory,
                    virtual_memory: 0,
                    disk_usage: (0, 0),
                    disk_read_speed: 0,
                    disk_write_speed: 0,
                    net_sent_rate: 0,
                    net_recv_rate: 0,
                    status: ProcessStatus::Run,
                    exe: exe.map(|s| std::sync::Arc::from(s.as_str())),
                    cmd: std::sync::Arc::from(Vec::<String>::new()),
                    cwd: None,
                    parent_pid,
                    session_id: None,
                    user_id: None,
                    start_time,
                    run_time,
                    name_lower: std::sync::Arc::from(name.to_lowercase().as_str()),
                    throttled: crate::throttle::EcoQoSState::default(),
                    signature_status: crate::security::SignatureStatus::default(),
                    parent_chain: Vec::new(),
                });
            }
        }

        if unsafe { Process32Next(snapshot, &mut entry) }.is_err() {
            break;
        }
    }

    let _ = unsafe { CloseHandle(snapshot) }.ok();

    result
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

/// TCP 连接统计。
///
/// `retransmitted_segs` / `reset_segs` / `failed_connections` 是阶段 5 D2
/// 引入的「传输质量」指标：
/// - retransmitted_segs：累计重传段数（Windows `dwRetransSegs` / Linux
///   `/proc/net/snmp` 的 `RetransSegs`）。
/// - reset_segs：累计发送的 RST 段数（Windows `dwOutRsts` / Linux `OutRsts`）。
/// - failed_connections：失败的连接尝试（Windows `dwAttemptFails` / Linux
///   `AttemptFails`）。Windows / Linux 都能在普通权限下拿到。
///
/// 这些是**累计计数**，不是速率；UI 显示时需要做 delta 来换算成速率。
#[derive(Debug, Clone, Default)]
pub struct TcpStats {
    pub established: usize,
    pub time_wait: usize,
    pub close_wait: usize,
    pub listen: usize,
    pub retransmitted_segs: u64,
    pub reset_segs: u64,
    pub failed_connections: u64,
    /// 累计输出段数,作为重传率 / RST 率的分母。Windows `dw64OutSegs`
    /// / Linux `/proc/net/snmp` 的 `OutSegs`。
    pub out_segs: u64,
}

/// 查询所有网卡的 IPv4 地址（Windows API GetAdaptersAddresses）
#[cfg(target_os = "windows")]
fn query_adapter_ipv4_addresses() -> Vec<NetAdapterInfo> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
        GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::Networking::WinSock::{ADDRESS_FAMILY, AF_INET, SOCKADDR_IN};
    use windows::core::HRESULT;
    use windows::core::PCWSTR;

    let mut adapters: Vec<NetAdapterInfo> = Vec::new();

    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    let family: u32 = AF_INET.0 as u32;

    let mut buf_len: u32 = 0;
    let _ = unsafe { GetAdaptersAddresses(family, flags, None, None, &mut buf_len) };

    if buf_len == 0 {
        return adapters;
    }

    let mut buffer: Vec<u8> = vec![0u8; buf_len as usize];
    let head = buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;

    let result = unsafe { GetAdaptersAddresses(family, flags, None, Some(head), &mut buf_len) };

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
            if sa.sa_family == ADDRESS_FAMILY(AF_INET.0) {
                let sin = unsafe { &*(addr.Address.lpSockaddr as *const SOCKADDR_IN) };
                let addr_bytes = unsafe { sin.sin_addr.S_un.S_addr.to_ne_bytes() };
                ipv4 = Some(format!(
                    "{}.{}.{}.{}",
                    addr_bytes[0], addr_bytes[1], addr_bytes[2], addr_bytes[3]
                ));
                break;
            }
            ua = addr.Next;
        }

        adapters.push(NetAdapterInfo { name, ipv4 });
        current = adapter.Next;
    }

    adapters
}

/// 查询 TCP 连接状态统计（轻量版，只计数不关联进程）+ 传输质量计数。
///
/// 传输质量指标（`retransmitted_segs` / `reset_segs` / `failed_connections`）
/// 来自 `GetTcpStatisticsEx2` 的 `MIB_TCPSTATS2`，按 IPv4 + IPv6 各跑一次
/// 再累加 —— 同样的值在 `GetTcpStatistics2` 上会重复（family-agnostic 时
/// Windows 内部把两栈合并），所以这里只读 Ex2。
#[cfg(target_os = "windows")]
fn query_tcp_stats() -> TcpStats {
    use windows::Win32::NetworkManagement::IpHelper::{GetTcpStatisticsEx2, MIB_TCPSTATS2};
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};

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
            match crate::port_map::TcpState::from_state_str(Some(&state_str)) {
                crate::port_map::TcpState::Established => stats.established += 1,
                crate::port_map::TcpState::TimeWait => stats.time_wait += 1,
                crate::port_map::TcpState::CloseWait => stats.close_wait += 1,
                crate::port_map::TcpState::Listen => stats.listen += 1,
                crate::port_map::TcpState::Other => {}
            }
        }
    }

    // IPv4 + IPv6 各一份，累加。GetTcpStatisticsEx2 返回 0 表示成功；
    // 失败（非管理员 / 无 TCP 协议栈）时静默保留 0 —— 不影响 established 计数。
    for family in [AF_INET.0 as u32, AF_INET6.0 as u32] {
        let mut raw: MIB_TCPSTATS2 = unsafe { std::mem::zeroed() };
        let rv = unsafe { GetTcpStatisticsEx2(&mut raw, family) };
        if rv != 0 {
            continue;
        }
        stats.retransmitted_segs += raw.dwRetransSegs as u64;
        stats.reset_segs += raw.dwOutRsts as u64;
        stats.failed_connections += raw.dwAttemptFails as u64;
        stats.out_segs += raw.dw64OutSegs;
    }

    stats
}

/// 进程状态枚举（v0.6.0 阶段 4）。
///
/// 用 `Copy` 枚举替代 `format!("{:?}", sysinfo::ProcessStatus)` 的 String 分配，
/// 让 `ProcessInfo::status` 字段无堆开销。变体名按 sysinfo 0.34.2 真实命名对齐
/// （`Tracing` / `LockBlocked` / `UninterruptibleDiskSleep`），不沿用 stage-4.md
/// 早期猜测的 `Traced` / `DeadLock`。
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "PascalCase")]
pub enum ProcessStatus {
    Idle,
    Run,
    Sleep,
    Stop,
    Zombie,
    Tracing,
    Dead,
    Wakekill,
    Waking,
    Parked,
    LockBlocked,
    UninterruptibleDiskSleep,
    #[default]
    Unknown,
}

impl From<sysinfo::ProcessStatus> for ProcessStatus {
    fn from(s: sysinfo::ProcessStatus) -> Self {
        match s {
            sysinfo::ProcessStatus::Idle => Self::Idle,
            sysinfo::ProcessStatus::Run => Self::Run,
            sysinfo::ProcessStatus::Sleep => Self::Sleep,
            sysinfo::ProcessStatus::Stop => Self::Stop,
            sysinfo::ProcessStatus::Zombie => Self::Zombie,
            sysinfo::ProcessStatus::Tracing => Self::Tracing,
            sysinfo::ProcessStatus::Dead => Self::Dead,
            sysinfo::ProcessStatus::Wakekill => Self::Wakekill,
            sysinfo::ProcessStatus::Waking => Self::Waking,
            sysinfo::ProcessStatus::Parked => Self::Parked,
            sysinfo::ProcessStatus::LockBlocked => Self::LockBlocked,
            sysinfo::ProcessStatus::UninterruptibleDiskSleep => Self::UninterruptibleDiskSleep,
            sysinfo::ProcessStatus::Unknown(_) => Self::Unknown,
        }
    }
}

impl ProcessStatus {
    /// 短代号（TUI 表格列宽紧凑时可用）。
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Idle => "I",
            Self::Run => "R",
            Self::Sleep => "S",
            Self::Stop => "T",
            Self::Zombie => "Z",
            Self::Tracing => "Tr",
            Self::Dead => "D",
            Self::Wakekill => "Wk",
            Self::Waking => "Wg",
            Self::Parked => "P",
            Self::LockBlocked => "Lk",
            Self::UninterruptibleDiskSleep => "Ds",
            Self::Unknown => "?",
        }
    }

    /// 完整英文名（鼠标悬停 / detail view）。
    #[must_use]
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Run => "Running",
            Self::Sleep => "Sleeping",
            Self::Stop => "Stopped",
            Self::Zombie => "Zombie",
            Self::Tracing => "Tracing",
            Self::Dead => "Dead",
            Self::Wakekill => "Wakekill",
            Self::Waking => "Waking",
            Self::Parked => "Parked",
            Self::LockBlocked => "LockBlocked",
            Self::UninterruptibleDiskSleep => "UninterruptibleDiskSleep",
            Self::Unknown => "Unknown",
        }
    }

    /// 序列化 / 表格显示用的稳定字符串（与原 `format!("{:?}")` 等价）。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Run => "Run",
            Self::Sleep => "Sleep",
            Self::Stop => "Stop",
            Self::Zombie => "Zombie",
            Self::Tracing => "Tracing",
            Self::Dead => "Dead",
            Self::Wakekill => "Wakekill",
            Self::Waking => "Waking",
            Self::Parked => "Parked",
            Self::LockBlocked => "LockBlocked",
            Self::UninterruptibleDiskSleep => "UninterruptibleDiskSleep",
            Self::Unknown => "Unknown",
        }
    }
}

impl std::fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 进程信息
///
/// v0.6.0 阶段 4：`name / cmd / exe / cwd / user_id` 全部 Arc 化，
/// HeavyWorker 每次构造一次，后续读取 / clone 都是原子计数；
/// `status` 改 `ProcessStatus` Copy 枚举；
/// 新增 `name_lower` 预计算字段（serde skip，不影响 .prec 录屏文件）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: std::sync::Arc<str>,
    pub cpu_usage: f32,
    pub memory: u64,
    pub virtual_memory: u64,
    /// (read_bytes, written_bytes) — cumulative totals since process start
    pub disk_usage: (u64, u64),
    /// Disk read speed in bytes/sec (computed from delta / elapsed)
    pub disk_read_speed: u64,
    /// Disk write speed in bytes/sec (computed from delta / elapsed)
    pub disk_write_speed: u64,
    /// 上行字节速率 bytes/sec（阶段 7 D1，net_flow worker 推送；worker 不可用时保持 0）
    pub net_sent_rate: u64,
    /// 下行字节速率 bytes/sec（阶段 7 D1，net_flow worker 推送；worker 不可用时保持 0）
    pub net_recv_rate: u64,
    pub status: ProcessStatus,
    pub exe: Option<std::sync::Arc<str>>,
    pub cmd: std::sync::Arc<[String]>,
    pub cwd: Option<std::sync::Arc<str>>,
    pub parent_pid: Option<u32>,
    pub session_id: Option<u32>,
    pub user_id: Option<std::sync::Arc<str>>,
    pub start_time: u64,
    pub run_time: u64,
    /// v0.6.0 阶段 4：预计算的 lowercase name，搜索匹配用。
    /// heavy worker 一次性算好，避免每按键 `to_lowercase` 重建 Vec。
    /// `#[serde(skip)]`：录屏文件不持久化，重算成本低且能减小 .prec 体积。
    #[serde(skip)]
    pub name_lower: std::sync::Arc<str>,
    /// v0.7 阶段 6：Windows 11 EcoQoS / Efficiency Mode 状态（ADR-0014）。
    /// 由 HeavyWorker 批量 query 填入；非 Windows 平台恒为 `Unknown`。
    /// `#[serde(default)]`：旧录屏文件能反序列化（缺字段 → Unknown）。
    #[serde(default)]
    pub throttled: crate::throttle::EcoQoSState,
    /// v0.11.0 阶段 1：签名验证状态骨架。默认 `Pending`，阶段 4 由
    /// `BackgroundScorer` 调 `verify_signature` 异步填实。`#[serde(default)]`
    /// 让旧录屏文件能反序列化（缺字段 → Pending）。
    #[serde(default)]
    pub signature_status: crate::security::SignatureStatus,
    /// v0.11.0 阶段 1：父子链骨架。元组 `(pid, name)`，从该进程向上追溯到
    /// 根进程的完整链路。默认空 Vec，阶段 5 由 collect 时填实。
    /// `#[serde(default)]` 让旧录屏文件能反序列化（缺字段 → 空 Vec）。
    ///
    /// v0.17.0 stage 2 TD-47：元组类型从 `(u32, String)` 改 `(u32, Arc<str>)`，
    /// `build_parent_chain` body 用 `Arc::clone` 替换 `String::to_string`，零 heap
    /// alloc（仅 Vec header 分配，元素 name 走 Arc refcount 共享）。serde 透明转发
    /// 让旧 `.prec` 文件（String 序列化）能被新代码读，反之亦然。
    #[serde(default)]
    pub parent_chain: Vec<(u32, std::sync::Arc<str>)>,
}

impl Default for ProcessInfo {
    fn default() -> Self {
        static EMPTY_STR: std::sync::OnceLock<std::sync::Arc<str>> = std::sync::OnceLock::new();
        static EMPTY_CMD: std::sync::OnceLock<std::sync::Arc<[String]>> =
            std::sync::OnceLock::new();
        let empty_str = EMPTY_STR.get_or_init(|| std::sync::Arc::from(""));
        let empty_cmd = EMPTY_CMD.get_or_init(|| std::sync::Arc::from(Vec::<String>::new()));
        Self {
            pid: 0,
            name: std::sync::Arc::clone(empty_str),
            cpu_usage: 0.0,
            memory: 0,
            virtual_memory: 0,
            disk_usage: (0, 0),
            disk_read_speed: 0,
            disk_write_speed: 0,
            net_sent_rate: 0,
            net_recv_rate: 0,
            status: ProcessStatus::default(),
            exe: None,
            cmd: std::sync::Arc::clone(empty_cmd),
            cwd: None,
            parent_pid: None,
            session_id: None,
            user_id: None,
            start_time: 0,
            run_time: 0,
            name_lower: std::sync::Arc::clone(empty_str),
            throttled: crate::throttle::EcoQoSState::default(),
            signature_status: crate::security::SignatureStatus::default(),
            parent_chain: Vec::new(),
        }
    }
}

/// 进程视图模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessViewMode {
    #[default]
    List,
    Tree,
    AppGroup,
}

impl ProcessViewMode {
    #[must_use]
    pub fn toggle(&self) -> Self {
        match self {
            Self::List => Self::AppGroup,
            Self::Tree => Self::List,
            Self::AppGroup => Self::List,
        }
    }

    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::List => "列表",
            Self::Tree => "树形",
            Self::AppGroup => "应用",
        }
    }
}

/// 每磁盘 I/O 速率
#[derive(Debug, Clone)]
pub struct DiskIoInfo {
    pub name: String,
    pub mount_point: String,
    pub read_speed: u64,
    pub write_speed: u64,
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
    DiskRead,
    DiskWrite,
    /// 阶段 7 D1：按上行字节速率排序
    NetSent,
    /// 阶段 7 D1：按下行字节速率排序
    NetRecv,
}

impl SortField {
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Cpu => "CPU%",
            Self::Memory => "MEM%",
            Self::Pid => "PID",
            Self::Name => "名称",
            Self::Security => "安全分",
            Self::DiskRead => "磁盘R",
            Self::DiskWrite => "磁盘W",
            Self::NetSent => "↑网络",
            Self::NetRecv => "↓网络",
        }
    }

    /// 循环切换到下一个排序字段
    #[must_use]
    pub fn next(&self) -> Self {
        match self {
            Self::Cpu => Self::Memory,
            Self::Memory => Self::Pid,
            Self::Pid => Self::Name,
            Self::Name => Self::Security,
            Self::Security => Self::DiskRead,
            Self::DiskRead => Self::DiskWrite,
            Self::DiskWrite => Self::NetSent,
            Self::NetSent => Self::NetRecv,
            Self::NetRecv => Self::Cpu,
        }
    }

    /// 循环切换到上一个排序字段
    #[must_use]
    pub fn prev(&self) -> Self {
        match self {
            Self::Cpu => Self::NetRecv,
            Self::Memory => Self::Cpu,
            Self::Pid => Self::Memory,
            Self::Name => Self::Pid,
            Self::Security => Self::Name,
            Self::DiskRead => Self::Security,
            Self::DiskWrite => Self::DiskRead,
            Self::NetSent => Self::DiskWrite,
            Self::NetRecv => Self::NetSent,
        }
    }

    /// 稳定的字符串标识，用于持久化（不要随版本变化）。
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Pid => "pid",
            Self::Name => "name",
            Self::Security => "security",
            Self::DiskRead => "disk_read",
            Self::DiskWrite => "disk_write",
            Self::NetSent => "net_sent",
            Self::NetRecv => "net_recv",
        }
    }

    /// 与 `as_str` 对应；未知字符串返回 None（调用者回退到默认值）。
    #[must_use]
    pub fn parse_from_str(s: &str) -> Option<Self> {
        match s.trim() {
            "cpu" => Some(Self::Cpu),
            "memory" => Some(Self::Memory),
            "pid" => Some(Self::Pid),
            "name" => Some(Self::Name),
            "security" => Some(Self::Security),
            "disk_read" => Some(Self::DiskRead),
            "disk_write" => Some(Self::DiskWrite),
            "net_sent" => Some(Self::NetSent),
            "net_recv" => Some(Self::NetRecv),
            _ => None,
        }
    }
}

/// 共用进程排序：用于 `proc ls` / `proc export`。
///
/// - 非 Name 分支：所有比较器都 `.then(pid)` 作为 tie-breaker，避免同分进程顺序抖动。
/// - Name 分支：预建 lower-case Vec 后 `sort_by_key`，避免每个比较对调用一次 `to_lowercase`（N log N → N）。
pub fn sort_processes(procs: &mut Vec<ProcessInfo>, sort_field: SortField) {
    match sort_field {
        SortField::Name => {
            let mut keyed: Vec<(String, ProcessInfo)> = procs
                .drain(..)
                .map(|p| (p.name.to_lowercase(), p))
                .collect();
            keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.pid.cmp(&b.1.pid)));
            *procs = keyed.into_iter().map(|(_, p)| p).collect();
        }
        SortField::Pid => procs.sort_by_key(|p| p.pid),
        SortField::Cpu => procs.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.pid.cmp(&b.pid))
        }),
        SortField::Memory => procs.sort_by(|a, b| b.memory.cmp(&a.memory).then(a.pid.cmp(&b.pid))),
        SortField::Security => procs.sort_by_key(|p| p.pid),
        SortField::DiskRead => procs.sort_by(|a, b| {
            b.disk_read_speed
                .cmp(&a.disk_read_speed)
                .then(a.pid.cmp(&b.pid))
        }),
        SortField::DiskWrite => procs.sort_by(|a, b| {
            b.disk_write_speed
                .cmp(&a.disk_write_speed)
                .then(a.pid.cmp(&b.pid))
        }),
        SortField::NetSent => procs.sort_by(|a, b| {
            b.net_sent_rate
                .cmp(&a.net_sent_rate)
                .then(a.pid.cmp(&b.pid))
        }),
        SortField::NetRecv => procs.sort_by(|a, b| {
            b.net_recv_rate
                .cmp(&a.net_recv_rate)
                .then(a.pid.cmp(&b.pid))
        }),
    }
}

/// Background heavy-refresh result
struct HeavyResult {
    processes: HashMap<u32, ProcessInfo>,
}

/// Background heavy-refresh worker: owns its own sysinfo::System and runs
/// refresh_processes_specifics on a dedicated thread, returning results
/// via a channel so the main (UI) thread never blocks on process enumeration.
struct HeavyWorker {
    cmd_tx: std::sync::mpsc::Sender<f32>, // sends num_cpus to trigger refresh
    result_rx: std::sync::mpsc::Receiver<HeavyResult>,
}

impl HeavyWorker {
    fn start() -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<f32>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<HeavyResult>();

        std::thread::Builder::new()
            .name("proc-heavy-refresh".into())
            .spawn(move || {
                let mut sys = sysinfo::System::new_all();

                // Pre-warm: do one full refresh so the first real request is fast
                sys.refresh_processes_specifics(
                    sysinfo::ProcessesToUpdate::All,
                    false,
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory()
                        .with_disk_usage()
                        .with_exe(sysinfo::UpdateKind::Always)
                        .with_cwd(sysinfo::UpdateKind::Always)
                        .with_cmd(sysinfo::UpdateKind::Always),
                );

                while let Ok(num_cpus) = cmd_rx.recv() {
                    sys.refresh_processes_specifics(
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

                    let _alive_pids: std::collections::HashSet<u32> =
                        sys.processes().keys().map(|p| p.as_u32()).collect();

                    let mut processes = HashMap::new();
                    for (pid, proc) in sys.processes() {
                        let pid_u32 = pid.as_u32();
                        let normalized = proc.cpu_usage() / num_cpus;
                        let memory = {
                            let ws = proc.memory();
                            let vm = proc.virtual_memory();
                            let winapi = if ws == 0 {
                                query_process_memory_winapi(pid_u32)
                            } else {
                                None
                            };
                            ws.max(winapi.unwrap_or(0)).max(vm)
                        };
                        let disk = proc.disk_usage();
                        // v0.6.0 阶段 4：String → Arc<str>，构造一次后续 clone 全是
                        // 原子计数；name_lower 一次性算好，搜索路径不再每按键 to_lowercase。
                        let name_lossy = proc.name().to_string_lossy();
                        let name: std::sync::Arc<str> = std::sync::Arc::from(name_lossy.as_ref());
                        let name_lower: std::sync::Arc<str> =
                            std::sync::Arc::from(name_lossy.to_lowercase().as_str());
                        let cmd: std::sync::Arc<[String]> = std::sync::Arc::from(
                            proc.cmd()
                                .iter()
                                .map(|s| s.to_string_lossy().to_string())
                                .collect::<Vec<_>>(),
                        );
                        processes.insert(
                            pid_u32,
                            ProcessInfo {
                                pid: pid_u32,
                                name: std::sync::Arc::clone(&name),
                                cpu_usage: normalized, // raw; smoothing applied on main thread
                                memory,
                                virtual_memory: proc.virtual_memory(),
                                disk_usage: (disk.total_read_bytes, disk.total_written_bytes),
                                disk_read_speed: 0,
                                disk_write_speed: 0,
                                net_sent_rate: 0,
                                net_recv_rate: 0,
                                status: proc.status().into(),
                                exe: proc
                                    .exe()
                                    .map(|p| std::sync::Arc::from(p.to_string_lossy().as_ref())),
                                cmd,
                                cwd: proc
                                    .cwd()
                                    .map(|p| std::sync::Arc::from(p.to_string_lossy().as_ref())),
                                parent_pid: proc.parent().map(|p| p.as_u32()),
                                session_id: None,
                                user_id: proc
                                    .user_id()
                                    .map(|uid| std::sync::Arc::from(uid.to_string().as_str())),
                                start_time: proc.start_time(),
                                run_time: {
                                    let now = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs();
                                    now.saturating_sub(proc.start_time())
                                },
                                name_lower,
                                throttled: crate::throttle::EcoQoSState::default(),
                                signature_status: crate::security::SignatureStatus::default(),
                                parent_chain: Vec::new(),
                            },
                        );
                    }

                    // Merge missing processes
                    let existing_pids: std::collections::HashSet<u32> =
                        processes.keys().copied().collect();
                    for proc in collect_missing_processes(&existing_pids, &HashMap::new()) {
                        processes.insert(proc.pid, proc);
                    }

                    // v0.7 阶段 6：批量 query 当前所有 PID 的 EcoQoS 状态
                    // （ADR-0014）。每个周期一次批量调用，不在每帧 OpenProcess，
                    // 避免 500 进程下 500 次 syscall 风暴。失败的 PID 自然返回
                    // Unknown，UI 渲染时不显示 🍃。
                    let pids: Vec<u32> = processes.keys().copied().collect();
                    let throttle_map = crate::throttle::query_throttle_batch(&pids);
                    for (pid, state) in &throttle_map {
                        if let Some(p) = processes.get_mut(pid) {
                            p.throttled = *state;
                        }
                    }

                    // v0.11 阶段 5：批量填 parent_chain（stage-5.md 任务 1）。
                    // 先 collect 所有 chain 到独立 HashMap（不可变借用结束后再
                    // iter_mut 写入，绕开 Rust 借用规则）。防循环 + 32 层上限
                    // 由 `build_parent_chain` 内部保证。
                    let pid_to_chain: HashMap<u32, Vec<(u32, std::sync::Arc<str>)>> = processes
                        .keys()
                        .map(|&pid| {
                            (
                                pid,
                                crate::security::lineage::build_parent_chain(pid, &processes),
                            )
                        })
                        .collect();
                    for (pid, proc) in processes.iter_mut() {
                        if let Some(chain) = pid_to_chain.get(pid) {
                            proc.parent_chain = chain.clone();
                        }
                    }

                    let _ = result_tx.send(HeavyResult { processes });
                }
            })
            .expect("failed to spawn heavy-refresh thread");

        HeavyWorker { cmd_tx, result_rx }
    }

    /// Try to receive the latest heavy-refresh result. Non-blocking.
    fn try_recv(&self) -> Option<HeavyResult> {
        // Drain all pending results, keep only the latest
        let mut latest = None;
        while let Ok(r) = self.result_rx.try_recv() {
            latest = Some(r);
        }
        latest
    }
}

/// 后台轻量采集 worker:把 GPU / 温度 / 磁盘 IO / 电源信息从 TUI 主线程
/// 挪到独立线程。主线程 `refresh_light()` 只保留 CPU/内存/网络的快速刷新,
/// 其余改为 `try_recv` 最新 [`LightSnapshot`]。
///
/// 这些子项里 `components.refresh(true)`(WMI 温度)、`gpu_collector.refresh()`
/// (DXGI factory 反复创建)、`disks.refresh(true)` 都可能在 Windows 上单次
/// 阻塞 50~200ms,在 50ms 帧率的 TUI 主线程上肉眼可见卡顿。
///
/// 模式照搬 [`HeavyWorker`] / [`crate::port_worker::PortSnapshotWorker`]:
/// `mpsc::sync_channel(1)` + worker 自驱 1s 循环 + 主线程 `try_recv_latest`。
struct LightSnapshot {
    gpu_info: Vec<crate::gpu::GpuInfo>,
    temperatures: (Option<f32>, Option<f32>),
    disk_io_speeds: Vec<DiskIoInfo>,
    all_disks: Vec<DiskInfo>,
    /// 系统盘 (used, total)
    disk_usage: (u64, u64),
    throttle_info: Option<crate::throttle::ThrottleInfo>,
    /// Per-core CPU 频率（MHz），按 logical CPU 顺序对齐 `sys.cpus()`。
    /// 跨平台：Linux sysfs / Windows sysinfo / macOS sysinfo 都走 sysinfo。
    per_core_freq: Vec<u64>,
    /// Per-core CPU 温度（°C），与 `per_core_freq` 同长度；None 表示该核不可用。
    /// 当前实现：sysinfo 的 Components 通常只给一个全局 CPU 温度，无法分核；
    /// 把全局温度填到第 0 核，其余留 None。Sidebar 折叠模式仍用 `temperatures`。
    per_core_temp: Vec<Option<f32>>,
}

struct LightWorker {
    snapshot_rx: std::sync::mpsc::Receiver<LightSnapshot>,
    shutdown_tx: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl LightWorker {
    fn start() -> Self {
        let (snap_tx, snap_rx) = std::sync::mpsc::sync_channel::<LightSnapshot>(1);
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("proc-light-refresh".into())
            .spawn(move || light_worker_loop(snap_tx, shutdown_rx))
            .expect("failed to spawn light-refresh thread");

        Self {
            snapshot_rx: snap_rx,
            shutdown_tx: Some(shutdown_tx),
            thread: Some(handle),
        }
    }

    /// Drain 到最新一份;无新快照时返回 `None`。首次启动可阻塞等首帧。
    fn try_recv_latest(&self) -> Option<LightSnapshot> {
        let mut latest = None;
        while let Ok(s) = self.snapshot_rx.try_recv() {
            latest = Some(s);
        }
        latest
    }

    /// 阻塞等待首帧(最多 `timeout`)。`SystemSnapshot::new()` 用这个避免
    /// 启动后第一秒 sidebar 显示空白。
    fn recv_first(&self, timeout: Duration) -> Option<LightSnapshot> {
        match self.snapshot_rx.recv_timeout(timeout) {
            Ok(first) => {
                // 把可能已经排队的更 新帧也 drain 掉,只留最新
                let mut latest = first;
                while let Ok(s) = self.snapshot_rx.try_recv() {
                    latest = s;
                }
                Some(latest)
            }
            Err(_) => None,
        }
    }
}

impl Drop for LightWorker {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            drop(tx);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn light_worker_loop(
    snap_tx: std::sync::mpsc::SyncSender<LightSnapshot>,
    shutdown_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::sync::mpsc::{RecvTimeoutError, TrySendError};

    let mut disks = sysinfo::Disks::new_with_refreshed_list();
    let mut components = sysinfo::Components::new_with_refreshed_list();
    let mut gpu_collector = crate::gpu::GpuCollector::new();
    // 用于 per-core 频率采样：worker 自己持有一份 sysinfo::System，避免和主线程的
    // SysinfoRegistry 抢锁。频率只读不 refresh（sysinfo 在 Linux 上读 sysfs /
    // Windows 上读注册表，本身就是 syscall）。
    let mut sys_for_freq = sysinfo::System::new();
    let mut prev_disk_io: HashMap<String, (u64, u64)> = HashMap::new();
    let mut prev_disk_io_time = Instant::now();

    loop {
        components.refresh(true);
        let gpu_info = gpu_collector.refresh();

        // per-core 频率：refresh_cpu_usage 在 Linux 上需要一个 tick 才能 stabilize，
        // 这里只调 refresh_cpu_usage 是为了让 frequency() 字段被填到；实际上 sysinfo
        // 在 Linux 的 frequency 走 sysfs（不依赖两次 refresh 间隔），Windows 走注册表。
        sys_for_freq.refresh_cpu_usage();
        let per_core_freq = collect_per_core_freq(&sys_for_freq);
        let per_core_temp = collect_per_core_temp(&components, per_core_freq.len());

        let throttle_info = if let Some(info) = crate::throttle::query_processor_power_info() {
            crate::throttle::detect_throttle(&info)
        } else {
            None
        };

        disks.refresh(true);
        let now = Instant::now();
        let disk_elapsed = now
            .duration_since(prev_disk_io_time)
            .as_secs_f64()
            .max(0.001);

        let mut new_prev: HashMap<String, (u64, u64)> = HashMap::with_capacity(disks.list().len());
        let mut disk_io_speeds: Vec<DiskIoInfo> = Vec::with_capacity(disks.list().len());
        let mut all_disks: Vec<DiskInfo> = Vec::with_capacity(disks.list().len());

        for disk in disks.list() {
            let mount = disk.mount_point().to_string_lossy().to_string();
            let usage = disk.usage();
            let (read_speed, write_speed) = match prev_disk_io.get(&mount) {
                Some(&(prev_r, prev_w)) => (
                    ((usage.total_read_bytes.saturating_sub(prev_r)) as f64 / disk_elapsed) as u64,
                    ((usage.total_written_bytes.saturating_sub(prev_w)) as f64 / disk_elapsed)
                        as u64,
                ),
                None => (0, 0),
            };
            new_prev.insert(
                mount.clone(),
                (usage.total_read_bytes, usage.total_written_bytes),
            );
            disk_io_speeds.push(DiskIoInfo {
                name: disk.name().to_string_lossy().to_string(),
                mount_point: mount,
                read_speed,
                write_speed,
            });
            all_disks.push(DiskInfo {
                name: disk.name().to_string_lossy().to_string(),
                mount_point: disk.mount_point().to_string_lossy().to_string(),
                used: disk.total_space() - disk.available_space(),
                total: disk.total_space(),
                is_removable: disk.is_removable(),
            });
        }
        prev_disk_io = new_prev;
        prev_disk_io_time = now;

        // 温度
        let mut cpu_temp: Option<f32> = None;
        let mut gpu_temp: Option<f32> = None;
        for component in components.iter() {
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

        // 系统盘 = 最大非可移动磁盘
        let disk_usage = disks
            .list()
            .iter()
            .filter(|d| !d.is_removable())
            .max_by_key(|d| d.total_space())
            .map(|d| (d.total_space() - d.available_space(), d.total_space()))
            .unwrap_or((0, 0));

        let snapshot = LightSnapshot {
            gpu_info,
            temperatures: (cpu_temp, gpu_temp),
            disk_io_speeds,
            all_disks,
            disk_usage,
            throttle_info,
            per_core_freq,
            per_core_temp,
        };

        match snap_tx.try_send(snapshot) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => break,
        }

        match shutdown_rx.recv_timeout(Duration::from_secs(1)) {
            Ok(_) => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// SMART 磁盘健康后台 worker（阶段 5 B3）。
///
/// 模式照搬 [`LightWorker`]:独立线程 + `sync_channel(1)` + Drop 时
/// shutdown + join。poll 间隔 30s —— 比 LightWorker 慢,因为 SMART
/// 数据不需要秒级刷新,而 smartctl 子进程 / WMI 调用都不便宜。
struct SmartWorker {
    snapshot_rx: std::sync::mpsc::Receiver<Vec<crate::smart::SmartData>>,
    shutdown_tx: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SmartWorker {
    fn start() -> Self {
        let (snap_tx, snap_rx) = std::sync::mpsc::sync_channel::<Vec<crate::smart::SmartData>>(1);
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();

        let handle = std::thread::Builder::new()
            .name("proc-smart-refresh".into())
            .spawn(move || smart_worker_loop(snap_tx, shutdown_rx))
            .expect("failed to spawn smart-refresh thread");

        Self {
            snapshot_rx: snap_rx,
            shutdown_tx: Some(shutdown_tx),
            thread: Some(handle),
        }
    }

    fn try_recv_latest(&self) -> Option<Vec<crate::smart::SmartData>> {
        let mut latest = None;
        while let Ok(s) = self.snapshot_rx.try_recv() {
            latest = Some(s);
        }
        latest
    }

    /// 首帧同步等(最多 timeout),避免启动 sidebar SMART 徽章空白。
    fn recv_first(&self, timeout: Duration) -> Option<Vec<crate::smart::SmartData>> {
        match self.snapshot_rx.recv_timeout(timeout) {
            Ok(first) => {
                let mut latest = first;
                while let Ok(s) = self.snapshot_rx.try_recv() {
                    latest = s;
                }
                Some(latest)
            }
            Err(_) => None,
        }
    }
}

impl Drop for SmartWorker {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            drop(tx);
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// SMART worker 主循环。30s 一轮;每轮列出磁盘 → 各调 `read_smart` →
/// 收集到一个 Vec 推给主线程。单盘失败不阻塞其它盘。
///
/// 第一帧不等 30s —— 立即采集一次,sidebar 能在启动几秒内拿到徽章。
fn smart_worker_loop(
    snap_tx: std::sync::mpsc::SyncSender<Vec<crate::smart::SmartData>>,
    shutdown_rx: std::sync::mpsc::Receiver<()>,
) {
    use std::sync::mpsc::{RecvTimeoutError, TrySendError};

    const POLL_INTERVAL: Duration = Duration::from_secs(30);

    loop {
        let disks = crate::smart::list_disks();
        let snapshot: Vec<crate::smart::SmartData> = disks
            .iter()
            .filter_map(|dev| match crate::smart::read_smart(dev) {
                Ok(data) => Some(data),
                Err(e) => {
                    tracing::debug!("SMART 读取失败 ({}): {}", dev, e);
                    None
                }
            })
            .collect();

        match snap_tx.try_send(snapshot) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => break,
        }

        match shutdown_rx.recv_timeout(POLL_INTERVAL) {
            Ok(_) => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
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
    pub net_total_rx: u64,
    pub net_total_tx: u64,
    // GPU/温度/磁盘/电源 后台 worker —— 见 [`LightWorker`]。
    // 这些字段是 last-known 缓存,由 worker 每 1s 推送更新;
    // 主线程 `refresh_light()` 只 try_recv,不再做同步系统调用。
    light_worker: LightWorker,
    // SMART 磁盘健康后台 worker —— 阶段 5 B3 新增。30s 一次,
    // 比 LightWorker 慢,因为 SMART 数据不需要秒级刷新。
    smart_worker: SmartWorker,
    gpu_info: Vec<crate::gpu::GpuInfo>,
    temperatures_cache: (Option<f32>, Option<f32>),
    disk_io_speeds: Vec<DiskIoInfo>,
    disks_cache: Vec<DiskInfo>,
    /// 系统盘 (used, total) —— 由 worker 推送
    disk_usage_cache: (u64, u64),
    throttle_info: Option<crate::throttle::ThrottleInfo>,
    /// Per-core CPU 频率（MHz）—— 由 worker 推送
    per_core_freq_cache: Vec<u64>,
    /// Per-core CPU 温度（°C，None=该核不可用）—— 由 worker 推送
    per_core_temp_cache: Vec<Option<f32>>,
    /// SMART 数据缓存 —— 由 SmartWorker 30s 推送一次
    smart_cache: Vec<crate::smart::SmartData>,
    // Replay mode overrides
    replay_cpu: Option<f32>,
    replay_memory: Option<(u64, u64)>,
    replay_net: Option<(u64, u64)>,
    // Incremental process cache: avoids full Vec<ProcessInfo> rebuild every 2s
    process_cache: HashMap<u32, ProcessInfo>,
    // Background heavy-refresh worker
    heavy_worker: HeavyWorker,
    heavy_pending: bool,
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

impl SystemSnapshot {
    /// 初始化 sysinfo
    pub fn new() -> Result<Self> {
        let sys = sysinfo::System::new_all();
        let num_cpus = sys.cpus().len().max(1) as f32;
        let networks = sysinfo::Networks::new_with_refreshed_list();
        let (total_rx, total_tx) = sum_network_totals(&networks);
        let now = Instant::now();

        let light_worker = LightWorker::start();
        // 预热:阻塞等 worker 推第一帧(最多 5s),避免启动后第一秒
        // sidebar 的 GPU/温度/磁盘都显示空白。超时则保持空缓存,
        // worker 后续仍会异步推上来。
        let first_snap = light_worker.recv_first(Duration::from_secs(5));

        let smart_worker = SmartWorker::start();
        // SMART 首帧只等 2s —— read_smart 走子进程 / WMI,通常 1s 内能拿到;
        // 拿不到就先空,sidebar 显示 "-",worker 后续 30s 推上来。
        let first_smart = smart_worker.recv_first(Duration::from_secs(2));

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
            net_total_rx: total_rx,
            net_total_tx: total_tx,
            light_worker,
            smart_worker,
            gpu_info: first_snap
                .as_ref()
                .map(|s| s.gpu_info.clone())
                .unwrap_or_default(),
            temperatures_cache: first_snap
                .as_ref()
                .map(|s| s.temperatures)
                .unwrap_or((None, None)),
            disk_io_speeds: first_snap
                .as_ref()
                .map(|s| s.disk_io_speeds.clone())
                .unwrap_or_default(),
            disks_cache: first_snap
                .as_ref()
                .map(|s| s.all_disks.clone())
                .unwrap_or_default(),
            disk_usage_cache: first_snap.as_ref().map(|s| s.disk_usage).unwrap_or((0, 0)),
            throttle_info: first_snap.as_ref().and_then(|s| s.throttle_info.clone()),
            per_core_freq_cache: first_snap
                .as_ref()
                .map(|s| s.per_core_freq.clone())
                .unwrap_or_default(),
            per_core_temp_cache: first_snap
                .as_ref()
                .map(|s| s.per_core_temp.clone())
                .unwrap_or_default(),
            smart_cache: first_smart.unwrap_or_default(),
            replay_cpu: None,
            replay_memory: None,
            replay_net: None,
            process_cache: HashMap::new(),
            heavy_worker: HeavyWorker::start(),
            heavy_pending: false,
        })
    }

    /// 轻量刷新（每 1 秒）:CPU/内存/网络。GPU/温度/磁盘/电源由后台
    /// [`LightWorker`] 异步推送,这里只 `try_recv` 覆盖缓存。
    pub fn refresh_light(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
        self.networks.refresh(true);

        let (total_rx, total_tx) = sum_network_totals(&self.networks);
        let now = Instant::now();
        let elapsed = now
            .duration_since(self.prev_net_time)
            .as_secs_f64()
            .max(0.001);
        self.net_down_speed =
            ((total_rx.saturating_sub(self.prev_net_received)) as f64 / elapsed) as u64;
        self.net_up_speed =
            ((total_tx.saturating_sub(self.prev_net_transmitted)) as f64 / elapsed) as u64;
        self.prev_net_received = total_rx;
        self.prev_net_transmitted = total_tx;
        self.prev_net_time = now;
        self.net_total_rx = total_rx;
        self.net_total_tx = total_tx;

        // GPU/温度/磁盘/电源 —— 来自后台 worker 的最新一帧(若有)。
        // 无新帧就保留旧缓存,UI 不至于空白。
        if let Some(s) = self.light_worker.try_recv_latest() {
            self.gpu_info = s.gpu_info;
            self.temperatures_cache = s.temperatures;
            self.disk_io_speeds = s.disk_io_speeds;
            self.disks_cache = s.all_disks;
            self.disk_usage_cache = s.disk_usage;
            self.throttle_info = s.throttle_info;
            self.per_core_freq_cache = s.per_core_freq;
            self.per_core_temp_cache = s.per_core_temp;
        }

        // SMART 数据 —— 30s 推一次,这里只是 try_recv 缓存覆盖。
        // 没新帧不报错,UI 显示旧值。
        if let Some(s) = self.smart_worker.try_recv_latest() {
            self.smart_cache = s;
        }
    }

    /// 重量刷新（每 2 秒）：进程列表、EMA 平滑。磁盘/GPU 由 [`LightWorker`] 维护。
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

        let alive_pids: std::collections::HashSet<u32> =
            self.sys.processes().keys().map(|p| p.as_u32()).collect();
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
        if let Some(ref rx) = self.tasklist_rx
            && let Ok(map) = rx.try_recv()
        {
            self.tasklist_memory = map;
            self.tasklist_rx = None;
            self.tasklist_last_refresh = Instant::now();
        }
        if self.tasklist_last_refresh.elapsed() >= Duration::from_secs(30)
            && self.tasklist_rx.is_none()
        {
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

    /// Incremental heavy refresh: update process_cache in-place instead of rebuilding Vec.
    /// New processes get full allocation; existing processes only update numeric fields.
    /// Request a background heavy refresh, or apply pending results.
    /// Returns Ok(true) if new process data was applied, Ok(false) if still pending.
    /// Non-blocking: never blocks the main (UI) thread (except first call when cache is empty).
    pub fn refresh_heavy_incremental(&mut self) -> Result<bool> {
        // Non-blocking tasklist refresh
        if let Some(ref rx) = self.tasklist_rx
            && let Ok(map) = rx.try_recv()
        {
            self.tasklist_memory = map;
            self.tasklist_rx = None;
            self.tasklist_last_refresh = Instant::now();
        }
        if self.tasklist_last_refresh.elapsed() >= Duration::from_secs(30)
            && self.tasklist_rx.is_none()
        {
            let (tx, rx) = std::sync::mpsc::channel();
            self.tasklist_rx = Some(rx);
            self.tasklist_last_refresh = Instant::now();
            std::thread::spawn(move || {
                let map = query_all_process_memories_tasklist();
                let _ = tx.send(map);
            });
        }

        // Try to receive background heavy-refresh results
        if let Some(result) = self.heavy_worker.try_recv() {
            self.heavy_pending = false;
            self.apply_heavy_result(result);
            self.refresh_time = Instant::now();
            return Ok(true);
        }

        // If no pending request, trigger one
        if !self.heavy_pending {
            let _ = self.heavy_worker.cmd_tx.send(self.num_cpus);
            self.heavy_pending = true;

            // First-time population: block briefly so callers expecting synchronous
            // behavior (tests, initial load) get data immediately.
            if self.process_cache.is_empty()
                && let Ok(result) = self
                    .heavy_worker
                    .result_rx
                    .recv_timeout(Duration::from_secs(5))
            {
                self.heavy_pending = false;
                self.apply_heavy_result(result);
                self.refresh_time = Instant::now();
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn apply_heavy_result(&mut self, result: HeavyResult) {
        let alive_pids: std::collections::HashSet<u32> = result.processes.keys().copied().collect();
        self.prev_cpu.retain(|pid, _| alive_pids.contains(pid));
        self.process_cache.retain(|pid, _| alive_pids.contains(pid));

        for (pid_u32, mut proc) in result.processes {
            // Enhance memory with tasklist data (main thread has the latest)
            let tasklist = self.tasklist_memory.get(&pid_u32).copied().unwrap_or(0);
            proc.memory = proc.memory.max(tasklist);

            let smoothed = match self.prev_cpu.get(&pid_u32) {
                Some(prev) => CPU_EMA_ALPHA * proc.cpu_usage + (1.0 - CPU_EMA_ALPHA) * prev,
                None => proc.cpu_usage,
            };
            self.prev_cpu.insert(pid_u32, smoothed);

            if let Some(existing) = self.process_cache.get_mut(&pid_u32) {
                existing.cpu_usage = smoothed;
                existing.memory = proc.memory;
                existing.virtual_memory = proc.virtual_memory;
                existing.disk_usage = proc.disk_usage;
                existing.run_time = proc.run_time;
            } else {
                proc.cpu_usage = smoothed;
                self.process_cache.insert(pid_u32, proc);
            }
        }
    }

    /// Access the incremental process cache as a Vec (for compatibility)
    #[must_use]
    pub fn cached_processes_vec(&self) -> Vec<ProcessInfo> {
        self.process_cache.values().cloned().collect()
    }

    /// Access the incremental process cache as an `Arc<Vec>`, enabling cheap
    /// refcount-only clones for scoring/rendering paths that previously paid
    /// a full per-element deep copy every heavy refresh.
    #[must_use]
    pub fn cached_processes_arc(&self) -> std::sync::Arc<Vec<ProcessInfo>> {
        std::sync::Arc::new(self.process_cache.values().cloned().collect())
    }

    /// Access the incremental process cache directly
    #[must_use]
    pub fn process_cache(&self) -> &HashMap<u32, ProcessInfo> {
        &self.process_cache
    }

    /// 完整刷新（首次启动用）
    pub fn refresh(&mut self) -> Result<()> {
        self.refresh_light();
        self.refresh_heavy()
    }

    /// 获取全局 CPU 使用率 (0-100)
    #[must_use]
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
    #[must_use]
    pub fn memory_usage(&self) -> (u64, u64) {
        if let Some(mem) = self.replay_memory {
            return mem;
        }
        let total = self.sys.total_memory();
        let used = self.sys.used_memory();
        (used, total)
    }

    pub fn set_replay_metrics(
        &mut self,
        cpu: f32,
        mem_used: u64,
        mem_total: u64,
        net_down: u64,
        net_up: u64,
    ) {
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
    #[must_use]
    pub fn swap_usage(&self) -> (u64, u64) {
        (self.sys.used_swap(), self.sys.total_swap())
    }

    /// 获取系统盘（最大非可移动磁盘）使用情况 (已用 bytes, 总量 bytes)
    #[must_use]
    pub fn disk_usage(&self) -> (u64, u64) {
        self.disk_usage_cache
    }

    /// 获取系统运行时间（秒）
    #[must_use]
    pub fn uptime_secs() -> u64 {
        sysinfo::System::uptime()
    }

    /// 获取 CPU 和 GPU 温度 (cpu_temp, gpu_temp)，None 表示未检测到
    #[must_use]
    pub fn temperatures(&self) -> (Option<f32>, Option<f32>) {
        self.temperatures_cache
    }

    #[must_use]
    pub fn process_name_map(&self) -> std::collections::HashMap<u32, String> {
        self.sys
            .processes()
            .iter()
            .map(|(pid, proc)| (pid.as_u32(), proc.name().to_string_lossy().to_string()))
            .collect()
    }

    #[must_use]
    pub fn gpu_info(&self) -> &[crate::gpu::GpuInfo] {
        &self.gpu_info
    }

    #[must_use]
    pub fn throttle_info(&self) -> Option<&crate::throttle::ThrottleInfo> {
        self.throttle_info.as_ref()
    }

    /// Per-core CPU 频率（MHz）。空 Vec 表示该平台 / 当前会话不可用
    /// （如 Linux 无 cpufreq 驱动的虚拟机、或 sysinfo 读注册表失败的 Windows）。
    #[must_use]
    pub fn per_core_freq(&self) -> &[u64] {
        &self.per_core_freq_cache
    }

    /// Per-core CPU 温度（°C，None=该核不可用）。与 [`per_core_freq`] 同长度。
    #[must_use]
    pub fn per_core_temp(&self) -> &[Option<f32>] {
        &self.per_core_temp_cache
    }

    /// 获取上次刷新时间
    #[must_use]
    pub fn last_refresh(&self) -> Instant {
        self.refresh_time
    }

    /// 获取进程数量
    #[must_use]
    pub fn process_count(&self) -> usize {
        self.sys.processes().len()
    }

    /// Global aggregate disk I/O speed (sum of per-disk speeds).
    /// For per-disk breakdown, use `per_disk_io_speed()`.
    #[must_use]
    pub fn disk_io_speed(&self) -> (u64, u64) {
        let mut read: u64 = 0;
        let mut write: u64 = 0;
        for d in &self.disk_io_speeds {
            read += d.read_speed;
            write += d.write_speed;
        }
        (read, write)
    }

    /// Per-disk I/O speed.
    #[must_use]
    pub fn per_disk_io_speed(&self) -> Vec<DiskIoInfo> {
        self.disk_io_speeds.clone()
    }

    /// 获取所有磁盘信息列表
    #[must_use]
    pub fn all_disks(&self) -> Vec<DiskInfo> {
        self.disks_cache.clone()
    }

    /// 获取后台 SmartWorker 缓存的磁盘 SMART 数据(30s 一帧)。
    ///
    /// 返回 Vec 是因为多盘系统会有多条 SmartData。空 Vec 表示:
    /// - 系统无支持 SMART 的磁盘(USB / 虚拟磁盘);或
    /// - smartctl 未装 + WMI 无预测数据;或
    /// - 首帧还没回来(启动 2s 内)。
    #[must_use]
    pub fn smart_data(&self) -> Vec<crate::smart::SmartData> {
        self.smart_cache.clone()
    }

    /// 获取活跃网卡信息（过滤 APIPA 169.254.x.x 和回环 127.0.0.1）
    #[must_use]
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
    #[must_use]
    pub fn tcp_stats() -> TcpStats {
        query_tcp_stats()
    }
}

/// 检测当前进程是否以管理员权限运行
#[cfg(target_os = "windows")]
#[must_use]
pub fn is_elevated() -> bool {
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
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

/// 解析 Linux `/sys/devices/system/cpu/cpuN/cpufreq/scaling_cur_freq` 内容。
///
/// sysfs 写入的格式是 kHz 的纯数字 + 一个换行符（例 `"3400000\n"`）。
/// 我们要 MHz：`/1000`。返回 None 表示文件内容非数字或为空，
/// 调用方按"该核频率不可用"处理。
///
/// 单独抽出来是因为它跨平台可测——Windows / macOS 也能跑这个纯函数测试。
#[must_use]
pub fn parse_scaling_cur_freq(content: &str) -> Option<u64> {
    let trimmed = content.trim_ascii_end();
    let khz: u64 = trimmed.parse().ok()?;
    Some(khz / 1000)
}

/// 读 per-core CPU 频率（MHz），与 `sysinfo::System::cpus()` 顺序对齐。
///
/// Windows：sysinfo 走 `RegQueryValueEx` 读 `~MHz` 注册表项，per-processor。
fn collect_per_core_freq(sys: &sysinfo::System) -> Vec<u64> {
    let cpus = sys.cpus();
    if cpus.is_empty() {
        return Vec::new();
    }

    cpus.iter().map(|c| c.frequency()).collect()
}

/// 读 per-core CPU 温度。sysinfo 的 Components API 通常只暴露一个全局 CPU
/// 温度（ACPI ThermalZone / lm-sensors 的 "Core 0" 等），无法稳定分核。
///
/// 当前实现：
/// - 找到含 "cpu" / "core" 字样的 component，把温度填到第 0 核；
/// - 其余核留 `None`。
///
/// 这保证「至少有一个值」的视觉一致性，避免 sidebar 渲染 N 个 N/A。阶段 5
/// 可以考虑接 Win32 MSAcpi_ThermalZoneTemperature 或 Linux hwmon per-core 路径。
fn collect_per_core_temp(components: &sysinfo::Components, num_cores: usize) -> Vec<Option<f32>> {
    if num_cores == 0 {
        return Vec::new();
    }
    let mut out: Vec<Option<f32>> = (0..num_cores).map(|_| None).collect();
    for component in components.iter() {
        let label = component.label().to_lowercase();
        if (label.contains("cpu") || label.contains("core"))
            && let Some(t) = component.temperature()
            && t >= 0.0
        {
            out[0] = Some(t);
            break;
        }
    }
    out
}

#[cfg(test)]
mod collect_tests {
    use super::parse_scaling_cur_freq;

    #[test]
    fn parse_scaling_cur_freq_khz_to_mhz() {
        // Linux sysfs: 3400000 kHz → 3400 MHz
        assert_eq!(parse_scaling_cur_freq("3400000\n"), Some(3400));
        assert_eq!(parse_scaling_cur_freq("2500000"), Some(2500));
        // 超低频（idle 降频）也按整数除法处理。
        assert_eq!(parse_scaling_cur_freq("800000\n"), Some(800));
    }

    #[test]
    fn parse_scaling_cur_freq_rejects_garbage() {
        assert_eq!(parse_scaling_cur_freq(""), None);
        assert_eq!(parse_scaling_cur_freq("not a number\n"), None);
        assert_eq!(parse_scaling_cur_freq("3.4 GHz\n"), None);
    }

    #[test]
    fn parse_scaling_cur_freq_handles_trailing_whitespace() {
        // 内核有时带 \r\n 或多个空格
        assert_eq!(parse_scaling_cur_freq("2400000  \n"), Some(2400));
        assert_eq!(parse_scaling_cur_freq("  1600000\n"), None); // 前导空格 parse 失败
    }
}
