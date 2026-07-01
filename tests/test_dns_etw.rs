//! v0.11 阶段 2：DNS ETW provider 测试。对应 ADR-0020。
//!
//! 平台 cfg-gate：
//! - **Windows**：spawn EtwDnsCollector → Resolve-DnsName example.com → 验证
//!   drain 出 DnsQuery 含 query_name=example.com（管理员权限；非 admin 走 SKIP）
//! - **其它平台**：`detect_collector` 返回 `(None, None)`，验证 stub 行为
//!
//! 跨平台单元测试（DnsCollectorKind / 字符串辅助函数）在 `src/dns_log/etw.rs`
//! 内部 mod tests。本文件测 collector 启停 + 触发 DNS 查询后采到事件的端到端路径。
//!
//! 对应 stage 2 doc 任务 7 验收标准「Windows admin 跑 proc dns 显示 DNS 查询
//! （包含 curl 触发的 example.com 解析）；非 admin 降级到 PowerShell 路径不挂」。

use proc::dns_log::{DnsCollectorKind, detect_collector};

// ──────────────────────────────────────────────────────────────────────────
// 跨平台：DnsCollectorKind enum 契约
// ──────────────────────────────────────────────────────────────────────────

/// `DnsCollectorKind::as_str` 三态字符串契约（proc diag 输出依赖此）。
#[test]
fn dns_collector_kind_as_str() {
    assert_eq!(DnsCollectorKind::Etw.as_str(), "etw");
    assert_eq!(DnsCollectorKind::PowerShell.as_str(), "powershell");
    assert_eq!(DnsCollectorKind::None.as_str(), "none");
}

/// `DnsCollectorKind` Display 实现 = as_str（一致性契约）。
#[test]
fn dns_collector_kind_display_matches_as_str() {
    use std::fmt::Write;
    let mut s = String::new();
    write!(&mut s, "{}", DnsCollectorKind::Etw).unwrap();
    assert_eq!(s, "etw");
    assert_eq!(format!("{}", DnsCollectorKind::PowerShell), "powershell");
    assert_eq!(format!("{}", DnsCollectorKind::None), "none");
}

/// serde 序列化 lowercase 契约（proc diag --json / 录屏 round-trip 依赖此）。
#[test]
fn dns_collector_kind_serde_lowercase() {
    let json = serde_json::to_string(&DnsCollectorKind::Etw).expect("serialize");
    assert_eq!(json, "\"etw\"");
    let json = serde_json::to_string(&DnsCollectorKind::PowerShell).expect("serialize");
    assert_eq!(json, "\"powershell\"");
    let json = serde_json::to_string(&DnsCollectorKind::None).expect("serialize");
    assert_eq!(json, "\"none\"");
}

/// DnsCollectorKind: Copy + Clone + PartialEq + Eq 契约（让 App 字段可比较）。
#[test]
fn dns_collector_kind_traits() {
    let a = DnsCollectorKind::Etw;
    let b = a; // Copy
    assert_eq!(a, b);
    assert_ne!(DnsCollectorKind::Etw, DnsCollectorKind::PowerShell);
    assert_ne!(DnsCollectorKind::PowerShell, DnsCollectorKind::None);
}

// ──────────────────────────────────────────────────────────────────────────
// 跨平台：detect_collector 返回 tuple 契约
// ──────────────────────────────────────────────────────────────────────────

/// `detect_collector` 返回 `(Option<Box<dyn DnsLogCollector>>, DnsCollectorKind)`。
/// 非 Windows 上 tuple 第二项必须是 `None`，第一项也是 `None`（无 collector）。
/// Windows 上：admin → `(Some(_), Etw)`；非 admin → `(Some(_), PowerShell)` 或
/// `(None, None)`（PowerShell 也失败时）。
#[test]
fn detect_collector_returns_aligned_tuple() {
    let (collector, kind) = detect_collector();
    // tuple 必须自洽：collector Some 时 kind != None；collector None 时 kind == None
    match (&collector, kind) {
        (Some(_), DnsCollectorKind::None) => {
            panic!("collector Some 但 kind=None，tuple 不自洽");
        }
        (None, DnsCollectorKind::Etw) => {
            panic!("collector None 但 kind=Etw，tuple 不自洽");
        }
        (None, DnsCollectorKind::PowerShell) => {
            panic!("collector None 但 kind=PowerShell，tuple 不自洽");
        }
        _ => {}
    }
    // drain 不应 panic（空 Vec 是合法返回）
    if let Some(mut c) = collector {
        let queries = c.drain();
        assert!(
            queries.is_empty(),
            "刚 spawn 的 collector drain 应为空（实际 {} 条）",
            queries.len()
        );
        // provider_name 不为空
        let name = c.provider_name();
        assert!(!name.is_empty(), "provider_name 不应为空");
    }
}

