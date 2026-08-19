use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Paragraph, Tabs};

use crate::app::{App, AppMode};
use crate::tui::theme;

const TAB_NAMES: [&str; 6] = [
    "1:进程",
    "2:进程树",
    "3:端口",
    "4:U盘",
    "5:监控",
    "6:Docker",
];

fn tab_index(mode: &AppMode) -> usize {
    match mode {
        AppMode::ProcessList | AppMode::ProcessDetail | AppMode::Help | AppMode::Agent => 0,
        AppMode::PortMap => 2,
        AppMode::UsbAssistant => 3,
        AppMode::MonitorPanel => 4,
        AppMode::DockerPanel | AppMode::ContainerExec => 5,
        AppMode::Replay => {
            // Use the recorded frame's mode for tab highlighting
            unreachable!("Replay uses replay_frame_mode for tab index")
        }
    }
}

fn effective_tab_index(app: &App) -> usize {
    if app.mode == AppMode::Replay {
        tab_index(&app.replay_frame_mode())
    } else if app.mode == AppMode::ProcessList
        && app.process_panel.panel.process_view_mode == crate::collect::ProcessViewMode::Tree
    {
        1
    } else {
        tab_index(&app.mode)
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_toolbar(f, app, outer[0]);
    draw_middle(f, app, outer[1]);
    draw_footer(f, app, outer[2]);

    // Alert popup overlay
    if app.alert_popup_open {
        crate::tui::alert_badge::draw_alert_popup(f, app);
    }

    // v0.6.0 阶段 3：worker 崩溃 banner（顶部居中）。最近一条时间 + panic msg。
    if !app.active_crashes.is_empty() {
        draw_crash_banner(f, app);
    }

    // v0.7.0 阶段 3：Ctrl+P 命令面板浮层，渲染在最上层。
    if app.is_palette_open() {
        crate::tui::command_palette::draw(f, app);
    }
}

/// 渲染 worker crash banner。每条 `WorkerCrash` 一行 + 底部一行「按 D 关闭」。
///
/// v0.11.0 阶段 1（ADR-0019）：每条 crash 查 `workers.restart_status(name, now)`
/// 显示重启状态：restarting in Ns / restarted (retry #N) / permanent failure。
fn draw_crash_banner(f: &mut Frame, app: &App) {
    use ratatui::text::Span;
    use ratatui::widgets::{Block, Borders, Paragraph};

    let crashes = &app.active_crashes;
    let height = (crashes.len() as u16) + 3; // 标题(border) + 每条 1 行 + 提示 1 行 + 边框
    let area = crate::tui::centered_rect(80, height, f.area());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" ⚠ Worker 崩溃 ({}) ", crashes.len()),
            theme::style_danger(),
        ))
        .style(theme::style_danger());

    let now = std::time::SystemTime::now();
    let mut lines: Vec<ratatui::text::Line> = Vec::new();
    for crash in crashes.iter().rev().take(5) {
        // rev + take 5：显示最近 5 条（active_crashes 已限 10 条上限）。
        let elapsed = crash.timestamp.elapsed().unwrap_or_default();
        let mins = elapsed.as_secs() / 60;
        let secs = elapsed.as_secs() % 60;
        let time_str = if mins > 0 {
            format!("{mins}m{secs}s ago")
        } else {
            format!("{secs}s ago")
        };
        // panic message 可能很长，截到 60 字符。
        let msg = if crash.message.len() > 60 {
            format!("{}…", &crash.message[..60])
        } else {
            crash.message.clone()
        };
        // v0.11.0 阶段 1：查 restart_status 显示重启状态。
        let status = app.workers.restart_status(crash.worker, now);
        let restart_label = restart_label_for(&status);
        let restart_style = restart_style_for(&status);
        let mut spans = vec![
            Span::styled(format!(" {} ", crash.worker), theme::style_warning()),
            Span::styled(format!("({time_str}) "), theme::style_muted()),
            Span::styled(msg, theme::style_normal()),
        ];
        if !restart_label.is_empty() {
            spans.push(Span::styled(format!("  — {restart_label}"), restart_style));
        }
        lines.push(ratatui::text::Line::from(spans));
    }
    lines.push(ratatui::text::Line::from(""));
    lines.push(ratatui::text::Line::from(Span::styled(
        " 按 D 关闭提示",
        theme::style_muted(),
    )));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// 把 [`crate::workers::RestartStatus`] 转成 banner 显示文案。
