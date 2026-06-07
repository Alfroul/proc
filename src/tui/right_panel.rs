use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::classify;
use crate::collect::ProcessInfo;
use crate::format::{format_bytes, format_run_time};
use crate::tui::theme;

const LABEL_W: usize = 5;

fn wrap_text(text: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 || text.is_empty() {
        return vec![if text.is_empty() { "-".to_string() } else { text.to_string() }];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + max_w).min(chars.len());
        result.push(chars[start..end].iter().collect());
        start = end;
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

fn push_kv(lines: &mut Vec<Line<'static>>, label: &str, value: &str, value_w: usize) {
    push_kv_styled(lines, label, value, value_w, theme::style_normal());
}

fn push_kv_styled(lines: &mut Vec<Line<'static>>, label: &str, value: &str, value_w: usize, value_style: ratatui::style::Style) {
    let lbl = format!("  {:<w$}", label, w = LABEL_W);
    let wrapped = wrap_text(value, value_w);
    let indent = " ".repeat(LABEL_W + 2);
    for (i, part) in wrapped.iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(lbl.clone(), theme::style_muted()),
                Span::styled(part.clone(), value_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(indent.clone(), theme::style_muted()),
                Span::styled(part.clone(), value_style),
            ]));
        }
    }
}

fn count_preview_lines(proc: &ProcessInfo, all_procs: &[(classify::ProcessClass, ProcessInfo)], value_w: usize) -> u16 {
    let parent_name = proc.parent_pid.and_then(|ppid| {
        all_procs.iter().find(|(_, p)| p.pid == ppid).map(|(_, p)| p.name.clone())
    });
    let parent_display = match &parent_name {
        Some(name) => format!("{} ({})", proc.parent_pid.unwrap_or(0), name),
        None => proc.parent_pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string()),
    };

    let cmd_str = if proc.cmd.is_empty() { "-".to_string() } else { proc.cmd.join(" ") };
    let exe_str = proc.exe.as_deref().unwrap_or("-");
    let cwd_str = proc.cwd.as_deref().unwrap_or("-");

    let (disk_r, disk_w) = proc.disk_usage;
    let disk_display = format!("R:{} W:{}", format_bytes(disk_r), format_bytes(disk_w));

    let mut count: u16 = 1;
    count += wrap_text(&parent_display, value_w).len() as u16;
    count += wrap_text(&proc.pid.to_string(), value_w).len() as u16;
    count += wrap_text(&cmd_str, value_w).len() as u16;
    count += wrap_text(exe_str, value_w).len() as u16;
    count += wrap_text(cwd_str, value_w).len() as u16;
    count += wrap_text(&format_run_time(proc.run_time), value_w).len() as u16;
    count += wrap_text(&disk_display, value_w).len() as u16;
    count
}

fn build_preview_lines(
    proc: &ProcessInfo,
    all_procs: &[(classify::ProcessClass, ProcessInfo)],
    value_w: usize,
) -> Vec<Line<'static>> {
    let parent_name = proc.parent_pid.and_then(|ppid| {
        all_procs.iter().find(|(_, p)| p.pid == ppid).map(|(_, p)| p.name.clone())
    });
    let parent_display = match &parent_name {
        Some(name) => format!("{} ({})", proc.parent_pid.unwrap_or(0), name),
        None => proc.parent_pid.map(|p| p.to_string()).unwrap_or_else(|| "-".to_string()),
    };

    let cmd_str = if proc.cmd.is_empty() { "-".to_string() } else { proc.cmd.join(" ") };
    let exe_str = proc.exe.as_deref().unwrap_or("-").to_string();
    let cwd_str = proc.cwd.as_deref().unwrap_or("-").to_string();
    let (disk_r, disk_w) = proc.disk_usage;
    let disk_display = format!("R:{} W:{}", format_bytes(disk_r), format_bytes(disk_w));

    let title = Line::from(Span::styled(
        format!(" {} ", proc.name),
        theme::style_selected(),
    ));

    let mut lines = vec![title];
    push_kv(&mut lines, "PPID", &parent_display, value_w);
    push_kv(&mut lines, "PID", &proc.pid.to_string(), value_w);
    push_kv(&mut lines, "CMD", &cmd_str, value_w);
    push_kv(&mut lines, "EXE", &exe_str, value_w);
    push_kv_styled(&mut lines, "CWD", &cwd_str, value_w, Style::new().fg(Color::Rgb(255, 150, 180)));
    push_kv(&mut lines, "运行", &format_run_time(proc.run_time), value_w);
    push_kv(&mut lines, "I/O", &disk_display, value_w);
    lines
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    if area.width < 10 || area.height < 6 {
        return;
    }

    let value_w = (area.width as usize).saturating_sub(LABEL_W + 3);

    let processes = app.get_filtered_sorted_processes();
    let proc_info = processes.get(app.cursor_index).map(|(_, p)| p.clone());

    let preview_lines = match &proc_info {
        Some(proc_) => count_preview_lines(proc_, &processes, value_w),
        None => 2,
    };
    let preview_height = preview_lines.min(area.height.saturating_sub(4));
    let history_height = area.height.saturating_sub(preview_height + 1);

    let block = Block::default()
        .borders(Borders::RIGHT)
        .style(theme::style_normal());
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 4 {
        return;
    }

    let sep_row = preview_height as u16;
    let hist_start = sep_row + 1;

    let preview_area = Rect::new(inner.x, inner.y, inner.width, sep_row);
    let lines = match &proc_info {
        Some(proc_) => build_preview_lines(proc_, &processes, value_w),
        None => vec![Line::from(Span::styled("  (无选中进程)", theme::style_muted()))],
    };
    f.render_widget(Paragraph::new(lines), preview_area);

    let sep_area = Rect::new(inner.x, inner.y + sep_row, inner.width, 1);
    f.render_widget(Paragraph::new(Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        theme::style_muted(),
    ))), sep_area);

    if history_height > 2 {
        let hist_area = Rect::new(inner.x, inner.y + hist_start, inner.width, history_height.saturating_sub(1));
        let title = Line::from(Span::styled(" 操作历史", theme::style_selected()));
        let sep2 = Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            theme::style_muted(),
        ));

        let mut lines = vec![title, sep2];

        if app.op_history.is_empty() {
            lines.push(Line::from(Span::styled("  (无操作记录)", theme::style_muted())));
        } else {
            let visible = hist_area.height.saturating_sub(2) as usize;
            let history = &app.op_history;
            let start = if history.len() > visible {
                history.len() - visible
            } else {
                0
            };
            for record in history.iter().skip(start) {
                let time_span = Span::styled(format!(" {} ", record.time), theme::style_muted());
                let msg_span = Span::styled(record.message.clone(), theme::style_normal());
                lines.push(Line::from(vec![time_span, msg_span]));
            }
        }

        let p = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(p, hist_area);
    }
}
