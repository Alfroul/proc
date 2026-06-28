use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::docker::HealthStatus;
use crate::tui::theme;
use crate::view_models::docker_panel::DockerViewMode;

const COL_NAME: usize = 20;
const COL_IMAGE: usize = 14;
const COL_PORTS: usize = 14;
const COL_STATUS: usize = 8;
const COL_UPTIME: usize = 8;

/// 把 HealthStatus::Display 输出的英文短词翻译成中文（list 与 detail 复用）。
fn translate_health(s: &str) -> &str {
    match s {
        "healthy" => "健康",
        "unhealthy" => "不健康",
        "starting" => "启动中",
        _ => "-",
    }
}

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
    match app.docker_panel.panel.view_mode {
        DockerViewMode::Containers => {
            draw_container_list(f, chunks[1], app);
            draw_event_log(f, chunks[2], app);
        }
        DockerViewMode::Images => {
            draw_image_list(f, chunks[1], app);
            draw_image_hint(f, chunks[2], app);
        }
        DockerViewMode::Volumes => {
            draw_volume_list(f, chunks[1], app);
            draw_volume_hint(f, chunks[2], app);
        }
    }

    if app.docker_panel.panel.detail.is_some() {
        draw_detail_popup(f, area, app);
    }

    if app.docker_panel.panel.log_viewer.is_some() {
        draw_logs_overlay(f, area, app);
    }
}

fn draw_title(f: &mut Frame, area: Rect, app: &App) {
    let conn_status = if app.docker_panel.panel.connected {
        Span::styled(" ✅ 已连接 Docker ", theme::style_success())
    } else {
        Span::styled(" ❌ Docker 未运行 ", theme::style_danger())
    };

    let mode = app.docker_panel.panel.view_mode;
    let tabs_line = Line::from(vec![
        Span::styled(" 视图: ", theme::style_muted()),
        Span::styled(
            "[容器]",
            if mode == DockerViewMode::Containers {
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                theme::style_muted()
            },
        ),
        Span::raw(" "),
        Span::styled(
            "[镜像]",
            if mode == DockerViewMode::Images {
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                theme::style_muted()
            },
        ),
        Span::raw(" "),
        Span::styled(
            "[卷]",
            if mode == DockerViewMode::Volumes {
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                theme::style_muted()
            },
        ),
        Span::raw("  (Tab 切换)"),
        conn_status,
    ]);
    f.render_widget(Paragraph::new(vec![tabs_line]), area);
}

fn draw_container_list(f: &mut Frame, area: Rect, app: &App) {
    let containers = &app.docker_panel.panel.containers;

    let empty_msg = if app.docker_panel.panel.connected {
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
                let selected = i == app.docker_panel.panel.cursor;
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
                    HealthStatus::Healthy => ("健康", theme::style_success()),
                    HealthStatus::Unhealthy => ("不健康", theme::style_danger()),
                    HealthStatus::Starting => ("启动中", theme::style_warning()),
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
        state.select(Some(
            app.docker_panel.panel.cursor.min(containers.len() - 1),
        ));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_event_log(f: &mut Frame, area: Rect, app: &App) {
    let events = &app.docker_panel.panel.events;

    let items: Vec<ListItem> = if events.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            if app.docker_panel.panel.connected {
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

fn draw_image_list(f: &mut Frame, area: Rect, app: &App) {
    let images = &app.docker_panel.panel.images;

    let items: Vec<ListItem> = if images.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  暂无镜像（r 刷新）",
            theme::style_muted(),
        )))]
    } else {
        images
            .iter()
            .enumerate()
            .map(|(i, img)| {
                let selected = i == app.docker_panel.panel.images_cursor;
                let bg = if selected {
                    theme::accent()
                } else {
                    theme::bg_primary()
                };
                let name = if img.repo_tags.is_empty() {
                    format!("<none>:{}", img.short_id)
                } else {
                    img.repo_tags.join(", ")
                };
                let size = format_bytes(img.size);
                let used_marker = if img.in_use() { "▲" } else { " " };

                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", used_marker), theme::style_muted()),
                    Span::styled(pad_display(&name, 40), Style::default().bg(bg)),
                    Span::styled(" | ", theme::style_muted()),
                    Span::styled(pad_display(&img.short_id, 14), Style::default().bg(bg)),
                    Span::styled(" | ", theme::style_muted()),
                    Span::styled(pad_display(&size, 10), Style::default().bg(bg)),
                    Span::styled(" | ", theme::style_muted()),
                    Span::styled(format!("容器={}", img.containers), theme::style_muted()),
                ]))
            })
            .collect()
    };

    let list = List::new(items).block(Block::default().borders(Borders::NONE));
    let mut state = ListState::default();
    if !images.is_empty() {
        state.select(Some(
            app.docker_panel.panel.images_cursor.min(images.len() - 1),
        ));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_image_hint(f: &mut Frame, area: Rect, app: &App) {
    let hint = if let Some(target) = &app.docker_panel.panel.delete_pending {
        match target {
            crate::view_models::docker_panel::DeleteTarget::Image { display, .. } => {
                format!("⚠ 再按 d 删除 {} (Esc 取消)", display)
            }
            _ => "Esc 取消".to_string(),
        }
    } else {
        " ↑↓ 选择  r 刷新  d 删除选中  Tab 切视图".to_string()
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, theme::style_muted()))
            .block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn draw_volume_list(f: &mut Frame, area: Rect, app: &App) {
    let volumes = &app.docker_panel.panel.volumes;

    let items: Vec<ListItem> = if volumes.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  暂无 volume（r 刷新）",
            theme::style_muted(),
        )))]
    } else {
        volumes
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let selected = i == app.docker_panel.panel.volumes_cursor;
                let bg = if selected {
                    theme::accent()
                } else {
                    theme::bg_primary()
                };
                let used_marker = if v.in_use { "▲" } else { " " };
                let size = if v.size > 0 {
                    format_bytes(v.size)
                } else {
                    "-".to_string()
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", used_marker), theme::style_muted()),
                    Span::styled(pad_display(&v.name, 30), Style::default().bg(bg)),
                    Span::styled(" | ", theme::style_muted()),
                    Span::styled(pad_display(&v.driver, 10), Style::default().bg(bg)),
                    Span::styled(" | ", theme::style_muted()),
                    Span::styled(pad_display(&size, 10), Style::default().bg(bg)),
                    Span::styled(" | ", theme::style_muted()),
                    Span::styled(
                        if v.in_use { "使用中" } else { "未使用" },
                        if v.in_use {
                            theme::style_success()
                        } else {
                            theme::style_muted()
                        },
                    ),
                ]))
            })
            .collect()
    };

    let list = List::new(items).block(Block::default().borders(Borders::NONE));
    let mut state = ListState::default();
    if !volumes.is_empty() {
        state.select(Some(
            app.docker_panel.panel.volumes_cursor.min(volumes.len() - 1),
        ));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_volume_hint(f: &mut Frame, area: Rect, app: &App) {
    let hint = if let Some(target) = &app.docker_panel.panel.delete_pending {
        match target {
            crate::view_models::docker_panel::DeleteTarget::Volume { name } => {
                format!("⚠ 再按 d 删除 {} (Esc 取消)", name)
            }
            _ => "Esc 取消".to_string(),
        }
    } else {
        " ↑↓ 选择  r 刷新  d 删除选中  Tab 切视图".to_string()
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, theme::style_muted()))
            .block(Block::default().borders(Borders::TOP)),
        area,
    );
}

