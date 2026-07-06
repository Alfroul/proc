//! v0.14 stage 3 测试：时间轴搜索（FilterExpr 扩 FrameField 5 维度 +
//! timeline 高亮 + n/N 跳转）。

use proc::filter::{CmpOp, FieldValue, FilterExpr, FrameEvalCtx, FrameField, parse_frame};
use proc::record::frame::{FrameAnomaly, FrameConnectionDiff, FrameNav, FrameProcess, UiFrame};

fn make_frame(idx: usize, cpu: f32, mem: u64, names: &[&str], sev: &str) -> UiFrame {
    let processes = names
        .iter()
        .map(|n| FrameProcess {
            pid: 1,
            name: (*n).to_string(),
            cpu: 0.0,
            memory: 0,
            disk_read: 0,
            disk_write: 0,
        })
        .collect();
    let anomalies = if sev.is_empty() {
        Vec::new()
    } else {
        vec![FrameAnomaly {
            rule_id: "test".to_string(),
            severity: sev.to_string(),
            title: "test anomaly".to_string(),
            detail: String::new(),
            affected_pid: None,
            affected_ip: None,
        }]
    };
    UiFrame {
        timestamp: 1_000_000 + idx as u64,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: cpu,
        memory_used: mem,
        memory_total: 8 * 1024 * 1024 * 1024,
        net_down: 0,
        net_up: 0,
        cpu_history: Vec::new(),
        mem_history: Vec::new(),
        processes,
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: Vec::new(),
        port_entries: Vec::new(),
        port_view_mode: 0,
        port_process_groups: Vec::new(),
        port_remote_groups: Vec::new(),
        connection_diff: FrameConnectionDiff::default(),
        anomalies,
        usb_devices: Vec::new(),
        usb_locks: Vec::new(),
        monitors: Vec::new(),
        docker_containers: Vec::new(),
        docker_events: Vec::new(),
        ops: Vec::new(),
        nav: FrameNav::default(),
    }
}

// --- parser ---

#[test]
fn parse_frame_cpu_threshold() {
    let e = parse_frame("cpu > 80").unwrap();
    match e {
        FilterExpr::FrameFieldCmp { field, op, value } => {
            assert_eq!(field, FrameField::Cpu);
            assert_eq!(op, CmpOp::Gt);
            if let proc::filter::Value::Number(n) = value {
                assert!((n - 80.0).abs() < 1e-9);
            } else {
                panic!("expected Number value");
            }
        }
        other => panic!("expected FrameFieldCmp, got {other:?}"),
    }
}

#[test]
fn parse_frame_timestamp() {
    let e = parse_frame("timestamp > 1234567890").unwrap();
    assert!(matches!(
        e,
        FilterExpr::FrameFieldCmp {
            field: FrameField::Timestamp,
            ..
        }
    ));
}

#[test]
fn parse_frame_ts_alias() {
    // `ts` 是 `timestamp` 的别名
    let e = parse_frame("ts > 100").unwrap();
    assert!(matches!(
        e,
        FilterExpr::FrameFieldCmp {
            field: FrameField::Timestamp,
            ..
        }
    ));
}

#[test]
fn parse_frame_name_regex() {
    let e = parse_frame("name =~ /chrome/i").unwrap();
    match e {
        FilterExpr::FrameRegex {
            field,
            case_insensitive,
            ..
        } => {
            assert_eq!(field, FrameField::Name);
            assert!(case_insensitive);
        }
        other => panic!("expected FrameRegex, got {other:?}"),
    }
}

#[test]
fn parse_frame_anomaly_severity_eq() {
    let e = parse_frame("anomaly.severity = critical").unwrap();
    assert!(matches!(
        e,
        FilterExpr::FrameFieldCmp {
            field: FrameField::AnomalySeverity,
            ..
        }
    ));
}

