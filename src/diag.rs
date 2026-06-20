use std::io::{BufRead, BufReader, Read, Write as IoWrite};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub enum DiagnosticTool {
    Ping,
    DnsReverse,
    Whois,
    Traceroute,
    PortScan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticPhase {
    Menu,
    Running,
    Completed,
    Failed,
}

pub struct DiagnosticState {
    pub target_ip: IpAddr,
    pub phase: DiagnosticPhase,
    pub tool_index: usize,
    pub content: Vec<String>,
    pub scroll: u16,
    pub error_msg: Option<String>,
    pub auto_scroll: bool,
}

impl DiagnosticState {
    #[must_use]
    pub fn new(target_ip: IpAddr) -> Self {
        Self {
            target_ip,
            phase: DiagnosticPhase::Menu,
            tool_index: 0,
            content: Vec::new(),
            scroll: 0,
            error_msg: None,
            auto_scroll: true,
        }
    }

    #[must_use]
    pub fn tool_list() -> Vec<DiagnosticTool> {
        vec![
            DiagnosticTool::Ping,
            DiagnosticTool::DnsReverse,
            DiagnosticTool::Whois,
            DiagnosticTool::Traceroute,
            DiagnosticTool::PortScan,
        ]
    }

    #[must_use]
    pub fn tool_name(tool: &DiagnosticTool) -> &'static str {
        match tool {
            DiagnosticTool::Ping => "Ping",
            DiagnosticTool::DnsReverse => "DNS 反查",
            DiagnosticTool::Whois => "Whois",
            DiagnosticTool::Traceroute => "Traceroute",
            DiagnosticTool::PortScan => "端口探测",
        }
    }

    /// Whether this tool is unavailable for private/loopback IPs
    #[must_use]
    pub fn tool_unavailable_for_private(tool: &DiagnosticTool) -> bool {
        matches!(tool, DiagnosticTool::Whois | DiagnosticTool::Traceroute)
    }
}

#[must_use]
pub fn is_private_or_loopback(ip: &IpAddr) -> bool {
    ip.is_loopback() || is_private(ip)
}

fn is_private(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Run ping in a background thread, streaming output lines via mpsc channel.
///
/// Spawns ping.exe and streams its stdout line-by-line so a Ctrl+C can kill
/// the child between hops (rather than waiting for the whole run). Decodes
/// GBK-encoded output from Windows ping.exe with encoding_rs.
#[must_use]
pub fn run_ping(
    ip: IpAddr,
) -> (
    std::thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<String>,
) {
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = std::thread::spawn(move || {
        let mut child = match Command::new("ping")
            .args(["-n", "4", "-w", "1000", &ip.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(format!("执行 ping 失败: {}", e));
                return;
            }
        };

        stream_gbk_stdout(&mut child, &tx);
    });

    (handle, rx)
}

/// Run DNS reverse lookup in a background thread.
#[must_use]
pub fn run_dns_reverse(
    ip: IpAddr,
) -> (
    std::thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<String>,
) {
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = std::thread::spawn(move || {
        let _ = tx.send(format!("正在解析 {} ...", ip));

        match dns_lookup::lookup_addr(&ip) {
            Ok(hostname) => {
                let _ = tx.send(format!("{} → {}", ip, hostname));
            }
            Err(e) => {
                let _ = tx.send(format!("解析失败: {}", e));
            }
        }
    });

    (handle, rx)
}

/// Run Whois query in a background thread.
#[must_use]
pub fn run_whois(
    ip: IpAddr,
) -> (
    std::thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<String>,
) {
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = std::thread::spawn(move || {
        let _ = tx.send(format!("正在查询 {}...\n", ip));

        let server_addr: std::net::SocketAddr = match "whois.iana.org:43".parse() {
            Ok(a) => a,
            Err(e) => {
                let _ = tx.send(format!("地址解析失败: {}", e));
                return;
            }
        };

        let mut stream = match TcpStream::connect_timeout(&server_addr, Duration::from_secs(5)) {
            Ok(s) => s,
            Err(_) => {
                let _ = tx.send("连接超时（可能被防火墙拦截）".to_string());
                return;
            }
        };

        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));

        if let Err(e) = stream.write_all(format!("{}\r\n", ip).as_bytes()) {
            let _ = tx.send(format!("发送查询失败: {}", e));
            return;
        }

        let reader = BufReader::new(stream);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    let _ = tx.send(l);
                }
                Err(_) => break,
            }
        }
    });

    (handle, rx)
}

