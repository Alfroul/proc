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
    let e = parse("cpu >").unwrap_err();
    // TD-16：错误信息中文化，不再直出 nom 的 `TakeWhile1`。
    assert!(
        !e.msg.contains("TakeWhile1"),
        "msg should not leak nom ErrorKind: {}",
        e.msg
    );
}

#[test]
fn err_unbalanced_open_paren() {
    let e = parse("(cpu > 5").unwrap_err();
    // 缺 `)` → Char 错误 → 中文「缺少字符（括号/引号/斜杠）」。
    assert!(
        e.msg.contains("括号") || e.msg.contains("字符"),
        "expected 括号/字符 hint, got: {}",
        e.msg
    );
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

// ===== TD-16：错误信息中文化契约 =====
//
// 用户在 TUI / CLI 看到 `filter parse error at offset N: <msg>`，msg 必须是
// 中文友好提示，不得直出 nom 内部 ErrorKind（`TakeWhile1` / `Tag` / ...）。
// 这些 case 锁死映射表的对外契约。

#[test]
fn err_chinese_missing_value_after_cmp() {
    // `cpu >` → take_while1 在 value 位置匹配 0 字符 → 「缺少字段名/值」。
    let e = parse("cpu >").unwrap_err();
    assert!(
        e.msg.contains("缺少"),
        "expected 含「缺少」的提示, got: {}",
        e.msg
    );
    assert!(
        !e.msg.contains("TakeWhile1"),
        "must not leak nom ErrorKind: {}",
        e.msg
    );
}

#[test]
fn err_chinese_missing_value_after_eq() {
    // `name =` 同上：等号后无 value。
    let e = parse("name =").unwrap_err();
    assert!(
        e.msg.contains("缺少"),
        "expected 含「缺少」的提示, got: {}",
        e.msg
    );
}

#[test]
fn err_chinese_unbalanced_paren_hint() {
    // `(cpu > 5` → 缺 `)` → Char 错误映射到「缺少字符（括号/引号/斜杠）」。
    let e = parse("(cpu > 5").unwrap_err();
    assert!(
        e.msg.contains("括号"),
        "expected 含「括号」的提示, got: {}",
        e.msg
    );
}

#[test]
fn err_chinese_unknown_field_hint() {
    // `foo > 5` → parse_field 走 AlphaNumeric 错误分支 → 「未知字段名」。
    let e = parse("foo > 5").unwrap_err();
    assert!(
        e.msg.contains("未知") || e.msg.contains("字段"),
        "expected 含「未知/字段」的提示, got: {}",
        e.msg
    );
    assert!(
        !e.msg.contains("AlphaNumeric"),
        "must not leak nom ErrorKind: {}",
        e.msg
    );
}

#[test]
fn err_chinese_trailing_input_hint() {
    // `cpu > 5 extra` → parse() 入口检测到 trimmed 非空 → 「输入末尾出现多余内容」。
    let e = parse("cpu > 5 extra").unwrap_err();
    assert!(
        e.msg.contains("多余") || e.msg.contains("末尾"),
        "expected 含「多余/末尾」的提示, got: {}",
        e.msg
    );
    assert!(
        !e.msg.contains("trailing"),
        "must not leak English msg: {}",
        e.msg
    );
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

// ===== v0.8 阶段 3（TD-15）：Tree / AppGroup FilterExpr 集成 =====
//
// 这组测试覆盖 List 之外的两个视图接入 FilterExpr：
// - Tree view：`get_filtered_tree_visible(cached_processes)` 在 FilterExpr 模式下
//   用 pid→ProcessInfo 索引 apply AST。
// - AppGroup view：`app_group_filtered_visual_items(cached_processes)` 在 FilterExpr 模式下
//   Header 项按聚合值判断，Child 项按单进程判断。
// 同时覆盖 `:` 激活后非法输入保留上一次成功 AST 的契约（List 已有，验证 Tree/AppGroup 同款）。

use proc::app_group::{self, AppGroupItem};
use proc::collect::ProcessViewMode;
use proc::view_models::ProcessPanel;

/// 带 exe 路径的构造：compute_groups 按 exe_dir 分组，没 exe 会全部归到同组。
fn make_proc_with_exe(name: &str, cpu: f32, mem_bytes: u64, pid: u32, exe: &str) -> ProcessInfo {
    let mut p = make_proc(name, cpu, mem_bytes, pid);
    p.exe = Some(std::sync::Arc::from(exe));
    p
}

fn panel_with_procs(processes: &[ProcessInfo]) -> ProcessPanel {
    let mut panel = ProcessPanel::new(processes);
    // total_mem 给 0：FilterExpr 不依赖 mem_pct，仅做 size 占位。
    panel.init_tree(processes, 0);
    panel.rebuild_app_groups(processes);
    panel
}

fn activate_and_type(panel: &mut ProcessPanel, view: ProcessViewMode, text: &str) {
    let search = match view {
        ProcessViewMode::List => &mut panel.search,
        ProcessViewMode::Tree => &mut panel.tree_search,
        ProcessViewMode::AppGroup => &mut panel.app_group_search,
    };
    search.activate_filter_expr();
    for c in text.chars() {
        search.handle_input(key(KeyCode::Char(c)));
    }
}

#[test]
fn tree_filter_expr_cpu_gt_filters_low_cpu() {
    let procs = vec![
        make_proc("hot", 10.0, 0, 1),
        make_proc("cold", 1.0, 0, 2),
        make_proc("warm", 6.0, 0, 3),
    ];
    let mut panel = panel_with_procs(&procs);
    activate_and_type(&mut panel, ProcessViewMode::Tree, "cpu > 5");
    assert!(panel.tree_search.filter_expr.is_some());

    let visible = panel.get_filtered_tree_visible(&procs);
    let names: Vec<&str> = visible.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"hot"), "hot should pass: {:?}", names);
    assert!(names.contains(&"warm"), "warm should pass: {:?}", names);
    assert!(
        !names.iter().any(|n| n == &"cold"),
        "cold should be filtered out: {:?}",
        names
    );
}

