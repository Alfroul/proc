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
        app.process_panel.panel.sort_field,
        SortField::DiskRead | SortField::DiskWrite
    );
    let show_net = matches!(
        app.process_panel.panel.sort_field,
        SortField::NetSent | SortField::NetRecv
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
    } else if show_net {
        Row::new(vec![
            Cell::from("  "),
            Cell::from("PID"),
            Cell::from("CPU%"),
            Cell::from("MEM%"),
            Cell::from("内存"),
            Cell::from("↑网络"),
            Cell::from("↓网络"),
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
        .skip(app.process_panel.panel.scroll_offset)
        .take(rows_visible)
        .map(|(i, (idx, class))| {
            let proc = &app.cached_processes[*idx];
            let selected = app.process_panel.panel.selected_pids.contains(&proc.pid);
            let is_cursor = i == app.process_panel.panel.cursor_index;

            let checkbox = if selected { "☑ " } else { "☐ " };
            let mem_str = format_bytes(proc.memory);
            let cpu_str = format!("{:.1}", proc.cpu_usage);
            // v0.7 阶段 6：EcoQoS 🍃 标记（ADR-0014）。Eco 状态在 name 后追加，
            // Non-Eco 不渲染占位，避免列宽波动。
            // v0.11 阶段 4：签名状态 emoji（ADR-0021）。Trusted 🔒 / Unsigned|Revoked ⚠ /
            // Unknown ❓ / Pending|Signed 空串（不渲染占位）。status 字段由 App 在 poll
            // BackgroundScorer 结果后反向同步（src/app.rs::poll_background_scorer）。
            let name_str = format!(
                "{}{}{}",
                proc.name,
                proc.throttled.badge(),
                proc.signature_status.badge(),
            );
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
                    Cell::from(proc.status.to_string()),
                    Cell::from(class.label()).style(class_style(class)),
                    sec_cell,
                    Cell::from(name_str.clone()),
                ])
                .style(row_style)
            } else if show_net {
                let net_up = crate::format::format_speed(proc.net_sent_rate);
                let net_down = crate::format::format_speed(proc.net_recv_rate);
                Row::new(vec![
                    Cell::from(checkbox),
                    Cell::from(proc.pid.to_string()),
                    Cell::from(cpu_str),
                    Cell::from(mem_pct),
                    Cell::from(mem_str),
                    Cell::from(net_up),
                    Cell::from(net_down),
                    Cell::from(proc.status.to_string()),
                    Cell::from(class.label()).style(class_style(class)),
                    sec_cell,
                    Cell::from(name_str.clone()),
                ])
                .style(row_style)
            } else {
                Row::new(vec![
                    Cell::from(checkbox),
                    Cell::from(proc.pid.to_string()),
                    Cell::from(cpu_str),
                    Cell::from(mem_pct),
                    Cell::from(mem_str),
                    Cell::from(proc.status.to_string()),
                    Cell::from(class.label()).style(class_style(class)),
                    sec_cell,
                    Cell::from(name_str.clone()),
                ])
                .style(row_style)
            }
        })
        .collect();

    let widths: Vec<ratatui::layout::Constraint> = if show_disk || show_net {
        // 磁盘 / 网络两列宽度相同（都 9 字符 Length），合并分支避免重复
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

    let sort_indicator = format!(
        "排序: {} ◀▶切换",
        app.process_panel.panel.sort_field.label()
    );
    // v0.7 阶段 4：FilterExpr 模式下显示 mode 标识；parse 失败时改显 ⚠ 错误信息。
    let search_indicator = if app.process_panel.panel.search.is_active() {
        match app.process_panel.panel.search.mode {
            crate::search::QueryMode::Substring => {
                format!(
                    " | 搜索:{} | ESC取消",
                    app.process_panel.panel.search.query()
                )
            }
            crate::search::QueryMode::FilterExpr => {
                if let Some(err) = &app.process_panel.panel.search.filter_error {
                    // 错误截到 60 字符避免标题栏溢出（错误信息含 position + msg）。
                    let truncated = if err.chars().count() > 60 {
                        let mut s: String = err.chars().take(59).collect();
                        s.push('…');
                        s
                    } else {
                        err.clone()
                    };
                    format!(" | ⚠ {} | ESC取消", truncated)
                } else {
                    format!(
                        " | 过滤:{} | ESC取消",
                        app.process_panel.panel.search.query()
                    )
                }
            }
        }
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
        .panel
        .cursor_index
        .saturating_sub(app.process_panel.panel.scroll_offset);
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
