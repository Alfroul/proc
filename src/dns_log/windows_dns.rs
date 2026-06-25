//! Windows DNS 查询日志：PowerShell `Get-WinEvent` 子进程路线。
//!
//! # 数据流
//!
//! 1. spawn `powershell.exe -NoProfile -NonInteractive -Command <SCRIPT>`，
//!    SCRIPT 内部 `Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-DNS-Client/Operational'; Id=3010}`
//!    以 ~400ms 节奏轮询新事件，每个事件 emit 一行 JSON 到 stdout
//! 2. reader 线程 `BufReader::read_line` 流式读 stdout，逐行调
//!    [`parse_powershell_event`] 解析 → `DnsQuery` → 通过
//!    `sync_channel(1000)` 发到主线程（worker poll 1000ms 内会 drain）
//! 3. collector 持有 `Receiver<DnsQuery>`，`drain()` `try_recv` 所有可用
//! 4. `Drop`：drop rx → reader 下一次 send 失败 → reader 退出 → Child drop →
//!    PowerShell 进程 SIGTERM/kill
//!
//! # 事件 3010 字段
//!
//! Microsoft-Windows-DNS-Client/Operational event 3010 (`QueryResultsEx`) 字段：
//! - `Properties[0]` = QueryName (string)
//! - `Properties[1]` = QueryType (string，可能是数字 "1" 或助记符 "A")
//! - `Properties[2]` = QueryStatus (uint32，0 = success)
//! - `Properties[3]` = QueryResults (string，`;` 分隔的 IP 列表 + TTL 后缀)
//!
//! ProcessId 来自事件 header（`$e.ProcessId`），不需要单独解析。
//!
//! # 已知限制
//!
//! - 仅 IPv4/IPv6 地址（[`parse_query_results`] 解析失败的分片忽略）
//! - 依赖 Microsoft-Windows-DNS-Client/Operational channel（Win10+ 默认开启）
//! - 子进程 spawn 失败 / PowerShell 缺失 / channel 关闭 → collector `new()` 失败，
//!   worker 不启动，UI 显示空列表（不阻塞其它功能）
//! - PowerShell 启动延迟 ~300ms；首次 `Get-WinEvent` 查询历史 ~200ms；
//!   后续轮询 ~10-50ms（filter hashtable 走 ETL 索引）
//!
//! # 选型说明
//!
//! 阶段 8 原计划 Windows 走 ETW DNS-Client provider（`OpenTrace` + `ProcessTrace`）。
//! 实际落地走 PowerShell `Get-WinEvent` 子进程路线，理由：
//! - ETW 实时 session 需要单独消费者线程 + ~500 行 native FFI + schema 解析；
//! - PowerShell 路线复用项目既有子进程 collector 模式（参考 [`crate::smart`] /
//!   [`crate::net_flow::nethogs`]），~150 行代码即可覆盖；
//! - CPU 开销在 ~10 events/sec DNS 查询频率下可忽略。
//!
//! 详见 `docs/adr/0006-dns-subprocess-not-etw-dbus.md`。

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::dns_log::{DnsLogCollector, DnsQuery, DnsResult, parse_query_type};
use crate::error::{ProcError, Result};
use crate::security::restricted_spawn::{RestrictedChild, spawn_with_reduced_privileges};

/// reader → drain 之间的有界缓冲（FIFO，cap=1000）。
/// 主线程慢消费时，reader 线程 send 会阻塞（轻量背压）；1000 条 DNS 事件
/// 已是 ~10-100s 的余量，足够主线程 tick 50ms 速率消费。
const CHANNEL_CAPACITY: usize = 1000;

/// PowerShell 脚本：循环 `Get-WinEvent -FilterHashtable` 拉新事件，
/// 每事件一行 JSON emit。脚本本体写在单独常量里以便可读性 / 调试。
const POWERSHELL_SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$epoch = [DateTime]'1970-01-01T00:00:00Z'
$lastTime = [DateTime]::Now.AddSeconds(-2)
while ($true) {
    try {
        $events = Get-WinEvent -FilterHashtable @{
            LogName = 'Microsoft-Windows-DNS-Client/Operational'
            Id = 3010
            StartTime = $lastTime
        } -Oldest
        if ($events) {
            foreach ($e in $events) {
                if (-not $e.Properties -or $e.Properties.Count -lt 4) { continue }
                $obj = [ordered]@{
                    ts = [Math]::Round(($e.TimeCreated - $epoch).TotalMilliseconds)
                    pid = [int64]$e.ProcessId
                    name = [string]$e.Properties[0].Value
                    qtype = [string]$e.Properties[1].Value
                    status = [string]$e.Properties[2].Value
                    results = [string]$e.Properties[3].Value
                }
                $obj | ConvertTo-Json -Compress
            }
        }
    } catch {}
    $lastTime = [DateTime]::Now
    Start-Sleep -Milliseconds 400
}
"#;

