use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

use crate::error::{ProcError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp => write!(f, "TCP"),
            Self::Udp => write!(f, "UDP"),
        }
    }
}

/// 视图模式：按端口 / 按进程 / 按远程
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkViewMode {
    #[default]
    Port,
    Process,
    Remote,
}

impl NetworkViewMode {
    #[must_use]
    pub fn toggle(&self) -> Self {
        match self {
            Self::Port => Self::Process,
            Self::Process => Self::Remote,
            Self::Remote => Self::Port,
        }
    }
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Port => "按端口",
            Self::Process => "按进程",
            Self::Remote => "按远程",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortSortField {
    LocalPort,
    RemoteAddr,
    State,
    Process,
}

impl PortSortField {
    #[must_use]
    pub fn next(&self) -> Self {
        match self {
            Self::LocalPort => Self::RemoteAddr,
            Self::RemoteAddr => Self::State,
            Self::State => Self::Process,
            Self::Process => Self::LocalPort,
        }
    }
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::LocalPort => "端口",
            Self::RemoteAddr => "远程",
            Self::State => "状态",
            Self::Process => "进程",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortStateFilter {
    #[default]
    All,
    Established,
    Listening,
    Udp,
}

impl PortStateFilter {
    #[must_use]
    pub fn next(&self) -> Self {
        match self {
            Self::All => Self::Established,
            Self::Established => Self::Listening,
            Self::Listening => Self::Udp,
            Self::Udp => Self::All,
        }
    }
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::All => "全部",
            Self::Established => "ESTABLISHED",
            Self::Listening => "LISTEN",
            Self::Udp => "UDP",
        }
    }
}

#[must_use]
pub fn matches_filter(entry: &PortEntry, filter: &PortStateFilter) -> bool {
    match filter {
        PortStateFilter::All => true,
        PortStateFilter::Established => entry
            .state
            .as_deref()
            .is_some_and(|s| s.contains("Established")),
        PortStateFilter::Listening => {
            entry.protocol == Protocol::Tcp
                && entry.state.as_deref().is_some_and(|s| s.contains("Listen"))
        }
        PortStateFilter::Udp => entry.protocol == Protocol::Udp,
    }
}

#[must_use]
pub fn state_group(state: &Option<String>, protocol: &Protocol) -> u8 {
    if *protocol == Protocol::Udp {
        return 3;
    }
    match state.as_deref() {
        Some(s) if s.contains("Established") => 0,
        Some(s) if s.contains("Listen") => 1,
        _ => 2,
    }
}

pub fn sort_entries(entries: &mut [PortEntry], sort: PortSortField) {
    entries.sort_by(|a, b| {
        let a_group = state_group(&a.state, &a.protocol);
        let b_group = state_group(&b.state, &b.protocol);
        match a_group.cmp(&b_group) {
            std::cmp::Ordering::Equal => match sort {
                PortSortField::LocalPort => a.local_port.cmp(&b.local_port),
                PortSortField::RemoteAddr => {
                    let a_remote = a.remote_addr.map(|ip| ip.to_string()).unwrap_or_default();
                    let b_remote = b.remote_addr.map(|ip| ip.to_string()).unwrap_or_default();
                    a_remote.cmp(&b_remote)
                }
                PortSortField::State => a.state.cmp(&b.state),
                PortSortField::Process => a.process_name.cmp(&b.process_name),
            }
            .then_with(|| a.pid.cmp(&b.pid))
            .then_with(|| a.local_port.cmp(&b.local_port)),
            other => other,
        }
    });
}

#[must_use]
pub fn is_ipv6_duplicate(entry: &PortEntry, seen: &[(u16, u32, String)]) -> bool {
    if !entry.local_addr.to_string().contains(':') {
        return false;
    }
    seen.iter().any(|(port, pid, state)| {
        *port == entry.local_port
            && *pid == entry.pid
            && *state == entry.state.clone().unwrap_or_default()
    })
}

