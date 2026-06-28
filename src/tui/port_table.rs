use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState};

use crate::anomaly::AnomalySeverity;
use crate::app::App;
use crate::format::format_bytes;
use crate::port_map::{self, NetworkViewMode, Protocol, RemoteGroup};
use crate::tui::theme;

#[derive(Debug, Clone)]
struct DisplayRow {
    is_separator: bool,
    separator_text: String,
    protocol: Option<Protocol>,
    local_addr: String,
    remote_addr: String,
    state: String,
    pid: String,
    process_name: String,
}

fn draw_net_traffic_bar(f: &mut Frame, area: Rect, app: &App) {
    let adapters = app.snapshot.net_adapters();

    if adapters.is_empty() {
        let line = Line::from(vec![
            Span::styled(" 网卡: ", theme::style_muted()),
            Span::styled("未检测到活跃网络接口", theme::style_warning()),
        ]);
        let p = Paragraph::new(line);
        f.render_widget(p, area);
        return;
    }

    let adapter_name = adapters.first().map(|a| a.name.as_str()).unwrap_or("未知");

    let sparkline_str = build_sparkline(&app.port_panel.panel.connection_history);

    // 阶段 5 D2: TCP 传输质量片段。retrans/rst 都是累计值,展示时给百分比
    // 让数字可读。分母用 out_segs(累计输出段数);out_segs=0 时表示该平台
    // 没拿到 SNMP 数据(非 Linux / 非 Windows),显示 "-" 而非 "0%"。
    let tcp_stats = crate::collect::SystemSnapshot::tcp_stats();
    let quality_spans: Vec<Span> = if tcp_stats.out_segs > 0 {
        let retrans_rate = tcp_stats.retransmitted_segs as f64 / tcp_stats.out_segs as f64 * 100.0;
        let rst_rate = tcp_stats.reset_segs as f64 / tcp_stats.out_segs as f64 * 100.0;
        let retrans_style = if retrans_rate > 5.0 {
            theme::style_danger()
        } else if retrans_rate > 1.0 {
            theme::style_warning()
        } else {
            Style::default().fg(theme::success())
        };
        let rst_style = if rst_rate > 2.0 {
            theme::style_danger()
        } else if rst_rate > 0.5 {
            theme::style_warning()
        } else {
            Style::default().fg(theme::success())
        };
        vec![
            Span::styled("  重传 ", theme::style_muted()),
            Span::styled(format!("{:.1}%", retrans_rate), retrans_style),
            Span::styled(" RST ", theme::style_muted()),
            Span::styled(format!("{:.1}%", rst_rate), rst_style),
            Span::styled(" 失败 ", theme::style_muted()),
            Span::styled(
                format!("{}", tcp_stats.failed_connections),
                theme::style_header(),
            ),
        ]
    } else {
        vec![
            Span::styled("  重传 ", theme::style_muted()),
            Span::styled("-", theme::style_muted()),
            Span::styled(" RST ", theme::style_muted()),
            Span::styled("-", theme::style_muted()),
        ]
    };

    let mut spans = vec![
        Span::styled(format!(" {} ", adapter_name), theme::style_header()),
        Span::styled(" ▼ ", theme::style_muted()),
        Span::styled(
            format!("{}/s", format_bytes(app.snapshot.net_down_speed)),
            Style::default().fg(theme::success()),
        ),
        Span::styled(" 总 ", theme::style_muted()),
        Span::styled(
            format_bytes(app.snapshot.net_total_rx),
            theme::style_header(),
        ),
        Span::styled("  ▲ ", theme::style_muted()),
        Span::styled(
            format!("{}/s", format_bytes(app.snapshot.net_up_speed)),
            Style::default().fg(theme::warning()),
        ),
        Span::styled(" 总 ", theme::style_muted()),
        Span::styled(
            format_bytes(app.snapshot.net_total_tx),
            theme::style_header(),
        ),
        Span::styled(format!("  连接: {}", sparkline_str), theme::style_muted()),
        Span::styled(
            format!(" {}", app.port_panel.panel.connection_diff.active_count),
            theme::style_header(),
        ),
    ];
    spans.extend(quality_spans);
    let p = Paragraph::new(Line::from(spans));
    f.render_widget(p, area);
}

fn split_with_traffic_bar(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    (chunks[0], chunks[1])
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    // 阶段 8 D3：DNS 子视图优先（按 D 进入，覆盖常规端口视图）。
    if app.port_panel.panel.dns_view_active {
        draw_dns_view(f, area, app);
        return;
    }

    // v0.7 阶段 8：Flow 子视图（按 F 进入，覆盖常规端口视图）。ADR-0016。
    if app.port_panel.panel.flow_view_active {
        draw_flow_view(f, area, app);
        return;
    }

    if let Some(ref detail) = app.port_panel.panel.port_detail {
        draw_port_detail(f, area, detail);
    } else if app.port_panel.panel.show_anomaly_detail {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(12)])
            .split(area);

        draw_main_view(f, chunks[0], app);
        draw_anomaly_panel(f, chunks[1], app);
    } else {
        draw_main_view(f, area, app);
    }

    if app.port_panel.panel.show_diagnostics
        && let Some(ref diag) = app.port_panel.panel.diagnostic
    {
        match diag.phase {
            crate::diag::DiagnosticPhase::Menu => {
                draw_diagnostic_menu(f, area, app);
            }
            crate::diag::DiagnosticPhase::Running
            | crate::diag::DiagnosticPhase::Completed
            | crate::diag::DiagnosticPhase::Failed => {
                draw_diagnostic_result(f, area, app);
            }
        }
    }
}

