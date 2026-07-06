//! v0.14 stage 4 测试：录屏倒放（ReplayController 加 direction 字段 +
//! tick 双向分支 + r 键切方向 + timeline ▶/◀/⏸ 三态）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proc::record::Player;
use proc::record::frame::{FrameProcess, UiFrame};
use proc::record::writer::Recorder;
use proc::replay::{ReplayAction, ReplayController, ReplayDirection, ReplaySpeed};

// ── Fixtures ───────────────────────────────────────────────────────────────

fn make_test_frame(timestamp: u64, cpu: f32) -> UiFrame {
    UiFrame {
        timestamp,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: cpu,
        memory_used: 8 * 1024 * 1024 * 1024,
        memory_total: 16 * 1024 * 1024 * 1024,
        net_down: 0,
        net_up: 0,
        cpu_history: vec![],
        mem_history: vec![],
        processes: vec![FrameProcess {
            pid: 1,
            name: "p".to_string(),
            cpu,
            memory: 1024,
            disk_read: 0,
            disk_write: 0,
        }],
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: vec![],
        port_entries: vec![],
        port_view_mode: 0,
        port_process_groups: vec![],
        port_remote_groups: vec![],
        connection_diff: Default::default(),
        anomalies: vec![],
        usb_devices: vec![],
        usb_locks: vec![],
        monitors: vec![],
        docker_containers: vec![],
        docker_events: vec![],
        ops: vec![],
        nav: Default::default(),
    }
}

fn write_test_prec(dir: &std::path::Path, name: &str, frames: u64) -> std::path::PathBuf {
    let path = dir.join(name);
    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..frames {
        recorder.submit_frame(make_test_frame(1000 + i, 10.0 + i as f32));
    }
    recorder.stop().unwrap();
    path
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn start_at(
    player: Player,
    current: usize,
    playing: bool,
    direction: ReplayDirection,
) -> ReplayController {
    let mut ctrl = ReplayController::new();
    ctrl.start(player);
    if let Some(ts) = ctrl.timeline_state.as_mut() {
        ts.current_frame = current;
        ts.playing = playing;
        ts.direction = direction;
    }
    ctrl
}

// ── ReplayDirection enum ───────────────────────────────────────────────────

#[test]
fn direction_default_is_forward() {
    assert_eq!(ReplayDirection::default(), ReplayDirection::Forward);
}

#[test]
fn direction_toggle_round_trip() {
    let f = ReplayDirection::Forward;
    assert_eq!(f.toggle(), ReplayDirection::Reverse);
    assert_eq!(f.toggle().toggle(), ReplayDirection::Forward);
}

#[test]
fn direction_is_reverse_correct() {
    assert!(!ReplayDirection::Forward.is_reverse());
    assert!(ReplayDirection::Reverse.is_reverse());
}

#[test]
fn direction_icon_correct() {
    assert_eq!(ReplayDirection::Forward.icon(), "\u{25B6}"); // ▶
    assert_eq!(ReplayDirection::Reverse.icon(), "\u{25C0}"); // ◀
}

// ── start() 默认方向 ────────────────────────────────────────────────────────

#[test]
fn replay_start_default_direction_forward() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 3);
    let player = Player::open(path).unwrap();
    let ctrl = start_at(player, 0, false, ReplayDirection::Forward);
    let ts = ctrl.timeline_state.as_ref().unwrap();
    assert_eq!(ts.direction, ReplayDirection::Forward);
}

// ── r 键切方向 ──────────────────────────────────────────────────────────────

#[test]
fn r_key_toggles_direction_forward_to_reverse() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 5);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 0, false, ReplayDirection::Forward);

    let action = ctrl.handle_key(key(KeyCode::Char('r')));
    assert!(matches!(action, ReplayAction::DirectionToggled));
    let ts = ctrl.timeline_state.as_ref().unwrap();
    assert_eq!(ts.direction, ReplayDirection::Reverse);
}

#[test]
fn r_key_toggles_direction_reverse_to_forward() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 5);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 0, false, ReplayDirection::Reverse);

    let action = ctrl.handle_key(key(KeyCode::Char('r')));
    assert!(matches!(action, ReplayAction::DirectionToggled));
    let ts = ctrl.timeline_state.as_ref().unwrap();
    assert_eq!(ts.direction, ReplayDirection::Forward);
}

#[test]
fn r_key_does_not_conflict_with_shift_r() {
    // Shift+R 不应在 ReplayController 路径触发切方向（按 doc 决策保留 fallthrough）
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 5);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 0, false, ReplayDirection::Forward);

    let action = ctrl.handle_key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT));
    // Shift+R 未绑定 → Noop；方向不变
    assert!(matches!(action, ReplayAction::Noop));
    let ts = ctrl.timeline_state.as_ref().unwrap();
    assert_eq!(ts.direction, ReplayDirection::Forward);
}

// ── tick 双向分支 ───────────────────────────────────────────────────────────

#[test]
fn tick_forward_advances_frame() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 3);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 0, true, ReplayDirection::Forward);

    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::ApplyFrame));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 1);
}

#[test]
fn tick_reverse_decrements_frame() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 3);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 2, true, ReplayDirection::Reverse);

    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::ApplyFrame));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 1);
}