#[derive(Debug, Clone)]
pub struct PortEntry {
    pub protocol: Protocol,
    pub local_addr: IpAddr,
    pub local_port: u16,
    pub remote_addr: Option<IpAddr>,
    pub remote_port: Option<u16>,
    pub state: Option<String>,
    pub pid: u32,
    pub process_name: String,
}

pub fn scan_ports() -> Result<Vec<PortEntry>> {
    let started = std::time::Instant::now();
    let af_flags = netstat2::AddressFamilyFlags::IPV4 | netstat2::AddressFamilyFlags::IPV6;
    let proto_flags = netstat2::ProtocolFlags::TCP | netstat2::ProtocolFlags::UDP;

    let sockets_info = netstat2::get_sockets_info(af_flags, proto_flags)
        .map_err(|e| ProcError::port_scan_with("端口扫描失败", e))?;

    let name_map = crate::collect::sysinfo_with(|sys| {
        let mut map: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        for (pid, proc) in sys.processes() {
            map.insert(pid.as_u32(), proc.name().to_string_lossy().to_string());
        }
        map
    });
    let result = scan_ports_with_names(&sockets_info, &name_map);
    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        entries = result.as_ref().map(|v| v.len()).unwrap_or(0),
        "scan_ports 完成",
    );
    result
}

pub fn scan_ports_with_names(
    sockets_info: &[netstat2::SocketInfo],
    name_map: &std::collections::HashMap<u32, String>,
) -> Result<Vec<PortEntry>> {
    let mut entries = Vec::new();

    for si in sockets_info {
        let pid = si.associated_pids.first().copied().unwrap_or(0);
        let process_name = if pid > 0 {
            name_map
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| "-".to_string())
        } else {
            "-".to_string()
        };

        match &si.protocol_socket_info {
            netstat2::ProtocolSocketInfo::Tcp(tcp) => {
                entries.push(PortEntry {
                    protocol: Protocol::Tcp,
                    local_addr: tcp.local_addr,
                    local_port: tcp.local_port,
                    remote_addr: Some(tcp.remote_addr),
                    remote_port: Some(tcp.remote_port),
                    state: Some(format!("{}", tcp.state)),
                    pid,
                    process_name,
                });
            }
            netstat2::ProtocolSocketInfo::Udp(udp) => {
                entries.push(PortEntry {
                    protocol: Protocol::Udp,
                    local_addr: udp.local_addr,
                    local_port: udp.local_port,
                    remote_addr: None,
                    remote_port: None,
                    state: None,
                    pid,
                    process_name,
                });
            }
        }
    }

    entries.sort_by(|a, b| {
        a.protocol
            .to_string()
            .cmp(&b.protocol.to_string())
            .then(a.local_port.cmp(&b.local_port))
    });

    Ok(entries)
}

pub fn find_pid_by_port(port: u16) -> Result<Vec<PortEntry>> {
    let all = scan_ports()?;
    Ok(all.into_iter().filter(|e| e.local_port == port).collect())
}

pub fn find_ports_by_pid(pid: u32) -> Result<Vec<PortEntry>> {
    let all = scan_ports()?;
    Ok(all.into_iter().filter(|e| e.pid == pid).collect())
}

#[derive(Debug, Clone, Default)]
pub struct ProcessNetSummary {
    pub tcp_connections: usize,
    pub udp_connections: usize,
    pub established: usize,
    pub close_wait: usize,
    pub time_wait: usize,
    pub listening: usize,
    pub unique_remote_addrs: HashSet<std::net::IpAddr>,
}