#[test]
fn tree_filter_expr_pid_equality_lookup() {
    // 验证 FilterExpr 通过 pid→ProcessInfo 索引能命中（不只是 TreeNode 自带的 cpu/memory）。
    let procs = vec![
        make_proc("alpha", 0.0, 0, 1234),
        make_proc("beta", 0.0, 0, 5678),
    ];
    let mut panel = panel_with_procs(&procs);
    activate_and_type(&mut panel, ProcessViewMode::Tree, "pid = 1234");
    let visible = panel.get_filtered_tree_visible(&procs);
    let pids: Vec<u32> = visible.iter().map(|n| n.pid).collect();
    assert_eq!(pids, vec![1234]);
}

#[test]
fn tree_filter_expr_keeps_prev_ast_on_bad_input() {
    let procs = vec![make_proc("hot", 10.0, 0, 1), make_proc("cold", 1.0, 0, 2)];
    let mut panel = panel_with_procs(&procs);
    activate_and_type(&mut panel, ProcessViewMode::Tree, "cpu > 5");
    // 故意打一个破坏 parse 的字符：`cpu > 5)` → 右括号多余 → parse 失败。
    panel.tree_search.handle_input(key(KeyCode::Char(')')));
    assert!(panel.tree_search.filter_error.is_some());
    // 保留上一次成功 AST，cold 仍被过滤掉。
    let visible = panel.get_filtered_tree_visible(&procs);
    let names: Vec<&str> = visible.iter().map(|n| n.name.as_str()).collect();
    assert!(
        !names.iter().any(|n| n == &"cold"),
        "prev AST should still filter cold: {:?}",
        names
    );
}

#[test]
fn tree_substring_mode_unchanged_in_tree_view() {
    // 回归保护：Substring 模式 v0.6 行为不破坏。
    let procs = vec![
        make_proc("chrome.exe", 0.0, 0, 1),
        make_proc("firefox.exe", 0.0, 0, 2),
    ];
    let mut panel = panel_with_procs(&procs);
    panel.tree_search.activate_substring();
    for c in "chrom".chars() {
        panel.tree_search.handle_input(key(KeyCode::Char(c)));
    }
    let visible = panel.get_filtered_tree_visible(&procs);
    let names: Vec<&str> = visible.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"chrome.exe"));
    assert!(!names.iter().any(|n| n == &"firefox.exe"));
}

#[test]
fn tree_empty_filter_expr_returns_all_visible() {
    // ':' 激活后空 query → 不过滤（同 substring 空 query）。
    let procs = vec![make_proc("a", 0.0, 0, 1), make_proc("b", 0.0, 0, 2)];
    let mut panel = panel_with_procs(&procs);
    panel.tree_search.activate_filter_expr();
    let visible = panel.get_filtered_tree_visible(&procs);
    assert_eq!(visible.len(), 2);
}

fn item_stats(items: &[AppGroupItem]) -> (usize, usize) {
    let headers = items
        .iter()
        .filter(|i| matches!(i, AppGroupItem::Header { .. }))
        .count();
    let children = items
        .iter()
        .filter(|i| matches!(i, AppGroupItem::Child { .. }))
        .count();
    (headers, children)
}

