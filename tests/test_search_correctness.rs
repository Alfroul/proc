//! v0.6.0 阶段 4 — SearchState.query_lower 缓存正确性。
//!
//! 验证：
//! - `handle_input` 在 push / pop / Esc 时正确维护 `query_lower`
//! - 大小写混合 query 的 lowercase 缓存与 `query.to_lowercase()` 等价

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proc::search::SearchState;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

#[test]
fn query_lower_starts_empty() {
    let s = SearchState::new();
    assert_eq!(s.query_lower(), "");
    assert_eq!(s.query(), "");
}

#[test]
fn query_lower_tracks_lowercase_input() {
    let mut s = SearchState::new();
    s.active = true;
    for c in "chrome".chars() {
        s.handle_input(key(c));
    }
    assert_eq!(s.query(), "chrome");
    assert_eq!(s.query_lower(), "chrome");
}

#[test]
fn query_lower_lowercases_mixed_case_input() {
    let mut s = SearchState::new();
    s.active = true;
    for c in "Chrome.EXE".chars() {
        s.handle_input(key(c));
    }
    assert_eq!(s.query(), "Chrome.EXE");
    assert_eq!(s.query_lower(), "chrome.exe");
}

#[test]
fn backspace_updates_query_lower() {
    let mut s = SearchState::new();
    s.active = true;
    for c in "Discord".chars() {
        s.handle_input(key(c));
    }
    assert_eq!(s.query_lower(), "discord");

    s.handle_input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(s.query(), "Discor");
    assert_eq!(s.query_lower(), "discor");
}

#[test]
fn esc_clears_both_query_and_query_lower() {
    let mut s = SearchState::new();
    s.active = true;
    for c in "svchost".chars() {
        s.handle_input(key(c));
    }
    assert!(!s.query_lower().is_empty());

    s.handle_input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(s.query(), "");
    assert_eq!(s.query_lower(), "");
    assert!(!s.is_active());
}

#[test]
fn clear_resets_query_lower() {
    let mut s = SearchState::new();
    s.active = true;
    for c in "AbC".chars() {
        s.handle_input(key(c));
    }
    s.clear();
    assert_eq!(s.query_lower(), "");
    assert_eq!(s.query(), "");
}
