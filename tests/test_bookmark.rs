//! v0.14 stage 2 — 录屏书签系统集成测试。
//!
//! 覆盖：
//! 1. 录制路径：App::handle_key 在 recording_wanted 时按 `b` 触发 inline label，
//!    字符 push / Backspace pop / Enter 提交 / Esc 取消 / 默认 label 兜底。
//! 2. 回放路径：Player + ReplayController 加载 sidecar、`B` 键打开面板、
//!    Up/Down 移动 cursor、Enter 跳帧、`d` 删除、`e` 编辑、搜索过滤。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use proc::app::App;
use proc::app_panel::AppMode;
use proc::record::frame::{FrameProcess, UiFrame};
use proc::record::writer::Recorder;
use proc::record::{BookmarkFile, Player};

// ── Test fixtures ───────────────────────────────────────────────────────────

fn make_test_frame(timestamp: u64, cpu: f32) -> UiFrame {
    UiFrame {
        timestamp,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: cpu,
        memory_used: 8 * 1024 * 1024 * 1024,
        memory_total: 16 * 1024 * 1024 * 1024,
        net_down: 1000,
        net_up: 500,
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

/// 写一个含 N 帧的 .prec 文件，返回路径。
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

fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

// ── 录制路径：b 键 inline label ──────────────────────────────────────────────

#[test]
fn recording_b_key_starts_label_input() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);
    app.set_recording_frame_count(7);

    // 在 ProcessList 模式下按 b → 进入 inline label 状态
    assert_eq!(app.mode, AppMode::ProcessList);
    app.handle_key(key(KeyCode::Char('b')));
    assert!(
        app.pending_bookmark_label.is_some(),
        "b 键应触发 inline label 输入"
    );
    let pending = app.pending_bookmark_label.as_ref().unwrap();
    assert_eq!(
        pending.frame_idx, 7,
        "frame_idx 应是当前 recording_frame_count"
    );
    assert!(pending.input.is_empty());
}

#[test]
fn recording_label_input_appends_chars_until_enter() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);
    app.set_recording_frame_count(3);
    app.handle_key(key(KeyCode::Char('b')));

    for c in "CPU 飙升".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(
        app.pending_bookmark_label.as_ref().unwrap().input,
        "CPU 飙升"
    );

    // Enter 提交
    app.handle_key(key(KeyCode::Enter));
    assert!(app.pending_bookmark_label.is_none());
    assert_eq!(app.recording_bookmarks().len(), 1);
    let bm = &app.recording_bookmarks()[0];
    assert_eq!(bm.frame_idx, 3);
    assert_eq!(bm.label, "CPU 飙升");
}

#[test]
fn recording_label_empty_input_uses_default_label() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);
    app.set_recording_frame_count(0);
    app.handle_key(key(KeyCode::Char('b')));
    // 直接 Enter（空 label）
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.recording_bookmarks().len(), 1);
    let bm = &app.recording_bookmarks()[0];
    assert_eq!(bm.label, "书签 #1", "空 label 应默认 '书签 #N'");
}

#[test]
fn recording_label_backspace_pops_last_char() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);
    app.set_recording_frame_count(1);
    app.handle_key(key(KeyCode::Char('b')));
    for c in "hello".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    assert_eq!(app.pending_bookmark_label.as_ref().unwrap().input, "hello");

    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.pending_bookmark_label.as_ref().unwrap().input, "hell");

    app.handle_key(key(KeyCode::Enter));
    let bm = &app.recording_bookmarks()[0];
    assert_eq!(bm.label, "hell");
}

#[test]
fn recording_label_esc_cancels_without_saving() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);
    app.set_recording_frame_count(2);
    app.handle_key(key(KeyCode::Char('b')));
    for c in "test".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }

    app.handle_key(key(KeyCode::Esc));
    assert!(app.pending_bookmark_label.is_none());
    assert_eq!(app.recording_bookmarks().len(), 0, "Esc 取消不应保存书签");
}

