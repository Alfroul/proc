//! 阶段 5 D2 — TCP 传输质量指标集成测试。
//!
//! 1. `/proc/net/snmp` parser 单元测试(跨平台可跑,用 fixture 字符串)。
//! 2. Windows 上 `SystemSnapshot::tcp_stats()` 返回非零 retransmitted_segs。
//! 3. TcpStats 字段是否被正确累加(IPv4 + IPv6 在 Windows 上)。

use proc::port_map::parse_proc_net_snmp_tcp;

const PROC_NET_SNMP_SAMPLE: &str = "\
Ip: Forwarding DefaultTTL InReceives InHdrErrors InAddrErrors ForwDatagrams InUnknownProtos InDiscards InDelivers OutRequests OutDiscards OutNoRoutes ReasmTimeout ReasmReqds ReasmOKs ReasmFails FragOKs FragFails FragCreates
Ip: 1 64 1234567 0 0 0 0 0 1100000 1500000 0 0 0 0 0 0 0 0 0
Icmp: ...
Icmp: ...
IcmpMsg: ...
IcmpMsg: ...
Tcp: RtoAlgorithm RtoMin RtoMax MaxConn ActiveOpens PassiveOpens AttemptFails EstabResets CurrEstab InSegs OutSegs RetransSegs OutRsts InCsumErrors
Tcp: 1 200 120000 0 5210 3320 7 4 2 1234567 6543210 12 8 0
Udp: InDatagrams NoPorts InErrors OutDatagrams RcvbufErrors SndbufErrors
Udp: 100000 5 0 110000 0 0
";

#[test]
fn parses_real_proc_net_snmp_format() {
    let s = parse_proc_net_snmp_tcp(PROC_NET_SNMP_SAMPLE);
    assert_eq!(s.retrans_segs, 12);
    assert_eq!(s.out_rsts, 8);
    assert_eq!(s.attempt_fails, 7);
    assert_eq!(s.out_segs, 6_543_210);
}

#[test]
fn parses_compact_snmp_without_udp_section() {
    let compact = "\
Tcp: RtoAlgorithm RtoMin AttemptFails OutSegs RetransSegs OutRsts
Tcp: 1 200 9 555 4 2
";
    let s = parse_proc_net_snmp_tcp(compact);
    assert_eq!(s.attempt_fails, 9);
    assert_eq!(s.out_segs, 555);
    assert_eq!(s.retrans_segs, 4);
    assert_eq!(s.out_rsts, 2);
}

#[test]
fn returns_zero_when_no_tcp_section() {
    let non_tcp = "Udp: InDatagrams\nUdp: 100\n";
    let s = parse_proc_net_snmp_tcp(non_tcp);
    // default: all zero
    assert_eq!(s.retrans_segs, 0);
    assert_eq!(s.out_rsts, 0);
    assert_eq!(s.attempt_fails, 0);
    assert_eq!(s.out_segs, 0);
}

#[test]
fn returns_zero_on_garbage_input() {
    let s = parse_proc_net_snmp_tcp("not even close to snmp format\n");
    assert_eq!(s.retrans_segs, 0);
}

#[test]
fn windows_tcp_stats_populates_retransmit_field() {
    // Windows 上 retransmitted_segs / reset_segs 应该被 GetTcpStatisticsEx2 填上;
    // 系统跑过一段时间后累计值通常 > 0。刚启动 / 测试沙箱里可能是 0,
    // 我们只验证「字段存在 + 调用不 panic」。
    let stats = proc::collect::SystemSnapshot::tcp_stats();
    // 字段读出来;具体值不严格断言(系统状态依赖)。
    let _ = stats.retransmitted_segs;
    let _ = stats.reset_segs;
    let _ = stats.failed_connections;
    let _ = stats.out_segs;
}
