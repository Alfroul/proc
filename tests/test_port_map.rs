use proc::port_map::{self, Protocol};

#[test]
fn test_port_scan_returns_list() {
    let result = port_map::scan_ports();
    assert!(result.is_ok(), "scan_ports should not error");
    let entries = result.unwrap();
    // On a running system there should be at least some ports
    assert!(!entries.is_empty(), "should find some ports on the system");
}

#[test]
fn test_port_entry_protocol_display() {
    assert_eq!(format!("{}", Protocol::Tcp), "TCP");
    assert_eq!(format!("{}", Protocol::Udp), "UDP");
}

#[test]
fn test_port_entry_protocol_equality() {
    assert_eq!(Protocol::Tcp, Protocol::Tcp);
    assert_ne!(Protocol::Tcp, Protocol::Udp);
}

#[test]
fn test_find_pid_by_port_known_port() {
    // Scan all ports first to find one that exists
    let all = port_map::scan_ports().unwrap();
    if let Some(first) = all.first() {
        let port = first.local_port;
        let result = port_map::find_pid_by_port(port);
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }
}

#[test]
fn test_find_pid_by_port_unused_port() {
    // Use a very high port that's unlikely to be in use
    let result = port_map::find_pid_by_port(59999);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty(), "unused port should return empty");
}

#[test]
fn test_find_ports_by_pid_known_pid() {
    let all = port_map::scan_ports().unwrap();
    if let Some(first) = all.first() {
        let pid = first.pid;
        if pid > 0 {
            let result = port_map::find_ports_by_pid(pid);
            assert!(result.is_ok());
            assert!(!result.unwrap().is_empty());
        }
    }
}

#[test]
fn test_find_ports_by_pid_nonexistent() {
    let result = port_map::find_ports_by_pid(99999);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty(), "nonexistent PID should return empty");
}

#[test]
fn test_port_scan_sorted_by_port() {
    let entries = port_map::scan_ports().unwrap();
    for window in entries.windows(2) {
        if window[0].protocol == window[1].protocol {
            assert!(
                window[0].local_port <= window[1].local_port,
                "entries should be sorted by port within same protocol"
            );
        }
    }
}

#[test]
fn test_tcp_entries_have_remote() {
    let entries = port_map::scan_ports().unwrap();
    for entry in &entries {
        if entry.protocol == Protocol::Tcp {
            assert!(entry.remote_addr.is_some(), "TCP should have remote_addr");
            assert!(entry.remote_port.is_some(), "TCP should have remote_port");
            assert!(entry.state.is_some(), "TCP should have state");
        }
    }
}

#[test]
fn test_udp_entries_no_remote() {
    let entries = port_map::scan_ports().unwrap();
    for entry in &entries {
        if entry.protocol == Protocol::Udp {
            assert!(entry.remote_addr.is_none(), "UDP should not have remote_addr");
            assert!(entry.remote_port.is_none(), "UDP should not have remote_port");
            assert!(entry.state.is_none(), "UDP should not have state");
        }
    }
}
