use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;
use crate::classify;
use crate::format::{format_bytes, format_uptime};
use crate::tui::theme;

/// 缩短 GPU 名称，保留关键信息
fn shorten_gpu_name(name: &str) -> String {
    let name = name.replace("(R)", "").replace("(TM)", "");
    let name = name.trim();

    if let Some(rest) = name.strip_prefix("NVIDIA ") {
        if let Some(idx) = rest.find("Laptop") {
            return rest[..idx].trim().to_string();
        }
        if let Some(idx) = rest.find(" with") {
            return rest[..idx].trim().to_string();
        }
        return rest.to_string();
    }

    if name.starts_with("Intel") {
        if let Some(idx) = name.find("Graphics") {
            let before = name[..idx].trim();
            if let Some(short) = before.strip_prefix("Intel ") {
                return short.to_string();
            }
        }
        if let Some(rest) = name.strip_prefix("Intel ") {
            return rest.to_string();
        }
    }

    if name.starts_with("AMD ") {
        return name.strip_prefix("AMD ").unwrap_or(name).to_string();
    }

    if name.len() > 12 {
        name[..12].to_string()
    } else {
        name.to_string()
    }
}

fn make_bar(percent: u32) -> String {
    let filled = (percent as usize) / 10;
    let partial = (percent as usize) % 10;
    let empty = 10 - filled - if partial > 0 { 1 } else { 0 };

    let mut bar = String::new();
    for _ in 0..filled {
        bar.push('\u{2588}');
    }
    if partial > 0 {
        bar.push('\u{2593}');
    }
    for _ in 0..empty {
        bar.push('\u{2591}');
    }
    bar
}

