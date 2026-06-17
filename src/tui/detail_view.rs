//! 进程详情页 — Inspector 多 Tab 视图（阶段 13，ADR-0004）。
//!
//! 顶部 Tab 栏（概要 / 环境 / 网络 / DLL）+ 主体内容区。Tab 间切换由 App
//! 持有的 `inspection_tab` 决定，搜索 / 滚动状态也都在 App 上以保持 TUI
//! 层无状态。

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};

use crate::app::App;
use crate::app_panel::InspectionTab;
use crate::classify;
use crate::format::format_bytes;
use crate::inspect::{DllInfo, EnvVar, InspectionData};
use crate::port_map::{self, Protocol};
use crate::tui::security_badge;
use crate::tui::theme;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let Some(proc) = app.detail_process.as_ref() else {
        return;
    };

    // 顶部 Tab 栏（1 行）+ 主体（剩余空间）。
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
    let tab_area = chunks[0];
    let body_area = chunks[1];

    draw_tab_bar(f, tab_area, app);

    match app.inspection_tab {
        InspectionTab::Summary => draw_summary(f, body_area, app, proc),
        InspectionTab::Env => {
            let data = app.inspection_data.as_ref();
            draw_env_tab(f, body_area, app, data)
        }
        InspectionTab::Network => {
            let data = app.inspection_data.as_ref();
            draw_network_tab(f, body_area, app, data)
        }
        InspectionTab::Dlls => {
            let data = app.inspection_data.as_ref();
            draw_dlls_tab(f, body_area, app, data)
        }
    }
}

// ── Tab 栏 ───────────────────────────────────────────────────────────────────

fn draw_tab_bar(f: &mut Frame, area: Rect, app: &App) {
    let tabs = InspectionTab::all();
    let spans: Vec<Span> = tabs
        .iter()
        .flat_map(|tab| {
            let label = tab.label();
            let style = if *tab == app.inspection_tab {
                Style::default()
                    .fg(theme::accent())
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                theme::style_muted()
            };
            let separator = Span::styled(" │ ", theme::style_muted());
            let chip = Span::styled(format!(" {} ", label), style);
            vec![separator, chip]
        })
        .chain(std::iter::once(Span::styled(" │", theme::style_muted())))
        .collect();

    let search_hint = if app.inspection_search.is_active() {
        format!(" 搜索: {} | ESC=取消", app.inspection_search.query())
    } else if !app.inspection_search.query().is_empty() {
        format!(" 过滤: {}", app.inspection_search.query())
    } else {
        String::new()
    };

    let line = if search_hint.is_empty() {
        Line::from(spans)
    } else {
        let mut all = spans;
        all.push(Span::styled(search_hint, theme::style_warning()));
        Line::from(all)
    };

    let title = " 进程详情 — Tab/Shift+Tab 切换  r=刷新  /=搜索 ".to_string();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    f.render_widget(Paragraph::new(line).block(block), area);
}

// ── Summary Tab（原详情页内容，向后兼容） ─────────────────────────────────────

