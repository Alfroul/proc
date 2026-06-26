//! 阶段 13 — Inspector v1 TUI 集成测试（ADR-0004）。
//!
//! 三类用例：
//! 1. `InspectionTab` 枚举行为（unit）
//! 2. App state：默认值 + Tab/BackTab/r/Esc 在 detail 模式下的行为
//! 3. 跨平台：通过 `inspect::inspect(self)` 验证 DLL Tab 至少能拿到数据
//!
//! 注意：App::switch_mode 是私有的；这里通过设置 `app.mode = ProcessDetail` +
//! `app.inspector.detail_process` + `app.inspector.inspection_data` 直接复现 switch_mode 完成后的
//! 状态，再驱动 `handle_key` 验证 Tab / 搜索 / 刷新逻辑。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use proc::app::App;
use proc::app_panel::{AppMode, InspectionTab};
use proc::collect::ProcessInfo;
use proc::inspect;

// ── InspectionTab unit ───────────────────────────────────────────────────────

#[test]
fn inspection_tab_all_has_six_variants() {
    let all = InspectionTab::all();
    assert_eq!(all.len(), 6);
    assert!(all.contains(&InspectionTab::Summary));
    assert!(all.contains(&InspectionTab::Env));
    assert!(all.contains(&InspectionTab::Network));
    assert!(all.contains(&InspectionTab::Dlls));
    assert!(all.contains(&InspectionTab::Handles));
    assert!(all.contains(&InspectionTab::Memory));
}

#[test]
fn inspection_tab_next_cycles_all_six() {
    assert_eq!(InspectionTab::Summary.next(), InspectionTab::Env);
    assert_eq!(InspectionTab::Env.next(), InspectionTab::Network);
    assert_eq!(InspectionTab::Network.next(), InspectionTab::Dlls);
    assert_eq!(InspectionTab::Dlls.next(), InspectionTab::Handles);
    assert_eq!(InspectionTab::Handles.next(), InspectionTab::Memory);
    // 循环：Memory → Summary
    assert_eq!(InspectionTab::Memory.next(), InspectionTab::Summary);
}

#[test]
fn inspection_tab_prev_cycles_inverse_of_next() {
    for tab in InspectionTab::all() {
        assert_eq!(tab.next().prev(), *tab, "next/prev should be inverse");
    }
    // 循环：Summary → Memory（最后一个）
    assert_eq!(InspectionTab::Summary.prev(), InspectionTab::Memory);
}

#[test]
fn inspection_tab_labels_match_context_md() {
    assert_eq!(InspectionTab::Summary.label(), "概要");
    assert_eq!(InspectionTab::Env.label(), "环境");
    assert_eq!(InspectionTab::Network.label(), "网络");
    assert_eq!(InspectionTab::Dlls.label(), "DLL");
    assert_eq!(InspectionTab::Handles.label(), "句柄");
    assert_eq!(InspectionTab::Memory.label(), "内存");
}

// ── 阶段 1 新增：6 变体不变量加固 ─────────────────────────────────────────────

#[test]
fn inspection_tab_all_in_next_cycle_order() {
    // all() 的顺序必须与 next 循环顺序一致 —— Tab 栏从左到右就是 Tab 键的走向。
    let all = InspectionTab::all();
    for i in 0..all.len() {
        // InspectionTab: Copy —— 直接解引用拿到 owned value，避免 owned-vs-ref
        // 类型不匹配（`assert_eq!` 不接受 `InspectionTab` 与 `&&InspectionTab`）。
        let expected: InspectionTab = all[(i + 1) % all.len()];
        assert_eq!(
            all[i].next(),
            expected,
            "all()[{i}].next() should equal all()[(i+1) % len]"
        );
    }
}

#[test]
fn inspection_tab_next_prev_are_inverse_for_all_six() {
    // 任何 tab 的 next → prev 必须回到原 tab；反之亦然。这是 Tab/BackTab
    // 互相抵消的契约，单测固化避免未来加变体时漏写分支。
    for tab in InspectionTab::all() {
        assert_eq!(tab.next().prev(), *tab, "next/prev inverse for {tab:?}");
        assert_eq!(tab.prev().next(), *tab, "prev/next inverse for {tab:?}");
    }
}