/// 阶段 8 D3 DNS 子视图：列出 `App::dns_log_recent`（最近 1000 条查询）。
/// 列：时间 / PID / 进程名 / 类型 / 域名 / 结果。`/` 搜索过滤；`f` follow；
/// `c` 清空；`Esc`/`D` 退出。
fn draw_dns_view(f: &mut Frame, area: Rect, app: &App) {
    use crate::dns_log::DnsResult;

    let pp = &app.port_panel.panel;
    let recent = &app.dns_log_recent;

    // 标题栏：显示 collector 状态 + 条数 + 操作提示
    let header_line = if app.workers.dns_log_worker.is_some() {
        format!(
            " DNS 查询日志（仅内存 · {} 条 · {}）  D/Esc 退出 · / 搜索 · f follow · c 清空",
            recent.len(),
            if pp.dns_follow { "跟随" } else { "暂停" }
        )
    } else {
        " DNS 查询日志：此平台暂不支持（Windows 走 PowerShell Get-WinEvent，Linux/macOS 见 ADR-0006）".to_string()
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new(header_line).style(theme::style_header());
    f.render_widget(header, chunks[0]);

    // 搜索栏（激活时显示）
    let (table_area, search_area) = if pp.dns_search.is_active() {
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(chunks[1]);
        (inner[0], Some(inner[1]))
    } else {
        (chunks[1], None)
    };

    // 过滤后的索引（搜索 query 命中域名 / 进程名）
    let visible_indices = pp.dns_filtered_indices(recent);

    let header_row = Row::new(vec![
        Cell::from("时间"),
        Cell::from("PID"),
        Cell::from("进程名"),
        Cell::from("类型"),
        Cell::from("域名"),
        Cell::from("结果"),
    ])
    .style(theme::style_header());

    let rows_visible = table_area.height.saturating_sub(3) as usize;
    let scroll = pp.dns_scroll;
    let window: Vec<usize> = visible_indices
        .iter()
        .skip(scroll)
        .take(rows_visible)
        .copied()
        .collect();

    let rows: Vec<Row> = window
        .iter()
        .map(|&idx| {
            let q = &recent[idx];
            let ts = format_system_time(q.timestamp);
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
            let result_style = match &q.result {
                DnsResult::Success(_) => Style::default().fg(Color::Green),
                DnsResult::NxDomain | DnsResult::Error(_) => Style::default().fg(Color::Yellow),
                DnsResult::Timeout => Style::default().fg(Color::LightRed),
            };
            Row::new(vec![
                Cell::from(ts),
                Cell::from(q.pid.to_string()),
                Cell::from(q.process_name.clone()),
                Cell::from(q.query_type.clone()),
                Cell::from(q.query_name.clone()),
                Cell::from(result_str).style(result_style),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(18),
            Constraint::Length(6),
            Constraint::Min(20),
            Constraint::Length(30),
        ],
    )
    .header(header_row)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("DNS 查询日志（仅内存 · 隐私不持久化）"),
    );
    f.render_widget(table, table_area);

    if let Some(area) = search_area {
        let s = Paragraph::new(format!(" 搜索: {}", pp.dns_search.query()))
            .style(Style::default().fg(Color::Cyan));
        f.render_widget(s, area);
    }
}

/// 把 SystemTime 格式化成 `HH:MM:SS`（本地时区，复用 lib::local_offset_hours）。
fn format_system_time(t: std::time::SystemTime) -> String {
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

/// v0.7 阶段 8：Flow 子视图（ADR-0016）。列出 `App::flows`（最近一份
/// FlowAggregator drain 出的快照 + v0.10 阶段 3 Schannel SNI overlay）。
/// 列：PID / comm / 远端 / 端口 / 域名 / 时间。
///
/// - Linux + ebpf feature：source = Ebpf 路径（connect + DNS 关联，完整字段）。
/// - Windows admin：source = Schannel 路径（SNI 明文，远端/端口空 + JA4 空）。
/// - ebpf + schannel 都不在线：显示降级提示。
/// - flows 为空但 worker 已启用：显示「尚无 flow」。
/// - `bytes_out / bytes_in` MVP 留 0（Part B 接 tcp_sendmsg/recvmsg），不渲染。
fn draw_flow_view(f: &mut Frame, area: Rect, app: &App) {
    let pp = &app.port_panel.panel;
    let flows = &app.flows;

    let ebpf_enabled = crate::ebpf::EBPF_ENABLED;
    let schannel_on = app.workers.schannel_etw_worker.is_some();

    // 标题栏：worker 状态 + 条数 + 操作提示。v0.10 阶段 3：跨平台对齐
    // （ebpf / schannel 都有 → 显示数据来源 mix；单一来源 → 显示对应路径）。
    let ghost_count = flows.iter().filter(|f| f.is_ghost()).count();
    let header_line = if ebpf_enabled && app.workers.ebpf_worker.is_some() {
        if ghost_count > 0 {
            format!(
                " eBPF Flow graph（{} 条 · 👻{} 幽灵保留 ≤30s · connect + DNS 关联）  F/Esc 退出 · ↑↓滚动",
                flows.len(),
                ghost_count
            )
        } else {
            format!(
                " eBPF Flow graph（{} 条 · connect + DNS 关联）  F/Esc 退出 · ↑↓滚动",
                flows.len()
            )
        }
    } else if schannel_on {
        // v0.10 阶段 3：Windows admin 走 Schannel 路径（source = Schannel）。
        format!(
            " Schannel Flow graph（{} 条 · SNI 明文 · TLS handshake）  F/Esc 退出 · ↑↓滚动",
            flows.len()
        )
    } else if ebpf_enabled {
        " eBPF Flow graph：worker 启动失败（无权限？内核 < 5.10？），详见日志".to_string()
    } else {
        " Flow graph：需要 Linux + ebpf feature 或 Windows 管理员（Schannel ETW）".to_string()
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header = Paragraph::new(header_line).style(theme::style_header());
    f.render_widget(header, chunks[0]);

    let header_row = Row::new(vec![
        Cell::from("PID"),
        Cell::from("进程名"),
        Cell::from("远端"),
        Cell::from("端口"),
        Cell::from("SNI/域名"),
        Cell::from("首次见到"),
    ])
    .style(theme::style_header());

    let rows_visible = chunks[1].height.saturating_sub(3) as usize;
    let scroll = pp.flow_scroll;
    let window: Vec<&crate::ebpf::flow::ProcessFlow> =
        flows.iter().skip(scroll).take(rows_visible).collect();

    let rows: Vec<Row> = window
        .iter()
        .map(|flow| {
            // v0.10 阶段 3：SNI 优先（Schannel 路径 / ebpf 路径 sni 填上时），
            // 回退到 dns_name（ebpf 路径 DNS 关联命中），都没有显示 —。
            let name_str = flow
                .sni
                .clone()
                .or_else(|| flow.dns_name.clone())
                .unwrap_or_else(|| "—".into());
            let comm = if flow.comm.is_empty() {
                "?".to_string()
            } else {
                flow.comm.clone()
            };
            // Part B 任务 9：ghost flow（进程已退出，仍在 30s 保留窗口内）
            // 加 👻 前缀 + 灰色斜体渲染，区别于 live flow。
            let is_ghost = flow.is_ghost();
            let pid_str = if is_ghost {
                format!("👻{}", flow.pid)
            } else {
                flow.pid.to_string()
            };
            let row_style = if is_ghost {
                theme::style_muted().add_modifier(Modifier::ITALIC)
            } else {
                Style::default()
            };
            // v0.10 阶段 3：Schannel 路径 remote_addr 留空（Schannel event 不给
            // socket 元数据），显示 — 保持表格视觉对齐；remote_port = 0 时也显示 —。
            let remote_addr_cell = if flow.remote_addr.is_empty() {
                "—".to_string()
            } else {
                flow.remote_addr.clone()
            };
            let remote_port_cell = if flow.remote_port == 0 {
                "—".to_string()
            } else {
                flow.remote_port.to_string()
            };
            Row::new(vec![
                Cell::from(pid_str),
                Cell::from(comm),
                Cell::from(remote_addr_cell),
                Cell::from(remote_port_cell),
                Cell::from(name_str),
                Cell::from(format_system_time(flow.first_seen)),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(18),
            Constraint::Length(16),
            Constraint::Length(6),
            Constraint::Min(20),
            Constraint::Length(10),
        ],
    )
    .header(header_row)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("ProcessFlow（SNI/域名 · 隐私不持久化）"),
    );
    f.render_widget(table, chunks[1]);
}

fn draw_main_view(f: &mut Frame, area: Rect, app: &App) {
    match app.port_panel.panel.port_view_mode {
        NetworkViewMode::Process => {
            draw_process_view(f, area, app);
            return;
        }
        NetworkViewMode::Remote => {
            draw_remote_view(f, area, app);
            return;
        }
        NetworkViewMode::Port => {}
    }

    draw_port_view(f, area, app);
}

fn draw_port_view(f: &mut Frame, area: Rect, app: &App) {
    let (bar_area, table_area) = split_with_traffic_bar(area);
    draw_net_traffic_bar(f, bar_area, app);

    let mut entries = app.filtered_ports().to_vec();
    let rows_visible = table_area.height.saturating_sub(3) as usize;

    // Deduplicate IPv4/IPv6 entries
    let mut seen: Vec<(u16, u32, String)> = Vec::new();
    entries.retain(|e| {
        let key = (e.local_port, e.pid, e.state.clone().unwrap_or_default());
        if port_map::is_ipv6_duplicate(e, &seen) {
            return false;
        }
        seen.push(key);
        true
    });

    // Count by group
    let mut group_counts: [usize; 4] = [0, 0, 0, 0];
    for e in &entries {
        let g = port_map::state_group(&e.state, &e.protocol) as usize;
        group_counts[g] += 1;
    }

    let tcp_count = entries
        .iter()
        .filter(|e| e.protocol == Protocol::Tcp)
        .count();
    let udp_count = entries
        .iter()
        .filter(|e| e.protocol == Protocol::Udp)
        .count();

    // Build display rows with separators
    let mut display_rows: Vec<DisplayRow> = Vec::new();
    let group_labels = [
        ("建立连接", group_counts[0]),
        ("监听端口", group_counts[1]),
        ("其他TCP", group_counts[2]),
        ("UDP", group_counts[3]),
    ];

    let mut last_group: Option<u8> = None;
    for e in &entries {
        let g = port_map::state_group(&e.state, &e.protocol);
        if last_group != Some(g) {
            let (label, count) = group_labels[g as usize];
            if count > 0 {
                display_rows.push(DisplayRow {
                    is_separator: true,
                    separator_text: format!("─── {} ({}) ───", label, count),
                    protocol: None,
                    local_addr: String::new(),
                    remote_addr: String::new(),
                    state: String::new(),
                    pid: String::new(),
                    process_name: String::new(),
                });
            }
            last_group = Some(g);
        }

        let local = format!(
            "{}:{}",
            e.local_addr,
            port_map::format_port_service(e.local_port, e.protocol)
        );
        let remote = match (e.remote_addr, e.remote_port) {
            (Some(addr), Some(port)) => {
                let class = port_map::classify_ip(&addr);
                let label = port_map::ip_class_label(class);
                format!("→ {}:{} [{}]", addr, port, label)
            }
            _ => "  -".to_string(),
        };
        let state_str = e.state.as_deref().unwrap_or("-");

        display_rows.push(DisplayRow {
            is_separator: false,
            separator_text: String::new(),
            protocol: Some(e.protocol),
            local_addr: local,
            remote_addr: remote,
            state: state_str.to_string(),
            pid: e.pid.to_string(),
            process_name: e.process_name.clone(),
        });
    }

    // Map port_cursor to display row index (skip separators for cursor logic)
    let data_indices: Vec<usize> = display_rows
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.is_separator)
        .map(|(i, _)| i)
        .collect();

    let visible_rows: Vec<&DisplayRow> = display_rows
        .iter()
        .skip(app.port_panel.panel.port_scroll)
        .take(rows_visible)
        .collect();

    let header = Row::new(vec![
        Cell::from("协议"),
        Cell::from("本地地址"),
        Cell::from("远程地址"),
        Cell::from("状态"),
        Cell::from("PID"),
        Cell::from("进程名"),
    ])
    .style(theme::style_header());

    let scroll_offset = app.port_panel.panel.port_scroll;
    let rows: Vec<Row> = visible_rows
        .iter()
        .enumerate()
        .map(|(vi, row)| {
            if row.is_separator {
                return Row::new(vec![
                    Cell::from(row.separator_text.clone()),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(""),
                ])
                .style(theme::style_muted());
            }

            let global_display_i = scroll_offset + vi;
            let is_cursor = data_indices
                .iter()
                .position(|&di| di == global_display_i)
                .map(|pos| pos == app.port_panel.panel.port_cursor)
                .unwrap_or(false);

            let bg = if is_cursor {
                Color::DarkGray
            } else {
                Color::Reset
            };

            let proto_style = match row.protocol {
                Some(Protocol::Tcp) => Style::default().fg(Color::Cyan),
                Some(Protocol::Udp) => Style::default().fg(Color::Green),
                None => Style::default(),
            };

            Row::new(vec![
                Cell::from(row.protocol.map(|p| format!("[{}]", p)).unwrap_or_default())
                    .style(proto_style),
                Cell::from(row.local_addr.clone()),
                Cell::from(row.remote_addr.clone()),
                Cell::from(row.state.clone()),
                Cell::from(row.pid.clone()),
                Cell::from(row.process_name.clone()),
            ])
            .style(Style::default().fg(theme::text_primary()).bg(bg))
        })
        .collect();

    let filter_label = app.port_panel.panel.port_state_filter.label();
    let sort_label = app.port_panel.panel.port_sort_field.label();

    let search_indicator = if app.port_panel.panel.port_search.is_active() {
        format!(
            " 搜索: {} | ESC取消",
            app.port_panel.panel.port_search.query()
        )
    } else if !app.port_panel.panel.port_search.query().is_empty() {
        format!(" 过滤: {}", app.port_panel.panel.port_search.query())
    } else {
        String::new()
    };

    let mode_label = if app.port_panel.panel.port_is_admin {
        "增强模式 ✓"
    } else {
        "基础模式"
    };
    let diff = &app.port_panel.panel.connection_diff;
    let anomaly_part = anomaly_indicator(app).unwrap_or_default();
    let title = format!(
        "端口映射 | {} | TCP:{} UDP:{} | ⬆+{} ⬇-{} 活跃{}{} | 过滤:[{}] 排序:{} f切换 s排序{}",
        mode_label,
        tcp_count,
        udp_count,
        diff.new_count,
        diff.closed_count,
        diff.active_count,
        anomaly_part,
        filter_label,
        sort_label,
        search_indicator
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .style(theme::style_normal());

    let widths = [
        ratatui::layout::Constraint::Length(5),
        ratatui::layout::Constraint::Min(22),
        ratatui::layout::Constraint::Min(22),
        ratatui::layout::Constraint::Length(14),
        ratatui::layout::Constraint::Length(7),
        ratatui::layout::Constraint::Min(12),
    ];

    let table = Table::new(rows, widths).header(header).block(block);

    let mut state = TableState::default();
    let visible_cursor = app
        .port_panel
        .panel
        .port_cursor
        .saturating_sub(app.port_panel.panel.port_scroll);
    state.select(Some(visible_cursor));
    f.render_stateful_widget(table, table_area, &mut state);
}

fn draw_port_detail(f: &mut Frame, area: Rect, entry: &crate::port_map::PortEntry) {
    let service_info = port_map::service_name(entry.local_port, entry.protocol)
        .map(|name| format!(" ({})", name))
        .unwrap_or_default();

    let remote_display = match (entry.remote_addr, entry.remote_port) {
        (Some(addr), Some(port)) => {
            let class = port_map::classify_ip(&addr);
            let label = port_map::ip_class_label(class);
            format!("{}:{} [{}]", addr, port, label)
        }
        _ => "-".to_string(),
    };

    let lines = vec![
        format!(
            "端口详情 — {}:{} ({}){}",
            entry.local_addr, entry.local_port, entry.protocol, service_info
        ),
        String::new(),
        format!("  协议:     {}", entry.protocol),
        format!(
            "  本地地址: {}:{}{}",
            entry.local_addr, entry.local_port, service_info
        ),
        format!("  远程地址: {}", remote_display),
        format!("  状态:     {}", entry.state.as_deref().unwrap_or("-")),
        String::new(),
        format!("  进程 PID: {}", entry.pid),
        format!("  进程名:   {}", entry.process_name),
        String::new(),
        "  k=终止占用进程  Enter/Esc=返回".to_string(),
    ];

    let text: ratatui::text::Text = lines
        .into_iter()
        .map(|line| {
            ratatui::text::Line::from(ratatui::text::Span::styled(line, theme::style_normal()))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 端口详情 ")
        .style(theme::style_normal());

    let paragraph = ratatui::widgets::Paragraph::new(text)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    f.render_widget(paragraph, area);
}

fn draw_remote_view(f: &mut Frame, area: Rect, app: &App) {
    let (bar_area, table_area) = split_with_traffic_bar(area);
    draw_net_traffic_bar(f, bar_area, app);

    let groups = app.filtered_remote_groups().to_vec();
    let rows_visible = table_area.height.saturating_sub(3) as usize;

    if groups.is_empty() {
        let text = ratatui::text::Text::from("无匹配远程IP");
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" 按远程视图 | g=切换 ")
            .style(theme::style_normal());
        let p = ratatui::widgets::Paragraph::new(text).block(block);
        f.render_widget(p, table_area);
        return;
    }

    let cursor = app
        .port_panel
        .panel
        .port_remote_cursor
        .min(groups.len() - 1);
    let scroll = app
        .port_panel
        .panel
        .port_remote_scroll
        .min(groups.len().saturating_sub(1));
    let visible: Vec<&RemoteGroup> = groups.iter().skip(scroll).take(rows_visible).collect();

    let header = Row::new(vec![
        Cell::from("远程IP"),
        Cell::from("归属"),
        Cell::from("连接数"),
        Cell::from("进程"),
        Cell::from("协议"),
        Cell::from("状态"),
    ])
    .style(theme::style_header());

    let rows: Vec<Row> = visible
        .iter()
        .enumerate()
        .map(|(vi, group)| {
            let gi = scroll + vi;
            let is_cursor = gi == cursor;
            let bg = if is_cursor {
                Color::DarkGray
            } else {
                Color::Reset
            };

            let addr_str = group.remote_addr.to_string();

            let owner_label = if let Some(ref cp) = group.cloud_provider {
                format!("\u{2601} {}", cp)
            } else {
                port_map::ip_class_label(group.ip_class).to_string()
            };
            let owner_color = if group.cloud_provider.is_some() {
                Color::Blue
            } else {
                theme::text_primary()
            };

            let conn_str = format!(
                "{} /{} /{} /{}",
                group.established, group.listening, group.time_wait, group.close_wait
            );

            let mut proc_names: Vec<&str> =
                group.process_names.iter().map(|s| s.as_str()).collect();
            proc_names.sort();
            let proc_display = if proc_names.len() <= 3 {
                proc_names.join(",")
            } else {
                format!("{},...+{}", proc_names[..3].join(","), proc_names.len() - 3)
            };

            let proto_display = {
                let mut parts = Vec::new();
                if group.protocols.contains(&Protocol::Tcp) {
                    parts.push("TCP");
                }
                if group.protocols.contains(&Protocol::Udp) {
                    parts.push("UDP");
                }
                parts.join("/")
            };

            let status_parts = {
                let mut s = String::new();
                if group.established > 0 {
                    s.push_str(&format!("EST:{} ", group.established));
                }
                if group.listening > 0 {
                    s.push_str(&format!("LIST:{} ", group.listening));
                }
                if group.time_wait > 0 {
                    s.push_str(&format!("TW:{} ", group.time_wait));
                }
                if group.close_wait > 0 {
                    s.push_str(&format!("CW:{}", group.close_wait));
                }
                s.trim_end().to_string()
            };

            let est_color = if group.established > 10 {
                Color::Yellow
            } else {
                theme::text_primary()
            };
            let cw_color = if group.close_wait > 0 {
                Color::Red
            } else {
                theme::text_primary()
            };

            Row::new(vec![
                Cell::from(addr_str),
                Cell::from(owner_label).style(Style::default().fg(owner_color)),
                Cell::from(conn_str).style(Style::default().fg(est_color)),
                Cell::from(truncate_str(&proc_display, 24)),
                Cell::from(proto_display),
                Cell::from(status_parts).style(Style::default().fg(cw_color)),
            ])
            .style(Style::default().fg(theme::text_primary()).bg(bg))
        })
        .collect();

    let private_count = groups
        .iter()
        .filter(|g| g.ip_class == port_map::IpClass::Private)
        .count();
    let public_count = groups
        .iter()
        .filter(|g| g.ip_class == port_map::IpClass::Public)
        .count();

    let filter_label = app.port_panel.panel.port_state_filter.label();
    let sort_label = app.port_panel.panel.port_remote_sort.label();

    let search_indicator = if app.port_panel.panel.port_search.is_active() {
        format!(
            " 搜索: {} | ESC取消",
            app.port_panel.panel.port_search.query()
        )
    } else if !app.port_panel.panel.port_search.query().is_empty() {
        format!(" 过滤: {}", app.port_panel.panel.port_search.query())
    } else {
        String::new()
    };

    let anomaly_part = anomaly_indicator(app).unwrap_or_default();
    let title = format!(
        "按远程视图 | {}个远程IP | 内网:{} 外网:{}{} | 过滤:[{}] 排序:{}{}",
        groups.len(),
        private_count,
        public_count,
        anomaly_part,
        filter_label,
        sort_label,
        search_indicator
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .style(theme::style_normal());

    let widths = vec![
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(18),
        Constraint::Min(16),
        Constraint::Length(8),
        Constraint::Min(14),
    ];

    let table = Table::new(rows, widths).header(header).block(block);

    let selected_visual = cursor.saturating_sub(scroll);
    let mut state = TableState::default();
    state.select(Some(selected_visual));
    f.render_stateful_widget(table, table_area, &mut state);
}

fn draw_process_view(f: &mut Frame, area: Rect, app: &App) {
    let (bar_area, table_area) = split_with_traffic_bar(area);
    draw_net_traffic_bar(f, bar_area, app);

    let groups = app.filtered_process_groups().to_vec();
    let rows_visible = table_area.height.saturating_sub(3) as usize;

    let is_enhanced =
        app.port_panel.panel.port_is_admin && app.port_panel.panel.estats_collector.is_some();

    if groups.is_empty() {
        let mode_label = if is_enhanced {
            "增强模式 ✓"
        } else {
            "基础模式"
        };
        let text = ratatui::text::Text::from("无匹配进程");
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" 按进程视图 | {} | g=切换 ", mode_label))
            .style(theme::style_normal());
        let p = ratatui::widgets::Paragraph::new(text).block(block);
        f.render_widget(p, table_area);
        return;
    }

    let cursor = app
        .port_panel
        .panel
        .port_process_cursor
        .min(groups.len() - 1);
    let expanded_pid = app.port_panel.panel.port_expanded_pid;

    let mut group_visual_pos: Vec<usize> = Vec::with_capacity(groups.len());
    let mut all_rows: Vec<(usize, Option<usize>)> = Vec::new();
    let mut vi = 0;
    for (gi, group) in groups.iter().enumerate() {
        group_visual_pos.push(vi);
        all_rows.push((gi, None));
        vi += 1;
        if expanded_pid == Some(group.pid) {
            for ci in 0..group.connections.len() {
                all_rows.push((gi, Some(ci)));
                vi += 1;
            }
        }
    }

    let scroll = app
        .port_panel
        .panel
        .port_process_scroll
        .min(all_rows.len().saturating_sub(1));
    let visible: Vec<&(usize, Option<usize>)> =
        all_rows.iter().skip(scroll).take(rows_visible).collect();

    let header = if is_enhanced {
        Row::new(vec![
            Cell::from("进程"),
            Cell::from("PID"),
            Cell::from("TCP"),
            Cell::from("UDP"),
            Cell::from("EST"),
            Cell::from("LISTEN"),
            Cell::from("TW"),
            Cell::from("CW"),
            Cell::from("远程IP"),
            Cell::from("▲发送"),
            Cell::from("▼接收"),
            Cell::from("累计"),
        ])
        .style(theme::style_header())
    } else {
        Row::new(vec![
            Cell::from("进程"),
            Cell::from("PID"),
            Cell::from("TCP"),
            Cell::from("UDP"),
            Cell::from("EST"),
            Cell::from("LISTEN"),
            Cell::from("TW"),
            Cell::from("CW"),
            Cell::from("远程IP"),
        ])
        .style(theme::style_header())
    };

    let rows: Vec<Row> = visible
        .iter()
        .map(|(gi, conn_idx)| {
            let group = &groups[*gi];

            match conn_idx {
                None => {
                    let is_cursor = *gi == cursor;
                    let bg = if is_cursor {
                        Color::DarkGray
                    } else {
                        Color::Reset
                    };

                    let est_color = if group.established > 50 {
                        theme::warning()
                    } else {
                        theme::text_primary()
                    };
                    let tw_color = if group.time_wait > 20 {
                        theme::warning()
                    } else {
                        theme::text_primary()
                    };
                    let cw_color = if group.close_wait > 5 {
                        theme::danger()
                    } else {
                        theme::text_primary()
                    };

                    let name_str = truncate_str(&group.process_name, 20);

                    if is_enhanced {
                        let up_style = bandwidth_style(group.up_speed);
                        let down_style = bandwidth_style(group.down_speed);
                        let total_str = format_total_bandwidth(group.total_down, group.total_up);

                        Row::new(vec![
                            Cell::from(name_str),
                            Cell::from(group.pid.to_string()),
                            Cell::from(group.tcp_count.to_string()),
                            Cell::from(group.udp_count.to_string()),
                            Cell::from(group.established.to_string())
                                .style(Style::default().fg(est_color)),
                            Cell::from(group.listening.to_string()),
                            Cell::from(group.time_wait.to_string())
                                .style(Style::default().fg(tw_color)),
                            Cell::from(group.close_wait.to_string())
                                .style(Style::default().fg(cw_color)),
                            Cell::from(group.unique_remote_addrs.len().to_string()),
                            Cell::from(crate::format::format_speed(group.up_speed))
                                .style(Style::default().fg(up_style)),
                            Cell::from(crate::format::format_speed(group.down_speed))
                                .style(Style::default().fg(down_style)),
                            Cell::from(total_str),
                        ])
                        .style(Style::default().fg(theme::text_primary()).bg(bg))
                    } else {
                        Row::new(vec![
                            Cell::from(name_str),
                            Cell::from(group.pid.to_string()),
                            Cell::from(group.tcp_count.to_string()),
                            Cell::from(group.udp_count.to_string()),
                            Cell::from(group.established.to_string())
                                .style(Style::default().fg(est_color)),
                            Cell::from(group.listening.to_string()),
                            Cell::from(group.time_wait.to_string())
                                .style(Style::default().fg(tw_color)),
                            Cell::from(group.close_wait.to_string())
                                .style(Style::default().fg(cw_color)),
                            Cell::from(group.unique_remote_addrs.len().to_string()),
                        ])
                        .style(Style::default().fg(theme::text_primary()).bg(bg))
                    }
                }
                Some(ci) => {
                    let conn = &group.connections[*ci];
                    let total = group.connections.len();
                    let prefix = if *ci < total - 1 {
                        "  ├→"
                    } else {
                        "  └→"
                    };

                    let remote = match (conn.remote_addr, conn.remote_port) {
                        (Some(addr), Some(port)) => {
                            let class = port_map::classify_ip(&addr);
                            let label = port_map::ip_class_label(class);
                            format!("{}:{} [{}]", addr, port, label)
                        }
                        _ => "-".to_string(),
                    };
                    let state_str = conn.state.as_deref().unwrap_or("-");
                    let proto_style = if conn.protocol == Protocol::Tcp {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::Green)
                    };

                    if is_enhanced {
                        Row::new(vec![
                            Cell::from(format!("{} {}", prefix, remote)),
                            Cell::from(""),
                            Cell::from(format!("[{}]", conn.protocol)).style(proto_style),
                            Cell::from(""),
                            Cell::from(state_str.to_string()),
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                        ])
                        .style(theme::style_muted())
                    } else {
                        Row::new(vec![
                            Cell::from(format!("{} {}", prefix, remote)),
                            Cell::from(""),
                            Cell::from(format!("[{}]", conn.protocol)).style(proto_style),
                            Cell::from(""),
                            Cell::from(state_str.to_string()),
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                            Cell::from(""),
                        ])
                        .style(theme::style_muted())
                    }
                }
            }
        })
        .collect();

    let mode_label = if is_enhanced {
        "增强模式 ✓"
    } else {
        "基础模式"
    };
    let filter_label = app.port_panel.panel.port_state_filter.label();
    let sort_label = app.port_panel.panel.port_process_sort.label();

    let search_indicator = if app.port_panel.panel.port_search.is_active() {
        format!(
            " 搜索: {} | ESC取消",
            app.port_panel.panel.port_search.query()
        )
    } else if !app.port_panel.panel.port_search.query().is_empty() {
        format!(" 过滤: {}", app.port_panel.panel.port_search.query())
    } else {
        String::new()
    };

    let anomaly_part = anomaly_indicator(app).unwrap_or_default();
    let title = format!(
        "按进程视图 | {} | {}个进程{} | 过滤:[{}] 排序:{}{}",
        mode_label,
        groups.len(),
        anomaly_part,
        filter_label,
        sort_label,
        search_indicator
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .style(theme::style_normal());

    let widths = if is_enhanced {
        vec![
            Constraint::Min(18),
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(6),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(9),
        ]
    } else {
        vec![
            Constraint::Min(20),
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(8),
        ]
    };

    let table = Table::new(rows, widths).header(header).block(block);

    let cursor_visual = group_visual_pos.get(cursor).copied().unwrap_or(0);
    let selected_visual = cursor_visual.saturating_sub(scroll);

    let mut state = TableState::default();
    state.select(Some(selected_visual));
    f.render_stateful_widget(table, table_area, &mut state);
}

/// 按显示宽度截断字符串（汉字宽度按 2 计算），超出加 `…` 后缀。
fn truncate_str(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let total_width = unicode_width::UnicodeWidthStr::width(s);
    if total_width <= max_width {
        return s.to_string();
    }
    // 截断情形：留 1 列给 …
    let budget = max_width.saturating_sub(1);
    let mut out = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + w > budget {
            break;
        }
        out.push(ch);
        width += w;
    }
    out.push('…');
    out
}

#[must_use]
pub fn draw_placeholder(_area: Rect) -> String {
    "端口映射（开发中）".to_string()
}

const KB: u64 = 1024;
const MB: u64 = KB * 1024;

fn bandwidth_style(bytes_per_sec: u64) -> Color {
    if bytes_per_sec >= MB {
        Color::Green
    } else if bytes_per_sec >= 100 * KB {
        Color::Yellow
    } else {
        theme::text_primary()
    }
}

fn format_total_bandwidth(total_down: u64, total_up: u64) -> String {
    let total = total_down + total_up;
    if total == 0 {
        "-".to_string()
    } else {
        format_bytes(total)
    }
}

fn build_sparkline(history: &std::collections::VecDeque<usize>) -> String {
    const CHARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if history.is_empty() {
        return String::new();
    }
    let max = *history.iter().max().unwrap_or(&1).max(&1) as f64;
    history
        .iter()
        .map(|&v| {
            let idx = ((v as f64 / max) * (CHARS.len() - 1) as f64).round() as usize;
            CHARS[idx.min(CHARS.len() - 1)]
        })
        .collect()
}

fn anomaly_indicator(app: &App) -> Option<String> {
    let count = app.anomaly_count();
    if count == 0 {
        return None;
    }
    let visible = app.visible_anomalies();
    let max_severity = visible.iter().map(|a| a.severity).max();
    match max_severity {
        Some(AnomalySeverity::Critical) => Some(format!(" 🔴{}", count)),
        Some(AnomalySeverity::Warning) => Some(format!(" ⚠{}", count)),
        Some(AnomalySeverity::Info) => Some(format!(" ℹ{}", count)),
        None => None,
    }
}

fn draw_anomaly_panel(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);

    let visible = app.visible_anomalies();
    let cursor = app
        .port_panel
        .panel
        .anomaly_cursor
        .min(visible.len().saturating_sub(1));

    let severity_icon = |s: AnomalySeverity| -> (&'static str, Color) {
        match s {
            AnomalySeverity::Critical => ("🔴", Color::Red),
            AnomalySeverity::Warning => ("⚠", Color::Yellow),
            AnomalySeverity::Info => ("ℹ", Color::Cyan),
        }
    };

    let lines: Vec<Line> = if visible.is_empty() {
        vec![Line::from(Span::styled(
            "  无活跃异常 ✅",
            theme::style_muted(),
        ))]
    } else {
        visible
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                let (icon, color) = severity_icon(a.severity);
                let is_selected = i == cursor;
                let style = if is_selected {
                    Style::default()
                        .fg(color)
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(color)
                };
                let detail_style = if is_selected {
                    Style::default()
                        .fg(theme::text_primary())
                        .bg(Color::DarkGray)
                } else {
                    theme::style_muted()
                };

                let mut lines = vec![Line::from(vec![
                    Span::styled(format!("  {} ", icon), style),
                    Span::styled(&a.title, style),
                ])];
                if is_selected {
                    lines.push(Line::from(Span::styled(
                        format!("    {}", a.detail),
                        detail_style,
                    )));
                }
                lines
            })
            .collect()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " 异常详情 ({}条) | d=忽略 a/Esc=关闭 ",
            visible.len()
        ))
        .style(Style::default().fg(theme::text_primary()));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    f.render_widget(paragraph, area);
}

