//! v0.21 stage 3：AI Agent 面板全屏渲染（ADR-0031 D6）。
//!
//! 布局：上 ~70% 对话流滚动区（底部锚定，PgUp/PgDn 经
//! `scroll_from_bottom` 偏移）+ 底部输入框（AwaitingConfirm 态替换为高亮
//! 确认框）+ 状态行（provider/model · 步骤 · 用时 · 模式 · 键位）。
//!
//! 渲染只读 `app.agent_panel.panel` 状态零副作用；TextDelta 已在 App tick
//! 内批量 append（D6：单 drain 单重绘，本层不做节流）。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::agent::session::ConfirmDecision;
use crate::app::App;
use crate::tui::theme;
use crate::view_models::{AgentPanelMode, ChatEntry};

/// 贪心按 char 换行（中文无空格，word-wrap 无从谈起；英文单词可能被截断，
/// v1 接受——显示层 clipping 兜底）。
fn wrap_line(content: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![content.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut len = 0usize;
    for ch in content.chars() {
        if len >= width {
            out.push(std::mem::take(&mut cur));
            len = 0;
        }
        cur.push(ch);
        len += 1;
    }
    out.push(cur);
    out
}

/// 带前缀的换行（首行 prefix，续行等宽空格缩进——按 char 计数近似）。
fn wrapped_spans(prefix: &str, text: &str, style: Style, width: usize) -> Vec<Line<'static>> {
    let lines = wrap_line(&format!("{prefix}{text}"), width);
    let indent = " ".repeat(prefix.chars().count());
    lines
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            let content = if i == 0 { l } else { format!("{indent}{l}") };
            Line::from(Span::styled(content, style))
        })
        .collect()
}

/// 对话条目 → 渲染行（tool 步骤行 / confirm 块的参数摘要截断）。
fn entry_lines(entry: &ChatEntry, width: usize) -> Vec<Line<'static>> {
    match entry {
        ChatEntry::User(q) => wrapped_spans(" ❯ ", q, theme::style_info(), width),
        ChatEntry::AssistantStreaming(t) | ChatEntry::AssistantFinal(t) => {
            wrapped_spans(" ", t, theme::style_normal(), width)
        }
        ChatEntry::ToolCall {
            name,
            arguments,
            is_error,
            result_chars,
        } => {
            let args = arguments.to_string();
            let brief: String = args.chars().take(48).collect();
            let status = match is_error {
                None => " …".to_string(),
                Some(false) => format!(" ✓ ({result_chars} 字符)"),
                Some(true) => format!(" ✗ ({result_chars} 字符)"),
            };
            let style = if *is_error == Some(true) {
                theme::style_danger()
            } else {
                theme::style_muted()
            };
            wrapped_spans(" ⚙ ", &format!("{name} {brief}{status}"), style, width)
        }
        ChatEntry::Confirm {
            tool_name,
            summary,
            decision,
        } => {
            let verdict = match decision {
                Some(ConfirmDecision::Approved) => "  → 已确认执行",
                Some(ConfirmDecision::Denied) => "  → 已拒绝",
                None => "",
            };
            wrapped_spans(
                " ⚠ ",
                &format!("写操作确认 {tool_name}: {summary}{verdict}"),
                theme::style_warning(),
                width,
            )
        }
        ChatEntry::Error(e) => wrapped_spans(" ✗ ", e, theme::style_danger(), width),
        ChatEntry::Notice(n) => wrapped_spans(" ", n, theme::style_muted(), width),
    }
}

fn mode_label(mode: AgentPanelMode) -> &'static str {
    match mode {
        AgentPanelMode::Idle => "空闲",
        AgentPanelMode::Streaming => "生成中",
        AgentPanelMode::AwaitingConfirm => "等待确认",
    }
}

fn mode_keys(mode: AgentPanelMode) -> &'static str {
    match mode {
        AgentPanelMode::Idle => "Enter 发送 · Esc 退出",
        AgentPanelMode::Streaming => "Esc 中断",
        AgentPanelMode::AwaitingConfirm => "y 执行 · n 拒绝",
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Agent 面板渲染入口（layout::draw_middle 在 Help 同档位置调用）。
pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let panel = &app.agent_panel.panel;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" AI Agent ", theme::style_header()))
        .style(theme::style_normal());
    let inner = block.inner(area);
    f.render_widget(block, area);

    // AwaitingConfirm：输入框位置替换为高亮确认框（输入此时锁定）。
    let (bottom_h, confirm_mode) = if panel.mode == AgentPanelMode::AwaitingConfirm {
        (7u16, true)
    } else {
        (3u16, false)
    };
    let [conv_area, bottom_area, status_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(bottom_h),
        Constraint::Length(1),
    ])
    .areas(inner);

    draw_conversation(f, conv_area, panel);
    if confirm_mode {
        draw_confirm_box(f, bottom_area, panel);
    } else {
        draw_input_box(f, bottom_area, panel);
    }
    draw_status_line(f, status_area, panel);
}