#[test]
fn inspection_tab_next_six_times_returns_to_start() {
    // 6 变体意味着 next 连按 6 次必须回到起点。
    for start in InspectionTab::all() {
        let mut cur = *start;
        for _ in 0..6 {
            cur = cur.next();
        }
        assert_eq!(cur, *start, "next^6 must be identity for {start:?}");
    }
}

#[test]
fn inspection_tab_labels_are_all_distinct() {
    // 6 个标签必须互不相同 —— Tab 栏靠 label 区分，重名会导致用户混淆。
    let labels: Vec<&str> = InspectionTab::all().iter().map(|t| t.label()).collect();
    let unique_count = labels
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(
        unique_count,
        labels.len(),
        "labels must be distinct: {labels:?}"
    );
}

#[test]
fn inspection_tab_memory_tab_is_last_in_all() {
    // 顺序契约：Memory 必须是 all() 的最后一项，prev(Summary) 才会落到 Memory。
    // 加这条是因为阶段 4 实现 Handles/Memory 采集时容易把顺序改乱。
    let all = InspectionTab::all();
    assert_eq!(*all.last().unwrap(), InspectionTab::Memory);
    assert_eq!(all[all.len() - 2], InspectionTab::Handles);
}

#[test]
fn inspection_tab_default_is_summary() {
    // #[derive(Default)] 把 Summary 当默认；switch_mode 也依赖这个。
    let tab = InspectionTab::default();
    assert_eq!(tab, InspectionTab::Summary);
}

// ── App state defaults ───────────────────────────────────────────────────────

#[test]
fn app_inspector_defaults_are_clean() {
    let app = App::new().expect("App::new");
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Summary);
    assert!(
        app.inspector.inspection_data.is_none(),
        "fresh App should have no data"
    );
    assert_eq!(app.inspector.inspection_scroll, 0);
    assert!(!app.inspector.inspection_search.is_active());
    assert!(app.inspector.inspection_search.query().is_empty());
}

// ── Setup helper：模拟 switch_mode(ProcessDetail) 完成后的状态 ────────────────

fn enter_inspector_with_self_pid() -> App {
    let mut app = App::new().expect("App::new");
    let self_proc = build_self_proc_info();
    app.inspector.detail_process = Some(self_proc.clone());
    app.inspector.inspection_data = Some(inspect::inspect(self_proc.pid));
    app.inspector.inspection_tab = InspectionTab::Summary;
    app.inspector.inspection_scroll = 0;
    app.inspector.inspection_search.clear();
    app.mode = AppMode::ProcessDetail;
    app
}

fn build_self_proc_info() -> ProcessInfo {
    let pid = std::process::id();
    ProcessInfo {
        pid,
        start_time: 0,
        name: std::sync::Arc::from("self"),
        cpu_usage: 0.0,
        memory: 0,
        virtual_memory: 0,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: proc::collect::ProcessStatus::default(),
        exe: None,
        cmd: std::sync::Arc::from(Vec::<String>::new()),
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        run_time: 0,
        name_lower: std::sync::Arc::from("self"),
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

// ── Tab 切换 ──────────────────────────────────────────────────────────────────

#[test]
fn tab_key_cycles_inspector_tabs() {
    let mut app = enter_inspector_with_self_pid();
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Summary);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Env);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Network);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Dlls);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Handles);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Memory);

    // 循环：Memory → Summary
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Summary);
}

#[test]
fn backtab_cycles_in_reverse() {
    let mut app = enter_inspector_with_self_pid();
    app.inspector.inspection_tab = InspectionTab::Summary;

    // 6 变体下 BackTab 从 Summary 倒退到 Memory（最后一个）。
    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Memory);

    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Handles);
}

#[test]
fn tab_switch_resets_scroll() {
    let mut app = enter_inspector_with_self_pid();
    app.inspector.inspection_scroll = 42;

    press(&mut app, KeyCode::Tab);
    assert_eq!(
        app.inspector.inspection_scroll, 0,
        "切 Tab 时滚动位置应重置（避免落入新 Tab 的越界行）",
    );
}

// ── 搜索 ──────────────────────────────────────────────────────────────────────

#[test]
fn slash_enters_search_mode() {
    let mut app = enter_inspector_with_self_pid();
    assert!(!app.inspector.inspection_search.is_active());

    press(&mut app, KeyCode::Char('/'));
    assert!(app.inspector.inspection_search.is_active());
    assert!(app.inspector.inspection_search.query().is_empty());
}

