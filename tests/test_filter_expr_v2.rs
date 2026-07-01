//! v0.11.0 阶段 3：FilterExpr v2 网络字段集成测试（ADR-0011 v0.11 阶段 3 增量段）。
//!
//! 覆盖 v0.10 落地的 `ProcessFlow.sni` / `dns_name` / `remote_addr` 字段接入
//! FilterExpr AST 后的端到端契约：
//!
//! - Parser 解析网络字段表达式（`sni =~ /google\.com$/` / `dns_name = "x"` /
//!   `remote_addr in (...)` / `remote_port = 443` / `source = schannel`）。
//! - `FilterExpr::apply_network` 在 mock ProcessFlow 上正确判定。
//! - 网络字段 + process 字段混合（AND / OR / NOT）的组合语义。
//! - 旧表达式（`cpu > 5 AND name =~ /chrome/`）仍能解析（v0.7/v0.8 契约不破）。
//! - PortPanel `flow_filtered_indices` 在 FilterExpr 模式下正确收窄。
//! - Parser 错误中文化：未知字段提示含「未知」+ 字段支持列表。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proc::ebpf::flow::{FlowSource, ProcessFlow};
use proc::filter::{CmpOp, FilterExpr, NetworkEvalCtx, NetworkField, Value, parse};
use proc::view_models::PortPanel;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn mk_flow(
    pid: u32,
    sni: Option<&str>,
    dns: Option<&str>,
    addr: &str,
    port: u16,
    source: FlowSource,
) -> ProcessFlow {
    ProcessFlow {
        pid,
        start_time: 0,
        comm: format!("proc{pid}"),
        local_addr: String::new(),
        remote_addr: addr.into(),
        remote_port: port,
        bytes_out: 0,
        bytes_in: 0,
        dns_name: dns.map(str::to_string),
        sni: sni.map(str::to_string),
        source,
        first_seen: std::time::SystemTime::UNIX_EPOCH,
        last_seen: std::time::SystemTime::UNIX_EPOCH,
        exit_time: None,
    }
}

fn nctx<'a>(f: &'a ProcessFlow) -> NetworkEvalCtx<'a> {
    NetworkEvalCtx { flow: f }
}

// --- Parser：网络字段语法 ---

#[test]
fn parse_sni_regex_yields_network_regex() {
    let e = parse(r"sni =~ /google\.com$/").unwrap();
    match e {
        FilterExpr::NetworkRegex {
            field: NetworkField::Sni,
            ..
        } => {}
        other => panic!("expected NetworkRegex(Sni), got {:?}", other),
    }
}