#[test]
fn app_group_filter_expr_aggregate_cpu_header_match() {
    // 构造 3 个 chrome.exe 进程，每个 cpu=30，聚合 cpu=90。
    // Header 在 `cpu > 50` 下命中（聚合 90 > 50），整组保留。
    let procs = vec![
        make_proc_with_exe(
            "chrome.exe",
            30.0,
            100 * 1024 * 1024,
            100,
            "C:/chr/chrome.exe",
        ),
        make_proc_with_exe(
            "chrome.exe",
            30.0,
            100 * 1024 * 1024,
            101,
            "C:/chr/chrome.exe",
        ),
        make_proc_with_exe(
            "chrome.exe",
            30.0,
            100 * 1024 * 1024,
            102,
            "C:/chr/chrome.exe",
        ),
        make_proc_with_exe("idle.exe", 1.0, 10 * 1024 * 1024, 200, "D:/idle/idle.exe"),
    ];
    let mut panel = panel_with_procs(&procs);
    activate_and_type(&mut panel, ProcessViewMode::AppGroup, "cpu > 50");
    let items = panel.app_group_filtered_visual_items(&procs);
    let (headers, children) = item_stats(&items);
    // chrome 组聚合 cpu=90 命中（Header），未 expanded → 0 children。
    // idle.exe 单组聚合 cpu=1，被过滤掉。
    assert_eq!(headers, 1, "only chrome group Header should match");
    assert_eq!(children, 0);
}

#[test]
fn app_group_filter_expr_child_partial_match() {
    // Header 不命中，但组内某 child 命中 → Header + 命中的 Child（自动展开）。
    // 用 `pid = 11`：合成 Header 的 pid=0 不命中；child pid=11 命中。
    let procs = vec![
        make_proc_with_exe("mixed.exe", 1.0, 0, 10, "C:/m/mixed.exe"),
        make_proc_with_exe("mixed.exe", 80.0, 0, 11, "C:/m/mixed.exe"),
        make_proc_with_exe("mixed.exe", 1.0, 0, 12, "C:/m/mixed.exe"),
    ];
    let mut panel = panel_with_procs(&procs);
    activate_and_type(&mut panel, ProcessViewMode::AppGroup, "pid = 11");
    let items = panel.app_group_filtered_visual_items(&procs);
    let (headers, children) = item_stats(&items);
    // 合成 Header pid=0 不命中；child pid=11 命中，pid=10/12 不命中。
    assert_eq!(headers, 1, "Header forced visible by child match");
    assert_eq!(children, 1, "only pid=11 child matches");
    // 验证命中的 child 确实是 pid=11。
    for item in &items {
        if let AppGroupItem::Child {
            group_idx,
            child_idx,
        } = item
        {
            let pid = panel.app_groups[*group_idx].processes[*child_idx].pid;
            assert_eq!(pid, 11);
        }
    }
}

#[test]
fn app_group_filter_expr_memory_aggregate() {
    // 验证 mem 字段在 Header 走聚合：3 × 50MB = 150MB，`mem > 100mb` 命中。
    let procs = vec![
        make_proc_with_exe("svc.exe", 0.0, 50 * 1024 * 1024, 1, "C:/s/svc.exe"),
        make_proc_with_exe("svc.exe", 0.0, 50 * 1024 * 1024, 2, "C:/s/svc.exe"),
        make_proc_with_exe("svc.exe", 0.0, 50 * 1024 * 1024, 3, "C:/s/svc.exe"),
        make_proc_with_exe("tiny.exe", 0.0, 1024, 4, "D:/t/tiny.exe"),
    ];
    let mut panel = panel_with_procs(&procs);
    activate_and_type(&mut panel, ProcessViewMode::AppGroup, "mem > 100mb");
    let items = panel.app_group_filtered_visual_items(&procs);
    let (headers, _) = item_stats(&items);
    assert_eq!(headers, 1, "svc group aggregate 150MB should match");
}

#[test]
fn app_group_filter_expr_keeps_prev_ast_on_bad_input() {
    let procs = vec![
        make_proc_with_exe("hot.exe", 80.0, 0, 1, "C:/h/hot.exe"),
        make_proc_with_exe("cold.exe", 1.0, 0, 2, "D:/c/cold.exe"),
    ];
    let mut panel = panel_with_procs(&procs);
    activate_and_type(&mut panel, ProcessViewMode::AppGroup, "cpu > 50");
    panel.app_group_search.handle_input(key(KeyCode::Char(')')));
    assert!(panel.app_group_search.filter_error.is_some());
    // 保留上一次成功 AST，cold.exe 仍被过滤掉。
    let items = panel.app_group_filtered_visual_items(&procs);
    let (headers, _) = item_stats(&items);
    assert_eq!(headers, 1, "only hot.exe group matches");
}

#[test]
fn app_group_substring_mode_unchanged() {
    // 回归保护：Substring 模式 v0.7 行为不破坏。
    let procs = vec![
        make_proc_with_exe("chrome.exe", 0.0, 0, 1, "C:/chr/chrome.exe"),
        make_proc_with_exe("firefox.exe", 0.0, 0, 2, "D:/ff/firefox.exe"),
    ];
    let mut panel = panel_with_procs(&procs);
    panel.app_group_search.activate_substring();
    for c in "chrom".chars() {
        panel.app_group_search.handle_input(key(KeyCode::Char(c)));
    }
    let items = panel.app_group_filtered_visual_items(&procs);
    let (headers, _) = item_stats(&items);
    assert_eq!(headers, 1, "chrome group matches");
}

#[allow(dead_code)]
fn _silence_unused() {
    let _ = app_group::build_visual_items;
}
