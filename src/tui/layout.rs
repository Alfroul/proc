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
        AppMode::ProcessList | AppMode::ProcessDetail | AppMode::Help => 0,
        AppMode::PortMap => 2,
        AppMode::UsbAssistant => 3,
        AppMode::MonitorPanel => 4,
        AppMode::DockerPanel => 5,
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
        && app.process_panel.process_view_mode == crate::collect::ProcessViewMode::Tree
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
    match app.mode {
        AppMode::ProcessList | AppMode::Replay => {
            let show_right = app.mode != AppMode::Replay
                && app.process_panel.process_view_mode == crate::collect::ProcessViewMode::List;

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
        AppMode::ProcessList => match app.process_panel.process_view_mode {
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
        AppMode::Replay => {
            crate::tui::replay_panel::draw_timeline(f, area, app);
        }
        // Help mode is rendered by draw_middle before this function is reached.
        AppMode::Help => {}
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let base = match app.mode {
        AppMode::UsbAssistant => {
            " ↑↓移动  Enter选择设备  k终止安全进程  r刷新  w持续监测  Tab切换设备  q退出"
        }
        AppMode::MonitorPanel => " ↑↓移动  a添加监控  d删除  r重启  s暂停/恢复  q退出",
        AppMode::DockerPanel => " ↑↓移动  Enter详情  r重启  s停止  a监听事件  q退出",
        AppMode::PortMap => {
            " ↑↓移动  Enter展开/详情  g切换视图  a异常  f过滤  s排序  d诊断  /搜索  k终止  q退出"
        }
        AppMode::Replay => " Space播放/暂停  ←→快退/快进  +/-速度  q退出回放",
        AppMode::Help => " ↑↓/PgUp/PgDn滚动  Esc/q/? 返回  Home/End 顶/底",
        AppMode::ProcessList
            if app.process_panel.process_view_mode == crate::collect::ProcessViewMode::AppGroup =>
        {
            " ↑↓移动  Enter展开/折叠  Space选择  k终止  S排序  v切换视图  /搜索  q退出"
        }
        AppMode::ProcessList
            if app.process_panel.process_view_mode == crate::collect::ProcessViewMode::Tree =>
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
    let theme_label = format!(" [T] {} ", theme::theme_name());
    let help =
        Paragraph::new(format!("{}{}{}", msg, kill_msg, theme_label)).style(theme::style_muted());
    f.render_widget(help, area);
}
