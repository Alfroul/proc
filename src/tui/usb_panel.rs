use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{
    Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table, TableState, Wrap,
};

use crate::app::App;
use crate::format::format_bytes;
use crate::tui::theme;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    draw_device_list(f, chunks[0], app);
    draw_lock_list(f, chunks[1], app);
}

fn draw_device_list(f: &mut Frame, area: Rect, app: &App) {
    let devices = &app.usb_panel.devices;

    let rows: Vec<Row> = if devices.is_empty() {
        vec![Row::new(vec![
            Cell::from("未检测到可移除设备").style(theme::style_muted()),
        ])]
    } else {
        devices
            .iter()
            .enumerate()
            .map(|(i, dev)| {
                let size_info = format!(
                    "{}/{}",
                    format_bytes(dev.used_size),
                    format_bytes(dev.total_size)
                );
                let status = if dev.is_occupied {
                    "⚠ 有占用"
                } else {
                    "✓ 可弹出"
                };
                let style = if i == app.usb_panel.device_cursor {
                    theme::style_selected()
                } else if dev.is_occupied {
                    theme::style_warning()
                } else {
                    theme::style_success()
                };
                Row::new(vec![
                    Cell::from(format!("{}:", dev.drive_letter)).style(style),
                    Cell::from(dev.label.clone()).style(style),
                    Cell::from(size_info).style(style),
                    Cell::from(dev.file_system.clone()).style(style),
                    Cell::from(status).style(style),
                ])
            })
            .collect()
    };

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(16),
            Constraint::Length(6),
            Constraint::Length(10),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("盘符"),
            Cell::from("卷标"),
            Cell::from("容量"),
            Cell::from("文件系统"),
            Cell::from("状态"),
        ])
        .style(theme::style_header()),
    )
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .title(" 可移除设备 "),
    )
    .highlight_spacing(HighlightSpacing::Always);

    let mut state = TableState::default();
    if !devices.is_empty() && app.usb_panel.device_cursor < devices.len() {
        state.select(Some(app.usb_panel.device_cursor));
    }
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_lock_list(f: &mut Frame, area: Rect, app: &App) {
    let title_suffix = if let Some(dev) = app.usb_panel.devices.get(app.usb_panel.device_cursor) {
        format!(" 占用进程 - {}:", dev.drive_letter)
    } else {
        " 占用进程".to_string()
    };

    let locks = &app.usb_panel.locks;

    if locks.is_empty() {
        let has_occupied = app.usb_panel.devices.iter().any(|d| d.is_occupied);
        let msg = if app.usb_panel.devices.is_empty() {
            "未检测到可移除设备"
        } else if has_occupied {
            "按 Enter 查看占用进程详情，按 k 终止选中进程"
        } else if app.usb_panel.status_message.is_some() {
            app.usb_panel.status_message.as_deref().unwrap_or("")
        } else {
            "✅ 所有设备无占用进程，可以安全弹出"
        };
        let style = if app.usb_panel.devices.is_empty() {
            theme::style_muted()
        } else if has_occupied {
            theme::style_warning()
        } else {
            theme::style_success()
        };
        let p = Paragraph::new(msg)
            .style(style)
            .block(Block::default().borders(Borders::TOP).title(title_suffix));
        f.render_widget(p, area);
        return;
    }

    let rows: Vec<Row> = locks
        .iter()
        .enumerate()
        .map(|(i, (lock, risk))| {
            let ports = if lock.port_info.is_empty() {
                "-".to_string()
            } else {
                lock.port_info.join(", ")
            };
            let style = Style::default().fg(risk.color());
            let cursor_style = if i == app.usb_panel.lock_cursor {
                theme::style_selected()
            } else {
                style
            };
            Row::new(vec![
                Cell::from(risk.label()).style(style),
                Cell::from(lock.pid.to_string()).style(cursor_style),
                Cell::from(lock.process_name.clone()).style(cursor_style),
                Cell::from(lock.exe_path.as_deref().unwrap_or("-")).style(cursor_style),
                Cell::from(ports).style(cursor_style),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(15),
            Constraint::Min(20),
            Constraint::Length(16),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("风险"),
            Cell::from("PID"),
            Cell::from("进程名"),
            Cell::from("路径"),
            Cell::from("端口"),
        ])
        .style(theme::style_header()),
    )
    .block(Block::default().borders(Borders::TOP).title(title_suffix))
    .highlight_spacing(HighlightSpacing::Always);

    let mut state = TableState::default();
    if !locks.is_empty() && app.usb_panel.lock_cursor < locks.len() {
        state.select(Some(app.usb_panel.lock_cursor));
    }
    f.render_stateful_widget(table, area, &mut state);

    if let Some(ref msg) = app.usb_panel.status_message {
        let popup = Paragraph::new(msg.as_str())
            .style(theme::style_success())
            .wrap(Wrap { trim: true });
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(2)])
            .split(area);
        f.render_widget(popup, inner[1]);
    }
}