#[test]
fn tick_reverse_at_first_frame_pauses() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 3);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 0, true, ReplayDirection::Reverse);

    let action = ctrl.tick();
    // 到首帧暂停 → step=1 但 current_frame 仍 0（saturating_sub）
    // doc 设计：到边界自动暂停，但 step > 0 时返 ApplyFrame（已推进；不动也只是边界）
    assert!(matches!(action, ReplayAction::ApplyFrame));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 0);
    assert!(!ctrl.timeline_state.as_ref().unwrap().playing);
}

#[test]
fn tick_forward_at_last_frame_pauses() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 3);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 2, true, ReplayDirection::Forward);

    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::ApplyFrame));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 2);
    assert!(!ctrl.timeline_state.as_ref().unwrap().playing);
}

#[test]
fn tick_reverse_half_speed_every_other() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 3);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 2, true, ReplayDirection::Reverse);
    if let Some(ts) = ctrl.timeline_state.as_mut() {
        ts.speed = ReplaySpeed::Half;
    }

    // Half speed：第一个 tick step=0（half_tick 翻 0→1），不推进
    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::Noop));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 2);

    // 第二个 tick step=1（half_tick 翻 1→0），推进到 1
    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::ApplyFrame));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 1);
}

#[test]
fn tick_reverse_quad_speed_skips_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 8, true, ReplayDirection::Reverse);
    if let Some(ts) = ctrl.timeline_state.as_mut() {
        ts.speed = ReplaySpeed::Quad;
    }

    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::ApplyFrame));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 4);
}

#[test]
fn tick_reverse_with_step_larger_than_current_clamps_to_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 2, true, ReplayDirection::Reverse);
    if let Some(ts) = ctrl.timeline_state.as_mut() {
        ts.speed = ReplaySpeed::Quad;
    }

    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::ApplyFrame));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 0);
    assert!(!ctrl.timeline_state.as_ref().unwrap().playing);
}

#[test]
fn tick_does_not_advance_when_bookmark_panel_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 5, true, ReplayDirection::Forward);
    // 打开书签面板（B 键）
    ctrl.handle_key(KeyEvent::new(KeyCode::Char('B'), KeyModifiers::SHIFT));
    assert!(ctrl.bookmark_panel.is_some());

    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::Noop));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 5);
}

#[test]
fn tick_reverse_does_not_advance_when_paused() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 5, false, ReplayDirection::Reverse);

    let action = ctrl.tick();
    assert!(matches!(action, ReplayAction::Noop));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 5);
}

// ── 边界情况 ────────────────────────────────────────────────────────────────

#[test]
fn toggle_direction_keeps_current_frame() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 5, true, ReplayDirection::Forward);

    ctrl.handle_key(key(KeyCode::Char('r')));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 5);
    assert_eq!(
        ctrl.timeline_state.as_ref().unwrap().direction,
        ReplayDirection::Reverse
    );
}

#[test]
fn toggle_direction_does_not_reset_half_tick() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 5, true, ReplayDirection::Forward);
    if let Some(ts) = ctrl.timeline_state.as_mut() {
        ts.speed = ReplaySpeed::Half;
    }

    // 第一个 tick：half_tick 0→1，step=0
    ctrl.tick();
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().half_tick, 1);

    // 切方向不重置 half_tick
    ctrl.handle_key(key(KeyCode::Char('r')));
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().half_tick, 1);

    // 第二个 tick：half_tick 1→0，step=1，倒放推进（current 5→4）
    ctrl.tick();
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().half_tick, 0);
    assert_eq!(ctrl.timeline_state.as_ref().unwrap().current_frame, 4);
}

// ── 集成：search n/N 与 direction 解耦 ───────────────────────────────────────

#[test]
fn search_next_match_independent_of_direction() {
    use proc::filter::FilterExpr;
    use proc::replay::search::ReplaySearch;

    // 直接构造一个 ReplaySearch（不需要 Player），手动塞 matches
    let mut s = ReplaySearch::new();
    s.matches = vec![5, 10, 15];
    s.cursor = 0;

    // Forward 状态下按 n → cursor +1
    assert_eq!(s.next_match(), Some(10));
    assert_eq!(s.cursor, 1);

    // 模拟切方向（direction 字段不在 search 上，跳过 controller 路径）
    // 再按 n → cursor +1（与 direction 无关，n 始终往 matches 末尾移动）
    assert_eq!(s.next_match(), Some(15));
    assert_eq!(s.cursor, 2);

    // 模拟「direction = Reverse」后 n 行为不变（搜索是「跳转」语义）
    assert_eq!(s.next_match(), Some(15)); // clamp 末尾
    // N 始终往 matches 起点移动
    assert_eq!(s.prev_match(), Some(10));
    assert_eq!(s.cursor, 1);

    // 防止 unused warning
    let _: Option<&FilterExpr> = s.expr.as_ref();
}

// ── 集成：search input 与 direction 不冲突 ───────────────────────────────────

#[test]
fn r_key_ignored_when_search_input_active() {
    // search input 激活时 r 应推字符到搜索 input，不切方向
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 5);
    let player = Player::open(path).unwrap();
    let mut ctrl = start_at(player, 0, false, ReplayDirection::Forward);

    // 进入搜索输入态
    ctrl.handle_key(key(KeyCode::Char('/')));
    assert!(ctrl.search_input_active);

    // 按 r → 字符 push 到 search input
    let action = ctrl.handle_key(key(KeyCode::Char('r')));
    assert!(matches!(action, ReplayAction::SearchMatchesUpdated));
    assert_eq!(ctrl.search.input, "r");
    // 方向不变
    assert_eq!(
        ctrl.timeline_state.as_ref().unwrap().direction,
        ReplayDirection::Forward
    );
}
