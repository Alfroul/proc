//! Windows per-process 网络字节速率采集：IP Helper API 路线。
//!
//! # 数据流
//!
//! 1. `netstat2::get_sockets_info` 枚举所有 TCP 连接 + 拿到 PID（避免再写一遍
//!    `GetExtendedTcpTable` 的 unsafe 脚手架，复用已有依赖）
//! 2. `GetTcpTable2` + `GetPerTcpConnectionEStats` 拿每条 IPv4 TCP 连接的累计
//!    `DataBytesIn` / `DataBytesOut`（与 [`crate::estats`] 完全同款调用）
//! 3. 按 (local_addr, local_port, remote_addr, remote_port) join，把字节累加到
//!    PID 维度
//! 4. 与上次累计差值 / elapsed = bytes/sec
//!
//! # 已知限制
//!
//! - 仅 IPv4（与 [`crate::estats`] 保持一致；IPv6 走 `GetPerTcp6ConnectionEStats`
//!   是后续工作）
//! - 仅 TCP（UDP 无连接字节计数）
//! - 仅 estats enabled 的连接能拿到字节（`SetPerTcpConnectionEStats` 每次采样
//!   都调用以图覆盖新建连接）
//! - 非管理员：`SetPerTcpConnectionEStats` 通常仍能成功（不要求管理员），
//!   `GetPerTcpConnectionEStats` 失败的连接按 0 字节计入
//!
//! 关于 ETW 路线的选择见 `docs/adr/0005-netflow-windows-iphelper-not-etw.md`。

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

use crate::error::Result;
use crate::net_flow::{NetFlowCollector, ProcessNetRate};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ConnKey {
    local_addr: Ipv4Addr,
    local_port: u16,
    remote_addr: Ipv4Addr,
    remote_port: u16,
}

#[cfg(target_os = "windows")]
mod win32 {
    use super::*;
    use windows::Win32::Foundation::BOOLEAN;
    use windows::Win32::NetworkManagement::IpHelper::{
        GetPerTcpConnectionEStats, GetTcpTable2, MIB_TCPROW2, MIB_TCPTABLE2,
        SetPerTcpConnectionEStats, TCP_ESTATS_DATA_ROD_v0, TCP_ESTATS_DATA_RW_v0, TCP_ESTATS_TYPE,
    };

    fn u32_to_ipv4(v: u32) -> Ipv4Addr {
        Ipv4Addr::from(v.to_ne_bytes())
    }

    fn ntohs_port(v: u32) -> u16 {
        ((v & 0xFFFF) as u16).to_be()
    }

    fn make_tcprow_lh(
        row2: &MIB_TCPROW2,
    ) -> windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW_LH {
        windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW_LH {
            Anonymous: windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW_LH_0 {
                dwState: row2.dwState,
            },
            dwLocalAddr: row2.dwLocalAddr,
            dwLocalPort: row2.dwLocalPort,
            dwRemoteAddr: row2.dwRemoteAddr,
            dwRemotePort: row2.dwRemotePort,
        }
    }

    /// 一次性把所有 TCP 连接的 (ConnKey → bytes_in/out) 拿出来。
    /// 同时为每条连接打开 EStats 收集（首次调用拿不到字节；下次就能拿到）。
    fn sample_connection_bytes() -> HashMap<ConnKey, (u64, u64)> {
        let mut size = 0u32;
        unsafe {
            let _ = GetTcpTable2(None, &mut size, false);
        }
        if size == 0 {
            return HashMap::new();
        }

        let mut buf: Vec<u8> = vec![0u8; size as usize];
        let table_ptr = buf.as_mut_ptr() as *mut MIB_TCPTABLE2;
        let err = unsafe { GetTcpTable2(Some(table_ptr), &mut size, false) };
        if err != 0 {
            tracing::debug!("GetTcpTable2 失败: {err}");
            return HashMap::new();
        }

        let table = unsafe { &*table_ptr };
        let n = table.dwNumEntries as usize;
        let row_size = std::mem::size_of::<MIB_TCPROW2>();
        let header = std::mem::size_of::<u32>();
        if buf.len() < header + n * row_size {
            return HashMap::new();
        }
        let rows: &[MIB_TCPROW2] =
            unsafe { std::slice::from_raw_parts(buf.as_ptr().add(header) as *const _, n) };

        // 先 enable EStats collection，让没开过的连接下一轮采样能拿到字节。
        enable_estats_for_rows(rows);

        let mut out = HashMap::with_capacity(n);
        for row2 in rows {
            let local_addr = u32_to_ipv4(row2.dwLocalAddr);
            let local_port = ntohs_port(row2.dwLocalPort);
            let remote_addr = u32_to_ipv4(row2.dwRemoteAddr);
            let remote_port = ntohs_port(row2.dwRemotePort);
            if remote_addr.is_unspecified() && remote_port == 0 {
                // Listen-only 行没有对端，跳过
                continue;
            }
            let key = ConnKey {
                local_addr,
                local_port,
                remote_addr,
                remote_port,
            };

            let row_lh = make_tcprow_lh(row2);
            let mut rod: TCP_ESTATS_DATA_ROD_v0 = unsafe { std::mem::zeroed() };
            let rod_buf = unsafe {
                std::slice::from_raw_parts_mut(
                    &mut rod as *mut _ as *mut u8,
                    std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>(),
                )
            };
            let rc = unsafe {
                GetPerTcpConnectionEStats(
                    &row_lh,
                    TCP_ESTATS_TYPE(0),
                    None,
                    0,
                    None,
                    0,
                    Some(rod_buf),
                    0,
                )
            };
            if rc != 0 {
                // 连接未开 EStats / 已关闭 / 其它失败 → 不计
                continue;
            }
            out.insert(key, (rod.DataBytesIn, rod.DataBytesOut));
        }
        out
    }

