use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::App;
use crate::tui::theme;

/// Draw alert badge in sidebar or top area
pub fn draw_alert_badge(f: &mut Frame, area: Rect, app: &App) {
    let (info, warning, critical) = app.alert_manager.firing_counts();
    let total = info + warning + critical;

    if total == 0 {
        return;
    }

    let (icon, color) = if critical > 0 {
        // Blink effect: alternate between bright and dim
        let blink = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            % 1000
            < 500;
        let c = if blink {
            theme::danger()
        } else {
            theme::warning()
        };
        ("●", c)
    } else if warning > 0 {
        ("●", theme::warning())
    } else {
        ("●", theme::info())
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", icon),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("告警: {}", total), Style::default().fg(color)),
    ]);

    let p = Paragraph::new(line);
    f.render_widget(p, area);
}

/// Draw centered alert popup
pub fn draw_alert_popup(f: &mut Frame, app: &App) {
    let alerts = app.alert_manager.active_alerts();

    if alerts.is_empty() {
        let area = popup_area(f.area(), 40, 5);
        f.render_widget(Clear, area);
        let p = Paragraph::new("  无活跃告警")
            .style(theme::style_muted())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" 告警 ")
                    .style(theme::style_header()),
            );
        f.render_widget(p, area);
        return;
    }

    let popup_height = 20.min(5 + alerts.len() as u16);
    let area = popup_area(f.area(), 70, popup_height);
    f.render_widget(Clear, area);

    // Clamp scroll
    let max_scroll = alerts.len().saturating_sub(popup_height as usize - 4);
    let scroll = app.alert_scroll.min(max_scroll);

    let lines: Vec<Line> = alerts
        .iter()
        .skip(scroll)
        .take(popup_height as usize - 4)
        .map(|alert| {
            let (icon, color) = match alert.severity {
                crate::alert::AlertSeverity::Critical => ("🔴 严重", theme::danger()),
                crate::alert::AlertSeverity::Warning => ("⚠ 警告", theme::warning()),
                crate::alert::AlertSeverity::Info => ("ℹ 提示", theme::info()),
            };

            let pid_str = alert
                .related_pid
                .map(|p| format!(" (PID {})", p))
                .unwrap_or_default();

            let elapsed = alert.triggered_at.elapsed();
            let time_str = if elapsed.as_secs() < 60 {
                format!("{}秒前", elapsed.as_secs())
            } else {
                format!("{}分前", elapsed.as_secs() / 60)
            };

            Line::from(vec![
                Span::styled(
                    format!(" {} ", icon),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:.1}/{}", alert.current_value, alert.threshold),
                    theme::style_normal(),
                ),
                Span::styled(pid_str, theme::style_muted()),
                Span::styled(format!("  {}", time_str), theme::style_muted()),
            ])
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" 告警 ({}) - A 开关，Esc 关闭 ", alerts.len()))
        .style(theme::style_header());

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn popup_area(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}
