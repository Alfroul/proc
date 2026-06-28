//! v0.10 阶段 3：FlowSource enum + ProcessFlow.source 字段契约测试。
//!
//! 这些测试**跨平台**（不依赖 Windows / Linux / ebpf feature）：
//! - `FlowSource` enum Copy + Default + serde 行为
//! - `ProcessFlow.source` serde round-trip + **旧录屏兼容性**（v0.10 阶段 3
//!   之前的 `.prec` 没有 `source` 字段，反序列化时应得默认值 `Ebpf`，不报错）
//! - R15 跨平台：`sni` 字段（Windows Schannel 路径）也喂白名单检查（v0.10
//!   阶段 3 之前只查 `dns_name`，Linux eBPF 路径专属）
//! - overlay 契约：drain 出 `Vec<SniRecord>` 后，按 pid 匹配 `ProcessFlow`
//!   覆盖 sni + source = Schannel 的纯逻辑

use std::time::{Duration, SystemTime};

use proc::ebpf::flow::{FlowSource, ProcessFlow};
use proc::security::flow::{SniWhitelist, check_flow_risk};

/// 临时白名单 helper：写一行域名到 temp 文件，返回加载后的 SniWhitelist。
/// 与 src/security/flow.rs 内部测试不同——内部测试可访问私有字段 `domains`，
/// 外部测试走 `SniWhitelist::load_from` 公开 API。文件名含 PID + nanos 唯一，
/// 避免并行测试互相 remove。
fn whitelist_from_lines(content: &str) -> SniWhitelist {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "proc-test-flow-source-{pid}-{nanos}-{seq}.txt",
        pid = std::process::id(),
    ));
    std::fs::write(&path, content).expect("write temp whitelist");
    let wl = SniWhitelist::load_from(&path).expect("file exists → Some");
    let _ = std::fs::remove_file(&path);
    wl
}

// ---------- FlowSource enum 契约 ----------

/// `FlowSource` 是 Copy + Default + Eq，且默认 = Ebpf（与 v0.10 阶段 3 之前
/// 所有 flow 都来自 eBPF 路径的事实一致）。
#[test]
fn flow_source_default_is_ebpf() {
    assert_eq!(FlowSource::default(), FlowSource::Ebpf);
}

#[test]
fn flow_source_is_copy_and_eq() {
    let a = FlowSource::Schannel;
    let b = a; // Copy
    assert_eq!(a, b);
    assert_ne!(FlowSource::Ebpf, FlowSource::Schannel);
}

/// serde 序列化用 lowercase rename（"ebpf" / "schannel"），与 v0.10 阶段 3
/// `proc flows --json` 输出契约一致。
#[test]
fn flow_source_serde_lowercase() {
    let ebpf_json = serde_json::to_string(&FlowSource::Ebpf).expect("serialize Ebpf");
    assert_eq!(ebpf_json, "\"ebpf\"");
    let schannel_json = serde_json::to_string(&FlowSource::Schannel).expect("serialize Schannel");
    assert_eq!(schannel_json, "\"schannel\"");
    // round-trip
    let back: FlowSource = serde_json::from_str(&schannel_json).expect("deserialize Schannel");
    assert_eq!(back, FlowSource::Schannel);
}

// ---------- ProcessFlow.source 字段 ----------

fn mk_flow(source: FlowSource, sni: Option<&str>, dns_name: Option<&str>) -> ProcessFlow {
    ProcessFlow {
        pid: 1234,
        start_time: 999,
        comm: "curl".into(),
        local_addr: String::new(),
        remote_addr: "1.2.3.4".into(),
        remote_port: 443,
        bytes_out: 0,
        bytes_in: 0,
        dns_name: dns_name.map(str::to_string),
        sni: sni.map(str::to_string),
        source,
        first_seen: SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
        last_seen: SystemTime::UNIX_EPOCH + Duration::from_secs(1005),
        exit_time: None,
    }
}

/// ProcessFlow 整体 serde round-trip 含 source 字段。
#[test]
fn process_flow_with_source_serde_round_trip() {
    let flow = mk_flow(FlowSource::Schannel, Some("example.com"), None);
    let json = serde_json::to_string(&flow).expect("serialize");
    assert!(json.contains("\"source\":\"schannel\""));
    let back: ProcessFlow = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(flow, back);
    assert_eq!(back.source, FlowSource::Schannel);
}

/// **旧录屏兼容性**：v0.10 阶段 3 之前的 `.prec` 反序列化时，JSON 里没有
/// `source` 字段——`#[serde(default = "default_source")]` 应回退到 `Ebpf`，
/// 不报错，与历史行为一致（旧 flow 全部来自 eBPF 路径）。
#[test]
fn process_flow_source_falls_back_when_missing_in_old_recording() {
    // 手工构造 v0.10 阶段 2（无 source 字段）的 JSON 字面量。
    let old_json = r#"{
        "pid": 1234,
        "start_time": 999,
        "comm": "curl",
        "local_addr": "",
        "remote_addr": "1.2.3.4",
        "remote_port": 443,
        "bytes_out": 0,
        "bytes_in": 0,
        "dns_name": null,
        "sni": null,
        "first_seen": {
            "secs_since_epoch": 1000,
            "nanos_since_epoch": 0
        },
        "last_seen": {
            "secs_since_epoch": 1005,
            "nanos_since_epoch": 0
        },
        "exit_time": null
    }"#;
    let flow: ProcessFlow = serde_json::from_str(old_json).expect("旧录屏应反序列化成功");
    assert_eq!(flow.source, FlowSource::Ebpf, "缺 source 字段应默认 Ebpf");
    assert_eq!(flow.pid, 1234);
}