fn temp_span(label: &str, temp: f32) -> Span<'static> {
    let style = if temp >= 90.0 {
        theme::style_danger().add_modifier(Modifier::BOLD)
    } else if temp >= 80.0 {
        theme::style_danger()
    } else if temp >= 70.0 {
        theme::style_warning()
    } else {
        theme::style_success()
    };
    Span::styled(format!("{}:{:.0}\u{00B0}C", label, temp), style)
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .title(" \u{7CFB}\u{7EDF} ")
        .style(theme::style_normal());

    let inner = block.inner(area);
    f.render_widget(block, area);

    let cpu_pct = app.snapshot.cpu_usage() as u32;
    let (mem_used, mem_total) = app.snapshot.memory_usage();
    let mem_pct = if mem_total > 0 {
        (mem_used as f64 / mem_total as f64 * 100.0) as u32
    } else {
        0
    };

    let (swap_used, swap_total) = app.snapshot.swap_usage();
    let swap_pct = if swap_total > 0 {
        (swap_used as f64 / swap_total as f64 * 100.0) as u32
    } else {
        0
    };

    let uptime = format_uptime(crate::collect::SystemSnapshot::uptime_secs());

    let (cpu_temp, _) = app.snapshot.temperatures();
    let gpu_nvml_temp: Option<f32> = app.snapshot.gpu_info().iter().find_map(|g| g.temperature);

    let processes = &app.cached_processes;
    let counts = classify::classify_count(processes);

    let usb_count = app.usb_panel.devices.len();
    let usb_info = if usb_count > 0 {
        format!("{}\u{4E2A}", usb_count)
    } else {
        "\u{65E0}\u{8BBE}\u{5907}".to_string()
    };

    let all_disks = app.snapshot.all_disks();
    let adapters = app.snapshot.net_adapters();
    let tcp_stats = crate::collect::SystemSnapshot::tcp_stats();

    let mut lines: Vec<Line> = Vec::new();

    // CPU / MEM / SWP
    lines.push(Line::from(format!(
        "CPU {} {:>3}%",
        make_bar(cpu_pct.min(100)),
        cpu_pct
    )));
    lines.push(Line::from(format!(
        "MEM {} {:>3}% {}/{}",
        make_bar(mem_pct.min(100)),
        mem_pct,
        format_bytes(mem_used),
        format_bytes(mem_total)
    )));
    lines.push(Line::from(format!(
        "SWP {} {:>3}% {}/{}",
        make_bar(swap_pct.min(100)),
        swap_pct,
        format_bytes(swap_used),
        format_bytes(swap_total)
    )));

    // GPU
    let gpu_info = app.snapshot.gpu_info();
    for gpu in gpu_info {
        let gpu_pct = gpu.utilization_pct.min(100);
        let vram_pct = if gpu.vram_total > 0 {
            (gpu.vram_used as f64 / gpu.vram_total as f64 * 100.0) as u32
        } else {
            0
        };
        let short_name = shorten_gpu_name(&gpu.name);
        let bar_pct = gpu_pct.max(vram_pct);
        lines.push(Line::from(format!(
            "GPU {} {:>3}% {} {}/{}",
            make_bar(bar_pct.min(100)),
            gpu_pct,
            short_name,
            format_bytes(gpu.vram_used),
            format_bytes(gpu.vram_total)
        )));
    }

    // Temperature line with color-coded Spans
    match (cpu_temp, gpu_nvml_temp) {
        (Some(c), Some(g)) => {
            lines.push(Line::from(vec![
                temp_span("CPU", c),
                Span::raw(" "),
                temp_span("GPU", g),
            ]));
        }
        (Some(c), None) => {
            lines.push(Line::from(vec![temp_span("CPU", c)]));
        }
        (None, Some(g)) => {
            lines.push(Line::from(vec![temp_span("GPU", g)]));
        }
        (None, None) => {}
    }

    // Throttle status line
    if let Some(ref ti) = app.throttle_info {
        let reason = app.throttle_reason;
        match reason {
            crate::throttle::ThrottleReason::Thermal => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("CPU {}/{}MHz", ti.current_mhz, ti.max_mhz),
                        theme::style_danger(),
                    ),
                    Span::styled(
                        " \u{26A0}THERMAL",
                        theme::style_danger().add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            crate::throttle::ThrottleReason::Unknown
            | crate::throttle::ThrottleReason::PowerPolicy => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("CPU {}/{}MHz", ti.current_mhz, ti.max_mhz),
                        theme::style_warning(),
                    ),
                    Span::styled(
                        " \u{26A0}POWER",
                        theme::style_warning().add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            crate::throttle::ThrottleReason::Idle => {
                lines.push(Line::from(vec![Span::styled(
                    format!("CPU {}/{}MHz", ti.current_mhz, ti.max_mhz),
                    theme::style_muted(),
                )]));
            }
            crate::throttle::ThrottleReason::None => {
                lines.push(Line::from(vec![Span::styled(
                    format!("CPU {}/{}MHz", ti.current_mhz, ti.max_mhz),
                    theme::style_muted(),
                )]));
            }
        }
    }

    lines.push(Line::from(""));

    // Disks
    for disk in &all_disks {
        let pct = if disk.total > 0 {
            (disk.used as f64 / disk.total as f64 * 100.0) as u32
        } else {
            0
        };
        let short_mount = disk.mount_point.trim_end_matches('\\');
        let label = if short_mount.is_empty() {
            &disk.name
        } else {
            short_mount
        };
        let removable_tag = if disk.is_removable {
            " (\u{53EF}\u{79FB}\u{52A8})"
        } else {
            ""
        };
        lines.push(Line::from(format!(
            "DSK {} {:>3}% {}{} {}/{}",
            make_bar(pct.min(100)),
            pct,
            label,
            removable_tag,
            format_bytes(disk.used),
            format_bytes(disk.total)
        )));
    }

    let (disk_read, disk_write) = app.snapshot.disk_io_speed();
    lines.push(Line::from(format!(
        "I/O R:{}/s W:{}/s",
        format_bytes(disk_read),
        format_bytes(disk_write)
    )));

    // Per-disk I/O speeds
    for disk_io in app.snapshot.per_disk_io_speed() {
        let drive_letter = disk_io.mount_point.chars().next().unwrap_or('?');
        lines.push(Line::from(format!(
            "{}: R:{}/s W:{}/s",
            drive_letter,
            format_bytes(disk_io.read_speed),
            format_bytes(disk_io.write_speed)
        )));
    }

    lines.push(Line::from(""));

    // Network
    lines.push(Line::from(format!(
        "NET \u{2193}{}/s \u{603B}{}",
        format_bytes(app.snapshot.net_down_speed),
        format_bytes(app.snapshot.net_total_rx)
    )));
    lines.push(Line::from(format!(
        "    \u{2191}{}/s \u{603B}{}",
        format_bytes(app.snapshot.net_up_speed),
        format_bytes(app.snapshot.net_total_tx)
    )));

    for adapter in &adapters {
        if let Some(ipv4) = &adapter.ipv4 {
            lines.push(Line::from(format!("{} {}", adapter.name, ipv4)));
        }
    }

    lines.push(Line::from(format!(
        "TCP {} EST / {} TW / {} CW",
        tcp_stats.established, tcp_stats.time_wait, tcp_stats.close_wait
    )));

    lines.push(Line::from(""));

    // Process counts and uptime
    lines.push(Line::from(format!(
        "\u{8FDB}\u{7A0B}: {} ({}/{}/{})",
        processes.len(),
        counts.user,
        counts.system,
        counts.service
    )));
    lines.push(Line::from(format!("\u{8FD0}\u{884C} {}", uptime)));
    lines.push(Line::from(format!("U\u{76D8}: {}", usb_info)));

    let paragraph = Paragraph::new(lines).style(theme::style_normal());
    f.render_widget(paragraph, inner);

    // Alert badge at bottom of sidebar
    let (_, warning, critical) = app.alert_manager.firing_counts();
    if warning > 0 || critical > 0 {
        let badge_area = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(1),
            width: area.width,
            height: 1,
        };
        crate::tui::alert_badge::draw_alert_badge(f, badge_area, app);
    }
}
