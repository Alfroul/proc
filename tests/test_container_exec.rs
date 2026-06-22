//! 阶段 9 E2 — 容器 exec 集成测试。
//!
//! 覆盖：
//! - `detect_default_shell` 多 image 推断矩阵（跨 alpine / busybox / ubuntu / dev / 未知）
//! - `key_event_to_pty_bytes` 键盘事件 → ANSI 字节转换矩阵（Enter / Ctrl+C / Backspace / 方向 / Alt+x / 普通 char）
//! - vt100 color / attrs 转换（已在 src/tui/container_exec_view.rs 单元测试覆盖）
//!
//! 真实 PTY + docker 测试需要 Docker 在跑，CI 上 cfg-gate，本测试仅覆盖纯函数。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proc::docker::exec::detect_default_shell;
use proc::tui::container_exec_view::key_event_to_pty_bytes;

// ── detect_default_shell ──

#[test]
fn shell_alpine_family() {
    assert_eq!(detect_default_shell("alpine:3.18"), "/bin/sh");
    assert_eq!(detect_default_shell("alpine"), "/bin/sh");
    assert_eq!(detect_default_shell("ALPINE:latest"), "/bin/sh");
    assert_eq!(detect_default_shell("docker.io/library/alpine"), "/bin/sh");
    assert_eq!(detect_default_shell("busybox:stable"), "/bin/sh");
}

#[test]
fn shell_ubuntu_debian_family() {
    assert_eq!(detect_default_shell("ubuntu:22.04"), "/bin/bash");
    assert_eq!(detect_default_shell("ubuntu"), "/bin/bash");
    assert_eq!(detect_default_shell("debian:bookworm-slim"), "/bin/bash");
    assert_eq!(detect_default_shell("centos:stream9"), "/bin/bash");
    assert_eq!(detect_default_shell("fedora:40"), "/bin/bash");
}

#[test]
fn shell_dev_images() {
    assert_eq!(detect_default_shell("rust:1.75"), "/bin/bash");
    assert_eq!(detect_default_shell("golang:1.21-alpine"), "/bin/sh"); // alpine 优先
    assert_eq!(detect_default_shell("python:3.12"), "/bin/bash");
    assert_eq!(detect_default_shell("node:20"), "/bin/bash");
}

#[test]
fn shell_unknown_falls_back_to_sh() {
    assert_eq!(detect_default_shell("nginx:latest"), "/bin/sh");
    assert_eq!(detect_default_shell("postgres:16"), "/bin/sh");
    assert_eq!(detect_default_shell("redis:7"), "/bin/sh");
    assert_eq!(detect_default_shell(""), "/bin/sh");
    assert_eq!(detect_default_shell("my-custom-image:v1.2"), "/bin/sh");
}

#[test]
fn shell_returns_static_str() {
    // 编译期保证返回 'static str（避免 String 分配）。
    let s: &'static str = detect_default_shell("anything");
    assert!(!s.is_empty());
}

// ── key_event_to_pty_bytes ──

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

#[test]
fn key_enter_to_carriage_return() {
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Enter)),
        Some(b"\r".to_vec())
    );
}

#[test]
fn key_tab_and_backtab() {
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Tab)),
        Some(b"\t".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::BackTab)),
        Some(b"\x1b[Z".to_vec())
    );
}

#[test]
fn key_backspace_to_del_byte() {
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Backspace)),
        Some(b"\x7f".to_vec())
    );
}

#[test]
fn key_arrow_keys_to_ansi_sequences() {
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Up)),
        Some(b"\x1b[A".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Down)),
        Some(b"\x1b[B".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Right)),
        Some(b"\x1b[C".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Left)),
        Some(b"\x1b[D".to_vec())
    );
}

#[test]
fn key_navigation_keys_to_ansi() {
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Home)),
        Some(b"\x1b[H".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::End)),
        Some(b"\x1b[F".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Delete)),
        Some(b"\x1b[3~".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::PageUp)),
        Some(b"\x1b[5~".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::PageDown)),
        Some(b"\x1b[6~".to_vec())
    );
}

#[test]
fn key_ctrl_c_interrupt_byte() {
    assert_eq!(key_event_to_pty_bytes(ctrl('c')), Some(vec![0x03]));
}

#[test]
fn key_ctrl_d_eof_byte() {
    assert_eq!(key_event_to_pty_bytes(ctrl('d')), Some(vec![0x04]));
}

