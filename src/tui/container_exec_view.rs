//! 阶段 9 E2 — 容器 exec 嵌入式 PTY 视图。
//!
//! 渲染策略：
//! - 全屏区域：遍历 `App::container_exec_vt` 的 `vt100::Parser::screen().cell(r, c)`，
//!   把每个 cell 的字符 + 颜色写到 ratatui buffer。
//! - 顶部 1 行：容器名 + 进入/退出提示。
//! - 底部 1 行：PTY 尺寸 + 快捷键。
//!
//! 颜色：vt100 的 `Color` 枚举（Default / Idx / Rgb）转 ratatui `Color`。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;

/// 把 crossterm `KeyEvent` 转成字节序列写进 PTY（容器 stdin）。
///
/// 返回 `None` = 该键不转发（如 F1-F12 等无标准 ANSI 映射的键）。
/// 转换协议（stage-9.md 7.1）：
/// - `Enter` → `\r`
/// - `Tab` → `\t`；`BackTab`（Shift+Tab）→ `\x1b[Z`
/// - `Backspace` → `\x7f`
/// - 方向 / Home / End / PageUp / PageDown / Delete → ANSI 序列
/// - `Ctrl+字母` → 0x01-0x1A 控制字符（Ctrl+C=`\x03`、Ctrl+D=`\x04`、Ctrl+\\=`\x1c`）
/// - `Alt+x` → `\x1b` + x
/// - 普通字符 → 该字符 UTF-8 字节
#[must_use]
pub fn key_event_to_pty_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let bytes: Option<Vec<u8>> = match key.code {
        KeyCode::Enter => Some(b"\r".to_vec()),
        KeyCode::Tab => Some(b"\t".to_vec()),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Backspace => Some(b"\x7f".to_vec()),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::Char(c) => {
            let mods = key.modifiers;
            if mods.contains(KeyModifiers::CONTROL) {
                match c {
                    // 常用 Ctrl 快捷键显式列出（方便阅读），其余按 ASCII xor 0x60 规则转。
                    'c' => Some(vec![0x03]),
                    'd' => Some(vec![0x04]),
                    '\\' => Some(vec![0x1c]),
                    'a' => Some(vec![0x01]),
                    'e' => Some(vec![0x05]),
                    'k' => Some(vec![0x0b]),
                    'u' => Some(vec![0x15]),
                    'w' => Some(vec![0x17]),
                    'l' => Some(vec![0x0c]),
                    'r' => Some(vec![0x12]),
                    _ => Some(vec![(c as u8) & 0x1f]),
                }
            } else if mods.contains(KeyModifiers::ALT) {
                let mut v = vec![0x1b];
                v.push(c as u8);
                Some(v)
            } else {
                Some(c.to_string().into_bytes())
            }
        }
        // F-keys / Null / 其它无标准 PTY 映射 → 不转发。
        _ => None,
    };
    bytes
}