impl ProcessNetSummary {
    #[must_use]
    pub fn from_pid(pid: u32, entries: &[PortEntry]) -> Self {
        let mut s = Self::default();
        for e in entries {
            if e.pid != pid {
                continue;
            }
            match e.protocol {
                Protocol::Tcp => {
                    s.tcp_connections += 1;
                    match e.state.as_deref() {
                        Some(st) if st.contains("Established") => s.established += 1,
                        Some(st) if st.contains("CloseWait") || st.contains("CLOSE_WAIT") => {
                            s.close_wait += 1
                        }
                        Some(st) if st.contains("Listen") => s.listening += 1,
                        Some(st) if st.contains("TimeWait") || st.contains("TIME_WAIT") => {
                            s.time_wait += 1
                        }
                        _ => {}
                    }
                    if let Some(addr) = e.remote_addr
                        && !addr.is_unspecified()
                        && !addr.is_loopback()
                    {
                        s.unique_remote_addrs.insert(addr);
                    }
                }
                Protocol::Udp => s.udp_connections += 1,
            }
        }
        s
    }
}

#[derive(Debug, Clone)]
pub struct ProcessNetGroup {
    pub pid: u32,
    pub process_name: String,
    pub connections: Vec<PortEntry>,
    pub tcp_count: usize,
    pub udp_count: usize,
    pub established: usize,
    pub listening: usize,
    pub time_wait: usize,
    pub close_wait: usize,
    pub unique_remote_addrs: HashSet<IpAddr>,
    // Populated only in admin (enhanced) mode:
    pub down_speed: u64,
    pub up_speed: u64,
    pub total_down: u64,
    pub total_up: u64,
}

impl ProcessNetGroup {
    #[must_use]
    pub fn from_entries(entries: &[PortEntry]) -> Vec<Self> {
        let mut map: HashMap<(u32, String), Vec<&PortEntry>> = HashMap::new();
        for e in entries {
            let key = (e.pid, e.process_name.clone());
            map.entry(key).or_default().push(e);
        }

        let mut groups: Vec<Self> = map
            .into_iter()
            .map(|((pid, name), refs)| {
                let connections: Vec<PortEntry> = refs.iter().map(|r| (*r).clone()).collect();
                let mut g = Self {
                    pid,
                    process_name: name,
                    tcp_count: 0,
                    udp_count: 0,
                    established: 0,
                    listening: 0,
                    time_wait: 0,
                    close_wait: 0,
                    unique_remote_addrs: HashSet::new(),
                    connections,
                    down_speed: 0,
                    up_speed: 0,
                    total_down: 0,
                    total_up: 0,
                };
                for e in &g.connections {
                    match e.protocol {
                        Protocol::Tcp => {
                            g.tcp_count += 1;
                            match e.state.as_deref() {
                                Some(st) if st.contains("Established") => g.established += 1,
                                Some(st)
                                    if st.contains("CloseWait") || st.contains("CLOSE_WAIT") =>
                                {
                                    g.close_wait += 1
                                }
                                Some(st) if st.contains("Listen") => g.listening += 1,
                                Some(st) if st.contains("TimeWait") || st.contains("TIME_WAIT") => {
                                    g.time_wait += 1
                                }
                                _ => {}
                            }
                            if let Some(addr) = e.remote_addr
                                && !addr.is_unspecified()
                                && !addr.is_loopback()
                            {
                                g.unique_remote_addrs.insert(addr);
                            }
                        }
                        Protocol::Udp => g.udp_count += 1,
                    }
                }
                g
            })
            .collect();

        groups.sort_by_key(|a| a.process_name.to_lowercase());
        groups
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSortField {
    ConnectionCount,
    ProcessName,
    Pid,
}

impl ProcessSortField {
    #[must_use]
    pub fn next(&self) -> Self {
        match self {
            Self::ConnectionCount => Self::ProcessName,
            Self::ProcessName => Self::Pid,
            Self::Pid => Self::ConnectionCount,
        }
    }
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::ConnectionCount => "连接数",
            Self::ProcessName => "进程名",
            Self::Pid => "PID",
        }
    }
}

