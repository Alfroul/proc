use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::app::App;
use crate::classify::ProcessClass;
use crate::tui::theme;
use crate::tree::TreeFilter;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let visible = app.get_filtered_tree_visible();
    let rows_visible = area.height.saturating_sub(3) as usize;
    let (_, total_mem) = app.snapshot.memory_usage();

    let items: Vec<ListItem> = visible
        .iter()
        .skip(app.tree_scroll)
        .take(rows_visible)
        .enumerate()
        .map(|(i, node)| {
            let global_i = i + app.tree_scroll;
            let is_cursor = global_i == app.tree_cursor;
            let is_selected = app.tree_selected_indices.contains(&global_i);

            let indent = "  ".repeat(node.depth);
            let expand_icon = if node.children.is_empty() {
                "  "
            } else if node.expanded {
                "▼ "
            } else {
                "▶ "
            };

            let checkbox = if is_selected { "☑ " } else { "☐ " };

            let mem_pct = if total_mem > 0 {
                node.memory as f64 / total_mem as f64 * 100.0
            } else {
                0.0
            };

            let class_tag = format!("[{}]", node.class.label());

            let base_line = format!(
                "{}{}{}{:>6} {:>5.1}% {:>5.1}% {} {}",
                indent,
                expand_icon,
                checkbox,
                node.pid,
                node.cpu,
                mem_pct,
                class_tag,
                node.name,
            );

            let bg = if is_cursor {
                Color::DarkGray
            } else if is_selected {
                Color::Rgb(40, 40, 60)
            } else {
                Color::Reset
            };

            let base_style = Style::default()
                .fg(theme::text_primary())
                .bg(bg)
                .patch(class_style(&node.class));

            let mut spans = vec![Span::styled(base_line, base_style)];

            if node.is_orphan {
                spans.push(Span::styled(
                    " ⚠孤儿".to_string(),
                    Style::default().fg(Color::Yellow).bg(bg),
                ));
            }
            if node.is_zombie {
                spans.push(Span::styled(
                    " 💀僵尸".to_string(),
                    Style::default().fg(Color::Red).bg(bg).add_modifier(Modifier::BOLD),
                ));
            } else if node.is_stale {
                spans.push(Span::styled(
                    " ⏳残存".to_string(),
                    Style::default().fg(Color::DarkGray).bg(bg),
                ));
            }

            if let Some(ref safety) = node.kill_safety {
                let (tag, color) = match safety {
                    crate::tree::KillSafety::Safe => (" 可杀", Color::Green),
                    crate::tree::KillSafety::Caution => (" 有子进程", Color::Yellow),
                };
                spans.push(Span::styled(
                    tag.to_string(),
                    Style::default().fg(color).bg(bg),
                ));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let (orphan_count, zombie_count) = {
        let filtered = crate::tree::filter_tree(&app.tree_nodes, app.tree_filter);
        crate::tree::count_anomalies(&filtered)
    };

    let filter_label = match app.tree_filter {
        TreeFilter::All => "全部",
        TreeFilter::MyProcesses => "用户进程",
        TreeFilter::SystemProcesses => "系统进程",
    };

    let search_indicator = if app.tree_search_active {
        format!(" 搜索: {} | ESC取消", app.tree_search_query)
    } else if !app.tree_search_query.is_empty() {
        format!(" 过滤: {}", app.tree_search_query)
    } else {
        String::new()
    };

    let selected_count = app.tree_selected_indices.len();
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

    let title = format!(
        "进程树 | {} f切换 | Space选择 k终止 | o选孤儿 z选残存{}{}{}",
        filter_label, anomaly_info, selected_info, search_indicator
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .style(theme::style_normal());

    let list = List::new(items).block(block);

    let mut state = ListState::default();
    let visible_cursor = app.tree_cursor.saturating_sub(app.tree_scroll);
    state.select(Some(visible_cursor));
    f.render_stateful_widget(list, area, &mut state);
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
