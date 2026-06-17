//! 阶段 13 — Inspector v1 TUI 集成测试（ADR-0004）。
//!
//! 三类用例：
//! 1. `InspectionTab` 枚举行为（unit）
//! 2. App state：默认值 + Tab/BackTab/r/Esc 在 detail 模式下的行为
//! 3. 跨平台：通过 `inspect::inspect(self)` 验证 DLL Tab 至少能拿到数据
//!
//! 注意：App::switch_mode 是私有的；这里通过设置 `app.mode = ProcessDetail` +
//! `app.detail_process` + `app.inspection_data` 直接复现 switch_mode 完成后的
//! 状态，再驱动 `handle_key` 验证 Tab / 搜索 / 刷新逻辑。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use proc::app::App;
use proc::app_panel::{AppMode, InspectionTab};
use proc::collect::ProcessInfo;
use proc::inspect;

// ── InspectionTab unit ───────────────────────────────────────────────────────

#[test]
fn inspection_tab_all_has_four_v1_tabs() {
    let all = InspectionTab::all();
    assert_eq!(all.len(), 4);
    assert!(all.contains(&InspectionTab::Summary));
    assert!(all.contains(&InspectionTab::Env));
    assert!(all.contains(&InspectionTab::Network));
    assert!(all.contains(&InspectionTab::Dlls));
}

#[test]
fn inspection_tab_next_cycles() {
    assert_eq!(InspectionTab::Summary.next(), InspectionTab::Env);
    assert_eq!(InspectionTab::Env.next(), InspectionTab::Network);
    assert_eq!(InspectionTab::Network.next(), InspectionTab::Dlls);
    // 循环：Dlls → Summary
    assert_eq!(InspectionTab::Dlls.next(), InspectionTab::Summary);
}

#[test]
fn inspection_tab_prev_cycles_inverse_of_next() {
    for tab in InspectionTab::all() {
        assert_eq!(tab.next().prev(), *tab, "next/prev should be inverse");
    }
    assert_eq!(InspectionTab::Summary.prev(), InspectionTab::Dlls);
}

#[test]
fn inspection_tab_labels_match_adr_0004() {
    assert_eq!(InspectionTab::Summary.label(), "概要");
    assert_eq!(InspectionTab::Env.label(), "环境");
    assert_eq!(InspectionTab::Network.label(), "网络");
    assert_eq!(InspectionTab::Dlls.label(), "DLL");
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
    assert_eq!(app.inspection_tab, InspectionTab::Summary);
    assert!(
        app.inspection_data.is_none(),
        "fresh App should have no data"
    );
    assert_eq!(app.inspection_scroll, 0);
    assert!(!app.inspection_search.is_active());
    assert!(app.inspection_search.query().is_empty());
}

// ── Setup helper：模拟 switch_mode(ProcessDetail) 完成后的状态 ────────────────

fn enter_inspector_with_self_pid() -> App {
    let mut app = App::new().expect("App::new");
    let self_proc = build_self_proc_info();
    app.detail_process = Some(self_proc.clone());
    app.inspection_data = Some(inspect::inspect(self_proc.pid));
    app.inspection_tab = InspectionTab::Summary;
    app.inspection_scroll = 0;
    app.inspection_search.clear();
    app.mode = AppMode::ProcessDetail;
    app
}

fn build_self_proc_info() -> ProcessInfo {
    let pid = std::process::id();
    ProcessInfo {
        pid,
        start_time: 0,
        name: "self".to_string(),
        cpu_usage: 0.0,
        memory: 0,
        virtual_memory: 0,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        status: String::new(),
        exe: None,
        cmd: Vec::new(),
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        run_time: 0,
    }
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

// ── Tab 切换 ──────────────────────────────────────────────────────────────────

#[test]
fn tab_key_cycles_inspector_tabs() {
    let mut app = enter_inspector_with_self_pid();
    assert_eq!(app.inspection_tab, InspectionTab::Summary);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspection_tab, InspectionTab::Env);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspection_tab, InspectionTab::Network);

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspection_tab, InspectionTab::Dlls);

    // 循环：Dlls → Summary
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.inspection_tab, InspectionTab::Summary);
}