pub fn sort_process_groups(groups: &mut [ProcessNetGroup], sort: ProcessSortField) {
    groups.sort_by(|a, b| match sort {
        ProcessSortField::ConnectionCount => {
            let a_total = a.tcp_count + a.udp_count;
            let b_total = b.tcp_count + b.udp_count;
            b_total.cmp(&a_total).then(a.pid.cmp(&b.pid))
        }
        ProcessSortField::ProcessName => a
            .process_name
            .to_lowercase()
            .cmp(&b.process_name.to_lowercase())
            .then(a.pid.cmp(&b.pid)),
        ProcessSortField::Pid => a.pid.cmp(&b.pid),
    });
}

#[must_use]
pub fn service_name(port: u16, protocol: Protocol) -> Option<&'static str> {
    match protocol {
        Protocol::Tcp => match port {
            20 => Some("ftp-data"),
            21 => Some("ftp"),
            22 => Some("ssh"),
            23 => Some("telnet"),
            25 => Some("smtp"),
            53 => Some("dns"),
            80 => Some("http"),
            110 => Some("pop3"),
            143 => Some("imap"),
            443 => Some("https"),
            465 => Some("smtps"),
            587 => Some("smtp(submission)"),
            993 => Some("imaps"),
            995 => Some("pop3s"),
            1433 => Some("mssql"),
            1521 => Some("oracle"),
            3306 => Some("mysql"),
            3389 => Some("rdp"),
            5432 => Some("postgresql"),
            6379 => Some("redis"),
            8080 => Some("http-alt"),
            8443 => Some("https-alt"),
            9200 => Some("elasticsearch"),
            27017 => Some("mongodb"),
            _ => None,
        },
        Protocol::Udp => match port {
            53 => Some("dns"),
            67 | 68 => Some("dhcp"),
            123 => Some("ntp"),
            161 => Some("snmp"),
            5353 => Some("mdns"),
            1900 => Some("ssdp"),
            _ => None,
        },
    }
}

#[must_use]
pub fn format_port_service(port: u16, protocol: Protocol) -> String {
    match service_name(port, protocol) {
        Some(name) => format!("{} ({})", name, port),
        None => port.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpClass {
    Loopback,
    Private,
    LinkLocal,
    Public,
}

#[must_use]
pub fn classify_ip(addr: &IpAddr) -> IpClass {
    if addr.is_loopback() {
        IpClass::Loopback
    } else if is_private(addr) {
        IpClass::Private
    } else if is_link_local(addr) {
        IpClass::LinkLocal
    } else {
        IpClass::Public
    }
}

fn is_private(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => v6.is_unique_local(),
    }
}

fn is_link_local(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

#[must_use]
pub fn ip_class_label(class: IpClass) -> &'static str {
    match class {
        IpClass::Loopback => "本机",
        IpClass::Private => "内网",
        IpClass::LinkLocal => "链路本地",
        IpClass::Public => "公网",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudProvider {
    Aws,
    Gcp,
    Azure,
    Cloudflare,
    Fastly,
    Akamai,
}

impl std::fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aws => write!(f, "AWS"),
            Self::Gcp => write!(f, "GCP"),
            Self::Azure => write!(f, "Azure"),
            Self::Cloudflare => write!(f, "CF"),
            Self::Fastly => write!(f, "Fastly"),
            Self::Akamai => write!(f, "Akamai"),
        }
    }
}