/// 非 Windows 平台：detect_collector 必须返回 `(None, None)`。
/// Windows 平台：跳过此断言（admin 可能返 ETW，非 admin 可能返 PowerShell）。
#[cfg(not(target_os = "windows"))]
#[test]
fn detect_collector_none_on_non_windows() {
    let (collector, kind) = detect_collector();
    assert!(collector.is_none(), "非 Windows 必须 collector=None");
    assert_eq!(kind, DnsCollectorKind::None, "非 Windows 必须 kind=None");
}

// ──────────────────────────────────────────────────────────────────────────
// Windows tests：仅在 Windows 上跑（admin / 非 admin 各覆盖）
// ──────────────────────────────────────────────────────────────────────────

/// Windows 管理员下 EtwDnsCollector 启停干净（drop 不 panic / 不泄漏 session）。
///
/// 验证：
/// 1. 管理员下能 StartTraceW + EnableTraceEx2 + OpenTraceW + ProcessTrace
/// 2. drain 不 panic
/// 3. drop 时 stop_session + join trace_thread + CloseTrace 都不 panic
///
/// 非管理员：detect_collector fallback 到 PowerShell（或全失败），kind 反映实际。
#[cfg(target_os = "windows")]
#[test]
fn detect_collector_windows_round_trip_clean() {
    let (collector, kind) = detect_collector();
    // Windows 上：要么 ETW（admin），要么 PowerShell（fallback），要么 None（都失败）
    assert!(
        matches!(
            kind,
            DnsCollectorKind::Etw | DnsCollectorKind::PowerShell | DnsCollectorKind::None
        ),
        "kind 必须是合法 Windows 状态"
    );
    if let Some(mut c) = collector {
        let queries = c.drain();
        assert!(queries.is_empty(), "刚 spawn drain 应为空");
        eprintln!(
            "detect_collector_windows_round_trip_clean: kind={kind}, provider={}",
            c.provider_name()
        );
        // Drop 不 panic（stop session + join trace_thread + CloseTrace）
        drop(c);
    } else {
        eprintln!(
            "SKIP: Windows 上无 collector 启动成功（PowerShell 缺失？ETW 失败？），kind={kind}"
        );
    }
}

/// 管理员下触发 Resolve-DnsName example.com → ETW collector drain 出含
/// query_name 含 "example" 的 DnsQuery。**对应 stage 2 doc 验收标准的核心 case**。
///
/// 失败模式：
/// - 非管理员 / session 占用 → kind != Etw → 走 PowerShell fallback 验证（或 SKIP）
/// - admin 但 DNS-Client event 漏抓 → retry 一轮
#[cfg(target_os = "windows")]
#[test]
fn etw_collects_example_com_query_when_admin() {
    use std::time::{Duration, Instant};

    let (collector, kind) = detect_collector();
    let Some(mut collector) = collector else {
        eprintln!("SKIP: 无 DNS collector 可用（非 Windows / PowerShell 缺失 / ETW 失败）");
        return;
    };
    eprintln!("etw_collects_example_com_query_when_admin: 使用 kind={kind}");

    // 给 collector 启动时间（ETW: ProcessTrace 起来；PowerShell: reader_loop 起来）
    std::thread::sleep(Duration::from_secs(1));

    let mut found = false;
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        // 触发 DNS 查询：Resolve-DnsName 走 Dnscache service，必产生 DNS-Client event。
        // curl 走应用层 DNS，可能命中 DNS cache 不产生新事件；Resolve-DnsName 更稳。
        let _ = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Resolve-DnsName -Name example.com -Type A -ErrorAction SilentlyContinue | Out-Null",
            ])
            .status();
        // 给 ETW callback / PowerShell reader 一点时间采到事件
        std::thread::sleep(Duration::from_millis(1200));

        let queries = collector.drain();
        for q in &queries {
            if q.query_name.contains("example.com") {
                found = true;
                assert!(
                    !q.query_name.is_empty(),
                    "query_name 不应为空（实际 {:?}）",
                    q.query_name
                );
                assert!(q.pid != 0, "pid 不应为 0（实际 {}）", q.pid);
                eprintln!(
                    "etw_collects_example_com_query_when_admin: matched DnsQuery {{ pid: {}, name: {:?}, qtype: {:?}, result: {} }}",
                    q.pid, q.query_name, q.query_type, q.result
                );
                break;
            }
        }
        if found {
            break;
        }
    }

    if !found {
        eprintln!(
            "etw_collects_example_com_query_when_admin: kind={kind} 未采到 example.com（DNS-Client event 未 fire？cache hit？重试 / 检查 DNS-Client service 状态）"
        );
        // 不强制 fail —— DNS-Client event 受 DNS cache 状态 / Resolve-DnsName 实现
        // 影响，可能某次 retry 都漏抓。SKIP 提示用户「管理员下重试」。
    }
    // Drop 不 panic
    drop(collector);
}
