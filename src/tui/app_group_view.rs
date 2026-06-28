use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};

use crate::app::App;
use crate::app_group::AppGroupItem;
use crate::format::format_bytes;
use crate::tui::theme;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let items = app.process_panel.panel.app_group_filtered_visual_items();
    let rows_visible = area.height.saturating_sub(3) as usize;
    let groups = &app.process_panel.panel.app_groups;

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("进程数"),
        Cell::from("CPU%"),
        Cell::from("MEM%"),
        Cell::from("内存"),
        Cell::from("名称"),
    ])
    .style(theme::style_header());

    let rows: Vec<Row> = items
        .iter()
        .skip(app.process_panel.panel.app_group_scroll)
        .take(rows_visible)
        .enumerate()
        .map(|(i, item)| {
            let global_i = i + app.process_panel.panel.app_group_scroll;
            let is_cursor = global_i == app.process_panel.panel.app_group_cursor;

            let bg = if is_cursor {
                Color::DarkGray
            } else {
                Color::Reset
            };

            match *item {
                AppGroupItem::Header { group_idx } => draw_group_header(
                    groups,
                    group_idx,
                    bg,
                    app.process_panel.panel.app_group_expanded == Some(group_idx),
                ),
                AppGroupItem::Child {
                    group_idx,
                    child_idx,
                } => draw_child_row(groups, group_idx, child_idx, bg),
            }
        })
        .collect();

    let sort_label = app.process_panel.panel.app_group_sort.label();
    let search_indicator = if app.process_panel.panel.app_group_search.is_active()
        || !app.process_panel.panel.app_group_search.query().is_empty()
    {
        format!(" 搜索:{}", app.process_panel.panel.app_group_search.query())
    } else {
        String::new()
    };
    let title = format!(
        " 应用分组 ({}) | 排序: {} S切换{} ",
        groups.len(),
        sort_label,
        search_indicator
    );

    let widths = vec![
        ratatui::layout::Constraint::Length(3), // expand icon
        ratatui::layout::Constraint::Length(6), // count
        ratatui::layout::Constraint::Length(7), // CPU%
        ratatui::layout::Constraint::Length(7), // MEM%
        ratatui::layout::Constraint::Length(9), // memory
        ratatui::layout::Constraint::Min(10),   // name
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .style(theme::style_normal());

    let table = Table::new(rows, widths).header(header).block(block);

    let mut state = TableState::default();
    state.select(Some(
        app.process_panel
            .panel
            .app_group_cursor
            .saturating_sub(app.process_panel.panel.app_group_scroll),
    ));
    f.render_stateful_widget(table, area, &mut state);
}

fn draw_group_header(
    groups: &[crate::app_group::AppGroup],
    gi: usize,
    bg: Color,
    is_expanded: bool,
) -> Row<'static> {
    let group = &groups[gi];
    let expand_icon = if is_expanded { "▼" } else { "▶" };
    let count = format!("({})", group.processes.len());
    let cpu = format!("{:.1}", group.total_cpu);

    let total_mem: u64 = groups.iter().map(|g| g.total_memory).sum();
    let mem_pct = if total_mem > 0 {
        format!(
            "{:.1}",
            group.total_memory as f64 / total_mem as f64 * 100.0
        )
    } else {
        "0.0".to_string()
    };
    let mem = format_bytes(group.total_memory);

    let style = theme::style_header().bg(bg);

    Row::new(vec![
        Cell::from(expand_icon).style(style),
        Cell::from(count).style(theme::style_muted().bg(bg)),
        Cell::from(cpu).style(style),
        Cell::from(mem_pct).style(style),
        Cell::from(mem).style(style),
        Cell::from(group.display_name.clone()).style(style),
    ])
}

fn draw_child_row(
    groups: &[crate::app_group::AppGroup],
    gi: usize,
    ci: usize,
    bg: Color,
) -> Row<'static> {
    let group = &groups[gi];
    let proc = &group.processes[ci];
    let is_last = ci + 1 == group.processes.len();
    let branch = if is_last { "└" } else { "├" };

    let cpu = format!("{:.1}", proc.cpu_usage);

    let total_mem: u64 = groups.iter().map(|g| g.total_memory).sum();
    let mem_pct = if total_mem > 0 {
        format!("{:.1}", proc.memory as f64 / total_mem as f64 * 100.0)
    } else {
        "0.0".to_string()
    };
    let mem = format_bytes(proc.memory);

    let style = theme::style_normal().bg(bg);

    let role_str = proc
        .role_hint
        .as_deref()
        .map(|r| format!(" [{}]", r))
        .unwrap_or_default();

    let name_spans = vec![
        Span::styled(format!("{}── {} ({})", branch, proc.name, proc.pid), style),
        Span::styled(role_str, theme::style_info().bg(bg)),
    ];

    Row::new(vec![
        Cell::from(""),
        Cell::from("").style(style),
        Cell::from(cpu).style(style),
        Cell::from(mem_pct).style(style),
        Cell::from(mem).style(style),
        Cell::from(Line::from(name_spans)),
    ])
}
