use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Instant;

use crate::port_map::PortEntry;

#[derive(Debug, Clone)]
pub struct ConnectionEStats {
    pub local_addr: IpAddr,
    pub local_port: u16,
    pub remote_addr: IpAddr,
    pub remote_port: u16,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

pub struct EStatsCollector {
    last_sample: Vec<ConnectionEStats>,
    current_sample: Vec<ConnectionEStats>,
    last_time: Instant,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ConnKey {
    local_port: u16,
    remote_addr: IpAddr,
    remote_port: u16,
}

impl ConnKey {
    fn from_conn(c: &ConnectionEStats) -> Self {
        Self {
            local_port: c.local_port,
            remote_addr: c.remote_addr,
            remote_port: c.remote_port,
        }
    }
}

#[cfg(target_os = "windows")]
mod win32 {
    use super::*;

    fn u32_to_ipv4(v: u32) -> Ipv4Addr {
        Ipv4Addr::from(v.to_ne_bytes())
    }

    fn ntohs_port(v: u32) -> u16 {
        ((v & 0xFFFF) as u16).to_be()
    }

    fn make_tcprow_lh(
        row2: &windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW2,
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

    fn enable_estats_for_all() {
        use windows::Win32::Foundation::BOOLEAN;
        use windows::Win32::NetworkManagement::IpHelper::{
            GetTcpTable2, MIB_TCPTABLE2, SetPerTcpConnectionEStats, TCP_ESTATS_DATA_RW_v0,
        };

        let mut table_size = 0u32;
        unsafe {
            let _ = GetTcpTable2(None, &mut table_size, false);
        }
        if table_size == 0 {
            return;
        }

        let mut buf: Vec<u8> = vec![0u8; table_size as usize];
        let table_ptr = buf.as_mut_ptr() as *mut MIB_TCPTABLE2;

        let err = unsafe { GetTcpTable2(Some(table_ptr), &mut table_size, false) };
        if err != 0 {
            tracing::warn!("GetTcpTable2 failed during EStats enable: {}", err);
            return;
        }

        let table = unsafe { &*table_ptr };
        let num_entries = table.dwNumEntries as usize;
        // MIB_TCPTABLE2 has a flexible array member — slice from the raw buffer
        let rows: &[windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW2] = unsafe {
            let header = std::mem::size_of::<u32>(); // dwNumEntries
            let row_size =
                std::mem::size_of::<windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW2>();
            if buf.len() < header + num_entries * row_size {
                tracing::warn!("EStats: buffer too small for {} entries", num_entries);
                return;
            }
            std::slice::from_raw_parts(buf.as_ptr().add(header) as *const _, num_entries)
        };

        let mut rw: TCP_ESTATS_DATA_RW_v0 = unsafe { std::mem::zeroed() };
        rw.EnableCollection = BOOLEAN(1);

        let rw_bytes = unsafe {
            std::slice::from_raw_parts(
                &rw as *const _ as *const u8,
                std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>(),
            )
        };

        for row2 in rows {
            let row = make_tcprow_lh(row2);
            let result = unsafe {
                SetPerTcpConnectionEStats(
                    &row,
                    windows::Win32::NetworkManagement::IpHelper::TCP_ESTATS_TYPE(0),
                    rw_bytes,
                    0,
                    0,
                )
            };
            if result != 0 {
                tracing::trace!(
                    "SetPerTcpConnectionEStats failed for {:?}:{} → {}",
                    u32_to_ipv4(row2.dwLocalAddr),
                    ntohs_port(row2.dwLocalPort),
                    result
                );
            }
        }
    }

    fn sample_connections() -> Vec<ConnectionEStats> {
        use windows::Win32::NetworkManagement::IpHelper::{
            GetPerTcpConnectionEStats, GetTcpTable2, MIB_TCPTABLE2, TCP_ESTATS_DATA_ROD_v0,
        };

        let mut table_size = 0u32;
        unsafe {
            let _ = GetTcpTable2(None, &mut table_size, false);
        }
        if table_size == 0 {
            return Vec::new();
        }

        let mut buf: Vec<u8> = vec![0u8; table_size as usize];
        let table_ptr = buf.as_mut_ptr() as *mut MIB_TCPTABLE2;

        let err = unsafe { GetTcpTable2(Some(table_ptr), &mut table_size, false) };
        if err != 0 {
            tracing::warn!("GetTcpTable2 failed during sample: {}", err);
            return Vec::new();
        }

        let table = unsafe { &*table_ptr };
        let num_entries = table.dwNumEntries as usize;
        let rows: &[windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW2] = unsafe {
            let header = std::mem::size_of::<u32>();
            let row_size =
                std::mem::size_of::<windows::Win32::NetworkManagement::IpHelper::MIB_TCPROW2>();
            if buf.len() < header + num_entries * row_size {
                return Vec::new();
            }
            std::slice::from_raw_parts(buf.as_ptr().add(header) as *const _, num_entries)
        };

        let mut results = Vec::with_capacity(num_entries);

        for row2 in rows {
            let local_addr = IpAddr::V4(u32_to_ipv4(row2.dwLocalAddr));
            let local_port = ntohs_port(row2.dwLocalPort);
            let remote_addr = IpAddr::V4(u32_to_ipv4(row2.dwRemoteAddr));
            let remote_port = ntohs_port(row2.dwRemotePort);

            if remote_addr.is_unspecified() && remote_port == 0 {
                continue;
            }

            let row = make_tcprow_lh(row2);

            let mut rod: TCP_ESTATS_DATA_ROD_v0 = unsafe { std::mem::zeroed() };
            let rod_buf = unsafe {
                std::slice::from_raw_parts_mut(
                    &mut rod as *mut _ as *mut u8,
                    std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>(),
                )
            };

            let result = unsafe {
                GetPerTcpConnectionEStats(
                    &row,
                    windows::Win32::NetworkManagement::IpHelper::TCP_ESTATS_TYPE(0),
                    None,
                    0,
                    None,
                    0,
                    Some(rod_buf),
                    0,
                )
            };

            if result != 0 {
                continue;
            }

            results.push(ConnectionEStats {
                local_addr,
                local_port,
                remote_addr,
                remote_port,
                bytes_in: rod.DataBytesIn,
                bytes_out: rod.DataBytesOut,
            });
        }

        results
    }

    impl EStatsCollector {
        pub fn new() -> crate::error::Result<Self> {
            enable_estats_for_all();
            let current = sample_connections();
            Ok(Self {
                last_sample: Vec::new(),
                current_sample: current,
                last_time: Instant::now(),
            })
        }

        pub fn sample(&mut self) {
            enable_estats_for_all();
            self.last_sample = std::mem::take(&mut self.current_sample);
            self.current_sample = sample_connections();
            self.last_time = Instant::now();
        }

        #[must_use]
        pub fn connection_speed(&self, local_port: u16, remote_addr: &IpAddr) -> (u64, u64) {
            let elapsed = self.last_time.elapsed().as_secs_f64();
            if elapsed < 0.1 {
                return (0, 0);
            }

            let cur_map: HashMap<ConnKey, (u64, u64)> = self
                .current_sample
                .iter()
                .map(|c| (ConnKey::from_conn(c), (c.bytes_in, c.bytes_out)))
                .collect();

            let mut total_down: i64 = 0;
            let mut total_up: i64 = 0;

            for last in &self.last_sample {
                if last.local_port != local_port || &last.remote_addr != remote_addr {
                    continue;
                }
                let key = ConnKey::from_conn(last);
                if let Some(&(cur_in, cur_out)) = cur_map.get(&key) {
                    total_down += (cur_in as i64 - last.bytes_in as i64).max(0);
                    total_up += (cur_out as i64 - last.bytes_out as i64).max(0);
                }
            }

            (
                (total_down as f64 / elapsed) as u64,
                (total_up as f64 / elapsed) as u64,
            )
        }

        #[must_use]
        pub fn process_speed(&self, _pid: u32, entries: &[PortEntry]) -> (u64, u64, u64, u64) {
            let elapsed = self.last_time.elapsed().as_secs_f64();
            if elapsed < 0.1 {
                return (0, 0, 0, 0);
            }

            let cur_map: HashMap<ConnKey, (u64, u64)> = self
                .current_sample
                .iter()
                .map(|c| (ConnKey::from_conn(c), (c.bytes_in, c.bytes_out)))
                .collect();

            let proc_conns: Vec<ConnKey> = entries
                .iter()
                .filter(|e| e.protocol == crate::port_map::Protocol::Tcp)
                .filter_map(|e| {
                    Some(ConnKey {
                        local_port: e.local_port,
                        remote_addr: e.remote_addr?,
                        remote_port: e.remote_port?,
                    })
                })
                .collect();

            let mut ds: u64 = 0;
            let mut us: u64 = 0;
            let mut td: u64 = 0;
            let mut tu: u64 = 0;

            for key in &proc_conns {
                if let Some(&(cur_in, cur_out)) = cur_map.get(key) {
                    td += cur_in;
                    tu += cur_out;
                    for last in &self.last_sample {
                        if ConnKey::from_conn(last) == *key {
                            let di = (cur_in as i64 - last.bytes_in as i64).max(0) as u64;
                            let do_ = (cur_out as i64 - last.bytes_out as i64).max(0) as u64;
                            ds += (di as f64 / elapsed) as u64;
                            us += (do_ as f64 / elapsed) as u64;
                            break;
                        }
                    }
                }
            }

            (ds, us, td, tu)
        }
    }
}