#[test]
fn parse_dns_name_eq_yields_network_field_cmp() {
    let e = parse(r#"dns_name = "example.com""#).unwrap();
    match e {
        FilterExpr::NetworkFieldCmp {
            field: NetworkField::DnsName,
            op: CmpOp::Eq,
            value: Value::Text(s),
        } => {
            assert_eq!(s, "example.com");
        }
        other => panic!("expected NetworkFieldCmp(DnsName), got {:?}", other),
    }
}

#[test]
fn parse_remote_addr_in_yields_network_in() {
    let e = parse(r#"remote_addr in ("1.2.3.4", "5.6.7.8")"#).unwrap();
    match e {
        FilterExpr::NetworkIn {
            field: NetworkField::RemoteAddr,
            values,
        } => {
            assert_eq!(values.len(), 2);
        }
        other => panic!("expected NetworkIn(RemoteAddr), got {:?}", other),
    }
}

#[test]
fn parse_remote_port_eq_value_number() {
    let e = parse("remote_port = 443").unwrap();
    match e {
        FilterExpr::NetworkFieldCmp {
            field: NetworkField::RemotePort,
            op: CmpOp::Eq,
            value: Value::Number(n),
        } => {
            assert!((n - 443.0).abs() < 1e-9);
        }
        other => panic!("expected NetworkFieldCmp(RemotePort), got {:?}", other),
    }
}

#[test]
fn parse_source_eq_schannel_bare_string() {
    let e = parse("source = schannel").unwrap();
    match e {
        FilterExpr::NetworkFieldCmp {
            field: NetworkField::Source,
            op: CmpOp::Eq,
            value: Value::Text(s),
        } => assert_eq!(s, "schannel"),
        other => panic!("expected NetworkFieldCmp(Source), got {:?}", other),
    }
}

// --- Parser：错误中文化 ---

#[test]
fn err_unknown_field_message_lists_supported_fields() {
    // 未知字段错误信息应含「未知」+ 支持列表（含 cpu + sni）。
    let e = parse("unknown_field =~ /x/").unwrap_err();
    assert!(e.msg.contains("未知"), "msg: {}", e.msg);
    assert!(e.msg.contains("cpu"), "msg should list cpu: {}", e.msg);
    assert!(e.msg.contains("sni"), "msg should list sni: {}", e.msg);
    assert!(
        !e.msg.contains("AlphaNumeric"),
        "msg leaked nom kind: {}",
        e.msg
    );
}

// --- Parser：旧表达式兼容（v0.7/v0.8 契约不破） ---

#[test]
fn backward_compat_process_only_expr_still_parses() {
    let e = parse("cpu > 5 AND name =~ /chrome/").unwrap();
    assert!(matches!(e, FilterExpr::And(_, _)));
}

#[test]
fn backward_compat_mixed_process_and_network_expr_parses() {
    // 网络字段 + process 字段混合表达式（apply_network 时 process 分支返 false，
    // 但 parse 必须不挂）。
    let e = parse(r#"sni =~ /evil\.com/ OR cpu > 50"#).unwrap();
    assert!(matches!(e, FilterExpr::Or(_, _)));
}

// --- apply_network：mock flow 判定 ---

#[test]
fn apply_network_sni_regex_matches() {
    let f = mk_flow(1, Some("evil.com"), None, "1.2.3.4", 443, FlowSource::Ebpf);
    let e = parse(r"sni =~ /evil\.com/").unwrap();
    assert!(e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_sni_regex_misses_when_sni_none() {
    // sni = None → Text("") → 与 `evil\.com` 不匹配。
    let f = mk_flow(2, None, None, "1.2.3.4", 443, FlowSource::Ebpf);
    let e = parse(r"sni =~ /evil\.com/").unwrap();
    assert!(!e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_dns_name_eq_matches() {
    let f = mk_flow(
        3,
        None,
        Some("example.com"),
        "1.2.3.4",
        443,
        FlowSource::Ebpf,
    );
    let e = parse(r#"dns_name = "example.com""#).unwrap();
    assert!(e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_remote_addr_in_matches_member() {
    let f = mk_flow(4, None, None, "5.6.7.8", 80, FlowSource::Ebpf);
    let e = parse(r#"remote_addr in ("1.2.3.4", "5.6.7.8")"#).unwrap();
    assert!(e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_remote_addr_in_misses_non_member() {
    let f = mk_flow(5, None, None, "9.9.9.9", 80, FlowSource::Ebpf);
    let e = parse(r#"remote_addr in ("1.2.3.4", "5.6.7.8")"#).unwrap();
    assert!(!e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_source_eq_schannel_matches() {
    let f = mk_flow(6, Some("a.com"), None, "", 0, FlowSource::Schannel);
    let e = parse("source = schannel").unwrap();
    assert!(e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_source_eq_schannel_misses_for_ebpf() {
    let f = mk_flow(7, None, Some("x"), "1.1.1.1", 53, FlowSource::Ebpf);
    let e = parse("source = schannel").unwrap();
    assert!(!e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_remote_port_eq_matches() {
    let f = mk_flow(8, None, None, "1.1.1.1", 443, FlowSource::Ebpf);
    let e = parse("remote_port = 443").unwrap();
    assert!(e.apply_network(&nctx(&f)));
}

#[test]
fn apply_network_and_combinator() {
    // sni 命中 evil.com 且端口 443 → match；只命中一个 → 不 match。
    let e = parse(r"sni =~ /evil\.com/ AND remote_port = 443").unwrap();
    let matched = mk_flow(9, Some("evil.com"), None, "1.2.3.4", 443, FlowSource::Ebpf);
    let missed_port = mk_flow(10, Some("evil.com"), None, "1.2.3.4", 80, FlowSource::Ebpf);
    let missed_sni = mk_flow(11, Some("ok.com"), None, "1.2.3.4", 443, FlowSource::Ebpf);
    assert!(e.apply_network(&nctx(&matched)));
    assert!(!e.apply_network(&nctx(&missed_port)));
    assert!(!e.apply_network(&nctx(&missed_sni)));
}

#[test]
fn apply_network_not_combinator() {
    // NOT sni =~ /evil/ → 所有非 evil 的 flow 都命中。
    let e = parse(r"NOT sni =~ /evil/").unwrap();
    let clean = mk_flow(12, Some("ok.com"), None, "1.1.1.1", 80, FlowSource::Ebpf);
    let evil = mk_flow(13, Some("evil.com"), None, "1.1.1.1", 80, FlowSource::Ebpf);
    assert!(e.apply_network(&nctx(&clean)));
    assert!(!e.apply_network(&nctx(&evil)));
}

// --- Integration：PortPanel flow_filtered_indices（FilterExpr 模式） ---

fn activate_filter_and_type(panel: &mut PortPanel, query: &str) {
    // 模拟用户按 `:` 进入 FilterExpr 模式 + 逐字符输入。
    // handle_key 走 panel.handle_key 路径，但 PortPanel 是 pub，可直接调 SearchState。
    panel.flow_search.activate_filter_expr();
    for c in query.chars() {
        panel.flow_search.handle_input(key(KeyCode::Char(c)));
    }
}

#[test]
fn flow_filtered_indices_substring_matches_sni_or_comm() {
    let mut panel = PortPanel::new();
    let flows = vec![
        mk_flow(
            100,
            Some("evil.com"),
            None,
            "1.2.3.4",
            443,
            FlowSource::Ebpf,
        ),
        mk_flow(200, Some("ok.com"), None, "5.6.7.8", 80, FlowSource::Ebpf),
        mk_flow(300, None, None, "9.9.9.9", 53, FlowSource::Ebpf),
    ];

    // Substring: 默认 SearchState 模式。手动激活 + 输入。
    panel.flow_search.activate_substring();
    for c in "evil".chars() {
        panel.flow_search.handle_input(key(KeyCode::Char(c)));
    }
    let idx = panel.flow_filtered_indices(&flows);
    assert_eq!(idx, vec![0], "substring 应只命中 evil.com");

    // comm 也应能命中：proc200 含 "200"
    panel.flow_search.clear();
    panel.flow_search.activate_substring();
    for c in "200".chars() {
        panel.flow_search.handle_input(key(KeyCode::Char(c)));
    }
    let idx = panel.flow_filtered_indices(&flows);
    assert_eq!(idx, vec![1], "substring 应命中 comm=proc200");
}

#[test]
fn flow_filtered_indices_filter_expr_sni_regex() {
    let mut panel = PortPanel::new();
    let flows = vec![
        mk_flow(1, Some("evil.com"), None, "1.1.1.1", 443, FlowSource::Ebpf),
        mk_flow(
            2,
            Some("api.google.com"),
            None,
            "2.2.2.2",
            443,
            FlowSource::Ebpf,
        ),
        mk_flow(3, Some("ok.com"), None, "3.3.3.3", 80, FlowSource::Ebpf),
    ];

    activate_filter_and_type(&mut panel, r"sni =~ /\.com$/");
    let idx = panel.flow_filtered_indices(&flows);
    assert_eq!(idx.len(), 3, "三条 sni 都以 .com 结尾");

    // 收窄到 evil
    panel.flow_search.clear();
    activate_filter_and_type(&mut panel, r"sni =~ /evil/");
    let idx = panel.flow_filtered_indices(&flows);
    assert_eq!(idx, vec![0]);
}

#[test]
fn flow_filtered_indices_filter_expr_in_operator() {
    let mut panel = PortPanel::new();
    let flows = vec![
        mk_flow(1, None, None, "1.2.3.4", 443, FlowSource::Ebpf),
        mk_flow(2, None, None, "5.6.7.8", 443, FlowSource::Ebpf),
        mk_flow(3, None, None, "9.9.9.9", 443, FlowSource::Ebpf),
    ];

    activate_filter_and_type(&mut panel, r#"remote_addr in ("1.2.3.4", "5.6.7.8")"#);
    let idx = panel.flow_filtered_indices(&flows);
    assert_eq!(idx, vec![0, 1]);
}

#[test]
fn flow_filtered_indices_filter_expr_source_schannel() {
    let mut panel = PortPanel::new();
    let flows = vec![
        mk_flow(1, Some("a.com"), None, "", 0, FlowSource::Schannel),
        mk_flow(2, None, Some("x"), "1.1.1.1", 53, FlowSource::Ebpf),
        mk_flow(3, Some("b.com"), None, "", 0, FlowSource::Schannel),
    ];

    activate_filter_and_type(&mut panel, "source = schannel");
    let idx = panel.flow_filtered_indices(&flows);
    assert_eq!(idx, vec![0, 2]);
}

#[test]
fn flow_filtered_indices_filter_expr_parse_error_keeps_prev_ast() {
    // parse 错误时保留上一次成功 AST（与 List view 同款契约）。
    let mut panel = PortPanel::new();
    let flows = vec![
        mk_flow(1, Some("evil.com"), None, "1.1.1.1", 443, FlowSource::Ebpf),
        mk_flow(2, Some("ok.com"), None, "2.2.2.2", 80, FlowSource::Ebpf),
    ];

    // 先打合法表达式：sni =~ /evil/ → 命中 flow[0]
    activate_filter_and_type(&mut panel, r"sni =~ /evil/");
    assert_eq!(panel.flow_filtered_indices(&flows), vec![0]);

    // 接着打坏字符 `)` → parse 失败，但保留先前 AST。
    panel.flow_search.handle_input(key(KeyCode::Char(')')));
    assert!(panel.flow_search.filter_error.is_some(), "应有 parse 错误");
    assert!(
        panel.flow_search.filter_expr.is_some(),
        "parse 失败时保留先前 AST"
    );
    // 仍按 sni =~ /evil/ 过滤
    assert_eq!(panel.flow_filtered_indices(&flows), vec![0]);
}

#[test]
fn flow_filtered_indices_empty_query_returns_all() {
    let mut panel = PortPanel::new();
    let flows = vec![
        mk_flow(1, Some("a.com"), None, "1.1.1.1", 80, FlowSource::Ebpf),
        mk_flow(2, Some("b.com"), None, "2.2.2.2", 80, FlowSource::Ebpf),
    ];

    // 空 query（FilterExpr 模式但还没输入字符） → 返回全部。
    panel.flow_search.activate_filter_expr();
    assert_eq!(panel.flow_filtered_indices(&flows), vec![0, 1]);

    // Substring 模式同样
    panel.flow_search.clear();
    panel.flow_search.activate_substring();
    assert_eq!(panel.flow_filtered_indices(&flows), vec![0, 1]);
}

#[test]
fn flow_filtered_indices_process_field_returns_all_flows() {
    // process 字段表达式在 NetworkEvalCtx 下 apply 返 false（无 ProcessInfo）。
    // FilterExpr 模式下用 process 字段过滤 → 所有 flow 都不命中。
    let mut panel = PortPanel::new();
    let flows = vec![
        mk_flow(1, Some("a.com"), None, "1.1.1.1", 80, FlowSource::Ebpf),
        mk_flow(2, Some("b.com"), None, "2.2.2.2", 80, FlowSource::Ebpf),
    ];

    activate_filter_and_type(&mut panel, "cpu > 5");
    let idx = panel.flow_filtered_indices(&flows);
    assert!(
        idx.is_empty(),
        "process-字段表达式在 flow ctx 下应无命中，got {:?}",
        idx
    );
}