fn restart_label_for(status: &crate::workers::RestartStatus) -> String {
    use crate::workers::RestartStatus;
    match status {
        RestartStatus::Healthy => String::new(),
        RestartStatus::Restarting {
            retry_count,
            remaining_secs,
        } => {
            if *remaining_secs == 0 {
                format!("即将重启 (retry #{})", retry_count + 1)
            } else if *remaining_secs >= 60 {
                format!(
                    "{}min 后重启 (retry #{})",
                    remaining_secs / 60,
                    retry_count + 1
                )
            } else {
                format!("{}s 后重启 (retry #{})", remaining_secs, retry_count + 1)
            }
        }
        RestartStatus::Restarted {
            retry_count,
            elapsed_secs,
        } => {
            // 3 秒内显示「已重启」反馈，之后淡出（返回空 → banner 不显示状态）。
            if *elapsed_secs <= 3 {
                format!("✓ 已重启 (retry #{retry_count})")
            } else {
                String::new()
            }
        }
        RestartStatus::PermanentFailure { retry_count } => {
            format!("✗ 永久失败（已重试 {retry_count} 次，请重启 proc）")
        }
    }
}

/// 选 banner 重启状态的颜色：Restarting / PermanentFailure 用 danger，
/// Restarted 用 success 风格（绿色），Healthy / 空标签用 normal。
fn restart_style_for(status: &crate::workers::RestartStatus) -> ratatui::style::Style {
    use crate::workers::RestartStatus;
    match status {
        RestartStatus::Restarted { .. } => theme::style_success(),
        RestartStatus::PermanentFailure { .. } => theme::style_danger(),
        _ => theme::style_normal(),
    }
}

fn draw_toolbar(f: &mut Frame, app: &App, area: Rect) {
    let active_tab = effective_tab_index(app);
    let titles: Vec<ratatui::text::Line> = TAB_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i == active_tab {
                ratatui::text::Line::from(ratatui::text::Span::styled(
                    *name,
                    theme::style_selected(),
                ))
            } else {
                ratatui::text::Line::from(ratatui::text::Span::styled(*name, theme::style_muted()))
            }
        })
        .collect();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .style(theme::style_header()),
        )
        .select(active_tab);

    f.render_widget(tabs, area);

    // REC indicator (top-right corner)
    let rec_label = if app.is_recording() {
        let elapsed = app.recording_elapsed();
        format!(" ● REC {:02}:{:02} ", elapsed / 60, elapsed % 60)
    } else {
        " ● REC ".to_string()
    };
    let rec_width = rec_label.len() as u16;
    let rec_area = Rect {
        x: area.x + area.width.saturating_sub(rec_width + 1),
        y: area.y + 1,
        width: rec_width,
        height: 1,
    };
    let blink_on = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        % 1000
        < 600;
    let rec_style = if app.is_recording() {
        if blink_on {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        }
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let rec = Paragraph::new(Span::styled(rec_label, rec_style));
    f.render_widget(rec, rec_area);
}

fn draw_middle(f: &mut Frame, app: &App, area: Rect) {
    if app.mode == AppMode::Help {
        crate::tui::help_panel::draw(f, area, app);
        return;
    }
    // v0.21：Agent 全屏面板（与 Help 同档，占主区不渲染进程表）。
    if app.mode == AppMode::Agent {
        crate::tui::agent_panel::draw(f, area, app);
        return;
    }
    match app.mode {
        AppMode::ProcessList | AppMode::Replay => {
            let show_right = app.mode != AppMode::Replay
                && app.process_panel.panel.process_view_mode
                    == crate::collect::ProcessViewMode::List;

            if show_right {
                let [main_area, right_area] = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(0), Constraint::Length(60)])
                    .areas(area);

                let sidebar_rows = app.sidebar_height();
                let [sidebar_area, panel_area] = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(sidebar_rows), Constraint::Min(0)])
                    .areas(right_area);

                draw_main_panel(f, app, main_area);
                crate::tui::sidebar::draw(f, sidebar_area, app);
                crate::tui::right_panel::draw(f, panel_area, app);
            } else {
                draw_main_panel(f, app, area);
            }
        }
        _ => draw_main_panel(f, app, area),
    }
}