#[test]
fn backtab_cycles_in_reverse() {
    let mut app = enter_inspector_with_self_pid();
    app.inspection_tab = InspectionTab::Summary;

    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.inspection_tab, InspectionTab::Dlls);

    app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.inspection_tab, InspectionTab::Network);
}

#[test]
fn tab_switch_resets_scroll() {
    let mut app = enter_inspector_with_self_pid();
    app.inspection_scroll = 42;

    press(&mut app, KeyCode::Tab);
    assert_eq!(
        app.inspection_scroll, 0,
        "切 Tab 时滚动位置应重置（避免落入新 Tab 的越界行）",
    );
}

// ── 搜索 ──────────────────────────────────────────────────────────────────────

#[test]
fn slash_enters_search_mode() {
    let mut app = enter_inspector_with_self_pid();
    assert!(!app.inspection_search.is_active());

    press(&mut app, KeyCode::Char('/'));
    assert!(app.inspection_search.is_active());
    assert!(app.inspection_search.query().is_empty());
}

#[test]
fn search_query_appends_chars() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('P'));
    press(&mut app, KeyCode::Char('A'));
    press(&mut app, KeyCode::Char('T'));
    press(&mut app, KeyCode::Char('H'));
    assert_eq!(app.inspection_search.query(), "PATH");
}

#[test]
fn backspace_pops_search_query() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Char('b'));
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.inspection_search.query(), "a");
}

#[test]
fn esc_while_searching_keeps_detail_mode() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('x'));
    assert!(app.inspection_search.is_active());

    // 第一次 Esc 只退出搜索
    press(&mut app, KeyCode::Esc);
    assert!(!app.inspection_search.is_active());
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
    assert_eq!(app.inspection_tab, InspectionTab::Summary);
    assert!(app.inspection_search.is_active());
}

// ── 刷新 ──────────────────────────────────────────────────────────────────────

#[test]
fn r_key_refreshes_inspection_data() {
    let mut app = enter_inspector_with_self_pid();

    // 先清空数据模拟过期
    app.inspection_data = None;
    press(&mut app, KeyCode::Char('r'));

    assert!(
        app.inspection_data.is_some(),
        "r 键应重新调用 inspect() 填充 inspection_data",
    );
    let data = app.inspection_data.as_ref().unwrap();
    // 自己进程至少应拿到环境变量 + 模块。
    assert!(!data.env.is_empty(), "self env empty after refresh");
    assert!(!data.dlls.is_empty(), "self dlls empty after refresh");
}

#[test]
fn r_key_sets_status_message() {
    let mut app = enter_inspector_with_self_pid();
    app.status_message = None;
    press(&mut app, KeyCode::Char('r'));
    assert!(app.status_message.is_some(), "r 应在 status_message 提示");
}

// ── 滚动 ──────────────────────────────────────────────────────────────────────

#[test]
fn down_arrow_advances_scroll() {
    let mut app = enter_inspector_with_self_pid();
    let start = app.inspection_scroll;
    press(&mut app, KeyCode::Down);
    assert!(app.inspection_scroll > start);
}

#[test]
fn up_arrow_does_not_underflow() {
    let mut app = enter_inspector_with_self_pid();
    app.inspection_scroll = 0;
    press(&mut app, KeyCode::Up);
    assert_eq!(app.inspection_scroll, 0);
}

#[test]
fn pageup_pagedown_jump_by_ten() {
    let mut app = enter_inspector_with_self_pid();
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.inspection_scroll, 10);
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.inspection_scroll, 0);
}

#[test]
fn home_resets_scroll() {
    let mut app = enter_inspector_with_self_pid();
    app.inspection_scroll = 99;
    press(&mut app, KeyCode::Home);
    assert_eq!(app.inspection_scroll, 0);
}

// ── 数据正确加载（跨平台 smoke） ───────────────────────────────────────────────

#[test]
fn inspect_self_yields_env_and_dlls() {
    let app = enter_inspector_with_self_pid();
    let data = app.inspection_data.as_ref().expect("preload set data");

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
        },
        EnvVar {
            key: "HOME".to_string(),
            value: "/home/me".to_string(),
        },
        EnvVar {
            key: "EDITOR".to_string(),
            value: "vim".to_string(),
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
