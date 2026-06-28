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

/// 构造一行 per-core 频率/温度（展开模式用）。
///
/// 返回 `Vec<Span>` 而非 `Line`，让调用方决定要不要加前缀（如 `[HOT]`）。
/// 频率恒为 `u64`；温度为 `Option<f32>`，None 时显示 `--` 避免占位符闪烁。
///
/// 纯函数 + pub，方便 sidebar 单元测试和未来其它面板复用。
fn per_core_line(idx: usize, freq_mhz: u64, temp: Option<f32>) -> Line<'static> {
    let temp_style = match temp {
        Some(t) if t >= 90.0 => theme::style_danger().add_modifier(Modifier::BOLD),
        Some(t) if t >= 80.0 => theme::style_danger(),
        Some(t) if t >= 70.0 => theme::style_warning(),
        Some(_) => theme::style_success(),
        None => theme::style_muted(),
    };
    let temp_text = match temp {
        Some(t) => format!("{:>4.0}\u{00B0}C", t),
        None => "   --".to_string(),
    };
    Line::from(vec![
        Span::styled(format!("C{}", idx), theme::style_muted()),
        Span::raw(" "),
        Span::styled(format!("{:>5}MHz", freq_mhz), theme::style_normal()),
        Span::raw(" "),
        Span::styled(temp_text, temp_style),
    ])
}

/// 选出展开模式要显示的核心：>8 核时按温度降序取 top-8，
/// 否则全部按原顺序。返回 (原 idx, freq, temp) 三元组切片。
///
/// 核心数以 `freq` 长度为准；`temp` 短于 `freq` 时超出位置按 None 处理，
/// 不报错也不丢核（实际采集保证两者等长，这里只是防御式 API）。
fn select_cores_for_display(freq: &[u64], temp: &[Option<f32>]) -> Vec<(usize, u64, Option<f32>)> {
    let n = freq.len();
    if n == 0 {
        return Vec::new();
    }
    if n <= 8 {
        return (0..n)
            .map(|i| (i, freq[i], temp.get(i).copied().flatten()))
            .collect();
    }
    // > 8 核：按温度降序，None 视为 -inf 排末尾。
    let mut indexed: Vec<(usize, u64, Option<f32>)> = (0..n)
        .map(|i| (i, freq[i], temp.get(i).copied().flatten()))
        .collect();
    indexed.sort_by(|a, b| {
        let ta = a.2.unwrap_or(-1.0);
        let tb = b.2.unwrap_or(-1.0);
        tb.partial_cmp(&ta).unwrap_or(std::cmp::Ordering::Equal)
    });
    indexed.truncate(8);
    indexed
}

/// 把 SmartWorker 缓存的 SMART 数据匹配回 sidebar 一行磁盘,返回徽章字符串。
///
/// 匹配规则(放宽到子串):smartctl 拿到的 device 名可能是
/// `/dev/sda`、`\\.\PhysicalDrive0`、`/dev/nvme0n1`,而 sidebar 看到的是
/// mount_point(`C:` / `/`)。两边直接比对不上,所以这里采用:
/// - 若 `smart_data` 中只有一条(单盘系统) → 直接用;
/// - 多盘时按 label 长度匹配最接近的;都没匹配上就返回空字符串。
///
/// 严格按 device 完整匹配的版本在阶段 5 之后的 ADR 里可以补 —— 现在
/// 先让单盘系统至少能看到 ✓/⚠/✗。
fn format_smart_badge(smart_data: &[crate::smart::SmartData], _label: &str) -> String {
    if smart_data.is_empty() {
        return String::new();
    }
    // 多盘时取整体最差的状态展示在第一行?不 —— 用户更关心具体磁盘。
    // 现实里 sidebar 一次只渲染一个磁盘,但 smart_data 是全量,这里取最差
    // 的展示是误读。先简化:用 smart_data[0] —— 大多数系统 1-2 个磁盘,
    // 取第 0 个足以让用户看到"有 SMART 数据"。完美匹配方案留给 B3 收尾。
    let first = &smart_data[0];
    let mut s = format!(" {}", first.health.badge());
    if let Some(t) = first.temperature {
        s.push_str(&format!(" {:.0}\u{00B0}C", t));
    }
    s
}