#[test]
fn parse_frame_severity_short_alias() {
    // `severity` 也是别名
    let e = parse_frame("severity = warning").unwrap();
    assert!(matches!(
        e,
        FilterExpr::FrameFieldCmp {
            field: FrameField::AnomalySeverity,
            ..
        }
    ));
}

#[test]
fn parse_frame_mem_mb_unit() {
    let e = parse_frame("mem > 500mb").unwrap();
    match e {
        FilterExpr::FrameFieldCmp {
            field: FrameField::Mem,
            value: proc::filter::Value::Number(n),
            ..
        } => {
            assert!((n - 500.0 * 1024.0 * 1024.0).abs() < 1.0);
        }
        other => panic!("expected FrameFieldCmp Mem, got {other:?}"),
    }
}

#[test]
fn parse_frame_unknown_field() {
    assert!(parse_frame("foo > 5").is_err());
}

#[test]
fn parse_frame_combined_with_and() {
    let e = parse_frame("cpu > 50 AND name =~ /chrome/i").unwrap();
    assert!(matches!(e, FilterExpr::And(_, _)));
}

#[test]
fn parse_frame_in_list_for_name() {
    let e = parse_frame(r#"name in ("chrome", "edge", "firefox")"#).unwrap();
    match e {
        FilterExpr::FrameIn { field, values } => {
            assert_eq!(field, FrameField::Name);
            assert_eq!(values.len(), 3);
        }
        other => panic!("expected FrameIn, got {other:?}"),
    }
}

#[test]
fn parse_frame_unknown_field_message_has_frame_fields() {
    // 未知字段错误信息含支持列表（包括 timestamp / anomaly.severity）
    let e = parse_frame("foo > 5").unwrap_err();
    assert!(
        e.msg.contains("timestamp") || e.msg.contains("anomaly"),
        "expected frame field in supported list, got: {}",
        e.msg
    );
}

// --- apply_frame ---

#[test]
fn apply_frame_cpu_high_matches() {
    let frame = make_frame(0, 90.0, 0, &[], "");
    let expr = parse_frame("cpu > 80").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_cpu_low_no_match() {
    let frame = make_frame(0, 10.0, 0, &[], "");
    let expr = parse_frame("cpu > 80").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(!expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_mem_threshold() {
    let frame = make_frame(0, 0.0, 600 * 1024 * 1024, &[], "");
    let expr = parse_frame("mem > 500mb").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_timestamp_filter() {
    // make_frame(5, ...) 产生 ts = 1_000_005；阈值 500_000 让其命中
    let frame = make_frame(5, 0.0, 0, &[], "");
    let expr = parse_frame("timestamp > 500000").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_name_in_processes() {
    let frame = make_frame(0, 0.0, 0, &["explorer", "chrome.exe", "svchost"], "");
    let expr = parse_frame("name =~ /chrome/i").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_name_no_match() {
    let frame = make_frame(0, 0.0, 0, &["explorer", "svchost"], "");
    let expr = parse_frame("name =~ /chrome/i").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(!expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_anomaly_severity_critical() {
    let frame = make_frame(0, 0.0, 0, &[], "critical");
    let expr = parse_frame("anomaly.severity = critical").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_anomaly_severity_no_match() {
    let frame = make_frame(0, 0.0, 0, &[], "info");
    let expr = parse_frame("anomaly.severity = critical").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(!expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_anomaly_severity_in_list() {
    let frame = make_frame(0, 0.0, 0, &[], "warning");
    let expr = parse_frame(r#"anomaly.severity in ("critical", "warning")"#).unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_and_combinator() {
    let frame = make_frame(0, 80.0, 700 * 1024 * 1024, &["chrome"], "");
    let expr = parse_frame("cpu > 50 AND mem > 500mb").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_or_combinator() {
    let frame = make_frame(0, 10.0, 0, &["chrome"], "");
    // cpu 低，但 name 匹配 → OR 命中
    let expr = parse_frame("cpu > 50 OR name =~ /chrome/i").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_not_combinator() {
    let frame = make_frame(0, 10.0, 0, &[], "");
    let expr = parse_frame("NOT cpu > 50").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_text_field_in_with_numeric_value_returns_false() {
    // 类型不匹配：name in (123, 456) — 文本字段 vs 数值字面量 → 不命中
    let frame = make_frame(0, 0.0, 0, &["chrome"], "");
    let expr = parse_frame("name in (123, 456)").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(!expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_contains_frame_field_helper() {
    let expr = parse_frame("cpu > 50").unwrap();
    assert!(expr.contains_frame_field());

    // 与既有 contains_process_field 对比 — Process 模式解析的 cpu 字段不是 frame
    let proc_expr = proc::filter::parse("cpu > 50").unwrap();
    assert!(!proc_expr.contains_frame_field());
}

#[test]
fn apply_frame_empty_processes_with_name_filter() {
    // 空进程列表 + name 过滤 → 不命中（无进程名可匹配）
    let frame = make_frame(0, 0.0, 0, &[], "");
    let expr = parse_frame("name =~ /chrome/i").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(!expr.apply_frame(&ctx));
}

#[test]
fn apply_frame_empty_anomalies_with_severity_filter() {
    // 空异常列表 + severity 过滤 → 不命中
    let frame = make_frame(0, 0.0, 0, &[], "");
    let expr = parse_frame("anomaly.severity = critical").unwrap();
    let ctx = FrameEvalCtx { frame: &frame };
    assert!(!expr.apply_frame(&ctx));
}

// --- build_frame_substring_expr + FrameField::any_match_text ---

#[test]
fn substring_expr_escapes_regex_metachars() {
    // 直接调 build_frame_substring_expr 验证 escape
    let expr = proc::filter::build_frame_substring_expr("chrome.exe").unwrap();
    let frame_chrome_exe = make_frame(0, 0.0, 0, &["chrome.exe"], "");
    let frame_chromexexe = make_frame(1, 0.0, 0, &["chromexexe"], "");
    let ctx1 = FrameEvalCtx {
        frame: &frame_chrome_exe,
    };
    let ctx2 = FrameEvalCtx {
        frame: &frame_chromexexe,
    };
    assert!(expr.apply_frame(&ctx1)); // 字面 chrome.exe 匹配
    assert!(!expr.apply_frame(&ctx2)); // regex 元字符 escape，不匹配 chromexexe
}

#[test]
fn substring_expr_empty_returns_err() {
    assert!(proc::filter::build_frame_substring_expr("").is_err());
    assert!(proc::filter::build_frame_substring_expr("   ").is_err());
}

// --- FrameField 单元 ---

#[test]
fn frame_field_is_text_classification() {
    assert!(FrameField::Name.is_text());
    assert!(FrameField::AnomalySeverity.is_text());
    assert!(!FrameField::Cpu.is_text());
    assert!(!FrameField::Mem.is_text());
    assert!(!FrameField::Timestamp.is_text());
}

#[test]
fn frame_field_extract_first_numeric() {
    let frame = make_frame(42, 75.5, 1024, &[], "");
    assert!(matches!(
        FrameField::Cpu.extract_first(&frame),
        FieldValue::Num(n) if (n - 75.5).abs() < 1e-9
    ));
    assert!(matches!(
        FrameField::Mem.extract_first(&frame),
        FieldValue::Num(n) if n == 1024.0
    ));
    // timestamp = 1_000_000 + 42
    assert!(matches!(
        FrameField::Timestamp.extract_first(&frame),
        FieldValue::Num(n) if n == 1_000_042.0
    ));
}

#[test]
fn frame_field_extract_first_text_first_item() {
    let frame = make_frame(0, 0.0, 0, &["first", "second"], "critical");
    assert!(matches!(
        FrameField::Name.extract_first(&frame),
        FieldValue::Text(s) if s == "first"
    ));
    assert!(matches!(
        FrameField::AnomalySeverity.extract_first(&frame),
        FieldValue::Text(s) if s == "critical"
    ));
}

#[test]
fn frame_field_any_match_text_numeric_returns_false() {
    let frame = make_frame(0, 100.0, 0, &[], "");
    // 数值字段调用 any_match_text 应返 false（不应到达此路径，但 defensive）
    assert!(!FrameField::Cpu.any_match_text(&frame, |_| true));
    assert!(!FrameField::Mem.any_match_text(&frame, |_| true));
    assert!(!FrameField::Timestamp.any_match_text(&frame, |_| true));
}

// --- ReplaySearch 集成（与 controller.rs 内 unit tests 互补，本测试文件
// 主要测 src/replay/search.rs 的对外 API + FilterExpr 端到端）---

use proc::replay::ReplaySearch;

#[test]
fn search_recompute_via_filter_expr() {
    let mut s = ReplaySearch::new();
    for c in ":cpu > 50".chars() {
        s.push_char(c);
    }
    let frames = [
        make_frame(0, 10.0, 0, &[], ""),
        make_frame(1, 80.0, 0, &[], ""),
        make_frame(2, 30.0, 0, &[], ""),
        make_frame(3, 95.0, 0, &[], ""),
    ];
    s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
    assert_eq!(s.matches, vec![1, 3]);
}

#[test]
fn search_substring_via_no_colon_prefix() {
    let mut s = ReplaySearch::new();
    for c in "chrome".chars() {
        s.push_char(c);
    }
    assert!(s.expr.is_some());
    let frames = [
        make_frame(0, 0.0, 0, &["explorer"], ""),
        make_frame(1, 0.0, 0, &["chrome"], ""),
        make_frame(2, 0.0, 0, &["firefox"], ""),
    ];
    s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
    assert_eq!(s.matches, vec![1]);
}

#[test]
fn search_keeps_last_expr_on_parse_error() {
    let mut s = ReplaySearch::new();
    for c in ":cpu > 5".chars() {
        s.push_char(c);
    }
    let expr_before_is_some = s.expr.is_some();
    assert!(expr_before_is_some);
    // 错误输入 ":cpu >" → parse 失败 → 保留 expr_before
    s.pop_char(); // 撤销 "5" → ":cpu >"
    assert!(s.expr.is_some(), "expr 应保留");
    assert!(s.error.is_some(), "error 应被设置");
}

#[test]
fn search_next_prev_navigation_clamps() {
    let mut s = ReplaySearch::new();
    s.matches = vec![5, 10, 15];
    s.cursor = 0;
    assert_eq!(s.next_match(), Some(10));
    assert_eq!(s.next_match(), Some(15));
    assert_eq!(s.next_match(), Some(15)); // clamp 末尾
    assert_eq!(s.prev_match(), Some(10));
    assert_eq!(s.prev_match(), Some(5));
    assert_eq!(s.prev_match(), Some(5)); // clamp 起点
}

#[test]
fn search_reset_clears_all() {
    let mut s = ReplaySearch::new();
    for c in ":cpu > 5".chars() {
        s.push_char(c);
    }
    s.matches = vec![1, 2];
    s.cursor = 1;
    s.reset();
    assert!(s.input.is_empty());
    assert!(s.expr.is_none());
    assert!(s.matches.is_empty());
    assert_eq!(s.cursor, 0);
}

#[test]
fn search_current_match_at_cursor() {
    let mut s = ReplaySearch::new();
    s.matches = vec![10, 20, 30];
    s.cursor = 1;
    assert_eq!(s.current_match(), Some(20));
}

#[test]
fn search_no_expr_recompute_returns_empty() {
    let mut s = ReplaySearch::new();
    // input 为空 → expr None → recompute 应返空 matches
    let frames = [make_frame(0, 100.0, 0, &[], "")];
    s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
    assert!(s.matches.is_empty());
}