/// Run traceroute in a background thread, streaming output via mpsc channel.
///
/// Spawns tracert.exe and streams stdout line-by-line so Ctrl+C between hops
/// kills the child immediately (tracert can otherwise run for ~15s+).
#[must_use]
pub fn run_traceroute(
    ip: IpAddr,
) -> (
    std::thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<String>,
) {
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = std::thread::spawn(move || {
        let mut child = match Command::new("tracert")
            .args(["-d", "-h", "15", "-w", "1000", &ip.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(format!("执行 tracert 失败: {}", e));
                return;
            }
        };

        stream_gbk_stdout(&mut child, &tx);
    });

    (handle, rx)
}

/// Stream a GBK-encoded child's stdout to `tx`, checking `shutdown::requested()`
/// between lines. On shutdown or natural EOF the child is killed/reaped and
/// any non-empty stderr dump is forwarded as `错误: <text>`.
fn stream_gbk_stdout(child: &mut Child, tx: &std::sync::mpsc::Sender<String>) {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Some(stdout) = stdout {
        let mut reader = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::new();
        loop {
            if crate::shutdown::requested() {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break, // EOF
                Ok(_) => {
                    // Strip CRLF / LF, decode as GBK.
                    while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
                        buf.pop();
                    }
                    let (decoded, _, _) = encoding_rs::GBK.decode(&buf);
                    let _ = tx.send(decoded.to_string());
                }
                Err(_) => break,
            }
        }
    }

    // If shutdown fired during the read loop the child has already been
    // killed + reaped above. Otherwise drain its exit status and surface
    // any stderr so failures are visible in the TUI diagnostic view.
    if !crate::shutdown::requested() {
        let status = child.wait().ok();
        if !matches!(status.map(|s| s.success()), Some(true))
            && let Some(mut stderr) = stderr
        {
            let mut err_bytes = Vec::new();
            if stderr.read_to_end(&mut err_bytes).is_ok() && !err_bytes.is_empty() {
                let (decoded, _, _) = encoding_rs::GBK.decode(&err_bytes);
                let err_str = decoded.trim().to_string();
                if !err_str.is_empty() {
                    let _ = tx.send(format!("执行出错: {}", err_str));
                }
            }
        }
    }
}

const SCAN_PORTS: [(u16, &str); 15] = [
    (21, "FTP"),
    (22, "SSH"),
    (23, "Telnet"),
    (25, "SMTP"),
    (53, "DNS"),
    (80, "HTTP"),
    (110, "POP3"),
    (143, "IMAP"),
    (443, "HTTPS"),
    (465, "SMTPS"),
    (587, "SMTP"),
    (993, "IMAPS"),
    (995, "POP3S"),
    (3306, "MySQL"),
    (5432, "PostgreSQL"),
];

/// Run port scan in a background thread.
#[must_use]
pub fn run_port_scan(
    ip: IpAddr,
) -> (
    std::thread::JoinHandle<()>,
    std::sync::mpsc::Receiver<String>,
) {
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = std::thread::spawn(move || {
        let _ = tx.send(format!("正在扫描 {} 的常用端口...\n", ip));

        for &(port, name) in &SCAN_PORTS {
            let addr = SocketAddr::new(ip, port);
            let result = TcpStream::connect_timeout(&addr, Duration::from_millis(500));
            match result {
                Ok(_) => {
                    let _ = tx.send(format!("  {} ({}) — 开放", port, name));
                }
                Err(_) => {
                    let _ = tx.send(format!("  {} ({}) — 关闭", port, name));
                }
            }
        }

        let _ = tx.send(format!("\n扫描完成，共 {} 个端口", SCAN_PORTS.len()));
    });

    (handle, rx)
}