fn draw_summary(f: &mut Frame, area: Rect, app: &App, proc: &crate::collect::ProcessInfo) {
    let class = classify::classify_process(proc);
    let run_time_secs = proc.run_time;
    let hours = run_time_secs / 3600;
    let mins = (run_time_secs % 3600) / 60;
    let secs = run_time_secs % 60;

    let disk_read = format_bytes(proc.disk_usage.0);
    let disk_write = format_bytes(proc.disk_usage.1);

    let net_summary = match port_map::scan_ports() {
        Ok(entries) => port_map::ProcessNetSummary::from_pid(proc.pid, &entries),
        Err(_) => port_map::ProcessNetSummary::default(),
    };

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("进程详情 — {} (PID {})", proc.name, proc.pid),
            theme::style_normal(),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  分类:     {}", class.label()),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!(
                "  父进程:   {}",
                proc.parent_pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string())
            ),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!("  状态:     {}", proc.status),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!("  CPU:      {:.1}%", proc.cpu_usage),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!(
                "  内存:     {} (物理) / {} (虚拟)",
                format_bytes(proc.memory),
                format_bytes(proc.virtual_memory)
            ),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!("  磁盘:     读 {} / 写 {}", disk_read, disk_write),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!("  运行时长: {}h {}m {}s", hours, mins, secs),
            theme::style_normal(),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  可执行:   {}", proc.exe.as_deref().unwrap_or("-")),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!("  命令行:   {}", proc.cmd.join(" ")),
            theme::style_normal(),
        )),
        Line::from(Span::styled(
            format!("  工作目录: {}", proc.cwd.as_deref().unwrap_or("-")),
            theme::style_normal(),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  占用端口: {}", app.detail_port_info),
            theme::style_normal(),
        )),
    ];

    if net_summary.tcp_connections > 0 || net_summary.udp_connections > 0 {
        let close_wait_style = if net_summary.close_wait >= 10 {
            theme::style_danger()
        } else if net_summary.close_wait >= 3 {
            theme::style_warning()
        } else {
            theme::style_normal()
        };

        let cw = net_summary.close_wait;
        let base_net = format!(
            "  网络:     TCP {} (监听 {} / 建立 {} / CLOSE_WAIT ",
            net_summary.tcp_connections, net_summary.listening, net_summary.established,
        );
        let cw_str = format!(
            "{}{} / TIME_WAIT {}{}  UDP {}",
            if cw >= 3 { "⚠ " } else { "" },
            cw,
            net_summary.time_wait,
            ")",
            net_summary.udp_connections,
        );
        lines.push(Line::from(vec![
            Span::styled(base_net, theme::style_normal()),
            Span::styled(cw_str, close_wait_style),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  远程地址: {} 个", net_summary.unique_remote_addrs.len()),
            theme::style_normal(),
        )));
    }

    if let Some(score) = app.security_scores.get(&proc.pid) {
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            "  ── 安全分析 ──",
            theme::style_selected(),
        )));

        let score_style = security_badge::score_style(score.score);
        lines.push(Line::from(vec![
            Span::styled("  安全分:   ", theme::style_muted()),
            Span::styled(format!("{}", score.score), score_style),
            Span::styled(" / 100".to_string(), theme::style_muted()),
        ]));

        lines.push(Line::from(vec![
            Span::styled("  签名:     ", theme::style_muted()),
            Span::styled(score.signature.to_string(), theme::style_normal()),
        ]));

        if !score.factors.is_empty() {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                "  风险因子:",
                theme::style_warning(),
            )));
            for factor in &score.factors {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    • {} ", factor.description),
                        theme::style_normal(),
                    ),
                    Span::styled(format!("(-{})", factor.weight), theme::style_danger()),
                ]));
            }
        } else {
            lines.push(Line::from(Span::styled(
                "  无风险因子",
                theme::style_info(),
            )));
        }
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  ── 快捷键 ──".to_string(),
        theme::style_normal(),
    )));
    lines.push(Line::from(Span::styled(
        "  Tab=切Tab  /=搜索  r=刷新  k=终止  w=监控  c=复制  Esc=返回".to_string(),
        theme::style_normal(),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 概要 ")
        .style(theme::style_normal());

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.inspection_scroll as u16, 0));

    f.render_widget(paragraph, area);
}

// ── Env Tab ──────────────────────────────────────────────────────────────────

