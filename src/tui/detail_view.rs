//! 进程详情页 — Inspector 多 Tab 视图（阶段 13，ADR-0004）。
//!
//! 顶部 Tab 栏（概要 / 环境 / 网络 / DLL）+ 主体内容区。Tab 间切换由 App
//! 持有的 `inspection_tab` 决定，搜索 / 滚动状态也都在 App 上以保持 TUI
//! 层无状态。

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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
    let Some(proc) = app.inspector.detail_process.as_ref() else {
        return;
    };

    // 顶部 Tab 栏（1 行）+ 主体（剩余空间）。
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);
    let tab_area = chunks[0];
    let body_area = chunks[1];

    draw_tab_bar(f, tab_area, app);

    match app.inspector.inspection_tab {
        InspectionTab::Summary => draw_summary(f, body_area, app, proc),
        InspectionTab::Env => {
            let data = app.inspector.inspection_data.as_ref();
            draw_env_tab(f, body_area, app, data)
        }
        InspectionTab::Network => {
            let data = app.inspector.inspection_data.as_ref();
            draw_network_tab(f, body_area, app, data)
        }
        InspectionTab::Dlls => {
            let data = app.inspector.inspection_data.as_ref();
            draw_dlls_tab(f, body_area, app, data)
        }
        InspectionTab::Handles => draw_handles_tab(f, body_area, app),
        InspectionTab::Memory => draw_memory_tab(f, body_area, app),
    }
}

// ── Handles Tab（阶段 4，A1） ─────────────────────────────────────────────────