fn draw_logs_overlay(f: &mut Frame, area: Rect, app: &App) {
    // 日志占下半屏 60%。
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);
    let log_area = chunks[1];

    let Some(lv) = &app.docker_panel.panel.log_viewer else {
        return;
    };
    let container = lv.container.as_deref().unwrap_or("");
    let follow_marker = if lv.follow { " (follow)" } else { "" };

    let title = Line::from(vec![
        Span::styled(
            format!(" 📜 日志: {}{} ", container, follow_marker),
            theme::style_header(),
        ),
        Span::styled(
            "  ↑↓ 滚动  f follow  c 清屏  Esc 退出 ",
            theme::style_muted(),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    // 显示缓冲：从底部往上数 `scroll_from_bottom` 条，按可用高度截断。
    let total = lv.buffer.len();
    let height = log_area.height.saturating_sub(2) as usize; // 边框占 2 行
    let from_bottom = lv.scroll_from_bottom.unwrap_or(0);
    let end = total.saturating_sub(from_bottom);
    let start = end.saturating_sub(height);
    let visible: Vec<&crate::docker::logs::LogLine> =
        lv.buffer.iter().skip(start).take(end - start).collect();

    let lines: Vec<Line> = if visible.is_empty() {
        vec![Line::from(Span::styled(
            "  (等待日志...)",
            theme::style_muted(),
        ))]
    } else {
        visible
            .iter()
            .map(|log| {
                let style = if log.is_stderr {
                    theme::style_danger()
                } else {
                    theme::style_normal()
                };
                Line::from(Span::styled(&log.message, style))
            })
            .collect()
    };

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        log_area,
    );
}

fn draw_detail_popup(f: &mut Frame, area: Rect, app: &App) {
    let popup_area = centered_rect(60, 18, area);
    f.render_widget(ratatui::widgets::Clear, popup_area);

    let container = match &app.docker_panel.panel.detail {
        Some(c) => c,
        None => return,
    };

    let stats_section = match &app.docker_panel.panel.detail_stats {
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

    let health_raw = container.health.to_string();
    let health_display = translate_health(&health_raw);

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
        Line::from(format!("  Docker: {}", container.status)),
        Line::from(""),
        Line::from(Span::styled(
            " 资源使用:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];

    let mut all_lines = content;
    all_lines.extend(stats_section);

    // E4 — 容器内进程区块（按 t 展开 / 折叠）。
    if app.docker_panel.panel.show_top_processes {
        all_lines.push(Line::from(""));
        all_lines.push(Line::from(Span::styled(
            format!(
                " 容器内进程（{} 个，r 刷新）:",
                app.docker_panel.panel.top_processes.len()
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if app.docker_panel.panel.top_processes.is_empty() {
            all_lines.push(Line::from(Span::styled(
                "  （无进程或容器未运行）",
                theme::style_muted(),
            )));
        } else {
            for (i, p) in app
                .docker_panel
                .panel
                .top_processes
                .iter()
                .take(10)
                .enumerate()
            {
                all_lines.push(Line::from(format!(
                    "  {:>3}. {:<10} {}",
                    i + 1,
                    p.pid,
                    p.command.chars().take(80).collect::<String>()
                )));
            }
            if app.docker_panel.panel.top_processes.len() > 10 {
                all_lines.push(Line::from(format!(
                    "  ... +{} 个进程未显示",
                    app.docker_panel.panel.top_processes.len() - 10
                )));
            }
        }
    } else {
        all_lines.push(Line::from(""));
        all_lines.push(Line::from(Span::styled(
            "  (按 t 查看容器内进程，按 l 查看日志)",
            theme::style_muted(),
        )));
    }

    all_lines.push(Line::from(""));
    all_lines.push(Line::from(Span::styled(
        "  Esc 关闭 | t 进程 | l 日志",
        theme::style_muted(),
    )));

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