fn draw_diagnostic_menu(f: &mut Frame, area: Rect, app: &App) {
    let popup_area = crate::tui::centered_rect(60, 14, area);
    f.render_widget(Clear, popup_area);

    let Some(ref diag) = app.port_panel.panel.diagnostic else {
        return;
    };
    let tools = crate::diag::DiagnosticState::tool_list();
    let is_private = crate::diag::is_private_or_loopback(&diag.target_ip);

    let title = format!(" 网络诊断: {} ", diag.target_ip);

    let mut lines: Vec<Line> = Vec::new();
    for (i, tool) in tools.iter().enumerate() {
        let name = crate::diag::DiagnosticState::tool_name(tool);
        let unavailable =
            is_private && crate::diag::DiagnosticState::tool_unavailable_for_private(tool);

        let cursor = if i == diag.tool_index { " > " } else { "   " };

        if unavailable {
            lines.push(Line::from(vec![
                Span::styled(cursor, Style::default().fg(theme::accent())),
                Span::styled(
                    format!("{} (内网不可用)", name),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        } else if i == diag.tool_index {
            lines.push(Line::from(vec![
                Span::styled(cursor, Style::default().fg(theme::accent())),
                Span::styled(
                    name.to_string(),
                    Style::default()
                        .fg(theme::accent())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(cursor, Style::default()),
                Span::styled(name.to_string(), Style::default().fg(theme::text_primary())),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Enter=执行  Esc=关闭",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(theme::text_primary()));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, popup_area);
}

fn style_diagnostic_line(line: &str, tool_index: usize) -> Line<'static> {
    match tool_index {
        // Ping
        0 => {
            let lower = line.to_lowercase();
            if lower.contains("时间=") || lower.contains("time=") || lower.contains("time<") {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Green),
                ))
            } else if lower.contains("请求超时")
                || lower.contains("timed out")
                || lower.contains("丢失")
                || lower.contains("lost")
            {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Red),
                ))
            } else if lower.contains("packets")
                || lower.contains("数据包")
                || lower.contains("平均")
                || lower.contains("average")
                || lower.contains("minimum")
                || lower.contains("maximum")
                || lower.contains("最小")
                || lower.contains("最大")
            {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Yellow),
                ))
            } else {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(theme::text_primary()),
                ))
            }
        }
        // Port scan
        4 => {
            if line.contains("开放") {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Green),
                ))
            } else if line.contains("关闭") {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::DarkGray),
                ))
            } else if line.contains("扫描完成") {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(Color::Yellow),
                ))
            } else {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default().fg(theme::text_primary()),
                ))
            }
        }
        // Other tools (DNS, Whois, Traceroute)
        _ => Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(theme::text_primary()),
        )),
    }
}