// ---------- R15 跨平台：sni 喂白名单 ----------

fn mk_flow_for_r15(
    source: FlowSource,
    remote: &str,
    sni: Option<&str>,
    dns_name: Option<&str>,
    last_seen: SystemTime,
) -> ProcessFlow {
    ProcessFlow {
        pid: 1,
        start_time: 100,
        comm: String::new(),
        local_addr: String::new(),
        remote_addr: remote.into(),
        remote_port: 443,
        bytes_out: 0,
        bytes_in: 0,
        dns_name: dns_name.map(str::to_string),
        sni: sni.map(str::to_string),
        source,
        first_seen: last_seen,
        last_seen,
        exit_time: None,
    }
}

/// R15 Windows 路径命中：source = Schannel、sni = "evil.com"、白名单不含 → 扣 30 分。
/// v0.10 阶段 3 跨平台激活：之前只查 dns_name（Windows Schannel 路径 dns_name 永远
/// None → R15 永远不触发），现在 sni 也喂白名单。
#[test]
fn r15_schannel_sni_not_whitelisted_hits() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let flows = [mk_flow_for_r15(
        FlowSource::Schannel,
        "",
        Some("evil.example.com"),
        None,
        now,
    )];
    let refs: Vec<&ProcessFlow> = flows.iter().collect();
    let wl = whitelist_from_lines("good.example.com\n");
    let risk = check_flow_risk(&refs, Some(&wl), now).expect("Schannel SNI 不在白名单应命中");
    assert_eq!(risk.weight, 30);
    assert_eq!(risk.name, "r15_sni_not_whitelisted");
    assert!(risk.description.contains("evil.example.com"));
}

/// R15 Windows 路径放行：source = Schannel、sni 在白名单 → 不扣分。
#[test]
fn r15_schannel_sni_whitelisted_passes() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let flows = [mk_flow_for_r15(
        FlowSource::Schannel,
        "",
        Some("good.example.com"),
        None,
        now,
    )];
    let refs: Vec<&ProcessFlow> = flows.iter().collect();
    let wl = whitelist_from_lines("good.example.com\n");
    assert!(check_flow_risk(&refs, Some(&wl), now).is_none());
}

/// R15 SNI 优先于 dns_name：同时有 sni + dns_name 时，sni 决定命中（与 v0.10
/// 阶段 3 实装 `f.sni.as_deref().or_else(|| f.dns_name.as_deref())` 契约一致）。
#[test]
fn r15_sni_takes_priority_over_dns_name() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let flows = [mk_flow_for_r15(
        FlowSource::Ebpf,
        "1.2.3.4",
        Some("evil.example.com"),
        Some("good.example.com"),
        now,
    )];
    let refs: Vec<&ProcessFlow> = flows.iter().collect();
    let wl = whitelist_from_lines("good.example.com\n");
    let risk =
        check_flow_risk(&refs, Some(&wl), now).expect("sni 优先，dns_name 即使在白名单也命中");
    assert!(risk.description.contains("evil.example.com"));
}

/// R15 Schannel 路径 + 用户 touch 空白名单：所有 sni 视为不在白名单 → 命中扣分。
/// 与 v0.7 阶段 8 Part B 同款契约一致：用户显式创建文件 = 想启用 R15；
/// 空文件 = "所有 SNI 都不在白名单" = 所有有 sni 的外联都命中（用户自负）。
#[test]
fn r15_schannel_with_empty_whitelist_hits() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    let flows = [mk_flow_for_r15(
        FlowSource::Schannel,
        "",
        Some("any.example.com"),
        None,
        now,
    )];
    let refs: Vec<&ProcessFlow> = flows.iter().collect();
    let wl = whitelist_from_lines("# only a comment\n\n");
    let risk = check_flow_risk(&refs, Some(&wl), now).expect("空白名单 + sni 命中");
    assert_eq!(risk.weight, 30);
}

/// R15 跨 source 不影响 condition 2（端口扫描）：Schannel-only flow 的 remote_addr
/// 为空字符串，进入 distinct HashSet 时不贡献新 unique IP（即便贡献，距离阈值 50
/// 也很远，安全）。
#[test]
fn r15_port_scan_threshold_unreachable_with_schannel_only_empty_addrs() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
    // 60 条 Schannel flow，全部 remote_addr 空——distinct 集合只有 {""}，size=1。
    let flows: Vec<ProcessFlow> = (0..60)
        .map(|_| mk_flow_for_r15(FlowSource::Schannel, "", Some("a.com"), None, now))
        .collect();
    let refs: Vec<&ProcessFlow> = flows.iter().collect();
    assert!(
        check_flow_risk(&refs, None, now).is_none(),
        "remote_addr 全空时 distinct.len() = 1 远不及阈值 50"
    );
}
