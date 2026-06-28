//! v0.7.0 阶段 4：FilterExpr 集成测试（ADR-0011）。
//!
//! 覆盖：
//! - 合法表达式（字段比较 / 布尔组合 / 正则 / NOT + 括号 / 单位 / 百分比）
//! - 非法表达式（`>>` / 缺值 / 括号不匹配 / 未知字段 / 未闭合正则）
//! - substring ↔ FilterExpr 模式切换（通过 SearchState）
//! - parse 失败时保留上一次成功 AST（cached_sorted 不破坏）

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proc::collect::ProcessInfo;
use proc::filter::{EvalCtx, Field, FilterExpr, Value, parse};
use proc::search::SearchState;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn make_proc(name: &str, cpu: f32, mem_bytes: u64, pid: u32) -> ProcessInfo {
    ProcessInfo {
        name: std::sync::Arc::from(name),
        name_lower: std::sync::Arc::from(name.to_lowercase()),
        cpu_usage: cpu,
        memory: mem_bytes,
        pid,
        ..ProcessInfo::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn make_proc_full(
    name: &str,
    cpu: f32,
    mem_bytes: u64,
    pid: u32,
    cmd: &[&str],
    disk_read: u64,
    disk_write: u64,
    net_sent: u64,
    net_recv: u64,
) -> ProcessInfo {
    let cmd_vec: Vec<String> = cmd.iter().map(|s| (*s).to_string()).collect();
    ProcessInfo {
        cmd: std::sync::Arc::from(cmd_vec),
        disk_read_speed: disk_read,
        disk_write_speed: disk_write,
        net_sent_rate: net_sent,
        net_recv_rate: net_recv,
        ..make_proc(name, cpu, mem_bytes, pid)
    }
}

fn ctx<'a>(p: &'a ProcessInfo, score: Option<u32>) -> EvalCtx<'a> {
    EvalCtx {
        process: p,
        security_score: score,
    }
}

// ===== 合法表达式：apply 正确过滤 =====

#[test]
fn cpu_gt_filters_low_cpu() {
    let expr = parse("cpu > 5").unwrap();
    let hot = make_proc("hot", 10.0, 0, 1);
    let cold = make_proc("cold", 1.0, 0, 2);
    assert!(expr.apply(&ctx(&hot, None)));
    assert!(!expr.apply(&ctx(&cold, None)));
}

#[test]
fn cpu_percent_unit_equivalent_to_bare_number() {
    // 5 和 5% 在 cpu 字段上应等价（cpu_usage 本身就是 %）。
    let bare = parse("cpu > 5").unwrap();
    let pct = parse("cpu > 5%").unwrap();
    let p = make_proc("x", 10.0, 0, 1);
    assert_eq!(bare.apply(&ctx(&p, None)), pct.apply(&ctx(&p, None)));
}

#[test]
fn mem_with_mb_unit() {
    let expr = parse("mem > 100mb").unwrap();
    let big = make_proc("big", 0.0, 200 * 1024 * 1024, 1);
    let small = make_proc("small", 0.0, 50 * 1024 * 1024, 2);
    assert!(expr.apply(&ctx(&big, None)));
    assert!(!expr.apply(&ctx(&small, None)));
}

#[test]
fn mem_with_gb_unit() {
    let expr = parse("mem > 1gb").unwrap();
    let big = make_proc("big", 0.0, 2 * 1024 * 1024 * 1024, 1);
    assert!(expr.apply(&ctx(&big, None)));
}

#[test]
fn and_combinator_two_conditions() {
    let expr = parse("cpu > 5 AND mem > 100mb").unwrap();
    let both = make_proc("both", 10.0, 200 * 1024 * 1024, 1);
    let only_cpu = make_proc("only_cpu", 10.0, 50 * 1024 * 1024, 2);
    let only_mem = make_proc("only_mem", 1.0, 200 * 1024 * 1024, 3);
    assert!(expr.apply(&ctx(&both, None)));
    assert!(!expr.apply(&ctx(&only_cpu, None)));
    assert!(!expr.apply(&ctx(&only_mem, None)));
}

#[test]
fn or_combinator_either_condition() {
    let expr = parse("cpu > 5 OR mem > 100mb").unwrap();
    let only_cpu = make_proc("only_cpu", 10.0, 50 * 1024 * 1024, 1);
    let only_mem = make_proc("only_mem", 1.0, 200 * 1024 * 1024, 2);
    let neither = make_proc("neither", 1.0, 50 * 1024 * 1024, 3);
    assert!(expr.apply(&ctx(&only_cpu, None)));
    assert!(expr.apply(&ctx(&only_mem, None)));
    assert!(!expr.apply(&ctx(&neither, None)));
}

#[test]
fn not_with_parens_complex() {
    let expr = parse("NOT (cpu > 5 AND mem > 100mb)").unwrap();
    // 进程 cpu 低 + mem 低 → 内部 AND = false → NOT false = true
    let cool = make_proc("cool", 1.0, 50 * 1024 * 1024, 1);
    assert!(expr.apply(&ctx(&cool, None)));
    // 进程 cpu 高 + mem 高 → 内部 AND = true → NOT true = false
    let hot = make_proc("hot", 10.0, 200 * 1024 * 1024, 2);
    assert!(!expr.apply(&ctx(&hot, None)));
}

