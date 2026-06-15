use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

use crate::app::{App, AppMode, ReplaySpeed};
use crate::tui::theme;

pub fn draw_timeline(f: &mut Frame, area: Rect, app: &App) {
    let (ts, player) = match (&app.timeline_state, &app.replay_player) {
        (Some(ts), Some(player)) => (ts, player),
        _ => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" 回放模式 ")
                .style(theme::style_normal());
            let p = Paragraph::new(" 无录制数据 ").block(block);
            f.render_widget(p, area);
            return;
        }
    };

    let total = ts.total_frames;
    let current = ts.current_frame;

    // Split area: top for main panel, bottom for timeline
    let timeline_height = 5u16;
    let [main_area, timeline_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Min(0),
        ratatui::layout::Constraint::Length(timeline_height),
    ])
    .areas(area);

    // Render the panel that was active when the frame was recorded
    let frame_mode = app.replay_frame_mode();
    match frame_mode {
        AppMode::ProcessList => {
            crate::tui::process_table::draw(f, main_area, app);
        }
        AppMode::PortMap => {
            crate::tui::port_table::draw(f, main_area, app);
        }
        AppMode::UsbAssistant => {
            crate::tui::usb_panel::draw(f, main_area, app);
        }
        AppMode::MonitorPanel => {
            crate::tui::monitor_panel::draw(f, main_area, app);
        }
        AppMode::DockerPanel => {
            crate::tui::docker_panel::draw(f, main_area, app);
        }
        _ => {
            crate::tui::process_table::draw(f, main_area, app);
        }
    }

    // Draw timeline controls
    let block = Block::default()
        .borders(Borders::ALL)
        .style(theme::style_normal());

    let inner = block.inner(timeline_area);
    f.render_widget(block, timeline_area);

    let [info_area, gauge_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Length(1),
    ])
    .areas(inner);

    let icon = if ts.playing {
        "\u{25B6}" // ▶
    } else {
        "\u{23F8}" // ⏸
    };

    let speed_label = match ts.speed {
        ReplaySpeed::Half => "0.5x",
        ReplaySpeed::Normal => "1x",
        ReplaySpeed::Double => "2x",
        ReplaySpeed::Quad => "4x",
    };

    let (start_ts, end_ts) = player.time_range();
    let current_ts = player
        .frame_at(current)
        .map(|f| f.timestamp)
        .unwrap_or(start_ts);
    let _start_str = format_timestamp(start_ts);
    let current_str = format_timestamp(current_ts);
    let end_str = format_timestamp(end_ts);

    let duration = if end_ts > start_ts {
        format_duration(end_ts - start_ts)
    } else {
        "00:00".to_string()
    };

    // Show which panel was active during recording
    let mode_label = mode_display_name(&frame_mode);

    let info_line = Line::from(vec![
        Span::styled(format!(" {} ", icon), theme::style_selected()),
        Span::styled(format!("{} ", speed_label), theme::style_muted()),
        Span::styled(
            format!("{} / {} ", current_str, end_str),
            theme::style_normal(),
        ),
        Span::styled(format!("({})", duration), theme::style_muted()),
        Span::styled(
            format!("  帧 {}/{}", current + 1, total),
            theme::style_muted(),
        ),
        Span::styled(format!("  [{}]", mode_label), theme::style_selected()),
    ]);
    f.render_widget(Paragraph::new(info_line), info_area);

    let progress = if total > 0 {
        (current as f64 / total.saturating_sub(1).max(1) as f64).min(1.0)
    } else {
        0.0
    };

    let gauge_widget = Gauge::default()
        .gauge_style(theme::style_selected())
        .ratio(progress);
    f.render_widget(gauge_widget, gauge_area);
}

fn mode_display_name(mode: &AppMode) -> &'static str {
    match mode {
        AppMode::ProcessList => "进程",
        AppMode::PortMap => "端口",
        AppMode::UsbAssistant => "U盘",
        AppMode::MonitorPanel => "监控",
        AppMode::DockerPanel => "Docker",
        AppMode::ProcessDetail => "详情",
        _ => "进程列表",
    }
}

fn format_timestamp(ts: u64) -> String {
    let local = ts + 8 * 3600;
    let h = ((local / 3600) % 24) as u8;
    let m = ((local / 60) % 60) as u8;
    let s = (local % 60) as u8;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn format_duration(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{:02}:{:02}", m, s)
}