fn draw_diagnostic_result(f: &mut Frame, area: Rect, app: &App) {
    let popup_area = crate::tui::centered_rect(70, 20, area);
    f.render_widget(Clear, popup_area);

    let Some(ref diag) = app.port_panel.panel.diagnostic else {
        return;
    };
    let tools = crate::diag::DiagnosticState::tool_list();
    let tool_name = tools
        .get(diag.tool_index)
        .map(|t| crate::diag::DiagnosticState::tool_name(t))
        .unwrap_or("未知");

    let status = match diag.phase {
        crate::diag::DiagnosticPhase::Running => "运行中...",
        crate::diag::DiagnosticPhase::Completed => "完成",
        crate::diag::DiagnosticPhase::Failed => "失败",
        crate::diag::DiagnosticPhase::Menu => unreachable!(),
    };

    let title = format!(" {}: {} | {} ", tool_name, diag.target_ip, status);

    let content_height = popup_area.height.saturating_sub(4);
    let max_scroll = diag.content.len().saturating_sub(content_height as usize) as u16;
    let scroll = diag.scroll.min(max_scroll);

    let lines: Vec<Line> = diag
        .content
        .iter()
        .map(|line| style_diagnostic_line(line, diag.tool_index))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Style::default().fg(theme::text_primary()));

    let paragraph = Paragraph::new(lines).block(block).scroll((scroll, 0));

    f.render_widget(paragraph, popup_area);

    let footer = Line::from(Span::styled(
        " ↑↓滚动  Enter=重新选择  Esc=关闭",
        Style::default().fg(Color::DarkGray),
    ));
    let footer_area = Rect::new(
        popup_area.x + 1,
        popup_area.y + popup_area.height.saturating_sub(1),
        popup_area.width.saturating_sub(2),
        1,
    );
    f.render_widget(Paragraph::new(footer), footer_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 中文路径不再按"字符数"而是按"显示宽度"截断 —— 汉字宽度按 2。
    #[test]
    fn truncate_str_respects_wide_chars() {
        // 6 个汉字 = 12 列宽，max_width=10 应截断为 4 个汉字 + …（4*2 + 1 = 9 列）
        assert_eq!(truncate_str("汉字测试路径", 10), "汉字测试…");
        // max_width=12 时整串恰好放得下，不加 …
        assert_eq!(truncate_str("汉字测试路径", 12), "汉字测试路径");
        // ASCII + 中文混合
        assert_eq!(truncate_str("abc汉字", 7), "abc汉字");
        assert_eq!(truncate_str("abc汉字", 6), "abc汉…");
        // 边界
        assert_eq!(truncate_str("short", 24), "short");
        assert_eq!(truncate_str("abcdefghijklmnopqrstuvwxyz", 5), "abcd…");
    }
}