#[test]
fn recording_multiple_bookmarks_get_increasing_ids() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);

    app.set_recording_frame_count(5);
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Enter)); // 默认 label

    app.set_recording_frame_count(10);
    app.handle_key(key(KeyCode::Char('b')));
    for c in "second".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));

    app.set_recording_frame_count(15);
    app.handle_key(key(KeyCode::Char('b')));
    app.handle_key(key(KeyCode::Enter));

    let bookmarks = app.recording_bookmarks();
    assert_eq!(bookmarks.len(), 3);
    assert_eq!(bookmarks[0].id, 1);
    assert_eq!(bookmarks[0].frame_idx, 5);
    assert_eq!(bookmarks[1].id, 2);
    assert_eq!(bookmarks[1].frame_idx, 10);
    assert_eq!(bookmarks[2].id, 3);
    assert_eq!(bookmarks[2].frame_idx, 15);
}

#[test]
fn recording_b_key_ignored_in_replay_mode() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);

    // 写一个临时 .prec 让 ReplayController.start 可用
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 5);

    // 切到 Replay 模式
    let player = Player::open(path).unwrap();
    app.start_replay(player);
    assert_eq!(app.mode, AppMode::Replay);

    // 录制中按 b（在 Replay 模式下不应触发 inline label）
    app.handle_key(key(KeyCode::Char('b')));
    assert!(
        app.pending_bookmark_label.is_none(),
        "Replay 模式下 b 不应触发书签 inline label"
    );
}

#[test]
fn recording_b_key_ignored_when_search_active() {
    let mut app = App::new().expect("App::new");
    app.set_recording_wanted(true);
    // 进 search 模式（按 `/`）
    app.handle_key(key(KeyCode::Char('/')));

    // search 激活状态下按 b 应进搜索框，不应触发书签
    app.handle_key(key(KeyCode::Char('b')));
    assert!(
        app.pending_bookmark_label.is_none(),
        "search 激活时 b 不应触发书签"
    );
}

// ── 回放路径：sidecar 自动加载 + B 键面板 ────────────────────────────────────

#[test]
fn replay_loads_existing_sidecar_on_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "with_bm.prec", 10);

    // 预写 sidecar：3 个书签
    let mut file = BookmarkFile::empty_for(&path);
    file.add(2, 1002, "first".to_string(), 5000);
    file.add(5, 1005, "second".to_string(), 5001);
    file.add(8, 1008, "third".to_string(), 5002);
    file.write(&path);

    let player = Player::open(path).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    let bookmarks = app.replay.bookmarks.as_ref().expect("sidecar 应加载");
    assert_eq!(bookmarks.bookmarks.len(), 3);
    assert_eq!(bookmarks.bookmarks[0].label, "first");
    assert_eq!(bookmarks.bookmarks[2].frame_idx, 8);
}

#[test]
fn replay_b_key_opens_bookmark_panel() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 5);

    let player = Player::open(path).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    // B 键（Shift+B）打开书签面板（通过 app.handle_key 路由到 ReplayController）
    app.handle_key(shift(KeyCode::Char('B')));
    assert!(
        app.replay.bookmark_panel.is_some(),
        "Shift+B 应打开书签面板"
    );
}

#[test]
fn replay_bookmark_panel_esc_closes() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 5);

    let player = Player::open(path).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    app.handle_key(shift(KeyCode::Char('B')));
    assert!(app.replay.bookmark_panel.is_some());

    app.handle_key(key(KeyCode::Esc));
    assert!(app.replay.bookmark_panel.is_none(), "Esc 应关闭书签面板");
}

#[test]
fn replay_bookmark_panel_enter_jumps_to_frame() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 20);

    // 预写 sidecar：跳到第 15 帧
    let mut file = BookmarkFile::empty_for(&path);
    file.add(15, 1015, "jump target".to_string(), 5000);
    file.write(&path);

    let player = Player::open(path).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    let initial_frame = app.replay.timeline_state.as_ref().unwrap().current_frame;
    assert_eq!(initial_frame, 0);

    app.handle_key(shift(KeyCode::Char('B')));
    app.handle_key(key(KeyCode::Enter));

    let after_frame = app.replay.timeline_state.as_ref().unwrap().current_frame;
    assert_eq!(after_frame, 15, "Enter 应跳转到书签 frame_idx");
}

#[test]
fn replay_bookmark_panel_d_deletes_bookmark_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);

    let mut file = BookmarkFile::empty_for(&path);
    file.add(3, 1003, "to delete".to_string(), 5000);
    file.add(7, 1007, "keep".to_string(), 5001);
    file.write(&path);

    let player = Player::open(path.clone()).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    app.handle_key(shift(KeyCode::Char('B')));
    assert_eq!(app.replay.bookmarks.as_ref().unwrap().bookmarks.len(), 2);

    app.handle_key(key(KeyCode::Char('d')));
    assert_eq!(
        app.replay.bookmarks.as_ref().unwrap().bookmarks.len(),
        1,
        "d 应删除 cursor 处书签"
    );

    // sidecar 应持久化
    let reloaded = BookmarkFile::try_load(&path).expect("sidecar 应重 load 成功");
    assert_eq!(reloaded.bookmarks.len(), 1);
    assert_eq!(reloaded.bookmarks[0].label, "keep");
}

