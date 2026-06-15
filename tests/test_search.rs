use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proc::search::SearchState;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn test_new_state() {
    let s = SearchState::new();
    assert!(!s.is_active());
    assert!(s.query().is_empty());
}

#[test]
fn test_activate_and_type() {
    let mut s = SearchState::new();
    s.active = true;
    assert!(s.is_active());

    assert!(s.handle_input(key(KeyCode::Char('h'))));
    assert!(s.handle_input(key(KeyCode::Char('e'))));
    assert!(s.handle_input(key(KeyCode::Char('l'))));
    assert!(s.handle_input(key(KeyCode::Char('l'))));
    assert!(s.handle_input(key(KeyCode::Char('o'))));
    assert_eq!(s.query(), "hello");
}

#[test]
fn test_backspace() {
    let mut s = SearchState::new();
    s.query = "abc".to_string();
    s.active = true;

    assert!(s.handle_input(key(KeyCode::Backspace)));
    assert_eq!(s.query(), "ab");

    assert!(s.handle_input(key(KeyCode::Backspace)));
    assert_eq!(s.query(), "a");

    // Backspace on empty does nothing
    assert!(s.handle_input(key(KeyCode::Backspace)));
    assert_eq!(s.query(), "");
}

#[test]
fn test_enter_closes_search() {
    let mut s = SearchState::new();
    s.active = true;
    s.query = "test".to_string();

    assert!(s.handle_input(key(KeyCode::Enter)));
    assert!(!s.is_active());
    // Enter does NOT clear the query — keeps filter active
    assert_eq!(s.query(), "test");
}

#[test]
fn test_esc_closes_and_clears() {
    let mut s = SearchState::new();
    s.active = true;
    s.query = "test".to_string();

    assert!(s.handle_input(key(KeyCode::Esc)));
    assert!(!s.is_active());
    assert!(s.query().is_empty());
}

#[test]
fn test_clear() {
    let mut s = SearchState::new();
    s.active = true;
    s.query = "test".to_string();

    s.clear();
    assert!(!s.is_active());
    assert!(s.query().is_empty());
}

#[test]
fn test_unhandled_key() {
    let mut s = SearchState::new();
    s.active = true;

    assert!(!s.handle_input(key(KeyCode::Up)));
    assert!(!s.handle_input(key(KeyCode::Down)));
    assert!(!s.handle_input(key(KeyCode::Tab)));
    assert!(s.query().is_empty());
}