fn draw_main_panel(f: &mut Frame, app: &App, area: Rect) {
    match app.mode {
        AppMode::ProcessList => match app.process_panel.panel.process_view_mode {
            crate::collect::ProcessViewMode::List => {
                crate::tui::process_table::draw(f, area, app);
            }
            crate::collect::ProcessViewMode::Tree => {
                crate::tui::process_tree::draw(f, area, app);
            }
            crate::collect::ProcessViewMode::AppGroup => {
                crate::tui::app_group_view::draw(f, area, app);
            }
        },
        AppMode::ProcessDetail => {
            crate::tui::detail_view::draw(f, area, app);
        }
        AppMode::PortMap => {
            crate::tui::port_table::draw(f, area, app);
        }
        AppMode::UsbAssistant => {
            crate::tui::usb_panel::draw(f, area, app);
        }
        AppMode::MonitorPanel => {
            crate::tui::monitor_panel::draw(f, area, app);
        }
        AppMode::DockerPanel => {
            crate::tui::docker_panel::draw(f, area, app);
        }
        AppMode::ContainerExec => {
            crate::tui::container_exec_view::draw(f, area, app);
        }
        AppMode::Replay => {
            crate::tui::replay_panel::draw_timeline(f, area, app);
        }
        // Help mode is rendered by draw_middle before this function is reached.
        AppMode::Help => {}
        // v0.21：Agent mode 同 Help，由 draw_middle 提前渲染（全屏占位面板）。
        AppMode::Agent => {}
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let base = match app.mode {
        AppMode::UsbAssistant => {
            " ↑↓移动  Enter选择设备  k终止安全进程  r刷新  w持续监测  Tab切换设备  q退出"
        }
        AppMode::MonitorPanel => " ↑↓移动  a添加监控  d删除  r重启  s暂停/恢复  q退出",
        AppMode::DockerPanel => " ↑↓移动  Enter详情  r重启  s停止  a监听事件  e进容器 exec  q退出",
        AppMode::ContainerExec => {
            " Ctrl+D/Ctrl+\\ 退出  Ctrl+C 中断容器  按键透传容器  q 退出 exec"
        }
        AppMode::PortMap if app.port_panel.panel.dns_view_active => {
            " ↑↓滚动  /搜索  c清空  f切换follow  D/Esc退出DNS视图  q退出"
        }
        AppMode::PortMap => {
            " ↑↓移动  Enter展开/详情  g切换视图  a异常  f过滤  s排序  d诊断  DDNS日志  /搜索  k终止  q退出"
        }
        AppMode::Replay => " Space播放/暂停  ←→快退/快进  +/-速度  q退出回放",
        AppMode::Agent => {
            " Enter 发送  Esc 中断/退出  y·n 写操作确认  PgUp/PgDn 滚动  Ctrl+D 退出面板"
        }
        AppMode::Help => " ↑↓/PgUp/PgDn滚动  Esc/q/? 返回  Home/End 顶/底",
        AppMode::ProcessList
            if app.process_panel.panel.process_view_mode
                == crate::collect::ProcessViewMode::AppGroup =>
        {
            " ↑↓移动  Enter展开/折叠  Space选择  k终止  S排序  v切换视图  /搜索  q退出"
        }
        AppMode::ProcessList
            if app.process_panel.panel.process_view_mode
                == crate::collect::ProcessViewMode::Tree =>
        {
            " ↑↓移动  Enter展开/折叠  Space选择  o选孤儿  z选僵尸  f过滤  k终止  /搜索  q退出"
        }
        _ => " ↑↓移动  Space选择  Enter操作  /搜索  S安全排序  v应用视图  ?帮助  q退出",
    };
    let msg = match &app.status_message {
        Some(m) => format!(" {} | {}", m, base),
        None => base.to_string(),
    };
    let kill_msg = if app.kill_confirm {
        " ⚠ 确认终止? (y/n)"
    } else {
        ""
    };
    // 阶段 8 D3：DNS worker 活动时，状态栏左侧显示「仅内存」指示（隐私）。
    let dns_indicator = if app.workers.dns_log_worker.is_some() {
        " 📡DNS(仅内存) "
    } else {
        ""
    };
    let theme_label = format!(" [T] {} ", theme::theme_name());
    let help = Paragraph::new(format!(
        "{}{}{}{}",
        dns_indicator, msg, kill_msg, theme_label
    ))
    .style(theme::style_muted());
    f.render_widget(help, area);
}