/// reader 线程内部使用的反序列化结构。
/// `status` 是字符串（PowerShell `[string]` 转换），可能是 "0" / "14652" 等；
/// 解析失败按 `Error("unparsed")` 处理（高频 DNS 不应因解析失败阻塞）。
#[derive(Debug, Deserialize)]
struct PowershellEvent {
    ts: i64,
    pid: i64,
    name: String,
    qtype: String,
    status: String,
    results: String,
}

/// 把 PowerShell JSON 行解析为 [`DnsQuery`]。纯函数，单元测试覆盖。
///
/// - `pid < 0` / `pid > u32::MAX` → None（PowerShell 偶发返回 -1 / 0 表示 System Idle）
/// - `ts` 转 SystemTime 失败 → None（时间戳不在 u64 表示范围）
/// - `status` 解析 u32 失败 → `DnsResult::Error`，不丢弃整个事件
/// - process_name 由调用方（reader 线程）填充，这里返回字符串空占位
#[must_use]
pub fn parse_powershell_event(line: &str) -> Option<DnsQuery> {
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        return None;
    }
    let ev: PowershellEvent = serde_json::from_str(trimmed).ok()?;
    let pid_u32 = u32::try_from(ev.pid).ok()?;
    if pid_u32 == 0 {
        // System Idle Process — 跳过（噪声）
        return None;
    }
    let ts = unix_millis_to_system_time(ev.ts)?;
    let status_u32 = ev.status.trim().parse::<u32>().unwrap_or(u32::MAX);
    let result = if status_u32 == u32::MAX {
        DnsResult::Error(format!("unparsed:{}", ev.status))
    } else {
        DnsResult::from_windows_status(status_u32, &ev.results)
    };
    Some(DnsQuery {
        timestamp: ts,
        pid: pid_u32,
        start_time: 0,               // reader 线程 lookup 时填（P1-A5）
        process_name: String::new(), // reader 线程填充
        query_name: ev.name,
        query_type: parse_query_type(&ev.qtype),
        result,
    })
}

/// i64 毫秒时间戳 → SystemTime。负数或溢出返回 None。
fn unix_millis_to_system_time(ms: i64) -> Option<SystemTime> {
    if ms < 0 {
        return None;
    }
    let d = Duration::from_millis(ms as u64);
    UNIX_EPOCH.checked_add(d)
}

/// PID → (process_name, start_time) 缓存。10 秒刷一次全表（refresh_processes
/// 较重，DNS 事件高频，但 PID 名字变化不频繁）。
///
/// 阶段 11 P1-A5/D4：cache value 含 start_time。每次 lookup 先查 sysinfo 当前
/// `Process::start_time()`，若与 cache 里的不一致 → PID 被复用 → 重查并更新
/// cache，让 reader_loop 拿到正确的 (name, start_time) 填到 DnsQuery。
struct PidNameLookup {
    sys: sysinfo::System,
    last_refresh: Instant,
    cache: std::collections::HashMap<u32, (String, u64)>,
}

impl PidNameLookup {
    fn new() -> Self {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            false,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        Self {
            sys,
            last_refresh: Instant::now(),
            cache: std::collections::HashMap::new(),
        }
    }

    /// 返回 (process_name, start_time)。PID 不存在时返回 ("?", 0)。
    /// cache 命中且 start_time 与 sysinfo 当前值一致 → 直接返回；
    /// 否则重查并更新 cache。
    fn lookup(&mut self, pid: u32) -> (String, u64) {
        // sysinfo Process::start_time() 是 O(1) HashMap 查询（不 refresh）。
        let current_st = self
            .sys
            .process(sysinfo::Pid::from_u32(pid))
            .map(|p| p.start_time())
            .unwrap_or(0);

        // cache 命中且 start_time 一致 → 直接返回（PID 未复用）。
        if let Some((name, st)) = self.cache.get(&pid) {
            if *st == current_st {
                return (name.clone(), *st);
            }
            // start_time 不一致 → PID 被复用 → cache 失效，落到下面的重查路径。
        }

        if self.last_refresh.elapsed() > Duration::from_secs(10) {
            self.sys.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                false,
                sysinfo::ProcessRefreshKind::nothing(),
            );
            self.last_refresh = Instant::now();
            // refresh 后 sysinfo 内部表更新；current_st 用刷新后的值重算一次。
        }
        let (name, st) = self
            .sys
            .process(sysinfo::Pid::from_u32(pid))
            .map(|p| (p.name().to_string_lossy().into_owned(), p.start_time()))
            .unwrap_or_else(|| ("?".into(), 0));
        self.cache.insert(pid, (name.clone(), st));
        (name, st)
    }
}