fn draw_handles_tab(f: &mut Frame, area: Rect, app: &App) {
    use crate::inspect::HandleInfo;
    use crate::inspect::HandleKind;

    let query = app.inspector.inspection_search.query();
    let mut handles: Vec<&HandleInfo> = match app.inspector.inspection_handles_data {
        None => Vec::new(),
        Some(ref v) => v
            .iter()
            .filter(|h| {
                if query.is_empty() {
                    return true;
                }
                // 搜类型 / 名称 / 句柄值的 16 进制；name 字段在 Windows v1 上常为空。
                let q = query.to_lowercase();
                h.kind.label().to_lowercase().contains(&q)
                    || h.name.to_lowercase().contains(&q)
                    || format!("{:x}", h.raw_handle).contains(&q)
            })
            .collect(),
    };
    // 按类型分组、组内按句柄值排序，UI 稳定。
    handles.sort_by(|a, b| {
        a.kind
            .label()
            .cmp(b.kind.label())
            .then(a.raw_handle.cmp(&b.raw_handle))
    });

    let rows_visible = area.height.saturating_sub(4) as usize;
    let scroll = app
        .inspector
        .inspection_scroll
        .min(handles.len().saturating_sub(1));

    let title = format!(" 句柄列表 ({} 项) | /=搜索 ↑↓=滚动 ", handles.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if handles.is_empty() {
        let msg = if app.inspector.inspection_handles_data.is_none() {
            "数据采集中… 按 F5 刷新"
        } else if !query.is_empty() {
            "无匹配句柄 — 修改搜索或按 Esc 清空"
        } else {
            "⚠ 此进程当前无可见句柄 — 可能权限不足或进程已退出（按 F5 刷新）"
        };
        let p = Paragraph::new(Span::styled(msg, theme::style_warning())).block(block);
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("类型"),
        Cell::from("名称"),
        Cell::from("句柄"),
        Cell::from("访问"),
    ])
    .style(theme::style_header());

    let rows: Vec<Row> = handles
        .iter()
        .skip(scroll)
        .take(rows_visible)
        .map(|h| {
            let kind_style = match h.kind {
                HandleKind::File => Style::default().fg(Color::Cyan),
                HandleKind::RegistryKey | HandleKind::Directory => {
                    Style::default().fg(Color::Magenta)
                }
                HandleKind::Mutant | HandleKind::Semaphore | HandleKind::Event => {
                    Style::default().fg(Color::Yellow)
                }
                _ => theme::style_normal(),
            };
            let name = if h.name.is_empty() {
                "-".to_string()
            } else {
                h.name.clone()
            };
            let access = if h.granted_access == 0 {
                "-".to_string()
            } else {
                format!("0x{:08X}", h.granted_access)
            };
            Row::new(vec![
                Cell::from(h.kind.label()).style(kind_style),
                Cell::from(name).style(theme::style_normal()),
                Cell::from(format!("0x{:X}", h.raw_handle)).style(theme::style_muted()),
                Cell::from(access).style(theme::style_muted()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Min(30),
            Constraint::Length(18),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(block);

    f.render_widget(table, area);
}

// ── Memory Tab（阶段 4，A3） ─────────────────────────────────────────────────

fn draw_memory_tab(f: &mut Frame, area: Rect, app: &App) {
    use crate::inspect::MemoryRegion;

    let query = app.inspector.inspection_search.query();
    let mut regions: Vec<&MemoryRegion> = match app.inspector.inspection_memory_data {
        None => Vec::new(),
        Some(ref v) => v
            .iter()
            .filter(|r| {
                if query.is_empty() {
                    return true;
                }
                // 搜保护字符串 / 名称；base/size 数字不搜。
                let q = query.to_lowercase();
                r.protection.to_lowercase().contains(&q) || r.name.to_lowercase().contains(&q)
            })
            .collect(),
    };
    // 按基址升序，让相邻 region 视觉上靠在一起。
    regions.sort_by_key(|r| r.base_addr);

    let rows_visible = area.height.saturating_sub(4) as usize;
    let scroll = app
        .inspector
        .inspection_scroll
        .min(regions.len().saturating_sub(1));

    let total_size: u64 = regions.iter().map(|r| r.size).sum();
    let title = format!(
        " 内存映射 ({} 区域 / 共 {}) | /=搜索 ↑↓=滚动 ",
        regions.len(),
        format_bytes(total_size)
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if regions.is_empty() {
        let msg = if app.inspector.inspection_memory_data.is_none() {
            "数据采集中… 按 F5 刷新"
        } else if !query.is_empty() {
            "无匹配区域 — 修改搜索或按 Esc 清空"
        } else {
            "⚠ 此进程当前无可见内存区域 — 可能权限不足或进程已退出（按 F5 刷新）"
        };
        let p = Paragraph::new(Span::styled(msg, theme::style_warning())).block(block);
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![
        Cell::from("基址"),
        Cell::from("大小"),
        Cell::from("状态"),
        Cell::from("保护"),
        Cell::from("映射文件"),
    ])
    .style(theme::style_header());

    let rows: Vec<Row> = regions
        .iter()
        .skip(scroll)
        .take(rows_visible)
        .map(|r| {
            let state_style = match r.state {
                crate::inspect::MemoryState::Commit => Style::default().fg(Color::Green),
                crate::inspect::MemoryState::Reserve => Style::default().fg(Color::Yellow),
                crate::inspect::MemoryState::Free => Style::default().fg(Color::DarkGray),
                _ => theme::style_normal(),
            };
            let name = if r.name.is_empty() {
                "-".to_string()
            } else {
                r.name.clone()
            };
            Row::new(vec![
                Cell::from(format!("0x{:016X}", r.base_addr)).style(theme::style_muted()),
                Cell::from(format_bytes(r.size)).style(theme::style_normal()),
                Cell::from(state_label(r.state)).style(state_style),
                Cell::from(r.protection.clone()).style(theme::style_normal()),
                Cell::from(name).style(theme::style_muted()),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Min(30),
        ],
    )
    .header(header)
    .block(block);

    f.render_widget(table, area);
}

#[must_use]
fn state_label(state: crate::inspect::MemoryState) -> &'static str {
    use crate::inspect::MemoryState;
    match state {
        MemoryState::Commit => "Commit",
        MemoryState::Reserve => "Reserve",
        MemoryState::Free => "Free",
        MemoryState::Private => "Private",
        MemoryState::Shared => "Shared",
        MemoryState::Unknown => "?",
    }
}

// ── Tab 栏 ───────────────────────────────────────────────────────────────────

fn draw_tab_bar(f: &mut Frame, area: Rect, app: &App) {
    let tabs = InspectionTab::all();
    let spans: Vec<Span> = tabs
        .iter()
        .flat_map(|tab| {
            let label = tab.label();
            let style = if *tab == app.inspector.inspection_tab {
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

    let search_hint = if app.inspector.inspection_search.is_active() {
        format!(
            " 搜索: {} | ESC=取消",
            app.inspector.inspection_search.query()
        )
    } else if !app.inspector.inspection_search.query().is_empty() {
        format!(" 过滤: {}", app.inspector.inspection_search.query())
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

    let title = " 进程详情 — Tab/Shift+Tab 切换  F5=刷新  /=搜索 ".to_string();
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

    // 复用 port_panel 后台 worker 维护的 port_entries（每 ~3s 刷新一次），
    // 而不是每帧调 scan_ports()——后者走 netstat2 + sysinfo 全 PID 名表，
    // 详情页打开期间能直接造成卡帧。进程维度按 PID 过滤即可。
    let net_summary =
        port_map::ProcessNetSummary::from_pid(proc.pid, &app.port_panel.panel.port_entries);

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
    ];

    // 阶段 11 P1-A3：从 App::detail_priority 缓存读（进入详情页 / `F5` 刷新 /
    // `+/-` 调整 / heavy tick 4 处更新），避免每帧 4 次 syscall（OpenProcess +
    // GetPriorityClass + GetProcessAffinityMask + CloseHandle）。
    // 缓存 miss（详情页刚打开 heavy tick 未到）时 fallback 实时查，保留正确性。
    let (priority_label, affinity_label) = match &app.inspector.detail_priority {
        Some(p) => (p.0.clone(), p.1.clone()),
        None => {
            let pl = match crate::process_control::get_priority(proc.pid) {
                Ok(class) => class.label().to_string(),
                Err(_) => "-".to_string(),
            };
            let al = match crate::process_control::get_affinity(proc.pid) {
                Ok(mask) => format!("0x{:X} (CPU 数: {})", mask, u64::count_ones(mask)),
                Err(_) => "-".to_string(),
            };
            (pl, al)
        }
    };
    lines.push(Line::from(Span::styled(
        format!("  优先级:   {}  (+/- 调整)", priority_label),
        theme::style_normal(),
    )));
    lines.push(Line::from(Span::styled(
        format!("  Affinity: {}", affinity_label),
        theme::style_muted(),
    )));
    // v0.7 阶段 6：EcoQoS / Efficiency Mode（ADR-0014）。
    // 进程的 throttled 字段由 HeavyWorker 批量 query 填入；T 键切换。
    lines.push(Line::from(Span::styled(
        format!("  EcoQoS:   {}  (T 切换)", proc.throttled.label()),
        if proc.throttled == crate::throttle::EcoQoSState::Eco {
            theme::style_success()
        } else {
            theme::style_normal()
        },
    )));

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
        "  Tab=切Tab  /=搜索  F5=刷新  k=终止  w=监控  y=复制  +/-=优先级  Esc=返回".to_string(),
        theme::style_normal(),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 概要 ")
        .style(theme::style_normal());

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.inspector.inspection_scroll as u16, 0));

    f.render_widget(paragraph, area);
}

// ── Env Tab ──────────────────────────────────────────────────────────────────

fn draw_env_tab(f: &mut Frame, area: Rect, app: &App, data: Option<&InspectionData>) {
    let query = app.inspector.inspection_search.query();
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
    let scroll = app
        .inspector
        .inspection_scroll
        .min(env.len().saturating_sub(1));

    // v0.6.0 阶段 2：录屏中即便 env_reveal=true 也强制 mask（防录到真值）。
    let reveal = app.inspector.env_reveal && !app.is_recording();
    let badge = if reveal {
        "🔓env-reveal"
    } else {
        "🔒env-masked"
    };
    let title = format!(
        " 环境变量 ({} 项) | {badge} | /=搜索 ↑↓=滚动 v=切换 ",
        env.len(),
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if env.is_empty() {
        let msg = if data.is_none() {
            "数据采集中… 按 F5 刷新"
        } else if !query.is_empty() {
            "无匹配项 — 修改搜索或按 Esc 清空"
        } else {
            "⚠ 无环境变量 — 此进程可能已退出或权限不足（按 F5 刷新）"
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
            let value = v.render_value_owned(reveal);
            Row::new(vec![
                Cell::from(v.key.clone()).style(Style::default().fg(theme::accent())),
                Cell::from(value).style(theme::style_normal()),
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
    // 阶段 8 D3：顶部加「最近 5 条 DNS 查询」（按 PID 过滤）。
    // 没有 DNS 数据时（worker 未启动 / 此 PID 未查 DNS）省略，避免占垂直空间。
    //
    // 阶段 11 P1-A5：用 (pid, start_time) 元组过滤，避免 PID 复用时显示
    // 旧进程的 DNS 历史（如 chrome 退出后 PID 被新进程复用，新进程的
    // Network Tab 不应看到 chrome 的 DNS 查询）。
    let pid_key = app
        .inspector
        .detail_process
        .as_ref()
        .map(|p| (p.pid, p.start_time));
    let dns_recent_for_pid: Vec<&crate::dns_log::DnsQuery> = match pid_key {
        Some((pid, start_time)) => app
            .dns_log_recent
            .iter()
            .rev()
            .filter(|q| q.pid == pid && q.start_time == start_time)
            .take(5)
            .collect(),
        None => Vec::new(),
    };

    let inner_area = if dns_recent_for_pid.is_empty() {
        area
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(0)])
            .split(area);
        draw_dns_recent_for_pid(f, chunks[0], &dns_recent_for_pid);
        chunks[1]
    };

    draw_network_connections(f, inner_area, app, data);
}

/// 阶段 8 D3：Network Tab 顶部「最近 5 条 DNS 查询」面板。
fn draw_dns_recent_for_pid(f: &mut Frame, area: Rect, queries: &[&crate::dns_log::DnsQuery]) {
    use crate::dns_log::DnsResult;

    let header = Row::new(vec![
        Cell::from("时间"),
        Cell::from("类型"),
        Cell::from("域名"),
        Cell::from("结果"),
    ])
    .style(theme::style_header());

    let rows: Vec<Row> = queries
        .iter()
        .map(|q| {
            let ts = format_system_time_short(q.timestamp);
            let result_str = match &q.result {
                DnsResult::Success(ips) => {
                    if ips.is_empty() {
                        "OK".to_string()
                    } else {
                        let joined = ips
                            .iter()
                            .map(std::net::IpAddr::to_string)
                            .collect::<Vec<_>>()
                            .join(",");
                        format!("OK:{joined}")
                    }
                }
                DnsResult::NxDomain => "NXDOMAIN".to_string(),
                DnsResult::Timeout => "TIMEOUT".to_string(),
                DnsResult::Error(s) => format!("ERR:{s}"),
            };
            Row::new(vec![
                Cell::from(ts),
                Cell::from(q.query_type.clone()),
                Cell::from(q.query_name.clone()),
                Cell::from(result_str),
            ])
            .style(theme::style_normal())
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Min(20),
            Constraint::Length(30),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" 最近 DNS 查询（仅内存 · 最多 5 条） "),
    );
    f.render_widget(table, area);
}

fn format_system_time_short(t: std::time::SystemTime) -> String {
    let dur = match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return "?".into(),
    };
    let secs = dur.as_secs();
    let offset = crate::local_offset_hours() * 3600;
    let local = (secs as i64 + offset).max(0) as u64;
    let h = ((local % 86_400) / 3600) as u32;
    let m = ((local % 3600) / 60) as u32;
    let s = (local % 60) as u32;
    format!("{h:02}:{m:02}:{s:02}")
}

fn draw_network_connections(f: &mut Frame, area: Rect, app: &App, data: Option<&InspectionData>) {
    // Network Tab 不接搜索 —— 数据量通常很小；用户可走主面板 PortMap 深查。
    let entries: Vec<&crate::port_map::PortEntry> = match data {
        None => Vec::new(),
        Some(d) => d.net.iter().collect(),
    };

    let rows_visible = area.height.saturating_sub(4) as usize;
    let scroll = app
        .inspector
        .inspection_scroll
        .min(entries.len().saturating_sub(1));

    let title = format!(" 网络连接 ({} 项) | ↑↓=滚动 ", entries.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if entries.is_empty() {
        let msg = if data.is_none() {
            "数据采集中… 按 F5 刷新"
        } else {
            "⚠ 此进程当前无监听 / 连接（或权限不足）— 按 F5 刷新"
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
        Cell::from("RTT"),
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
            // RTT:Windows 管理员模式下由 GetPerTcpConnectionEStats 填充;
            // 其它平台 / 非 admin 默认 None → 显示 "-" 避免误读为"零延迟"。
            let rtt_cell = match e.rtt_ms {
                Some(ms) => Cell::from(format!("{}ms", ms)),
                None => Cell::from("-"),
            };
            Row::new(vec![
                Cell::from(format!("[{}]", e.protocol)).style(proto_style),
                Cell::from(local),
                Cell::from(remote),
                Cell::from(e.state.clone().unwrap_or_else(|| "-".to_string())),
                rtt_cell,
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
            Constraint::Min(20),
            Constraint::Min(20),
            Constraint::Length(11),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(block);

    f.render_widget(table, area);
}

// ── Dlls Tab ─────────────────────────────────────────────────────────────────

fn draw_dlls_tab(f: &mut Frame, area: Rect, app: &App, data: Option<&InspectionData>) {
    let query = app.inspector.inspection_search.query();
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
    let scroll = app
        .inspector
        .inspection_scroll
        .min(dlls.len().saturating_sub(1));

    let title = format!(" 模块列表 ({} 项) | /=搜索 ↑↓=滚动 ", dlls.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(theme::style_normal());

    if dlls.is_empty() {
        let msg = if data.is_none() {
            "数据采集中… 按 F5 刷新"
        } else if !query.is_empty() {
            "无匹配模块 — 修改搜索或按 Esc 清空"
        } else {
            "⚠ 无已加载模块 — 此进程可能已退出或权限不足（按 F5 刷新）"
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
