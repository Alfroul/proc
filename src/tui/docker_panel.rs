use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::docker::HealthStatus;
use crate::tui::theme;

const COL_NAME: usize = 20;
const COL_IMAGE: usize = 14;
const COL_PORTS: usize = 14;
const COL_STATUS: usize = 8;
const COL_UPTIME: usize = 8;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Percentage(60),
            Constraint::Percentage(40),
        ])
        .split(area);

    draw_title(f, chunks[0], app);
    draw_container_list(f, chunks[1], app);
    draw_event_log(f, chunks[2], app);

    if app.docker_panel.detail.is_some() {
        draw_detail_popup(f, area, app);
    }
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    let conn_status = if app.docker_panel.connected {
        Span::styled(" ✅ 已连接 Docker ", theme::style_success())
    } else {
        Span::styled(" ❌ Docker 未运行 ", theme::style_danger())
    };

    let containers = &app.docker_panel.containers;

    let title_line = Line::from(vec![
        Span::styled(" Docker 容器 ", theme::style_header()),
        conn_status,
        Span::styled(
            format!("  共 {} 个", containers.len()),
            theme::style_muted(),
        ),
    ]);

    let header_line = Line::from(vec![
        Span::styled("   ", theme::style_muted()),
        Span::styled(pad_display("名称", COL_NAME), theme::style_muted()),
        Span::styled(" | ", theme::style_muted()),
        Span::styled(pad_display("镜像", COL_IMAGE), theme::style_muted()),
        Span::styled(" | ", theme::style_muted()),
        Span::styled(pad_display("端口", COL_PORTS), theme::style_muted()),
        Span::styled(" | ", theme::style_muted()),
        Span::styled(pad_display("状态", COL_STATUS), theme::style_muted()),
        Span::styled(" | ", theme::style_muted()),
        Span::styled(pad_display("时长", COL_UPTIME), theme::style_muted()),
    ]);

    let para = Paragraph::new(vec![title_line, header_line]);
    f.render_widget(para, area);
}

fn draw_container_list(f: &mut Frame, area: Rect, app: &App) {
    let containers = &app.docker_panel.containers;

    let empty_msg = if app.docker_panel.connected {
        "  暂无容器"
    } else {
        "  Docker 未运行或未安装\n  提示: WSL Docker 需配置 TCP 端口 — dockerd -H tcp://0.0.0.0:2375"
    };

    let items: Vec<ListItem> = if containers.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            empty_msg,
            theme::style_muted(),
        )))]
    } else {
        containers
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let selected = i == app.docker_panel.cursor;
                let bg = if selected {
                    theme::accent()
                } else {
                    theme::bg_primary()
                };

                let (status_icon, status_style) = match c.state.as_str() {
                    "running" => ("▲", theme::style_success()),
                    "exited" | "dead" => ("■", theme::style_muted()),
                    _ => ("■", theme::style_muted()),
                };

                let _ = match c.health {
                    HealthStatus::Healthy => ("healthy", theme::style_success()),
                    HealthStatus::Unhealthy => ("unhealthy", theme::style_danger()),
                    HealthStatus::Starting => ("starting", theme::style_warning()),
                    HealthStatus::NotConfigured => ("-", theme::style_muted()),
                };

                let uptime = format_uptime(c.running_since);
                let sep = Span::styled(" | ", theme::style_muted());

                let line = Line::from(vec![
                    Span::styled(format!(" {} ", status_icon), status_style),
                    Span::styled(pad_display(&c.name, COL_NAME), Style::default().bg(bg)),
                    sep.clone(),
                    Span::styled(pad_display(&c.image, COL_IMAGE), Style::default().bg(bg)),
                    sep.clone(),
                    Span::styled(pad_display(&c.ports, COL_PORTS), theme::style_info()),
                    sep.clone(),
                    Span::styled(
                        pad_display(&c.status_text(), COL_STATUS),
                        Style::default().bg(bg),
                    ),
                    sep,
                    Span::styled(pad_display(&uptime, COL_UPTIME), theme::style_muted()),
                ]);

                ListItem::new(line)
            })
            .collect()
    };

    let list = List::new(items).block(Block::default().borders(Borders::NONE));

    let mut state = ListState::default();
    if !containers.is_empty() {
        state.select(Some(app.docker_panel.cursor.min(containers.len() - 1)));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_event_log(f: &mut Frame, area: Rect, app: &App) {
    let events = &app.docker_panel.events;

    let items: Vec<ListItem> = if events.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if app.docker_panel.connected {
                "  监听中 — 容器启停事件将实时显示"
            } else {
                "  暂无事件"
            },
            theme::style_muted(),
        )))]
    } else {
        events
            .iter()
            .take(30)
            .map(|event| {
                let action_style = match event.action.as_str() {
                    "die" | "stop" => theme::style_danger(),
                    "start" => theme::style_success(),
                    "health_status" => theme::style_warning(),
                    _ => theme::style_normal(),
                };

                let name = event
                    .container_name
                    .as_deref()
                    .unwrap_or(&event.container_id);

                let elapsed = event
                    .timestamp
                    .elapsed()
                    .unwrap_or(std::time::Duration::ZERO);
                let time_str = format_duration_short(elapsed);

                ListItem::new(Line::from(vec![
                    Span::styled(format!("[{}]", time_str), theme::style_muted()),
                    Span::raw(" "),
                    Span::styled(format!("{:<16}", event.action), action_style),
                    Span::raw(" "),
                    Span::styled(name, theme::style_normal()),
                ]))
            })
            .collect()
    };

    let list = List::new(items).block(Block::default().borders(Borders::TOP).title(Line::from(
        Span::styled(" 事件日志 ", theme::style_header()),
    )));

    f.render_widget(list, area);
}