pub struct PowershellDnsCollector {
    /// `Option<Mutex<Receiver>>` 以便 Drop 时手动控制销毁顺序（先关 channel 再 join）。
    /// Mutex 满足 `Sync` bound（mpsc::Receiver 仅 Send，不 Sync）。
    rx: Option<Mutex<Receiver<DnsQuery>>>,
    /// Child 共享给 reader 线程持有 + Drop 时主动 kill。reader 线程不直接操作，
    /// 只确保 Child 不被 drop。Drop 时 collector 锁住 mutex、take 出 child、kill。
    ///
    /// v0.6.0 阶段 2：类型从 `Child` 改为 `RestrictedChild` — spawn 走
    /// [`spawn_with_reduced_privileges`] 剥离继承的 SeDebugPrivilege，防止 elevated
    /// proc spawn 出的 PowerShell 子进程变成 credential theft 跳板（ADR-0008）。
    child: Arc<Mutex<Option<RestrictedChild>>>,
    reader_thread: Option<JoinHandle<()>>,
}

impl PowershellDnsCollector {
    /// 仅 Windows 编译。spawn PowerShell 子进程 + reader 线程。
    pub fn new() -> Result<Self> {
        // 先 probe：确认 powershell.exe 在 PATH（Windows 自带，但 Server Core / 容器可能没有）。
        let probe = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| ProcError::monitor(format!("powershell.exe 启动失败: {e}")))?;
        if !probe.success() {
            return Err(ProcError::monitor(
                "powershell.exe 退出码非零，DNS 日志采集不可用",
            ));
        }

        // v0.6.0 阶段 2：spawn 走 restricted token（DISABLE_MAX_PRIVILEGE），
        // 剥离 SeDebugPrivilege 等继承权限。spawn_with_reduced_privileges 内部
        // 在非 elevated 环境自动降级到普通 Command 并 tracing 一次 warn。
        let mut child = spawn_with_reduced_privileges(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                POWERSHELL_SCRIPT,
            ],
        )
        .map_err(|e| ProcError::monitor(format!("spawn powershell.exe 失败: {e}")))?;

        let stdout = child.stdout().ok_or_else(|| {
            ProcError::monitor("powershell.exe 未返回 stdout 管道，DNS 日志采集不可用")
        })?;
        finish_init_restricted(child, stdout)
    }
}

/// 把已 spawn 好的 child + stdout pipe 组装成完整 PowershellDnsCollector。
fn finish_init_restricted(
    child: RestrictedChild,
    stdout: std::fs::File,
) -> Result<PowershellDnsCollector> {
    let (tx, rx) = mpsc::sync_channel::<DnsQuery>(CHANNEL_CAPACITY);

    // Child 由 collector + reader 共享。reader 线程只持有 Arc clone
    // 作为「保活」句柄（不直接操作 Child），实际 kill 由 collector Drop 触发。
    let child_shared: Arc<Mutex<Option<RestrictedChild>>> = Arc::new(Mutex::new(Some(child)));
    let child_for_reader = Arc::clone(&child_shared);

    let handle = thread::Builder::new()
        .name("dns-log-reader".into())
        .spawn(move || reader_loop(child_for_reader, stdout, tx))
        .map_err(|e| ProcError::monitor(format!("spawn dns-log-reader 失败: {e}")))?;

    Ok(PowershellDnsCollector {
        rx: Some(Mutex::new(rx)),
        child: child_shared,
        reader_thread: Some(handle),
    })
}

/// reader 线程主体：阻塞读 stdout → 解析 → 查 PID 名 → 发到 channel。
///
/// Child 共享句柄保活，但 kill 由 collector Drop 触发（`reader_loop` 不操作）。
/// collector Drop kill 子进程 → stdout 管道关闭 → read_line 返回 0（EOF）→ 循环退出。
///
/// v0.6.0 阶段 2：stdout 类型从 `std::process::ChildStdout` 改为 `std::fs::File` —
/// `BufReader::new(stdout)` 两者都接受（都 impl Read）。类型变化对 reader_loop
/// 实现透明，仅签名调整。
fn reader_loop(
    _child_keepalive: Arc<Mutex<Option<RestrictedChild>>>,
    stdout: std::fs::File,
    tx: SyncSender<DnsQuery>,
) {
    let mut reader = BufReader::new(stdout);
    let mut lookup = PidNameLookup::new();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF — PowerShell 进程结束 / 被 kill
            Ok(_) => {}
            Err(_) => break, // pipe broken
        }

        let Some(mut query) = parse_powershell_event(&line) else {
            continue;
        };
        let (name, start_time) = lookup.lookup(query.pid);
        query.process_name = name;
        query.start_time = start_time;

        if tx.send(query).is_err() {
            // channel 关闭 → 主线程在清理
            break;
        }
    }

    // reader 退出后顺手 kill 一次（idempotent），防孤儿 PowerShell。
    // collector Drop 通常已先 kill；这里覆盖 reader 先退出（EOF）的场景。
    if let Ok(mut guard) = _child_keepalive.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl DnsLogCollector for PowershellDnsCollector {
    fn drain(&mut self) -> Vec<DnsQuery> {
        let Some(rx_lock) = self.rx.as_ref() else {
            return Vec::new();
        };
        let rx = rx_lock.lock().expect("dns log rx mutex poisoned");
        let mut out = Vec::new();
        while let Ok(q) = rx.try_recv() {
            out.push(q);
        }
        out
    }

    fn provider_name(&self) -> &'static str {
        "windows-powershell"
    }
}

