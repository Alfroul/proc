use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

use crate::app::App;
use crate::classify::ProcessClass;
use crate::collect::SortField;
use crate::format::format_bytes;
use crate::tui::security_badge;
use crate::tui::theme;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let sorted = app.get_filtered_sorted_processes();

    let rows_visible = area.height.saturating_sub(3) as usize;
    let show_disk = matches!(
        app.process_panel.sort_field,
        SortField::DiskRead | SortField::DiskWrite
    );

    let header = if show_disk {
        Row::new(vec![
            Cell::from("  "),
            Cell::from("PID"),
            Cell::from("CPU%"),
            Cell::from("MEM%"),
            Cell::from("内存"),
            Cell::from("磁盘R"),
            Cell::from("磁盘W"),
            Cell::from("状态"),
            Cell::from("分类"),
            Cell::from("安全"),
            Cell::from("名称"),
        ])
        .style(theme::style_header())
    } else {
        Row::new(vec![
            Cell::from("  "),
            Cell::from("PID"),
            Cell::from("CPU%"),
            Cell::from("MEM%"),
            Cell::from("内存"),
            Cell::from("状态"),
            Cell::from("分类"),
            Cell::from("安全"),
            Cell::from("名称"),
        ])
        .style(theme::style_header())
    };

    let rows: Vec<Row> = sorted
        .iter()
        .enumerate()
        .skip(app.process_panel.scroll_offset)
        .take(rows_visible)
        .map(|(i, (idx, class))| {
            let proc = &app.cached_processes[*idx];
            let selected = app.process_panel.selected_pids.contains(&proc.pid);
            let is_cursor = i == app.process_panel.cursor_index;

            let checkbox = if selected { "☑ " } else { "☐ " };
            let mem_str = format_bytes(proc.memory);
            let cpu_str = format!("{:.1}", proc.cpu_usage);
            let (_, total_mem) = app.snapshot.memory_usage();
            let mem_pct = if total_mem > 0 {
                format!("{:.1}", proc.memory as f64 / total_mem as f64 * 100.0)
            } else {
                "0.0".to_string()
            };

            let bg = if is_cursor {
                Color::DarkGray
            } else if selected {
                Color::Rgb(40, 40, 60)
            } else {
                Color::Reset
            };

            let row_style = Style::default().fg(theme::text_primary()).bg(bg);

            let sec_cell = if let Some(score) = app.security_scores.get(&proc.pid) {
                let text = if score.score >= 90 {
                    String::new()
                } else if score.score >= 60 {
                    format!("{}", score.score)
                } else if score.score >= 30 {
                    format!("!{}", score.score)
                } else {
                    format!("!!{}", score.score)
                };
                Cell::from(text).style(security_badge::score_style(score.score))
            } else {
                Cell::from("")
            };

            if show_disk {
                let disk_r = crate::format::format_speed(proc.disk_read_speed);
                let disk_w = crate::format::format_speed(proc.disk_write_speed);
                Row::new(vec![
                    Cell::from(checkbox),
                    Cell::from(proc.pid.to_string()),
                    Cell::from(cpu_str),
                    Cell::from(mem_pct),
                    Cell::from(mem_str),
                    Cell::from(disk_r),
                    Cell::from(disk_w),
                    Cell::from(proc.status.clone()),
                    Cell::from(class.label()).style(class_style(class)),
                    sec_cell,
                    Cell::from(proc.name.clone()),
                ])
                .style(row_style)
            } else {
                Row::new(vec![
                    Cell::from(checkbox),
                    Cell::from(proc.pid.to_string()),
                    Cell::from(cpu_str),
                    Cell::from(mem_pct),
                    Cell::from(mem_str),
                    Cell::from(proc.status.clone()),
                    Cell::from(class.label()).style(class_style(class)),
                    sec_cell,
                    Cell::from(proc.name.clone()),
                ])
                .style(row_style)
            }
        })
        .collect();

    let widths: Vec<ratatui::layout::Constraint> = if show_disk {
        vec![
            ratatui::layout::Constraint::Length(2),
            ratatui::layout::Constraint::Length(7),
            ratatui::layout::Constraint::Length(7),
            ratatui::layout::Constraint::Length(7),
            ratatui::layout::Constraint::Length(9),
            ratatui::layout::Constraint::Length(9),
            ratatui::layout::Constraint::Length(9),
            ratatui::layout::Constraint::Length(6),
            ratatui::layout::Constraint::Length(4),
            ratatui::layout::Constraint::Length(5),
            ratatui::layout::Constraint::Min(10),
        ]
    } else {
        vec![
            ratatui::layout::Constraint::Length(2),
            ratatui::layout::Constraint::Length(7),
            ratatui::layout::Constraint::Length(7),
            ratatui::layout::Constraint::Length(7),
            ratatui::layout::Constraint::Length(9),
            ratatui::layout::Constraint::Length(6),
            ratatui::layout::Constraint::Length(4),
            ratatui::layout::Constraint::Length(5),
            ratatui::layout::Constraint::Min(10),
        ]
    };

    let sort_indicator = format!("排序: {} ◀▶切换", app.process_panel.sort_field.label());
    let search_indicator = if app.process_panel.search.is_active() {
        format!(" | 搜索: {} | ESC取消", app.process_panel.search.query())
    } else {
        String::new()
    };

    let title = format!("进程列表 | {}{}", sort_indicator, search_indicator);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .style(theme::style_normal());

    let table = Table::new(rows, widths).header(header).block(block);

    let mut state = TableState::default();
    let visible_cursor = app
        .process_panel
        .cursor_index
        .saturating_sub(app.process_panel.scroll_offset);
    state.select(Some(visible_cursor));
    f.render_stateful_widget(table, area, &mut state);
}

fn class_style(class: &ProcessClass) -> Style {
    match class {
        ProcessClass::UserApp => theme::style_normal(),
        ProcessClass::SystemProcess => theme::style_info(),
        ProcessClass::WindowsService => theme::style_warning(),
        ProcessClass::Kernel => theme::style_danger(),
        ProcessClass::Unknown => theme::style_muted(),
    }
}

#[must_use]
pub fn draw_placeholder(_area: Rect) -> String {
    "进程列表（加载中...）".to_string()
}