#[test]
fn search_query_appends_chars() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('P'));
    press(&mut app, KeyCode::Char('A'));
    press(&mut app, KeyCode::Char('T'));
    press(&mut app, KeyCode::Char('H'));
    assert_eq!(app.inspector.inspection_search.query(), "PATH");
}

#[test]
fn backspace_pops_search_query() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('b'));
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.inspector.inspection_search.query(), "a");
}

#[test]
fn esc_while_searching_keeps_detail_mode() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('x'));
    assert!(app.inspector.inspection_search.is_active());

    // 第一次 Esc 只退出搜索
    press(&mut app, KeyCode::Esc);
    assert!(!app.inspector.inspection_search.is_active());
    assert_eq!(
        app.mode,
        AppMode::ProcessDetail,
        "Esc 在搜索中只退搜索，不退页面"
    );

    // 第二次 Esc 才回 ProcessList
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.mode, AppMode::ProcessList);
}

#[test]
fn tab_is_ignored_while_searching() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('q'));

    // 搜索中按 Tab 不应切 Tab —— 否则会丢搜索内容
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Summary);
    assert!(app.inspector.inspection_search.is_active());
}

// ── 刷新 ──────────────────────────────────────────────────────────────────────

#[test]
fn f5_key_refreshes_inspection_data() {
    let mut app = enter_inspector_with_self_pid();

    // 先清空数据模拟过期
    app.inspector.inspection_data = None;
    press(&mut app, KeyCode::F(5));

    assert!(
        app.inspector.inspection_data.is_some(),
        "F5 键应重新调用 inspect() 填充 inspection_data",
    );
    let data = app.inspector.inspection_data.as_ref().unwrap();
    // 自己进程至少应拿到环境变量 + 模块。
    assert!(!data.env.is_empty(), "self env empty after refresh");
    assert!(!data.dlls.is_empty(), "self dlls empty after refresh");
}

#[test]
fn f5_key_sets_status_message() {
    let mut app = enter_inspector_with_self_pid();
    app.status_message = None;
    press(&mut app, KeyCode::F(5));
    assert!(app.status_message.is_some(), "F5 应在 status_message 提示");
}

// ── 滚动 ──────────────────────────────────────────────────────────────────────

#[test]
fn down_arrow_advances_scroll() {
    let mut app = enter_inspector_with_self_pid();
    let start = app.inspector.inspection_scroll;
    press(&mut app, KeyCode::Down);
    assert!(app.inspector.inspection_scroll > start);
}

#[test]
fn up_arrow_does_not_underflow() {
    let mut app = enter_inspector_with_self_pid();
    app.inspector.inspection_scroll = 0;
    press(&mut app, KeyCode::Up);
    assert_eq!(app.inspector.inspection_scroll, 0);
}

#[test]
fn pageup_pagedown_jump_by_ten() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.inspector.inspection_scroll, 10);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.inspector.inspection_scroll, 0);
}

#[test]
fn home_resets_scroll() {
    let mut app = enter_inspector_with_self_pid();
    app.inspector.inspection_scroll = 99;
    press(&mut app, KeyCode::Home);
    assert_eq!(app.inspector.inspection_scroll, 0);
}

// ── 数据正确加载（跨平台 smoke） ───────────────────────────────────────────────

#[test]
fn inspect_self_yields_env_and_dlls() {
    let app = enter_inspector_with_self_pid();
    let data = app
        .inspector
        .inspection_data
        .as_ref()
        .expect("preload set data");

    // 自己进程的 env 应至少含 PATH（CI 上偶尔清空时退化为非空）
    let has_path = data.env.iter().any(|v| v.key.eq_ignore_ascii_case("PATH"));
    assert!(
        has_path || !data.env.is_empty(),
        "expected PATH or any var, got env with {} entries",
        data.env.len(),
    );

    // 模块至少 1 个
    assert!(!data.dlls.is_empty(), "expected ≥1 self module");
}