    fn enable_estats_for_rows(rows: &[MIB_TCPROW2]) {
        let mut rw: TCP_ESTATS_DATA_RW_v0 = unsafe { std::mem::zeroed() };
        rw.EnableCollection = BOOLEAN(1);
        let rw_bytes = unsafe {
            std::slice::from_raw_parts(
                &rw as *const _ as *const u8,
                std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>(),
            )
        };
        for row2 in rows {
            let row_lh = make_tcprow_lh(row2);
            let _ =
                unsafe { SetPerTcpConnectionEStats(&row_lh, TCP_ESTATS_TYPE(0), rw_bytes, 0, 0) };
        }
    }

    pub(super) fn sample_all() -> HashMap<ConnKey, (u64, u64)> {
        sample_connection_bytes()
    }
}

#[cfg(not(target_os = "windows"))]
mod win32 {
    use super::*;
    pub(super) fn sample_all() -> HashMap<ConnKey, (u64, u64)> {
        HashMap::new()
    }
}

/// 跨平台 collector 外壳：Windows 上调 win32 模块，其它平台 new() 直接失败。
pub struct IphelperCollector {
    last_per_pid: HashMap<u32, (u64, u64)>,
    last_time: Instant,
}

impl IphelperCollector {
    #[cfg(target_os = "windows")]
    pub fn new() -> Result<Self> {
        let initial = sample_per_pid_cumulative();
        Ok(Self {
            last_per_pid: initial,
            last_time: Instant::now(),
        })
    }

    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Result<Self> {
        Err(crate::error::ProcError::monitor(
            "IphelperCollector 仅 Windows 支持，当前平台不可用",
        ))
    }
}

#[cfg(target_os = "windows")]
impl NetFlowCollector for IphelperCollector {
    fn per_process_rates(&mut self) -> Vec<ProcessNetRate> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_secs_f64().max(0.001);
        let current = sample_per_pid_cumulative();

        let mut rates: Vec<ProcessNetRate> = Vec::with_capacity(current.len());
        for (pid, (cur_in, cur_out)) in &current {
            let (sent_per_sec, recv_per_sec) = self
                .last_per_pid
                .get(pid)
                .map(|(prev_in, prev_out)| {
                    // PID 复用 / 进程重启 → 累计回退，按 0 计
                    let d_in = cur_in.saturating_sub(*prev_in);
                    let d_out = cur_out.saturating_sub(*prev_out);
                    (
                        (d_out as f64 / elapsed) as u64,
                        (d_in as f64 / elapsed) as u64,
                    )
                })
                .unwrap_or((0, 0));

            // 全零行不返回，避免 noise
            if sent_per_sec == 0 && recv_per_sec == 0 {
                continue;
            }
            rates.push(ProcessNetRate {
                pid: *pid,
                start_time: 0, // 主线程按 PID 查 cached_processes 拿真实 start_time
                bytes_sent_per_sec: sent_per_sec,
                bytes_recv_per_sec: recv_per_sec,
            });
        }

        self.last_per_pid = current;
        self.last_time = now;
        rates
    }

    fn provider_name(&self) -> &'static str {
        "windows-iphelper"
    }
}

/// 一次完整采样：拿每条 IPv4 TCP 连接的累计字节，按 PID 聚合。
///
/// Windows 上结合 `GetTcpTable2` (字节) + `netstat2::get_sockets_info` (PID)，
/// 按 ConnKey join。Linux / macOS 上返回空 HashMap（不应被调用，但保安全）。
#[cfg(target_os = "windows")]
fn sample_per_pid_cumulative() -> HashMap<u32, (u64, u64)> {
    use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo};

    let conn_bytes = win32::sample_all();
    if conn_bytes.is_empty() {
        return HashMap::new();
    }

    let af = AddressFamilyFlags::IPV4;
    let proto = ProtocolFlags::TCP;
    let sockets = netstat2::get_sockets_info(af, proto).unwrap_or_default();

    let mut per_pid: HashMap<u32, (u64, u64)> = HashMap::new();
    for sock in &sockets {
        let tcp = match &sock.protocol_socket_info {
            ProtocolSocketInfo::Tcp(t) => t,
            ProtocolSocketInfo::Udp(_) => continue,
        };
        let local_v4 = match tcp.local_addr {
            IpAddr::V4(v) => v,
            IpAddr::V6(_) => continue,
        };
        let remote_v4 = match tcp.remote_addr {
            IpAddr::V4(v) => v,
            IpAddr::V6(_) => continue,
        };
        let key = ConnKey {
            local_addr: local_v4,
            local_port: tcp.local_port,
            remote_addr: remote_v4,
            remote_port: tcp.remote_port,
        };
        let Some(&(bytes_in, bytes_out)) = conn_bytes.get(&key) else {
            continue;
        };
        // netstat2 的 associated_pids 通常是 1 元素 Vec；防御性遍历
        for &pid in &sock.associated_pids {
            let e = per_pid.entry(pid).or_insert((0, 0));
            e.0 = e.0.saturating_add(bytes_in);
            e.1 = e.1.saturating_add(bytes_out);
        }
    }
    per_pid
}

#[cfg(not(target_os = "windows"))]
fn sample_per_pid_cumulative() -> HashMap<u32, (u64, u64)> {
    HashMap::new()
}
