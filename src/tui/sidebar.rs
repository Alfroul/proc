use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;
use crate::classify;
use crate::format::{format_bytes, format_uptime};
use crate::tui::theme;

/// 缩短 GPU 名称，保留关键信息
fn shorten_gpu_name(name: &str) -> String {
    // "NVIDIA GeForce RTX 4050 Laptop GPU" → "RTX 4050"
    // "Intel(R) UHD Graphics" → "UHD"
    // "AMD Radeon RX 7900 XTX" → "RX 7900X"
    let name = name.replace("(R)", "").replace("(TM)", "");
    let name = name.trim();

    // NVIDIA: extract "RTX 30xx" / "GTX 16xx" / "RTX 40xx" etc.
    if let Some(rest) = name.strip_prefix("NVIDIA ") {
        if let Some(idx) = rest.find("Laptop") {
            return rest[..idx].trim().to_string();
        }
        if let Some(idx) = rest.find(" with") {
            return rest[..idx].trim().to_string();
        }
        return rest.to_string();
    }

    // Intel: "UHD Graphics" → "UHD", "Iris Xe" → "Iris Xe"
    if name.starts_with("Intel") {
        if let Some(idx) = name.find("Graphics") {
            let before = name[..idx].trim();
            // "Intel UHD" → "UHD"
            if let Some(short) = before.strip_prefix("Intel ") {
                return short.to_string();
            }
        }
        if let Some(rest) = name.strip_prefix("Intel ") {
            return rest.to_string();
        }
    }

    // AMD: keep as-is if short enough
    if name.starts_with("AMD ") {
        return name.strip_prefix("AMD ").unwrap_or(name).to_string();
    }

    // Fallback: truncate to 12 chars
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
        bar.push('█');
    }
    if partial > 0 {
        bar.push('▓');
    }
    for _ in 0..empty {
        bar.push('░');
    }
    bar
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .title(" 系统 ")
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
    let gpu_nvml_temp: Option<f32> = app.snapshot.gpu_info().iter()
        .find_map(|g| g.temperature);

    let temp_line = match (cpu_temp, gpu_nvml_temp) {
        (Some(c), Some(g)) => format!("CPU:{:.0}°C GPU:{:.0}°C", c, g),
        (Some(c), None) => format!("CPU:{:.0}°C", c),
        (None, Some(g)) => format!("GPU:{:.0}°C", g),
        (None, None) => String::new(),
    };

    let processes = &app.cached_processes;
    let counts = classify::classify_count(&processes);

    let usb_count = app.usb_devices.len();
    let usb_info = if usb_count > 0 {
        format!("{}个", usb_count)
    } else {
        "无设备".to_string()
    };

    let all_disks = app.snapshot.all_disks();
    let adapters = app.snapshot.net_adapters();
    let tcp_stats = crate::collect::SystemSnapshot::tcp_stats();

    let mut content = String::new();

    content.push_str(&format!("CPU {} {:>3}%\n", make_bar(cpu_pct.min(100)), cpu_pct));
    content.push_str(&format!("MEM {} {:>3}% {}/{}\n",
        make_bar(mem_pct.min(100)), mem_pct,
        format_bytes(mem_used), format_bytes(mem_total)));
    content.push_str(&format!("SWP {} {:>3}% {}/{}\n",
        make_bar(swap_pct.min(100)), swap_pct,
        format_bytes(swap_used), format_bytes(swap_total)));

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
        content.push_str(&format!("GPU {} {:>3}% {} {}/{}\n",
            make_bar(bar_pct.min(100)), gpu_pct,
            short_name,
            format_bytes(gpu.vram_used), format_bytes(gpu.vram_total)));
    }

    if !temp_line.is_empty() {
        content.push_str(&temp_line);
        content.push('\n');
    }

    content.push('\n');

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
        let removable_tag = if disk.is_removable { " (可移动)" } else { "" };
        content.push_str(&format!("DSK {} {:>3}% {}{} {}/{}\n",
            make_bar(pct.min(100)), pct,
            label, removable_tag,
            format_bytes(disk.used), format_bytes(disk.total)));
    }

    let (disk_read, disk_write) = app.snapshot.disk_io_speed(processes);
    content.push_str(&format!("I/O R:{}/s W:{}/s\n",
        format_bytes(disk_read), format_bytes(disk_write)));

    content.push('\n');

    content.push_str(&format!("NET ↓{}/s 总{}\n",
        format_bytes(app.snapshot.net_down_speed),
        format_bytes(app.snapshot.net_total_rx)));
    content.push_str(&format!("    ↑{}/s 总{}\n",
        format_bytes(app.snapshot.net_up_speed),
        format_bytes(app.snapshot.net_total_tx)));

    for adapter in &adapters {
        if let Some(ipv4) = &adapter.ipv4 {
            content.push_str(&format!("{} {}\n", adapter.name, ipv4));
        }
    }

    content.push_str(&format!("TCP {} EST / {} TW / {} CW\n",
        tcp_stats.established, tcp_stats.time_wait, tcp_stats.close_wait));

    content.push('\n');

    content.push_str(&format!("进程: {} ({}/{}/{})\n",
        processes.len(), counts.user, counts.system, counts.service));
    content.push_str(&format!("运行 {}\n", uptime));
    content.push_str(&format!("U盘: {}\n", usb_info));

    let paragraph = Paragraph::new(content).style(theme::style_normal());
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