/// 对话流滚动区：entries 全量组装 Lines，底部锚定切片
/// （`start = total - viewport - scroll_from_bottom`，PgUp 后钉住不跟随）。
fn draw_conversation(f: &mut Frame, area: Rect, panel: &crate::view_models::AgentPanel) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let width = area.width.saturating_sub(1) as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, entry) in panel.entries.iter().enumerate() {
        // 轮次间隔：User 行前空一行（首条除外）。
        if i > 0 && matches!(entry, ChatEntry::User(_)) {
            lines.push(Line::from(""));
        }
        lines.extend(entry_lines(entry, width));
    }
    if panel.entries.is_empty() {
        let hint = if panel.provider_detail.is_empty() {
            " 输入 query 开始对话（provider 构造失败见下方错误）"
        } else {
            " 输入 query 开始对话 · 写操作将弹出 y/n 确认"
        };
        lines.push(Line::from(Span::styled(hint, theme::style_muted())));
    }

    let total = lines.len();
    let viewport = area.height as usize;
    let start = total.saturating_sub(viewport + panel.scroll_from_bottom);
    let end = start.saturating_add(viewport).min(total);
    let visible: Vec<Line> = lines[start..end].to_vec();
    f.render_widget(Paragraph::new(visible), area);
}

fn draw_input_box(f: &mut Frame, area: Rect, panel: &crate::view_models::AgentPanel) {
    let locked = panel.mode == AgentPanelMode::Streaming;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if locked {
            theme::style_muted()
        } else {
            theme::style_normal()
        })
        .title(Span::styled(
            if locked { " 生成中… " } else { " 输入 " },
            theme::style_header(),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 {
        return;
    }
    let text = if locked {
        "生成中，输入锁定（Esc 中断）".to_string()
    } else {
        format!("{}█", panel.input)
    };
    // 输入超宽时显示尾部（光标可见）。
    let max = inner.width.saturating_sub(1) as usize;
    let text: String = if text.chars().count() > max {
        text.chars().skip(text.chars().count() - max).collect()
    } else {
        text
    };
    let style = if locked {
        theme::style_muted()
    } else {
        theme::style_normal()
    };
    f.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), inner);
}

/// AwaitingConfirm 高亮确认框：summary 强制展示（风险 5 mitigate 1）+
/// `[y] 执行 [n] 拒绝` 键位行。
fn draw_confirm_box(f: &mut Frame, area: Rect, panel: &crate::view_models::AgentPanel) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(theme::style_warning())
        .title(Span::styled(" ⚠ 写操作确认 ", theme::style_warning()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let width = inner.width.saturating_sub(1) as usize;
    let (tool, summary) = panel
        .pending_confirm
        .as_ref()
        .map(|req| (req.tool_name.clone(), req.summary.clone()))
        .unwrap_or_else(|| ("?".to_string(), String::new()));

    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(" {tool}: "),
        theme::style_warning(),
    )));
    lines.extend(wrapped_spans("   ", &summary, theme::style_normal(), width));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" [y] 执行 ", theme::style_danger()),
        Span::styled(" [n] 拒绝 ", theme::style_normal()),
        Span::styled(" Esc=拒绝", theme::style_muted()),
    ]));

    let viewport = inner.height as usize;
    let start = lines.len().saturating_sub(viewport);
    let visible: Vec<Line> = lines[start..].to_vec();
    f.render_widget(Paragraph::new(visible), inner);
}

/// 状态行：provider · 步骤 · 用时 · 模式 · 键位。
fn draw_status_line(f: &mut Frame, area: Rect, panel: &crate::view_models::AgentPanel) {
    let elapsed = panel
        .finished_after
        .unwrap_or_else(|| panel.query_started.map(|t| t.elapsed()).unwrap_or_default());
    let provider = if panel.provider_detail.is_empty() {
        "会话不可用".to_string()
    } else {
        panel.provider_detail.clone()
    };
    let line = Line::from(vec![
        Span::styled(format!(" {provider}"), theme::style_info()),
        Span::styled(
            format!(
                " · 步骤 {} · {}",
                panel.tool_steps,
                format_duration(elapsed)
            ),
            theme::style_normal(),
        ),
        Span::styled(
            format!(" · {} · {}", mode_label(panel.mode), mode_keys(panel.mode)),
            theme::style_muted(),
        ),
        Span::styled(" · Ctrl+D 退出面板", theme::style_muted()),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