#[test]
fn replay_bookmark_panel_e_edits_label_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 10);

    let mut file = BookmarkFile::empty_for(&path);
    file.add(5, 1005, "old label".to_string(), 5000);
    file.write(&path);

    let player = Player::open(path.clone()).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    app.handle_key(shift(KeyCode::Char('B')));
    app.handle_key(key(KeyCode::Char('e')));

    // 编辑模式下输入新 label
    for c in "new label".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    app.handle_key(key(KeyCode::Enter));

    let bm = &app.replay.bookmarks.as_ref().unwrap().bookmarks[0];
    assert_eq!(bm.label, "new label");

    let reloaded = BookmarkFile::try_load(&path).expect("sidecar 应重 load 成功");
    assert_eq!(reloaded.bookmarks[0].label, "new label");
}

#[test]
fn replay_bookmark_panel_search_filters_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 50);

    let mut file = BookmarkFile::empty_for(&path);
    file.add(2, 1002, "CPU 飙升".to_string(), 5000);
    file.add(10, 1010, "mem leak".to_string(), 5001);
    file.add(25, 1025, "docker restart".to_string(), 5002);
    file.write(&path);

    let player = Player::open(path).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    app.handle_key(shift(KeyCode::Char('B')));
    // 输入 "cpu" 过滤
    for c in "cpu".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }

    let indices = app.replay.filtered_bookmark_indices();
    assert_eq!(indices.len(), 1, "搜索 'cpu' 应只命中 'CPU 飙升'");
    let bm = &app.replay.bookmarks.as_ref().unwrap().bookmarks[indices[0]];
    assert_eq!(bm.label, "CPU 飙升");
}

#[test]
fn replay_bookmark_panel_arrow_keys_move_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 50);

    let mut file = BookmarkFile::empty_for(&path);
    file.add(2, 1002, "a".to_string(), 5000);
    file.add(10, 1010, "b".to_string(), 5001);
    file.add(25, 1025, "c".to_string(), 5002);
    file.write(&path);

    let player = Player::open(path).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    app.handle_key(shift(KeyCode::Char('B')));
    assert_eq!(app.replay.bookmark_panel.as_ref().unwrap().cursor, 0);

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.replay.bookmark_panel.as_ref().unwrap().cursor, 1);

    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.replay.bookmark_panel.as_ref().unwrap().cursor, 2);

    // 在末尾再按 Down 不应越界
    app.handle_key(key(KeyCode::Down));
    assert_eq!(app.replay.bookmark_panel.as_ref().unwrap().cursor, 2);

    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.replay.bookmark_panel.as_ref().unwrap().cursor, 1);

    // 在首端再按 Up 不应越界
    app.handle_key(key(KeyCode::Up));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(app.replay.bookmark_panel.as_ref().unwrap().cursor, 0);
}

#[test]
fn replay_tick_paused_when_bookmark_panel_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_test_prec(dir.path(), "test.prec", 50);

    let mut file = BookmarkFile::empty_for(&path);
    file.add(10, 1010, "marker".to_string(), 5000);
    file.write(&path);

    let player = Player::open(path).unwrap();
    let mut app = App::new().expect("App::new");
    app.start_replay(player);

    // 启动播放
    app.replay.timeline_state.as_mut().unwrap().playing = true;

    // 打开书签面板 → tick 应返 Noop（不推进帧）
    app.handle_key(shift(KeyCode::Char('B')));
    let before = app.replay.timeline_state.as_ref().unwrap().current_frame;
    // 直接调 ReplayController.tick()（面板打开时应 Noop）
    let action = app.replay.tick();
    use proc::replay::ReplayAction;
    assert!(
        matches!(action, ReplayAction::Noop),
        "书签面板打开时 tick 应返 Noop"
    );
    let after = app.replay.timeline_state.as_ref().unwrap().current_frame;
    assert_eq!(before, after, "书签面板打开时 tick 不应推进帧");
}