#[test]
fn env_filter_skips_non_matching_keys() {
    use proc::inspect::EnvVar;

    // 直接验证渲染层使用的过滤逻辑：query 不空时 key/value 都不命中则被丢弃。
    let env = [
        EnvVar {
            key: "PATH".to_string(),
            value: "/usr/bin".to_string(),
            is_secret: false,
        },
        EnvVar {
            key: "HOME".to_string(),
            value: "/home/me".to_string(),
            is_secret: false,
        },
        EnvVar {
            key: "EDITOR".to_string(),
            value: "vim".to_string(),
            is_secret: false,
        },
    ];
    let q = "home";
    let filtered: Vec<&EnvVar> = env
        .iter()
        .filter(|v| v.key.to_lowercase().contains(q) || v.value.to_lowercase().contains(q))
        .collect();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].key, "HOME");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_dlls_include_libc_via_proc_maps() {
    // Linux DLL Tab 走 /proc/<pid>/maps；自己进程至少应能解析出 libc 或 ld。
    // 注意：proc 本身是 Rust 静态二进制，但仍会 dynamic-link libc。
    let dlls = inspect::dlls::collect_dlls(std::process::id()).expect("self maps");
    assert!(!dlls.is_empty(), "expected ≥1 module from /proc/self/maps");

    // libc / ld / libgcc 至少命中一个 —— 如果都没找到，说明 /proc 解析挂了。
    let known = dlls.iter().any(|d| {
        let p = d.path.to_lowercase();
        p.contains("libc") || p.contains("/ld-") || p.contains("libgcc") || p.contains("libpthread")
    });
    assert!(
        known,
        "expected libc/ld/libgcc in self maps, got: {:?}",
        dlls.iter().take(3).map(|d| &d.path).collect::<Vec<_>>()
    );
}

// ===========================================================================
// 阶段 4：Handles/Memory Tab 接线 + A4 优先级快捷键
// ===========================================================================

#[test]
fn app_inspector_defaults_handles_and_memory_are_none() {
    // 阶段 1 加的占位字段；阶段 4 switch_mode(ProcessDetail) 才填。
    // 这里验证 fresh App 上保持 None，避免误以为「详情页加载失败」。
    let app = App::new().expect("App::new");
    assert!(
        app.inspector.inspection_handles_data.is_none(),
        "fresh App should have handles_data = None"
    );
    assert!(
        app.inspector.inspection_memory_data.is_none(),
        "fresh App should have memory_data = None"
    );
}

#[test]
fn f5_refresh_sets_inspection_handles_and_memory_data() {
    // 模拟 switch_mode(ProcessDetail) 后的状态：detail_process + inspection_data
    // 已加载，但 handles/memory 仍 None（helper 没填）。按 F5 后应同时刷新三份数据。
    let mut app = enter_inspector_with_self_pid();
    app.inspector.inspection_handles_data = None;
    app.inspector.inspection_memory_data = None;

    press(&mut app, KeyCode::F(5));

    // F5 路径会调 collect_handles / collect_memory（unwrap_or_default 兜底），
    // 成功路径下两者都变 Some。
    assert!(
        app.inspector.inspection_handles_data.is_some(),
        "F5 应填充 inspection_handles_data"
    );
    assert!(
        app.inspector.inspection_memory_data.is_some(),
        "F5 应填充 inspection_memory_data"
    );
}

#[test]
fn plus_key_in_detail_writes_status_message() {
    // A4：详情页 +/- 调整优先级 —— 成功/失败都写 status_message（不会静默吞键）。
    let mut app = enter_inspector_with_self_pid();
    app.status_message = None;
    press(&mut app, KeyCode::Char('+'));
    assert!(
        app.status_message.is_some(),
        "+ 应该写一条状态消息（即使权限不足）"
    );
}

#[test]
fn minus_key_in_detail_writes_status_message() {
    let mut app = enter_inspector_with_self_pid();
    app.status_message = None;
    press(&mut app, KeyCode::Char('-'));
    assert!(
        app.status_message.is_some(),
        "- 应该写一条状态消息（即使权限不足）"
    );
}

#[test]
fn plus_key_does_not_quit_or_change_mode() {
    // +/- 不应误触发退出 / 切 mode（防止与 Replay 的 +/- 速度调节搞混）。
    let mut app = enter_inspector_with_self_pid();
    let mode_before = app.mode;
    press(&mut app, KeyCode::Char('+'));
    assert_eq!(app.mode, mode_before);
    assert!(!app.should_quit, "+ 不应触发退出");
}

