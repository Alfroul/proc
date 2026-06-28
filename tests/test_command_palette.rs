//! v0.7.0 阶段 3 — 命令面板 Ctrl+P 集成测试。
//!
//! 覆盖 spec 要求的 7 个 case：
//! 1. Normal → Ctrl+P → active_layer 变 Palette
//! 2. Palette + 输入 "kill" → matched ≤ items.len()，匹配项含 kill
//! 3. Palette + Esc → active_layer 变 Normal
//! 4. Palette + Enter → 执行 action 后 active_layer 变 Normal
//! 5. Palette + `1`-`6` → 不切面板（被拦截）
//! 6. Normal + `/` → current_layer 变 Search
//! 7. Search + Ctrl+P → current_layer 变 Palette（搜索中断）

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use proc::app::{App, AppLayer};
use proc::app_panel::AppMode;

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

// ── Test 1 ───────────────────────────────────────────────────────────────────

#[test]
fn normal_layer_ctrl_p_opens_palette() {
    let mut app = App::new().expect("App::new");
    // 启动后处于 Normal 层、ProcessList 模式。
    assert_eq!(app.current_layer(), AppLayer::Normal);
    assert_eq!(app.mode, AppMode::ProcessList);

    app.handle_key(ctrl(KeyCode::Char('p')));
    assert_eq!(app.current_layer(), AppLayer::Palette);
    assert!(app.is_palette_open());
}

// ── Test 2 ───────────────────────────────────────────────────────────────────

#[test]
fn palette_input_kill_matches_kill_actions() {
    let mut app = App::new().expect("App::new");
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert!(app.is_palette_open());

    for c in "kill".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let palette = &app.command_palette;
    let matched = palette.matched_indices();
    assert!(
        matched.len() <= palette.items().len(),
        "matched {} should be ≤ total {}",
        matched.len(),
        palette.items().len()
    );
    // 每个匹配项 label 含 "kill"（大小写不敏感）。
    for &idx in matched {
        let label = palette.items()[idx].label.to_lowercase();
        assert!(
            label.contains("kill"),
            "matched label {label:?} should contain 'kill'"
        );
    }
    // 至少匹配 KillCursor / ForceKillCursor 两条。
    assert!(
        matched.len() >= 2,
        "expected ≥2 kill actions matched, got {}",
        matched.len()
    );
}

// ── Test 3 ───────────────────────────────────────────────────────────────────

#[test]
fn palette_esc_returns_to_normal_layer() {
    let mut app = App::new().expect("App::new");
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert!(app.is_palette_open());

    app.handle_key(key(KeyCode::Esc));
    assert!(!app.is_palette_open());
    assert_eq!(app.current_layer(), AppLayer::Normal);
}

// ── Test 4 ───────────────────────────────────────────────────────────────────

#[test]
fn palette_enter_executes_and_closes() {
    let mut app = App::new().expect("App::new");
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert!(app.is_palette_open());

    // 默认选中第 0 项（"Switch to Process List"）。Enter 执行后层回到 Normal，
    // 模式仍是 ProcessList（无副作用，但状态消息可能更新）。
    app.handle_key(key(KeyCode::Enter));
    assert!(!app.is_palette_open());
    assert_eq!(app.current_layer(), AppLayer::Normal);
}

// ── Test 5 ───────────────────────────────────────────────────────────────────

#[test]
fn palette_digit_keys_do_not_switch_panels() {
    let mut app = App::new().expect("App::new");
    let initial_mode = app.mode;
    app.handle_key(ctrl(KeyCode::Char('p')));
    // 在 palette 内按 1-6：应当被 palette 拦截喂给输入框，不会切面板。
    for code in [
        KeyCode::Char('1'),
        KeyCode::Char('2'),
        KeyCode::Char('3'),
        KeyCode::Char('4'),
        KeyCode::Char('5'),
        KeyCode::Char('6'),
    ] {
        app.handle_key(key(code));
        assert_eq!(app.mode, initial_mode, "按 {code:?} 不该切换面板");
        assert!(app.is_palette_open(), "palette 应保持打开");
    }
    // 输入框累积 1-6
    assert_eq!(app.command_palette.input(), "123456");
}

// ── Test 6 ───────────────────────────────────────────────────────────────────

#[test]
fn slash_in_normal_activates_search_layer() {
    let mut app = App::new().expect("App::new");
    assert_eq!(app.current_layer(), AppLayer::Normal);

    app.handle_key(key(KeyCode::Char('/')));
    assert!(
        app.process_panel.panel.search.is_active(),
        "ProcessPanel 搜索应被激活"
    );
    assert_eq!(
        app.current_layer(),
        AppLayer::Search,
        "current_layer 应派生为 Search"
    );
}

// ── Test 7 ───────────────────────────────────────────────────────────────────

#[test]
fn search_layer_ctrl_p_interrupts_to_palette() {
    let mut app = App::new().expect("App::new");
    // 先进入搜索层
    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(app.current_layer(), AppLayer::Search);

    // 搜索中按 Ctrl+P 切到命令面板
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert_eq!(app.current_layer(), AppLayer::Palette);

    // Esc 关闭 palette 后，因为 ProcessPanel 搜索仍 active，会回到 Search 层
    // —— 这与 spec 一致（搜索状态不被 palette 清空，用户可继续搜索）。
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.current_layer(), AppLayer::Search);
}

// ── 额外回归：Ctrl+P 在 Help 模式下不打开 palette（被 Help 拦截） ────────────

#[test]
fn ctrl_p_in_help_does_not_open_palette() {
    let mut app = App::new().expect("App::new");
    // 进入 Help 模式（`?`）
    app.handle_key(key(KeyCode::Char('?')));
    assert_eq!(app.mode, AppMode::Help);

    // Ctrl+P 在 Help 模式下应该被忽略（Help 是「学习中」，不该被打断）。
    app.handle_key(ctrl(KeyCode::Char('p')));
    assert!(
        !app.is_palette_open(),
        "Help 模式下 Ctrl+P 不该打开命令面板"
    );
    assert_eq!(app.mode, AppMode::Help);
}

// ── 额外回归：palette action 执行后真的生效（SwitchPanel 案例） ──────────────

#[test]
fn palette_action_switch_panel_actually_switches() {
    let mut app = App::new().expect("App::new");
    assert_eq!(app.mode, AppMode::ProcessList);

    app.handle_key(ctrl(KeyCode::Char('p')));
    // fuzzy 搜 "port" → Switch to Port Panel 应是头号匹配（label 含 "Port"）。
    for c in "port panel".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    // 检查第 0 个匹配就是 Switch to Port Panel
    let palette = &app.command_palette;
    let top_idx = palette.matched_indices()[0];
    let top_label = palette.items()[top_idx].label;
    assert!(
        top_label.contains("Port Panel"),
        "expected top match to be Port Panel, got {top_label:?}"
    );

    // Enter 执行 → 模式应切到 PortMap
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.mode, AppMode::PortMap);
    assert!(!app.is_palette_open());
}
