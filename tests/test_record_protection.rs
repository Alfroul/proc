//! v0.6.0 阶段 2 — 录屏防护集成测试。
//!
//! 覆盖 pending_record_confirm 状态机的 4 个状态转换：
//! - idle → pending（按 R）
//! - pending → recording（按 y）
//! - pending → idle（按 n / Esc / q）
//! - recording → idle（按 R 停止）
//!
//! 以及录屏中强制 mask 的不变量：recording_wanted=true 时即便 env_reveal=true，
//! detail_view 计算的 `reveal` 也必须是 false（用相同表达式验证）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use proc::app::App;

#[test]
fn fresh_app_no_pending_confirm() {
    let app = App::new().expect("App::new");
    assert!(
        !app.pending_record_confirm,
        "fresh App should not be pending"
    );
    assert!(!app.is_recording(), "fresh App should not be recording");
}

#[test]
fn pressing_r_enters_pending_confirm_not_recording() {
    let mut app = App::new().expect("App::new");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    assert!(
        app.pending_record_confirm,
        "R should enter pending_record_confirm state",
    );
    assert!(
        !app.is_recording(),
        "R alone should NOT start recording — must confirm with y",
    );
    assert!(
        app.status_message
            .as_ref()
            .map(|m| m.contains("DNS") || m.contains("进程"))
            .unwrap_or(false),
        "status should warn about DNS / process cmd capture",
    );
}

#[test]
fn pending_confirm_y_starts_recording_and_resets_env_reveal() {
    let mut app = App::new().expect("App::new");
    // 用户先打开 env_reveal（reveal 真值），再触发录屏
    app.inspector.env_reveal = true;

    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    assert!(app.pending_record_confirm);

    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(!app.pending_record_confirm, "y should clear pending flag");
    assert!(app.is_recording(), "y should start recording");
    assert!(
        !app.inspector.env_reveal,
        "recording start must force-reset env_reveal to mask secrets",
    );
}

#[test]
fn pending_confirm_n_cancels() {
    let mut app = App::new().expect("App::new");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    assert!(app.pending_record_confirm);

    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(!app.pending_record_confirm);
    assert!(!app.is_recording(), "n should not start recording");
}

#[test]
fn pending_confirm_esc_cancels() {
    let mut app = App::new().expect("App::new");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.pending_record_confirm);
    assert!(!app.is_recording());
}

#[test]
fn pending_confirm_q_cancels() {
    let mut app = App::new().expect("App::new");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(!app.pending_record_confirm);
    assert!(!app.is_recording());
}

#[test]
fn pending_confirm_random_key_swallowed() {
    let mut app = App::new().expect("App::new");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    // 按上下方向键不应退出 pending 状态
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    assert!(
        app.pending_record_confirm,
        "non y/n/Esc/q keys must be swallowed in pending state",
    );
    assert!(!app.is_recording());
}

#[test]
fn second_r_in_pending_state_cancels() {
    // 用户按 R 弹确认后改主意，再按一次 R → 取消（而不是开始录屏）
    let mut app = App::new().expect("App::new");
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    assert!(app.pending_record_confirm);
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    assert!(!app.pending_record_confirm);
    assert!(!app.is_recording());
}

#[test]
fn recording_r_again_stops_recording() {
    let mut app = App::new().expect("App::new");
    // 走完整流程启动录屏
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.is_recording());

    // 再按 R 停止
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    assert!(!app.is_recording(), "R during recording should stop it");
    assert!(!app.pending_record_confirm, "no re-enter pending on stop");
}

#[test]
fn reveal_during_recording_forced_mask() {
    // 验证 detail_view draw_env_tab 用的不变量表达式
    // `reveal = app.inspector.env_reveal && !app.is_recording()`
    let mut app = App::new().expect("App::new");

    // 模拟用户在录屏中（用公共 API 触发完整状态机；不直接写 私有字段）
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.is_recording());

    // 录屏中尝试开 env_reveal — detail_view 用 reveal = env_reveal && !recording
    app.inspector.env_reveal = true; // 即便 UI 不让切，这里直接置位验证不变量
    let reveal = app.inspector.env_reveal && !app.is_recording();
    assert!(!reveal, "recording must force mask even if env_reveal=true");
}

#[test]
fn detail_v_key_blocked_during_recording() {
    // 在 ProcessDetail 模式 + 录屏中按 v，env_reveal 必须保持 false
    use proc::app_panel::AppMode;
    use proc::collect::ProcessInfo;
    use proc::inspect;

    let mut app = App::new().expect("App::new");
    let pid = std::process::id();
    let proc = ProcessInfo {
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
        throttled: proc::throttle::EcoQoSState::default(),
    };
    app.inspector.detail_process = Some(proc.clone());
    app.inspector.inspection_data = Some(inspect::inspect(pid));
    app.mode = AppMode::ProcessDetail;

    // 进入录屏
    app.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert!(app.is_recording());

    // 详情页按 v — 应该被拒
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    assert!(
        !app.inspector.env_reveal,
        "v during recording must NOT enable env_reveal",
    );
}
