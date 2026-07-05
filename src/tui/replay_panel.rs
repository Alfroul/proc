use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph};

use crate::app::{App, AppMode, ReplaySpeed};
use crate::tui::theme;

pub fn draw_timeline(f: &mut Frame, area: Rect, app: &App) {
    let (ts, player) = match (&app.replay.timeline_state, &app.replay.replay_player) {
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
        // v0.14 stage 2：书签面板入口提示（无书签时也显示，提示用户功能存在）
        Span::styled(
            "  [B 书签]",
            if app
                .replay
                .bookmarks()
                .map(|f| !f.bookmarks.is_empty())
                .unwrap_or(false)
            {
                theme::style_selected()
            } else {
                theme::style_muted()
            },
        ),
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

    // v0.14 stage 2：书签面板 modal（B 键打开）
    if app.replay.bookmark_panel.is_some() {
        draw_bookmark_panel(f, area, app);
    }
}

/// v0.14 stage 2：书签面板 modal — 居中浮层，列出全部书签（可子串过滤）。
fn draw_bookmark_panel(f: &mut Frame, area: Rect, app: &App) {
    let Some(file) = app.replay.bookmarks() else {
        return;
    };
    let Some(panel) = app.replay.bookmark_panel() else {
        return;
    };

    // 居中浮层 70% × 60%
    let popup = centered_rect(70, 60, area);
    f.render_widget(Clear, popup);

    let title = if file.bookmarks.is_empty() {
        " 书签（暂无）— Esc 关闭 ".to_string()
    } else {
        format!(
            " 书签 ({}) — Up/Down 选择 · Enter 跳转 · e 编辑 · d 删除 · / 搜索 · Esc 关闭 ",
            file.bookmarks.len()
        )
    };

    let border_style = Style::default().add_modifier(Modifier::REVERSED);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(border_style);
    f.render_widget(block, popup);

    let inner = Rect::new(
        popup.x + 1,
        popup.y + 1,
        popup.width.saturating_sub(2),
        popup.height.saturating_sub(2),
    );
    let [list_area, search_area] =
        ratatui::layout::Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    // 过滤后的书签索引（与 ReplayController::filtered_bookmark_indices 一致）
    let q = panel.search_query.trim().to_lowercase();
    let filtered_indices: Vec<usize> = if q.is_empty() {
        (0..file.bookmarks.len()).collect()
    } else {
        file.bookmarks
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.label.to_lowercase().contains(&q)
                    || b.frame_idx.to_string().contains(&q)
                    || b.id.to_string().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    };

    // 书签列表
    let items: Vec<ListItem> = filtered_indices
        .iter()
        .map(|&i| {
            let bm = &file.bookmarks[i];
            let label_display = if panel.editing_id == Some(bm.id) {
                let mut s = panel.editing_label.clone().unwrap_or_default();
                if panel.is_editing() {
                    s.push('_');
                }
                s
            } else {
                bm.label.clone()
            };
            let ts = format_timestamp(bm.timestamp_secs);
            ListItem::new(Line::from(vec![
                Span::raw(format!("#{}  ", bm.id)),
                Span::raw(format!("帧 {:>5}  ", bm.frame_idx)),
                Span::raw(format!("{}  ", ts)),
                Span::raw(label_display),
            ]))
        })
        .collect();

    let list = List::new(items)
        .style(theme::style_normal())
        .highlight_style(theme::style_selected().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    if !filtered_indices.is_empty() {
        state.select(Some(panel.cursor.min(filtered_indices.len() - 1)));
    }
    f.render_stateful_widget(list, list_area, &mut state);

    // 搜索行
    let search_text = if panel.is_editing() {
        format!(
            " 编辑中: {}_  （Enter 提交 / Esc 取消）",
            panel.editing_label.as_deref().unwrap_or("")
        )
    } else {
        format!(" 搜索: {}_", panel.search_query)
    };
    let search_para = Paragraph::new(search_text)
        .style(theme::style_muted())
        .alignment(Alignment::Left);
    f.render_widget(search_para, search_area);
}

/// 居中浮层尺寸（百分比 × 高度）。
fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let x = r.width.saturating_sub(popup_width) / 2;
    let y = r.height.saturating_sub(height) / 2;
    Rect::new(
        r.x + x,
        r.y + y,
        popup_width.min(r.width),
        height.min(r.height),
    )
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