#[test]
fn switching_to_handles_tab_preserves_data() {
    // 进详情页 → 切到 Handles Tab → inspection_handles_data 不应被 Tab 切换清掉。
    let mut app = enter_inspector_with_self_pid();
    // 手动塞一个非空 handles_data（模拟 switch_mode 已加载）。
    app.inspector.inspection_handles_data = Some(vec![proc::inspect::HandleInfo {
        raw_handle: 0xDEADBEEF,
        kind: proc::inspect::HandleKind::File,
        name: "test.txt".to_string(),
        granted_access: 0x12345678,
    }]);
    press(&mut app, KeyCode::Tab); // Summary → Env
    press(&mut app, KeyCode::Tab); // Env → Network
    press(&mut app, KeyCode::Tab); // Network → Dlls
    press(&mut app, KeyCode::Tab); // Dlls → Handles
    assert_eq!(app.inspector.inspection_tab, InspectionTab::Handles);
    let data = app
        .inspector
        .inspection_handles_data
        .as_ref()
        .expect("handles preserved");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].name, "test.txt");
}

// ── 阶段 6：键位冲突修复（r→F5 刷新 / c→y 复制 / docker r→Shift+R restart） ────

#[test]
fn y_key_in_detail_triggers_clipboard_copy() {
    // v0.6.0 阶段 6：原详情页 'c' 复制迁移到 'y'（vim yank）。
    let mut app = enter_inspector_with_self_pid();
    app.status_message = None;
    press(&mut app, KeyCode::Char('y'));
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("复制")),
        "y 应触发复制并写 status_message，实际：{:?}",
        app.status_message
    );
}

#[test]
fn c_key_in_detail_shows_deprecation_warning() {
    // v0.6.0 阶段 8（REVIEW-7.md P1-6）：详情页 'c' 不再静默落入全局，
    // 也不再复制。返回 deprecation warning 指引用户到 'y'（vim yank）。
    // v0.7.0 计划移除该 deprecation 分支。
    let mut app = enter_inspector_with_self_pid();
    let sidebar_before = app.sidebar_expanded;
    app.status_message = None;
    press(&mut app, KeyCode::Char('c'));
    assert_eq!(
        app.sidebar_expanded, sidebar_before,
        "详情页 'c' 不应再触发侧边栏折叠（已捕获为 deprecation warning）"
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("v0.7.0") && m.contains('y')),
        "详情页 'c' 应写 deprecation warning 含 'v0.7.0' 与 'y'，实际：{:?}",
        app.status_message
    );
}

#[test]
fn r_key_in_detail_shows_deprecation_warning() {
    // v0.6.0 阶段 8（REVIEW-7.md P1-6）：详情页 'r' 不再静默 noop，
    // 返回 deprecation warning 指引用户到 F5。v0.7.0 计划移除。
    let mut app = enter_inspector_with_self_pid();
    app.inspector.inspection_data = None;
    app.status_message = None;
    press(&mut app, KeyCode::Char('r'));
    assert!(
        app.inspector.inspection_data.is_none(),
        "详情页 'r' 不应再触发刷新（已迁移到 F5）"
    );
    assert!(
        app.status_message
            .as_deref()
            .is_some_and(|m| m.contains("v0.7.0") && m.contains("F5")),
        "详情页 'r' 应写 deprecation warning 含 'v0.7.0' 与 'F5'，实际：{:?}",
        app.status_message
    );
}

#[test]
fn f5_key_resets_scroll_to_zero() {
    // F5 刷新路径会把 inspection_scroll 重置为 0（数据变短后避免 scroll 越界）。
    let mut app = enter_inspector_with_self_pid();
    app.inspector.inspection_scroll = 42;
    press(&mut app, KeyCode::F(5));
    assert_eq!(
        app.inspector.inspection_scroll, 0,
        "F5 应把 scroll 重置为 0"
    );
}

#[test]
fn y_key_does_not_quit_or_change_mode() {
    // y 不应误触发退出 / 切 mode（vim yank 风格仅触发剪贴板）。
    let mut app = enter_inspector_with_self_pid();
    let mode_before = app.mode;
    press(&mut app, KeyCode::Char('y'));
    assert_eq!(app.mode, mode_before, "y 不应切换 mode");
    assert!(!app.should_quit, "y 不应触发退出");
}

#[test]
fn f5_key_does_not_quit_or_change_mode() {
    // F5 仅刷新 Inspector 数据，不应误触发退出 / 切 mode。
    let mut app = enter_inspector_with_self_pid();
    let mode_before = app.mode;
    press(&mut app, KeyCode::F(5));
    assert_eq!(app.mode, mode_before, "F5 不应切换 mode");
    assert!(!app.should_quit, "F5 不应触发退出");
}