fn draw_detail_popup(f: &mut Frame, area: Rect, app: &App) {
    let popup_area = centered_rect(60, 18, area);
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let container = match &app.docker_panel.detail {
        Some(c) => c,
        None => return,
    };

    let stats_section = match &app.docker_panel.detail_stats {
        Some(s) => vec![
            Line::from(format!("  CPU:  {:.1}%", s.cpu_percent)),
            Line::from(format!(
                "  内存: {} / {}",
                format_bytes(s.memory_usage),
                format_bytes(s.memory_limit)
            )),
            Line::from(format!(
                "  网络: ↓{} ↑{}",
                format_bytes(s.network_in),
                format_bytes(s.network_out)
            )),
        ],
        None => vec![Line::from(Span::styled(
            "  资源统计不可用",
            theme::style_muted(),
        ))],
    };

    let (status_icon, status_style) = match container.state.as_str() {
        "running" => ("▲ 运行中", theme::style_success()),
        "exited" => ("■ 已停止", theme::style_muted()),
        _ => ("■ 未知", theme::style_muted()),
    };

    let health_display = container.health.to_string();

    let content = vec![
        Line::from(Span::styled(
            format!(" 容器详情: {}", container.name),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  ID:     "),
            Span::styled(&container.id, theme::style_muted()),
        ]),
        Line::from(format!("  镜像:   {}", container.image)),
        Line::from(vec![
            Span::raw("  状态:   "),
            Span::styled(status_icon.to_string(), status_style),
        ]),
        Line::from(format!("  健康:   {}", health_display)),
        Line::from(format!("  状态:   {}", container.status)),
        Line::from(""),
        Line::from(Span::styled(
            " 资源使用:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];

    let mut all_lines = content;
    all_lines.extend(stats_section);
    all_lines.push(Line::from(""));
    all_lines.push(Line::from(Span::styled("  Esc 关闭", theme::style_muted())));

    let paragraph = Paragraph::new(all_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(theme::style_normal()),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, popup_area);
}

fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    super::centered_rect(percent_x, height, r)
}

/// 按显示宽度（CJK 字符算 2 宽）截断并右补空格，确保终端对齐
fn pad_display(s: &str, target_width: usize) -> String {
    let mut display_width = 0usize;
    let mut byte_end = 0;
    for (i, ch) in s.char_indices() {
        let w = if ch.is_ascii() { 1 } else { 2 };
        if display_width + w > target_width {
            break;
        }
        display_width += w;
        byte_end = i + ch.len_utf8();
    }
    let truncated = &s[..byte_end];
    let padding = target_width.saturating_sub(display_width);
    format!("{}{}", truncated, " ".repeat(padding))
}

fn format_uptime(since: Option<std::time::SystemTime>) -> String {
    let since = match since {
        Some(s) => s,
        None => return "-".to_string(),
    };
    let elapsed = match since.elapsed() {
        Ok(d) => d,
        Err(_) => return "-".to_string(),
    };
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{}秒", secs)
    } else if secs < 3600 {
        format!("{}分", secs / 60)
    } else if secs < 86400 {
        format!("{}时", secs / 3600)
    } else {
        format!("{}天", secs / 86400)
    }
}

fn format_duration_short(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

use crate::format::format_bytes;

trait ContainerStatusText {
    fn status_text(&self) -> String;
}

impl ContainerStatusText for crate::docker::ContainerInfo {
    fn status_text(&self) -> String {
        match self.state.as_str() {
            "running" => "运行中".to_string(),
            "exited" => "已停止".to_string(),
            "paused" => "已暂停".to_string(),
            "restarting" => "重启中".to_string(),
            "dead" => "异常".to_string(),
            "created" => "已创建".to_string(),
            _ => self.state.clone(),
        }
    }
}