#[must_use]
pub fn detect_cloud_provider(addr: &IpAddr) -> Option<CloudProvider> {
    let v4 = match addr {
        IpAddr::V4(v4) => *v4,
        IpAddr::V6(_) => return None,
    };
    let octets = v4.octets();
    let first = octets[0];

    macro_rules! in_range {
        ($lo:expr, $hi:expr) => {
            ($lo..=$hi).contains(&first)
        };
    }

    // AWS
    if in_range!(3, 3)
        || in_range!(13, 13)
        || in_range!(15, 15)
        || in_range!(18, 18)
        || in_range!(34, 35)
        || in_range!(52, 52)
        || in_range!(54, 55)
    {
        return Some(CloudProvider::Aws);
    }

    // Azure
    if in_range!(20, 20) || in_range!(40, 40) {
        return Some(CloudProvider::Azure);
    }
    if first == 52 && octets[1] >= 96 && octets[1] <= 111 {
        return Some(CloudProvider::Azure);
    }
    if first == 4 && octets[1] >= 128 && octets[1] <= 191 {
        return Some(CloudProvider::Azure);
    }

    // GCP
    if first == 8 && octets[1] == 8 && octets[2] == 8 {
        return Some(CloudProvider::Gcp);
    }
    if first == 35 && octets[1] >= 192 {
        return Some(CloudProvider::Gcp);
    }
    if first == 34 && (octets[1] >= 64 && octets[1] <= 127) {
        return Some(CloudProvider::Gcp);
    }
    if in_range!(104, 104) && octets[1] >= 154 && octets[1] <= 155 {
        return Some(CloudProvider::Gcp);
    }

    // Cloudflare
    if first == 103 && octets[1] == 21 && octets[2] == 244 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 131 && octets[1] == 0 && octets[2] == 72 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 141 && octets[1] == 101 && octets[2] >= 64 && octets[2] <= 127 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 108 && octets[1] == 162 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 162 && octets[1] == 159 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 172 && octets[1] == 64 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 188 && octets[1] == 114 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 190 && octets[1] == 93 {
        return Some(CloudProvider::Cloudflare);
    }
    if first == 197 && octets[1] == 234 {
        return Some(CloudProvider::Cloudflare);
    }

    // Fastly
    if first == 151 && octets[1] == 101 {
        return Some(CloudProvider::Fastly);
    }
    if first == 199 && octets[1] == 232 {
        return Some(CloudProvider::Fastly);
    }

    // Akamai
    if first == 23 && octets[1] <= 79 {
        return Some(CloudProvider::Akamai);
    }
    if first == 72 && (octets[1] >= 246 && octets[1] <= 247) {
        return Some(CloudProvider::Akamai);
    }
    if first == 95 && (octets[1] >= 100 && octets[1] <= 103) {
        return Some(CloudProvider::Akamai);
    }
    if first == 184 && octets[1] == 24 {
        return Some(CloudProvider::Akamai);
    }

    None
}

#[derive(Debug, Clone)]
pub struct RemoteGroup {
    pub remote_addr: IpAddr,
    pub ip_class: IpClass,
    pub cloud_provider: Option<CloudProvider>,
    pub connections: Vec<PortEntry>,
    pub process_names: HashSet<String>,
    pub established: usize,
    pub listening: usize,
    pub time_wait: usize,
    pub close_wait: usize,
    pub protocols: HashSet<Protocol>,
}