#[test]
fn key_ctrl_backslash_sigquit() {
    assert_eq!(key_event_to_pty_bytes(ctrl('\\')), Some(vec![0x1c]));
}

#[test]
fn key_ctrl_letters_to_control_bytes() {
    // Ctrl+A=0x01, Ctrl+E=0x05, Ctrl+K=0x0b, Ctrl+U=0x15, Ctrl+W=0x17, Ctrl+L=0x0c, Ctrl+R=0x12
    assert_eq!(key_event_to_pty_bytes(ctrl('a')), Some(vec![0x01]));
    assert_eq!(key_event_to_pty_bytes(ctrl('e')), Some(vec![0x05]));
    assert_eq!(key_event_to_pty_bytes(ctrl('k')), Some(vec![0x0b]));
    assert_eq!(key_event_to_pty_bytes(ctrl('u')), Some(vec![0x15]));
    assert_eq!(key_event_to_pty_bytes(ctrl('w')), Some(vec![0x17]));
    assert_eq!(key_event_to_pty_bytes(ctrl('l')), Some(vec![0x0c]));
    assert_eq!(key_event_to_pty_bytes(ctrl('r')), Some(vec![0x12]));
    // 未显式列出的 Ctrl+字母：按 ASCII & 0x1f 规则。
    // Ctrl+Z = 'z' as u8 (122) & 0x1f = 26 = 0x1A
    assert_eq!(key_event_to_pty_bytes(ctrl('z')), Some(vec![0x1a]));
    // Ctrl+] = ']' as u8 (93) & 0x1f = 29 = 0x1D
    assert_eq!(key_event_to_pty_bytes(ctrl(']')), Some(vec![0x1d]));
}

#[test]
fn key_alt_letter_to_esc_prefix() {
    // Alt+x = \x1b + 'x'
    let bytes = key_event_to_pty_bytes(alt('x')).unwrap();
    assert_eq!(bytes, vec![0x1b, b'x']);
    // Alt+Enter → \x1b + \r（crossterm 在 ALT 修饰时 KeyCode 仍是 Enter，但我们的实现走 _ 分支）
    // 注：实际 crossterm ALT+Enter 是 Enter+ALT，按当前实现走 Enter 分支返回 \r，不处理 ALT 修饰。
    // 这是有意的（vim 等程序的 Alt 修饰键很少与 Enter 组合）。
    let alt_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
    assert_eq!(key_event_to_pty_bytes(alt_enter), Some(b"\r".to_vec()));
}

#[test]
fn key_plain_char_to_utf8_bytes() {
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Char('a'))),
        Some(b"a".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Char('1'))),
        Some(b"1".to_vec())
    );
    assert_eq!(
        key_event_to_pty_bytes(key(KeyCode::Char(' '))),
        Some(b" ".to_vec())
    );
    // 中文 → UTF-8 三字节
    let bytes = key_event_to_pty_bytes(key(KeyCode::Char('中'))).unwrap();
    assert_eq!(bytes, "中".as_bytes());
}

#[test]
fn key_unmapped_returns_none() {
    // F1-F12 / Null 等无标准 PTY 映射。
    assert_eq!(key_event_to_pty_bytes(key(KeyCode::F(1))), None);
    assert_eq!(key_event_to_pty_bytes(key(KeyCode::F(12))), None);
    assert_eq!(key_event_to_pty_bytes(key(KeyCode::Null)), None);
    // CapsLock / ScrollLock / Menu 等修改键事件（crossterm 的 KeyCode::Modifier变种在 0.28 不存在）。
    // 仅测已知 None 路径。
}

// ── ContainerExec::start（PATH 无 docker 时优雅报错）──
//
// 默认 ignore：需要真实 docker daemon 在跑，CI 上 `cargo test -- --ignored` 触发。
// 阶段 9 验收主要靠用户在 TUI 内手动验证（按 e 进容器跑 ls 等命令）。

#[test]
#[ignore = "需要真实 docker daemon；cargo test -- --ignored 触发"]
fn container_exec_start_no_docker_returns_error() {
    let result =
        proc::docker::exec::ContainerExec::start("proc-test-definitely-not-exists-xyz", &[], None);
    let err = match result {
        Ok(_) => panic!("expected Err for non-existent container"),
        Err(e) => e.to_string(),
    };
    assert!(
        err.contains("docker") || err.contains("spawn") || err.contains("exec"),
        "err should mention docker/spawn/exec, got: {err}"
    );
}
