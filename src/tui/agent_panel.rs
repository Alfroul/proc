//! v0.21 stage 1：AI Agent 面板占位渲染（ADR-0031）。
//!
//! 全屏面板（与 Help / Replay 同档）。stage 1 只渲染标题 + 实装进度提示 +
//! 键位行；对话流 / 输入框 / streaming 增量 / confirm 确认框 stage 3 实装。
//! 渲染不 panic、不读 session（stage 1 无 session 可读）。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;
use crate::tui::theme;

/// 占位面板渲染：顶部标题行 + 中部提示区 + 底部键位行。
pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " AI Agent（v0.21 stage 3 实装） ",
            theme::style_header(),
        ))
        .style(theme::style_normal());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [hint_area, status_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let hint = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            " 会话层（AgentSession + run_streaming + confirm 通道）stage 2 落地",
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            " 面板交互（对话流 / 输入框 / streaming 渲染 / y·n 确认）stage 3 落地",
            theme::style_normal(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " 进入方式：Ctrl+P 命令面板搜「AI Agent」",
            theme::style_muted(),
        )),
    ]);
    f.render_widget(hint, hint_area);

    // 状态行：面板状态机当前值（stage 1 恒 Idle；stage 3 随 SessionEvent 变化）。
    let mode_label = match app.agent_panel.panel.mode {
        crate::view_models::AgentPanelMode::Idle => "空闲",
        crate::view_models::AgentPanelMode::Streaming => "生成中",
        crate::view_models::AgentPanelMode::AwaitingConfirm => "等待确认",
    };
    let status = Paragraph::new(Line::from(vec![
        Span::styled(format!(" 状态: {} ", mode_label), theme::style_info()),
        Span::styled("· Ctrl+D / Esc 退出", theme::style_muted()),
    ]));
    f.render_widget(status, status_area);
}