impl RemoteGroup {
    #[must_use]
    pub fn from_entries(entries: &[PortEntry]) -> Vec<Self> {
        let mut map: HashMap<IpAddr, Vec<&PortEntry>> = HashMap::new();

        let local_addr_v4: IpAddr = "0.0.0.0".parse().unwrap();
        let local_addr_v6: IpAddr = "::".parse().unwrap();

        for e in entries {
            let addr = match e.remote_addr {
                Some(a) if !a.is_unspecified() => a,
                _ => {
                    let key = if e.local_addr.is_unspecified() {
                        if e.local_addr == local_addr_v6 {
                            local_addr_v4
                        } else {
                            e.local_addr
                        }
                    } else {
                        e.local_addr
                    };
                    map.entry(key).or_default().push(e);
                    continue;
                }
            };
            map.entry(addr).or_default().push(e);
        }

        let mut groups: Vec<Self> = map
            .into_iter()
            .map(|(addr, refs)| {
                let connections: Vec<PortEntry> = refs.iter().map(|r| (*r).clone()).collect();
                let mut g = Self {
                    remote_addr: addr,
                    ip_class: classify_ip(&addr),
                    cloud_provider: detect_cloud_provider(&addr),
                    connections,
                    process_names: HashSet::new(),
                    established: 0,
                    listening: 0,
                    time_wait: 0,
                    close_wait: 0,
                    protocols: HashSet::new(),
                };
                for e in &g.connections {
                    g.process_names.insert(e.process_name.clone());
                    g.protocols.insert(e.protocol);
                    match e.protocol {
                        Protocol::Tcp => match e.state.as_deref() {
                            Some(st) if st.contains("Established") => g.established += 1,
                            Some(st) if st.contains("CloseWait") || st.contains("CLOSE_WAIT") => {
                                g.close_wait += 1
                            }
                            Some(st) if st.contains("Listen") => g.listening += 1,
                            Some(st) if st.contains("TimeWait") || st.contains("TIME_WAIT") => {
                                g.time_wait += 1
                            }
                            _ => {}
                        },
                        Protocol::Udp => {}
                    }
                }
                g
            })
            .collect();

        groups.sort_by(|a, b| {
            let a_total = a.established + a.listening + a.time_wait + a.close_wait;
            let b_total = b.established + b.listening + b.time_wait + b.close_wait;
            b_total.cmp(&a_total)
        });

        groups
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSortField {
    ConnectionCount,
    IpAddress,
    ProcessCount,
}

impl RemoteSortField {
    #[must_use]
    pub fn next(&self) -> Self {
        match self {
            Self::ConnectionCount => Self::IpAddress,
            Self::IpAddress => Self::ProcessCount,
            Self::ProcessCount => Self::ConnectionCount,
        }
    }
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::ConnectionCount => "连接数",
            Self::IpAddress => "IP地址",
            Self::ProcessCount => "进程数",
        }
    }
}

pub fn sort_remote_groups(groups: &mut [RemoteGroup], sort: RemoteSortField) {
    groups.sort_by(|a, b| match sort {
        RemoteSortField::ConnectionCount => {
            let a_total = a.connections.len();
            let b_total = b.connections.len();
            b_total.cmp(&a_total)
        }
        RemoteSortField::IpAddress => a.remote_addr.cmp(&b.remote_addr),
        RemoteSortField::ProcessCount => b.process_names.len().cmp(&a.process_names.len()),
    });
}

#[derive(Debug, Clone, Default)]
pub struct ConnectionDiff {
    pub new_count: usize,
    pub closed_count: usize,
    pub active_count: usize,
    pub close_wait_count: usize,
    pub time_wait_count: usize,
}

fn conn_key(e: &PortEntry) -> (Protocol, IpAddr, u16, Option<IpAddr>, Option<u16>, u32) {
    (
        e.protocol,
        e.local_addr,
        e.local_port,
        e.remote_addr,
        e.remote_port,
        e.pid,
    )
}

pub fn diff_connections(prev: &[PortEntry], current: &[PortEntry]) -> ConnectionDiff {
    let prev_keys: HashSet<_> = prev.iter().map(conn_key).collect();
    let cur_keys: HashSet<_> = current.iter().map(conn_key).collect();

    let new_count = cur_keys.difference(&prev_keys).count();
    let closed_count = prev_keys.difference(&cur_keys).count();
    let active_count = current.len();
    let close_wait_count = current
        .iter()
        .filter(|e| {
            e.state
                .as_deref()
                .is_some_and(|s| s.contains("CloseWait") || s.contains("CLOSE_WAIT"))
        })
        .count();
    let time_wait_count = current
        .iter()
        .filter(|e| {
            e.state
                .as_deref()
                .is_some_and(|s| s.contains("TimeWait") || s.contains("TIME_WAIT"))
        })
        .count();

    ConnectionDiff {
        new_count,
        closed_count,
        active_count,
        close_wait_count,
        time_wait_count,
    }
}