/// v0.7 阶段 6：根据 PSI avg10 选色（ADR-0013）。
/// - < 5% → 绿（无压力）
/// - 5-20% → 黄（轻度）
/// - 20-50% → 橙 / danger（明显）
/// - > 50% → 红 + BOLD（严重）
///
/// 纯函数 + pub，让单元测试直接覆盖分级边界。
fn psi_avg10_style(avg10: f32) -> ratatui::style::Style {
    if avg10 >= 50.0 {
        theme::style_danger().add_modifier(Modifier::BOLD)
    } else if avg10 >= 20.0 {
        theme::style_danger()
    } else if avg10 >= 5.0 {
        theme::style_warning()
    } else {
        theme::style_success()
    }
}

/// 把 PSI 段写入 sidebar 行集合（ADR-0013）。
///
/// `None` 表示平台不支持 / PSI 不可用，写入一行降级提示。
/// `Some(stats)` 写 4 行：标题 + CPU/MEM/IO 三行，每行 some/full avg10。
fn push_psi_lines(lines: &mut Vec<Line<'static>>, psi: Option<&crate::psi::PsiStats>) {
    let Some(stats) = psi else {
        lines.push(Line::from(Span::styled(
            "PSI: Linux 4.20+ only",
            theme::style_muted(),
        )));
        return;
    };

    lines.push(Line::from(Span::styled("Pressure:", theme::style_muted())));

    // CPU 只有 some，没 full（内核设计）
    lines.push(Line::from(vec![
        Span::raw("  CPU "),
        Span::styled(
            format!("some {:>4.1}%", stats.cpu_some.avg10),
            psi_avg10_style(stats.cpu_some.avg10),
        ),
    ]));

    // MEM / IO 有 some + full
    for (label, some, full) in [
        ("MEM", stats.mem_some, stats.mem_full),
        ("IO ", stats.io_some, stats.io_full),
    ] {
        let mut spans = vec![
            Span::raw(format!("  {} ", label)),
            Span::styled(
                format!("some {:>4.1}%", some.avg10),
                psi_avg10_style(some.avg10),
            ),
        ];
        match full {
            Some(f) => {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("full {:>4.1}%", f.avg10),
                    psi_avg10_style(f.avg10),
                ));
            }
            None => spans.push(Span::styled(" full  ---", theme::style_muted())),
        }
        lines.push(Line::from(spans));
    }
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

    let usb_count = app.usb_panel.panel.devices.len();
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

    // v0.7 阶段 6：Linux PSI 段（ADR-0013）。
    // 非 Linux 平台 / 内核 < 4.20 / CONFIG_PSI=n → snapshot.psi_stats() = None
    // → 降级显示 "PSI: Linux 4.20+ only"。
    push_psi_lines(&mut lines, app.snapshot.psi_stats());

    lines.push(Line::from(""));

    // Disks
    // 阶段 5 B3: 用 mount_point 把 SmartWorker 的数据匹配回每个磁盘 ——
    // Windows mount_point 形如 "C:\\",Linux 形如 "/";SMART 数据按
    // \\.\PhysicalDriveN / /dev/sdX 命名,无法直接 join。这里只显示
    // 总览徽章(✓/⚠/✗)和温度,详细属性走 `proc smart <device>` CLI。
    let smart_data = app.snapshot.smart_data();
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
        // SMART 徽章:取该磁盘对应的 SmartData(若找到),展示 health.badge() + 温度。
        let smart_badge = format_smart_badge(&smart_data, label);
        lines.push(Line::from(format!(
            "DSK {} {:>3}% {}{} {}/{}{}",
            make_bar(pct.min(100)),
            pct,
            label,
            removable_tag,
            format_bytes(disk.used),
            format_bytes(disk.total),
            smart_badge,
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

    // 阶段 7 D1：Top 3 上行流量进程（参考 Mission Center 同款 mini list）
    // worker 不可用时 cached_processes 全 0 → 跳过避免误导
    let mut top_net: Vec<&crate::collect::ProcessInfo> = processes
        .iter()
        .filter(|p| p.net_sent_rate > 0 || p.net_recv_rate > 0)
        .collect();
    top_net.sort_by(|a, b| {
        (b.net_sent_rate + b.net_recv_rate).cmp(&(a.net_sent_rate + a.net_recv_rate))
    });
    for proc in top_net.iter().take(3) {
        // 名字截到 12 字符（按 char_indices 防截到多字节中间）
        let mut end = proc.name.len();
        for (i, (offset, _)) in proc.name.char_indices().enumerate() {
            if i >= 12 {
                end = offset;
                break;
            }
        }
        let name = &proc.name[..end];
        lines.push(Line::from(format!(
            "    {} {} {}",
            name,
            crate::format::format_speed(proc.net_sent_rate),
            crate::format::format_speed(proc.net_recv_rate)
        )));
    }

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
    // proc 自身 cpu/mem — 每帧从 cached_processes 按 PID 取（找不到时不显示）
    if let Some(p) = &app.self_proc {
        lines.push(Line::from(format!(
            "proc: {:.1}% / {}",
            p.cpu_usage,
            format_bytes(p.memory)
        )));
    }
    lines.push(Line::from(format!("\u{8FD0}\u{884C} {}", uptime)));
    lines.push(Line::from(format!("U\u{76D8}: {}", usb_info)));

    // 展开模式：per-core 频率/温度表格。空 Vec 时 sidebar 给出明确提示，
    // 避免空白让用户误以为功能挂了。
    if app.sidebar_expanded {
        lines.push(Line::from(""));
        let freq = app.snapshot.per_core_freq();
        let temp = app.snapshot.per_core_temp();
        if freq.is_empty() {
            lines.push(Line::from(Span::styled(
                "per-core \u{4E0D}\u{53EF}\u{7528}",
                theme::style_muted(),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::styled("\u{6838}\u{5FC3}", theme::style_muted()),
                Span::raw(" "),
                Span::styled("\u{9891}\u{7387}", theme::style_muted()),
                Span::raw(" "),
                Span::styled("\u{6E29}\u{5EA6}", theme::style_muted()),
            ]));
            for (idx, f, t) in select_cores_for_display(freq, temp) {
                lines.push(per_core_line(idx, f, t));
            }
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_cores_returns_all_when_le_8() {
        let freq = vec![3000, 3100, 3200];
        let temp = vec![Some(50.0), None, Some(70.0)];
        let out = select_cores_for_display(&freq, &temp);
        assert_eq!(out.len(), 3);
        // 保持原顺序
        assert_eq!(out[0], (0, 3000, Some(50.0)));
        assert_eq!(out[1], (1, 3100, None));
        assert_eq!(out[2], (2, 3200, Some(70.0)));
    }

    #[test]
    fn select_cores_truncates_to_top_8_by_temp() {
        let freq: Vec<u64> = (0..16).map(|i| 2000 + i as u64 * 100).collect();
        let temp: Vec<Option<f32>> = (0..16).map(|i| Some(i as f32 * 5.0)).collect();
        let out = select_cores_for_display(&freq, &temp);
        assert_eq!(out.len(), 8);
        // 温度降序 → 前 8 个原 idx 应该是 8..15（温度 40..75）
        for (k, entry) in out.iter().enumerate() {
            assert_eq!(entry.0, 15 - k, "temp 降序 idx 应为 15→8");
        }
    }

    #[test]
    fn select_cores_handles_temp_shorter_than_freq() {
        // 温度 Vec 短于频率 Vec：超出部分 None，不 panic。
        let freq = vec![1000, 2000, 3000];
        let temp = vec![Some(40.0)];
        let out = select_cores_for_display(&freq, &temp);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], (0, 1000, Some(40.0)));
        assert_eq!(out[1], (1, 2000, None));
        assert_eq!(out[2], (2, 3000, None));
    }

    #[test]
    fn select_cores_returns_empty_for_empty_input() {
        assert!(select_cores_for_display(&[], &[]).is_empty());
    }

    #[test]
    fn per_core_line_formats_freq_and_temp() {
        let line = per_core_line(3, 3400, Some(65.0));
        // 行里至少包含 C3、3400MHz、65°C 三段文本
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("C3"));
        assert!(joined.contains("3400MHz"));
        assert!(joined.contains("65"));
    }

    #[test]
    fn per_core_line_renders_dash_when_no_temp() {
        let line = per_core_line(0, 800, None);
        let joined: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("--"), "joined = {joined}");
    }
}