impl Drop for PowershellDnsCollector {
    fn drop(&mut self) {
        // 1. 先 kill 子进程：PowerShell 死 → stdout 关闭 → reader 的 read_line
        //    返回 0（EOF）→ reader 退出。否则 PowerShell 不产生输出时 reader
        //    永远阻塞在 read_line，Drop 的 join 会卡死。
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        // 2. 关 channel（防御性；reader 应已通过 EOF 退出）
        self.rx.take();
        // 3. 等 reader 线程
        if let Some(h) = self.reader_thread.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn parse_powershell_event_basic_success() {
        let line = r#"{"ts":1781308800123,"pid":1234,"name":"example.com.","qtype":"1","status":"0","results":"1.2.3.4;;"}"#;
        let q = parse_powershell_event(line).expect("parse");
        assert_eq!(q.pid, 1234);
        assert_eq!(q.query_name, "example.com.");
        assert_eq!(q.query_type, "A");
        match q.result {
            DnsResult::Success(ips) => {
                assert_eq!(ips.len(), 1);
                assert_eq!(ips[0], "1.2.3.4".parse::<IpAddr>().unwrap());
            }
            other => panic!("expected Success, got {other:?}"),
        }
        // process_name 留给 reader 线程填，纯函数返回空
        assert_eq!(q.process_name, "");
    }

    #[test]
    fn parse_powershell_event_nxdomain() {
        let line = r#"{"ts":1000,"pid":50,"name":"nope.invalid.","qtype":"28","status":"14652","results":""}"#;
        let q = parse_powershell_event(line).expect("parse");
        assert_eq!(q.query_type, "AAAA");
        assert!(matches!(q.result, DnsResult::NxDomain));
    }

    #[test]
    fn parse_powershell_event_timeout() {
        let line = r#"{"ts":1000,"pid":50,"name":"slow.example.","qtype":"1","status":"10060","results":""}"#;
        let q = parse_powershell_event(line).expect("parse");
        assert!(matches!(q.result, DnsResult::Timeout));
    }

    #[test]
    fn parse_powershell_event_invalid_pid_dropped() {
        let line = r#"{"ts":1000,"pid":-1,"name":"x.","qtype":"1","status":"0","results":""}"#;
        assert!(parse_powershell_event(line).is_none());
    }

    #[test]
    fn parse_powershell_event_pid_zero_dropped() {
        // PID 0 = System Idle Process，纯噪声
        let line = r#"{"ts":1000,"pid":0,"name":"x.","qtype":"1","status":"0","results":""}"#;
        assert!(parse_powershell_event(line).is_none());
    }

    #[test]
    fn parse_powershell_event_status_unparseable_kept_as_error() {
        let line =
            r#"{"ts":1000,"pid":50,"name":"weird.","qtype":"1","status":"garbage","results":""}"#;
        let q = parse_powershell_event(line).expect("parse");
        match q.result {
            DnsResult::Error(s) => assert!(s.contains("unparsed"), "s = {s}"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn parse_powershell_event_non_json_dropped() {
        assert!(parse_powershell_event("").is_none());
        assert!(parse_powershell_event("   ").is_none());
        assert!(parse_powershell_event("not json").is_none());
        // 必须以 `{` 开头（容忍 PowerShell 偶发输出 PowerShell 错误前缀）
        assert!(parse_powershell_event("Get-WinEvent : No events found").is_none());
    }

    #[test]
    fn parse_powershell_event_mnemonic_qtype_passes_through() {
        let line = r#"{"ts":1000,"pid":50,"name":"x.","qtype":"HTTPS","status":"0","results":""}"#;
        let q = parse_powershell_event(line).expect("parse");
        // 已是助记符 → 原样返回（parse_query_type 不做反向映射）
        assert_eq!(q.query_type, "HTTPS");
    }

    #[test]
    fn unix_millis_to_system_time_boundaries() {
        assert!(unix_millis_to_system_time(-1).is_none());
        assert!(unix_millis_to_system_time(0).is_some());
        let st = unix_millis_to_system_time(1_781_308_800_000).unwrap();
        let dur = st.duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(dur.as_millis(), 1_781_308_800_000);
    }
}
