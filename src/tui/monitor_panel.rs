use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::monitor::{MonitorStatus, MonitorTarget, RestartPolicy};
use crate::tui::theme;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_monitor_list(f, chunks[0], app);
    draw_notifications(f, chunks[1], app);

    if let Some(ref submenu) = app.monitor_add_submenu {
        draw_add_submenu(f, area, submenu);
    }
}

fn draw_monitor_list(f: &mut Frame, area: Rect, app: &App) {
    let monitors = app.monitor_manager.list_monitors();

    let items: Vec<ListItem> = if monitors.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  无监控条目 — 按 a 添加监控",
            theme::style_muted(),
        )))]
    } else {
        monitors
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let selected = i == app.monitor_cursor;
                let bg = if selected {
                    theme::accent()
                } else {
                    theme::bg_primary()
                };

                let type_icon = match &entry.target {
                    MonitorTarget::ByPid { .. } => "PID",
                    MonitorTarget::ByPort { .. } => "PRT",
                    MonitorTarget::ByCommand { .. } => "CMD",
                };

                let target_desc = match &entry.target {
                    MonitorTarget::ByPid { pid } => format!("PID {}", pid),
                    MonitorTarget::ByPort { port } => format!("Port {}", port),
                    MonitorTarget::ByCommand { cmd, .. } => {
                        if cmd.len() > 20 {
                            format!("{}...", &cmd[..20])
                        } else {
                            cmd.clone()
                        }
                    }
                };

                let status_style = match entry.status {
                    MonitorStatus::Running => theme::style_success(),
                    MonitorStatus::Stopped => theme::style_muted(),
                    MonitorStatus::Crashed => theme::style_danger(),
                    MonitorStatus::Paused => theme::style_warning(),
                };

                let policy_str = match &entry.restart_policy {
                    RestartPolicy::NotifyOnly => "通知".to_string(),
                    RestartPolicy::AutoRestart {
                        max_retries,
                        base_backoff: _,
                        max_backoff: _,
                    } => format!("自动重启(max:{})", max_retries),
                };

                let line = Line::from(vec![
                    Span::styled(format!(" {:>3} ", type_icon), theme::style_info()),
                    Span::styled(format!(" {:<20}", target_desc), Style::default().bg(bg)),
                    Span::styled(
                        format!(" {:>6} ", entry.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string())),
                        Style::default().bg(bg),
                    ),
                    Span::styled(format!(" {:>6} ", entry.status.to_string()), status_style),
                    Span::styled(format!(" {:>3} ", entry.crash_count), Style::default().bg(bg)),
                    Span::styled(format!(" {}", policy_str), theme::style_muted()),
                ]);

                ListItem::new(line)
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .title(Line::from(vec![
                Span::styled(" 监控面板 ", theme::style_header()),
                Span::styled(" a:添加 d:删除 r:重启 s:暂停 ", theme::style_muted()),
            ])),
    );

    let mut state = ListState::default();
    if !monitors.is_empty() {
        state.select(Some(app.monitor_cursor.min(monitors.len() - 1)));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_notifications(f: &mut Frame, area: Rect, app: &App) {
    let notifications = app.monitor_manager.notifications();

    let items: Vec<ListItem> = if notifications.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  暂无通知记录",
            theme::style_muted(),
        )))]
    } else {
        notifications
            .iter()
            .rev()
            .take(20)
            .map(|record| {
                let elapsed = record
                    .timestamp
                    .elapsed()
                    .unwrap_or(std::time::Duration::ZERO);
                let time_str = format_duration(elapsed);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("[{}]", time_str), theme::style_muted()),
                    Span::raw(" "),
                    Span::styled(&record.message, theme::style_normal()),
                ]))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::TOP)
            .title(Line::from(Span::styled(" 通知记录 ", theme::style_header()))),
    );

    f.render_widget(list, area);
}

fn draw_add_submenu(f: &mut Frame, area: Rect, submenu: &crate::app::MonitorAddSubmenu) {
    let popup_area = centered_rect(50, 12, area);
    f.render_widget(Clear, popup_area);

    let content = match submenu {
        crate::app::MonitorAddSubmenu::SelectType => {
            vec![
                Line::from(Span::styled("添加监控", Style::default().add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from("  1 — 按 PID 监控"),
                Line::from("  2 — 按端口监控"),
                Line::from("  3 — 按命令监控（自动重启）"),
                Line::from(""),
                Line::from(Span::styled("  Esc 取消", theme::style_muted())),
            ]
        }
        crate::app::MonitorAddSubmenu::EnterPid { input } => {
            vec![
                Line::from(Span::styled("按 PID 监控", Style::default().add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from(format!("  PID: {}", input)),
                Line::from(""),
                Line::from(Span::styled("  Enter 确认 | Esc 取消", theme::style_muted())),
            ]
        }
        crate::app::MonitorAddSubmenu::EnterPort { input } => {
            vec![
                Line::from(Span::styled("按端口监控", Style::default().add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from(format!("  端口号: {}", input)),
                Line::from(""),
                Line::from(Span::styled("  Enter 确认 | Esc 取消", theme::style_muted())),
            ]
        }
        crate::app::MonitorAddSubmenu::EnterCommand { cmd_input, args_input, cwd_input, retries_input } => {
            vec![
                Line::from(Span::styled("按命令监控（自动重启）", Style::default().add_modifier(Modifier::BOLD))),
                Line::from(""),
                Line::from(format!("  命令: {}", cmd_input)),
                Line::from(format!("  参数: {}", args_input)),
                Line::from(format!("  工作目录: {}", cwd_input)),
                Line::from(format!("  最大重试: {}", retries_input)),
                Line::from(""),
                Line::from(Span::styled("  Enter 确认 | Esc 取消", theme::style_muted())),
            ]
        }
    };

    let paragraph = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).style(theme::style_normal()))
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, popup_area);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    super::centered_rect(percent_x, height, r)
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}秒前", secs)
    } else if secs < 3600 {
        format!("{}分前", secs / 60)
    } else {
        format!("{}时前", secs / 3600)
    }
}