fn draw_env_tab(f: &mut Frame, area: Rect, app: &App, data: Option<&InspectionData>) {
    let query = app.inspection_search.query();
    let env: Vec<&EnvVar> = match data {
        None => Vec::new(),
        Some(d) => d
            .env
            .iter()
            .filter(|v| {
                query.is_empty() || matches_query(&v.key, query) || matches_query(&v.value, query)
            })
            .collect(),
    };

    let rows_visible = area.height.saturating_sub(4) as usize;
    let scroll = app.inspection_scroll.min(env.len().saturating_sub(1));

    let title = format!(" 环境变量 ({} 项) | /=搜索 ↑↓=滚动 ", env.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if env.is_empty() {
        let msg = if data.is_none() {
            "数据采集中… 按 r 刷新"
        } else if !query.is_empty() {
            "无匹配项 — 修改搜索或按 Esc 清空"
        } else {
            "⚠ 无环境变量 — 此进程可能已退出或权限不足（按 r 刷新）"
        };
        let p = Paragraph::new(Span::styled(msg, theme::style_warning())).block(block);
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![Cell::from("键"), Cell::from("值")]).style(theme::style_header());

    let rows: Vec<Row> = env
        .iter()
        .skip(scroll)
        .take(rows_visible)
        .map(|v| {
            Row::new(vec![
                Cell::from(v.key.clone()).style(Style::default().fg(theme::accent())),
                Cell::from(v.value.clone()).style(theme::style_normal()),
            ])
        })
        .collect();

    let table = Table::new(rows, [Constraint::Min(20), Constraint::Min(40)])
        .header(header)
        .block(block);

    f.render_widget(table, area);
}

// ── Network Tab ──────────────────────────────────────────────────────────────

fn draw_network_tab(f: &mut Frame, area: Rect, app: &App, data: Option<&InspectionData>) {
    // Network Tab 不接搜索 —— 数据量通常很小；用户可走主面板 PortMap 深查。
    let entries: Vec<&crate::port_map::PortEntry> = match data {
        None => Vec::new(),
        Some(d) => d.net.iter().collect(),
    };

    let rows_visible = area.height.saturating_sub(4) as usize;
    let scroll = app.inspection_scroll.min(entries.len().saturating_sub(1));

    let title = format!(" 网络连接 ({} 项) | ↑↓=滚动 ", entries.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if entries.is_empty() {
        let msg = if data.is_none() {
            "数据采集中… 按 r 刷新"
        } else {
            "⚠ 此进程当前无监听 / 连接（或权限不足）— 按 r 刷新"
        };
        let p = Paragraph::new(Span::styled(msg, theme::style_warning())).block(block);
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("协议"),
        Cell::from("本地"),
        Cell::from("远程"),
        Cell::from("状态"),
        Cell::from("PID"),
        Cell::from("进程名"),
    ])
    .style(theme::style_header());

    let rows: Vec<Row> = entries
        .iter()
        .skip(scroll)
        .take(rows_visible)
        .map(|e| {
            let proto_style = if e.protocol == Protocol::Tcp {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Green)
            };
            let local = format!("{}:{}", e.local_addr, e.local_port);
            let remote = match (e.remote_addr, e.remote_port) {
                (Some(addr), Some(port)) => format!("{}:{}", addr, port),
                _ => "-".to_string(),
            };
            Row::new(vec![
                Cell::from(format!("[{}]", e.protocol)).style(proto_style),
                Cell::from(local),
                Cell::from(remote),
                Cell::from(e.state.clone().unwrap_or_else(|| "-".to_string())),
                Cell::from(e.pid.to_string()),
                Cell::from(e.process_name.clone()),
            ])
            .style(theme::style_normal())
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Min(22),
            Constraint::Min(22),
            Constraint::Length(12),
            Constraint::Length(7),
            Constraint::Min(12),
        ],
    )
    .header(header)
    .block(block);

    f.render_widget(table, area);
}

// ── Dlls Tab ─────────────────────────────────────────────────────────────────

fn draw_dlls_tab(f: &mut Frame, area: Rect, app: &App, data: Option<&InspectionData>) {
    let query = app.inspection_search.query();
    // 阶段 13 任务：按 path 字母排序。inspect() 返回的 Vec 顺序不保证；
    // 这里再排一次，保证 UI 稳定。
    let mut dlls: Vec<&DllInfo> = match data {
        None => Vec::new(),
        Some(d) => d
            .dlls
            .iter()
            .filter(|d| query.is_empty() || matches_query(&d.path, query))
            .collect(),
    };
    dlls.sort_by(|a, b| a.path.cmp(&b.path));

    let rows_visible = area.height.saturating_sub(4) as usize;
    let scroll = app.inspection_scroll.min(dlls.len().saturating_sub(1));

    let title = format!(" 模块列表 ({} 项) | /=搜索 ↑↓=滚动 ", dlls.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if dlls.is_empty() {
        let msg = if data.is_none() {
            "数据采集中… 按 r 刷新"
        } else if !query.is_empty() {
            "无匹配模块 — 修改搜索或按 Esc 清空"
        } else {
            "⚠ 无已加载模块 — 此进程可能已退出或权限不足（按 r 刷新）"
        };
        let p = Paragraph::new(Span::styled(msg, theme::style_warning())).block(block);
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("路径"),
        Cell::from("基址"),
        Cell::from("大小"),
    ])
    .style(theme::style_header());

    let rows: Vec<Row> = dlls
        .iter()
        .skip(scroll)
        .take(rows_visible)
        .map(|d| {
            Row::new(vec![
                Cell::from(d.path.clone()).style(theme::style_normal()),
                Cell::from(format!("0x{:016X}", d.base_addr)).style(theme::style_muted()),
                Cell::from(format_bytes(d.size)).style(theme::style_muted()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Min(50),
            Constraint::Length(20),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(block);

    f.render_widget(table, area);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// 大小写不敏感的子串匹配（Env key/value + Dll path 都用得到）。
fn matches_query(haystack: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    haystack.to_lowercase().contains(&q)
}

#[must_use]
pub fn draw_placeholder(_area: Rect) -> String {
    "进程详情（开发中）".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_query_case_insensitive() {
        assert!(matches_query(
            "C:\\Windows\\System32\\kernel32.dll",
            "KERNEL32"
        ));
        assert!(matches_query("PATH", "path"));
        assert!(matches_query("/usr/lib/libc.so.6", "libc"));
        assert!(!matches_query("PATH", "tmp"));
        assert!(matches_query("anything", "")); // 空 query 不过滤
    }
}