#[test]
fn regex_name_case_insensitive() {
    // 大写 CHROME 应匹配小写进程名 chrome.exe
    let expr = parse("name =~ /CHROME/i").unwrap();
    let chrome = make_proc("chrome.exe", 0.0, 0, 1);
    let firefox = make_proc("firefox.exe", 0.0, 0, 2);
    assert!(expr.apply(&ctx(&chrome, None)));
    assert!(!expr.apply(&ctx(&firefox, None)));
}

#[test]
fn regex_name_case_sensitive() {
    // 不带 i flag：CHROME 不会匹配 chrome.exe
    let expr = parse("name =~ /CHROME/").unwrap();
    let chrome = make_proc("chrome.exe", 0.0, 0, 1);
    assert!(!expr.apply(&ctx(&chrome, None)));
}

#[test]
fn regex_cmd_substring() {
    let expr = parse("cmd =~ /--headless/").unwrap();
    let p = make_proc_full(
        "chrome",
        0.0,
        0,
        1,
        &["chrome", "--headless", "--disable-gpu"],
        0,
        0,
        0,
        0,
    );
    assert!(expr.apply(&ctx(&p, None)));
}

#[test]
fn pid_equality() {
    let expr = parse("pid = 1234").unwrap();
    let target = make_proc("x", 0.0, 0, 1234);
    let other = make_proc("y", 0.0, 0, 5678);
    assert!(expr.apply(&ctx(&target, None)));
    assert!(!expr.apply(&ctx(&other, None)));
}

#[test]
fn security_score_filter() {
    let expr = parse("security_score < 80").unwrap();
    let risky = make_proc("risky", 0.0, 0, 1);
    let safe = make_proc("safe", 0.0, 0, 2);
    // security_score 通过 EvalCtx 传入（模拟 App::security_scores lookup）
    assert!(expr.apply(&ctx(&risky, Some(50))));
    assert!(!expr.apply(&ctx(&safe, Some(100))));
}

#[test]
fn disk_read_with_kb_unit() {
    let expr = parse("disk_read > 100kb").unwrap();
    let busy = make_proc_full("busy", 0.0, 0, 1, &[], 200_000, 0, 0, 0);
    let idle = make_proc_full("idle", 0.0, 0, 2, &[], 1_000, 0, 0, 0);
    assert!(expr.apply(&ctx(&busy, None)));
    assert!(!expr.apply(&ctx(&idle, None)));
}

#[test]
fn complex_real_world_expression() {
    // 典型 bottom 式查询：cpu 高 OR mem 大，且不是 root
    let expr = parse("(cpu > 50 OR mem > 500mb) AND NOT user = root").unwrap();
    let hot = make_proc("miner", 80.0, 100 * 1024 * 1024, 1);
    let big = make_proc("database", 5.0, 800 * 1024 * 1024, 2);
    let cool = make_proc("idle", 1.0, 50 * 1024 * 1024, 3);
    assert!(expr.apply(&ctx(&hot, None)));
    assert!(expr.apply(&ctx(&big, None)));
    assert!(!expr.apply(&ctx(&cool, None)));
}

// ===== 非法表达式：parse 失败 + 错误信息友好 =====

#[test]
fn err_double_operator() {
    let e = parse("cpu >> 5").unwrap_err();
    assert!(!e.msg.is_empty());
}

#[test]
fn err_missing_value() {
    assert!(parse("cpu >").is_err());
}

#[test]
fn err_unbalanced_open_paren() {
    assert!(parse("(cpu > 5").is_err());
}

#[test]
fn err_unbalanced_close_paren() {
    assert!(parse("cpu > 5)").is_err());
}

#[test]
fn err_unknown_field() {
    let e = parse("foo > 5").unwrap_err();
    // 错误信息能让人定位问题
    assert!(!e.msg.is_empty());
}

#[test]
fn err_unterminated_regex() {
    assert!(parse("name =~ /chrome").is_err());
}

#[test]
fn err_lowercase_keyword_not_recognized() {
    // 关键字大小写敏感：`and` 不是 `AND`。
    // 整表达式 `cpu > 5 and mem > 100` 会被解析为 `cpu > 5`，然后 `and mem > 100` 是 trailing。
    assert!(parse("cpu > 5 and mem > 100").is_err());
}

#[test]
fn err_keyword_word_boundary() {
    // ORANGE 不应被识别为 OR 前缀。
    // 注意：name = ORANGE 是合法（ORANGE 是 bare string），所以这里用一个会产生
    // 错误的边界 case。
    assert!(parse("cpu > 5 ORANGE").is_err());
}

#[test]
fn err_empty_input() {
    assert!(parse("").is_err());
}

// ===== substring ↔ FilterExpr 模式切换 =====

#[test]
fn search_state_default_is_substring() {
    let s = SearchState::new();
    assert_eq!(s.mode, proc::search::QueryMode::Substring);
}

