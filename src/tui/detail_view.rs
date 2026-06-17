use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::App;
use crate::classify;
use crate::port_map;
use crate::tui::security_badge;
use crate::tui::theme;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let proc = match &app.detail_process {
        Some(p) => p,
        None => return,
    };

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

    // Security analysis section
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
        "  k=终止  w=添加监控  c=复制  Enter/Esc=返回".to_string(),
        theme::style_normal(),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 进程详情 ")
        .style(theme::style_normal());

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}

use crate::format::format_bytes;

#[must_use]
pub fn draw_placeholder(_area: Rect) -> String {
    "进程详情（开发中）".to_string()
}