/// 把 vt100 的 Color 转 ratatui Color。
fn vt_color_to_ratatui(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// 把 vt100 cell 的 bold/italic/underline/inverse 转 ratatui Modifier。
fn vt_attrs_to_modifier(cell: &vt100::Cell) -> Modifier {
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    m
}

/// 渲染容器 exec 视图。layout.rs::draw_main_panel 在 ContainerExec 分支调用。
pub fn draw(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    // 顶部 1 行容器名 + 退出提示；底部 1 行快捷键；中间 PTY 输出。
    let [header_area, pty_area, footer_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Min(0),
        ratatui::layout::Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, header_area, app);
    draw_pty(f, pty_area, app);
    draw_footer(f, footer_area, app);
}

fn draw_header(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let container_name = app
        .container_exec
        .as_ref()
        .map(|ce| ce.container.as_str())
        .unwrap_or("<未连接>");
    let title = format!(" 容器 exec：{container_name} ");
    let exit_msg = app.container_exec_exit_msg.as_deref().unwrap_or("");
    let line = if exit_msg.is_empty() {
        format!("{title}（Ctrl+D 退出 / Ctrl+C 中断容器 / Ctrl+\\ SIGQUIT）")
    } else {
        format!("{title} — {exit_msg}")
    };
    let p = Paragraph::new(line).style(crate::tui::theme::style_header());
    f.render_widget(p, area);
}

fn draw_footer(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let (cols, rows) = app
        .container_exec
        .as_ref()
        .map(|ce| ce.size())
        .unwrap_or((80, 24));
    let base = format!(" PTY {cols}×{rows}  按键直接透传容器（Enter/Ctrl+C/方向键已映射） ");
    let p = Paragraph::new(base).style(crate::tui::theme::style_muted());
    f.render_widget(p, area);
}

fn draw_pty(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    // 无边框占满：让嵌入式终端最大化可用面积。
    let block = Block::default()
        .borders(Borders::NONE)
        .style(crate::tui::theme::style_normal());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(parser) = app.container_exec_vt.as_ref() else {
        let p = Paragraph::new(" PTY 未就绪 — 等待 docker exec 启动...")
            .style(crate::tui::theme::style_muted());
        f.render_widget(p, inner);
        return;
    };
    let screen = parser.screen();

    // vt100::Screen::size() 返回 (cols, rows) 都是 u16。
    let (screen_cols, screen_rows) = screen.size();
    let pty_rows = screen_rows.min(inner.height);
    let pty_cols = screen_cols.min(inner.width);

    let buf: &mut Buffer = f.buffer_mut();
    for y in 0..pty_rows {
        for x in 0..pty_cols {
            // vt100::Screen::cell(row, col) → Option<&Cell>。
            if let Some(cell) = screen.cell(y, x) {
                let Some(cell_buf) = buf.cell_mut((inner.x + x, inner.y + y)) else {
                    continue;
                };
                // cell.contents() 包含 1+ 个 char（含 zero-width combining）。
                let symbol = cell.contents();
                let display = if symbol.is_empty() {
                    " ".into()
                } else {
                    symbol
                };
                cell_buf.set_symbol(&display);
                cell_buf.set_fg(vt_color_to_ratatui(cell.fgcolor()));
                cell_buf.set_bg(vt_color_to_ratatui(cell.bgcolor()));
                let modi = vt_attrs_to_modifier(cell);
                if !modi.is_empty() {
                    cell_buf.set_style(Style::default().add_modifier(modi));
                }
            }
        }
    }

    // 渲染光标。vt100::Screen::cursor_position() 返回 (row, col)。
    let (cur_row, cur_col) = screen.cursor_position();
    if cur_row < pty_rows && cur_col < pty_cols {
        if let Some(cell_buf) = buf.cell_mut((inner.x + cur_col, inner.y + cur_row)) {
            cell_buf.set_style(
                Style::default()
                    .bg(Color::Gray)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_color_maps_to_reset() {
        assert_eq!(vt_color_to_ratatui(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn idx_color_maps_to_indexed() {
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Idx(31)),
            Color::Indexed(31)
        );
        assert_eq!(vt_color_to_ratatui(vt100::Color::Idx(0)), Color::Indexed(0));
    }

    #[test]
    fn rgb_color_maps_to_rgb() {
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Rgb(0xFF, 0x00, 0x00)),
            Color::Rgb(0xFF, 0x00, 0x00)
        );
        assert_eq!(
            vt_color_to_ratatui(vt100::Color::Rgb(0xFF, 0xFF, 0xFF)),
            Color::Rgb(0xFF, 0xFF, 0xFF)
        );
    }

    // -------- key_event_to_pty_bytes 单测（阶段 11 P1-E1） --------

    fn ev(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ev_ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn ev_alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    #[test]
    fn pty_enter_produces_cr() {
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Enter)),
            Some(b"\r".to_vec())
        );
    }

    #[test]
    fn pty_tab_produces_tab_char() {
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Tab)),
            Some(b"\t".to_vec())
        );
    }

    #[test]
    fn pty_backtab_produces_csi_z() {
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::BackTab)),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn pty_backspace_produces_del() {
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Backspace)),
            Some(b"\x7f".to_vec())
        );
    }

    #[test]
    fn pty_arrow_keys_produce_ansi_cursor_sequences() {
        assert_eq!(key_event_to_pty_bytes(ev(KeyCode::Up)), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Down)),
            Some(b"\x1b[B".to_vec())
        );
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Right)),
            Some(b"\x1b[C".to_vec())
        );
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Left)),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn pty_home_end_produce_ansi_sequences() {
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Home)),
            Some(b"\x1b[H".to_vec())
        );
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::End)),
            Some(b"\x1b[F".to_vec())
        );
    }

    #[test]
    fn pty_del_pageup_pagedown_produce_ansi_sequences() {
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Delete)),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::PageUp)),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::PageDown)),
            Some(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn pty_ctrl_common_shortcuts_explicit_mappings() {
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('c')), Some(vec![0x03]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('d')), Some(vec![0x04]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('\\')), Some(vec![0x1c]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('a')), Some(vec![0x01]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('e')), Some(vec![0x05]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('k')), Some(vec![0x0b]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('u')), Some(vec![0x15]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('w')), Some(vec![0x17]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('l')), Some(vec![0x0c]));
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('r')), Some(vec![0x12]));
    }

    #[test]
    fn pty_ctrl_other_letters_apply_xor_0x1f_rule() {
        // 'x' as u8 = 0x78, & 0x1f = 0x18
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('x')), Some(vec![0x18]));
        // 'z' as u8 = 0x7a, & 0x1f = 0x1a
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('z')), Some(vec![0x1a]));
        // 'b' as u8 = 0x62, & 0x1f = 0x02
        assert_eq!(key_event_to_pty_bytes(ev_ctrl('b')), Some(vec![0x02]));
    }

    #[test]
    fn pty_alt_plus_char_produces_esc_prefix() {
        assert_eq!(key_event_to_pty_bytes(ev_alt('x')), Some(vec![0x1b, b'x']));
        assert_eq!(key_event_to_pty_bytes(ev_alt('A')), Some(vec![0x1b, b'A']));
    }

    #[test]
    fn pty_plain_char_produces_utf8_bytes() {
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Char('a'))),
            Some(b"a".to_vec())
        );
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Char('A'))),
            Some(b"A".to_vec())
        );
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Char('1'))),
            Some(b"1".to_vec())
        );
    }

    #[test]
    fn pty_non_ascii_char_produces_utf8() {
        // '中' UTF-8 = E4 B8 AD
        assert_eq!(
            key_event_to_pty_bytes(ev(KeyCode::Char('中'))),
            Some(vec![0xe4, 0xb8, 0xad])
        );
    }

    #[test]
    fn pty_function_keys_return_none() {
        assert_eq!(key_event_to_pty_bytes(ev(KeyCode::F(1))), None);
        assert_eq!(key_event_to_pty_bytes(ev(KeyCode::F(5))), None);
        assert_eq!(key_event_to_pty_bytes(ev(KeyCode::F(12))), None);
    }

    #[test]
    fn pty_null_returns_none() {
        assert_eq!(key_event_to_pty_bytes(ev(KeyCode::Null)), None);
    }
}