#[test]
fn search_state_activate_substring_via_slash_pattern() {
    // TUI 按 `/` 时面板直接 `self.search.active = true`，模式默认就是 Substring。
    let mut s = SearchState::new();
    s.active = true;
    assert_eq!(s.mode, proc::search::QueryMode::Substring);
}

#[test]
fn search_state_activate_filter_expr_via_colon() {
    let mut s = SearchState::new();
    s.activate_filter_expr();
    assert_eq!(s.mode, proc::search::QueryMode::FilterExpr);
    assert!(s.is_active());
    assert!(s.filter_expr.is_none());
    assert!(s.filter_error.is_none());
}

#[test]
fn search_state_filter_expr_parses_on_input() {
    let mut s = SearchState::new();
    s.activate_filter_expr();
    for c in "cpu > 5".chars() {
        s.handle_input(key(KeyCode::Char(c)));
    }
    assert!(s.filter_expr.is_some(), "expected parsed AST");
    assert!(s.filter_error.is_none(), "expected no parse error");
}

#[test]
fn search_state_filter_expr_error_on_bad_input() {
    let mut s = SearchState::new();
    s.activate_filter_expr();
    // 输入到一半（`cpu >`）应触发 parse error
    for c in "cpu >".chars() {
        s.handle_input(key(KeyCode::Char(c)));
    }
    assert!(s.filter_error.is_some(), "expected error mid-input");
    // 没有先前成功 AST，filter_expr 保持 None
    assert!(s.filter_expr.is_none());
}

#[test]
fn search_state_keeps_prev_ast_on_parse_error() {
    // 关键场景（cached_sorted 不破坏）：用户输完整 AST 后再打字符让 parse 失败，
    // filter_expr 保留上一次成功值，让 cache 继续按旧 AST 过滤。
    let mut s = SearchState::new();
    s.activate_filter_expr();
    for c in "cpu > 5".chars() {
        s.handle_input(key(KeyCode::Char(c)));
    }
    assert!(s.filter_expr.is_some());

    // 打一个破坏 parse 的字符（右括号多余）
    s.handle_input(key(KeyCode::Char(')')));
    assert!(s.filter_error.is_some());
    // FilterExpr 内含 regex::Regex 无法 PartialEq，靠 is_some + apply 间接验证。
    // expr 仍是 `cpu > 5`，cpu=10.0 应匹配；若被清空 apply 会 panic（None unwrap）。
    let expr = s.filter_expr.as_ref().expect("prev AST must be retained");
    let hot = make_proc("hot", 10.0, 0, 1);
    assert!(expr.apply(&ctx(&hot, None)));
}

#[test]
fn search_state_esc_resets_to_substring() {
    let mut s = SearchState::new();
    s.activate_filter_expr();
    for c in "cpu > 5".chars() {
        s.handle_input(key(KeyCode::Char(c)));
    }
    s.handle_input(key(KeyCode::Esc));
    assert_eq!(s.mode, proc::search::QueryMode::Substring);
    assert!(s.query.is_empty());
    assert!(s.filter_expr.is_none());
    assert!(s.filter_error.is_none());
    assert!(!s.is_active());
}

#[test]
fn search_state_substring_path_still_works() {
    // v0.6 行为 100% 保留：substring 模式下 query/query_lower 同步增量。
    let mut s = SearchState::new();
    s.active = true;
    for c in "Chrome".chars() {
        s.handle_input(key(KeyCode::Char(c)));
    }
    assert_eq!(s.query(), "Chrome");
    assert_eq!(s.query_lower(), "chrome");
    assert_eq!(s.mode, proc::search::QueryMode::Substring);
    assert!(s.filter_expr.is_none());
}

// ===== 应用层：FilterExpr 边界场景 =====

#[test]
fn apply_to_field_mismatch_returns_false() {
    // 数值字段对文本字面量 → 类型不匹配 → false（不 panic）
    let expr = parse("cpu > 5").unwrap();
    let p = make_proc("x", 10.0, 0, 1);
    assert!(expr.apply(&ctx(&p, None)));

    // 手动构造一个类型不匹配的 AST（用户写不出，但单测覆盖）
    let bad_expr = FilterExpr::FieldCmp {
        field: Field::Cpu,
        op: proc::filter::CmpOp::Gt,
        value: Value::Text("chrome".to_string()),
    };
    assert!(!bad_expr.apply(&ctx(&p, None)));
}

#[test]
fn parse_error_carries_position_for_ui() {
    // UI 需要 position 来渲染「⚠ Filter syntax error at position N: ...」
    let e = parse("cpu >").unwrap_err();
    // position 至少大于 0（错在 input 末尾）
    assert!(e.position > 0);
}

#[test]
fn whitespace_in_expressions_is_flexible() {
    // 用户可能写紧凑 `cpu>5` 或松散 `cpu   >   5`，都应接受。
    assert!(parse("cpu>5").is_ok());
    assert!(parse("cpu   >   5").is_ok());
    assert!(parse("cpu > 5 AND  mem > 100").is_ok());
}
