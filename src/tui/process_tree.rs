use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

use crate::app::App;
use crate::classify::ProcessClass;
use crate::format::format_bytes;
use crate::tree::TreeFilter;
use crate::tui::theme;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let visible = app.process_panel.get_filtered_tree_visible();
    let rows_visible = area.height.saturating_sub(3) as usize;

    let header = Row::new(vec![
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
    .style(theme::style_header());

    let rows: Vec<Row> = visible
        .iter()
        .skip(app.process_panel.tree_scroll)
        .take(rows_visible)
        .enumerate()
        .map(|(i, node)| {
            let global_i = i + app.process_panel.tree_scroll;
            let is_cursor = global_i == app.process_panel.tree_cursor;
            let is_selected = app.process_panel.tree_selected_pids.contains(&node.pid);

            let checkbox = if is_selected { "☑ " } else { "☐ " };

            let indent = "  ".repeat(node.depth);
            let expand_icon = if node.children.is_empty() {
                "  "
            } else if node.expanded {
                "▼ "
            } else {
                "▶ "
            };

            let name_text = format!("{}{}{}", indent, expand_icon, node.name);

            let bg = if is_cursor {
                Color::DarkGray
            } else if is_selected {
                Color::Rgb(40, 40, 60)
            } else {
                Color::Reset
            };

            let row_style = Style::default().fg(theme::text_primary()).bg(bg);

            let sec_cell = if let Some(score) = app.security_scores.get(&node.pid) {
                let text = if score.score >= 90 {
                    String::new()
                } else if score.score >= 60 {
                    format!("{}", score.score)
                } else if score.score >= 30 {
                    format!("!{}", score.score)
                } else {
                    format!("!!{}", score.score)
                };
                Cell::from(text).style(crate::tui::security_badge::score_style(score.score))
            } else {
                Cell::from("")
            };

            // Name cell with tree indent + anomaly badges
            let mut name_spans: Vec<Span> = vec![Span::styled(
                name_text,
                Style::default()
                    .fg(theme::text_primary())
                    .bg(bg)
                    .patch(class_style(&node.class)),
            )];

            if node.is_orphan {
                name_spans.push(Span::styled(
                    " ⚠孤儿".to_string(),
                    Style::default().fg(Color::Yellow).bg(bg),
                ));
            }
            if node.is_zombie {
                name_spans.push(Span::styled(
                    " 💀僵尸".to_string(),
                    Style::default()
                        .fg(Color::Red)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ));
            } else if node.is_stale {
                name_spans.push(Span::styled(
                    " ⏳残存".to_string(),
                    Style::default().fg(Color::DarkGray).bg(bg),
                ));
            }

            if let Some(ref safety) = node.kill_safety {
                let (tag, color) = match safety {
                    crate::tree::KillSafety::Safe => (" 可杀", Color::Green),
                    crate::tree::KillSafety::Caution => (" 有子进程", Color::Yellow),
                };
                name_spans.push(Span::styled(
                    tag.to_string(),
                    Style::default().fg(color).bg(bg),
                ));
            }

            Row::new(vec![
                Cell::from(checkbox).style(row_style),
                Cell::from(node.pid.to_string()).style(row_style),
                Cell::from(format!("{:.1}", node.cpu)).style(row_style),
                Cell::from(format!("{:.1}", node.mem_pct)).style(row_style),
                Cell::from(format_bytes(node.memory)).style(row_style),
                Cell::from(node.status.clone()).style(row_style),
                Cell::from(node.class.label()).style(class_style(&node.class).bg(bg)),
                sec_cell,
                Cell::from(Line::from(name_spans)),
            ])
        })
        .collect();

    let widths = vec![
        ratatui::layout::Constraint::Length(2), // checkbox
        ratatui::layout::Constraint::Length(7), // PID
        ratatui::layout::Constraint::Length(7), // CPU%
        ratatui::layout::Constraint::Length(7), // MEM%
        ratatui::layout::Constraint::Length(9), // 内存
        ratatui::layout::Constraint::Length(6), // 状态
        ratatui::layout::Constraint::Length(4), // 分类
        ratatui::layout::Constraint::Length(5), // 安全
        ratatui::layout::Constraint::Min(10),   // 名称
    ];

    let (orphan_count, zombie_count) = {
        let filtered =
            crate::tree::filter_tree(&app.process_panel.tree_nodes, app.process_panel.tree_filter);
        crate::tree::count_anomalies(&filtered)
    };

    let filter_label = match app.process_panel.tree_filter {
        TreeFilter::All => "全部",
        TreeFilter::MyProcesses => "用户进程",
        TreeFilter::SystemProcesses => "系统进程",
    };

    let search_indicator = if app.process_panel.tree_search.is_active() {
        format!(
            " | 搜索: {} | ESC取消",
            app.process_panel.tree_search.query()
        )
    } else if !app.process_panel.tree_search.query().is_empty() {
        format!(" | 过滤: {}", app.process_panel.tree_search.query())
    } else {
        String::new()
    };

    let selected_count = app.process_panel.tree_selected_pids.len();
    let selected_info = if selected_count > 0 {
        format!(" | 已选{}个", selected_count)
    } else {
        String::new()
    };

    let anomaly_info = if orphan_count > 0 || zombie_count > 0 {
        format!(" | 孤儿: {} 僵尸: {}", orphan_count, zombie_count)
    } else {
        String::new()
    };

    let sort_indicator = format!("排序: {} ◀▶切换", app.process_panel.tree_sort_field.label());

    let title = format!(
        "进程树 | {} f切换 | {}{}{}{}",
        filter_label, sort_indicator, anomaly_info, selected_info, search_indicator
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .style(theme::style_normal());

    let table = Table::new(rows, widths).header(header).block(block);

    let mut state = TableState::default();
    let visible_cursor = app
        .process_panel
        .tree_cursor
        .saturating_sub(app.process_panel.tree_scroll);
    state.select(Some(visible_cursor));
    f.render_stateful_widget(table, area, &mut state);
}

fn class_style(class: &ProcessClass) -> Style {
    match class {
        ProcessClass::UserApp => Style::default().fg(Color::White),
        ProcessClass::SystemProcess => Style::default().fg(Color::Blue),
        ProcessClass::WindowsService => Style::default().fg(Color::Yellow),
        ProcessClass::Kernel => Style::default().fg(Color::Red),
        ProcessClass::Unknown => Style::default().fg(Color::DarkGray),
    }
}

pub fn draw_placeholder(_area: Rect) -> String {
    "进程树（开发中）".to_string()
}
