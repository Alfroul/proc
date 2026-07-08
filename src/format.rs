/// Format bytes as human-readable string (e.g., "1.5GB", "200MB", "50KB", "128B")
///
/// v0.17 stage 3 TD-44：B 档（bytes < 1024）走 itoa 路径跳过 std `format!` 抽象
/// （省 1 次 heap alloc + 1 次 fmt 调度）。MB / KB / GB 档仍走 f64 `{:.1}` / `{:.0}`
/// 路径（itoa 不处理 f64）。
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0}KB", bytes as f64 / KB as f64)
    } else {
        let mut buf = itoa::Buffer::new();
        format!("{}B", buf.format(bytes))
    }
}

#[must_use]
pub fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let mins = (seconds % 3600) / 60;
    let mut d_buf = itoa::Buffer::new();
    let mut h_buf = itoa::Buffer::new();
    let mut m_buf = itoa::Buffer::new();
    if days > 0 {
        format!("{}天{}小时", d_buf.format(days), h_buf.format(hours))
    } else if hours > 0 {
        format!("{}小时{}分", h_buf.format(hours), m_buf.format(mins))
    } else {
        format!("{}分钟", m_buf.format(mins))
    }
}

/// Format throughput as a 1-decimal-place string with SI units (decimal, not binary).
/// Mirrors the convention used in port/process tables: 1.5GB/s, 200MB/s, 50KB/s, 128B/s.
///
/// v0.17 stage 3 TD-44：B/s 档（bytes_per_sec < 1000）走 itoa 路径，其它档保留
/// f64 `{:.1}` 路径。
#[must_use]
pub fn format_speed(bytes_per_sec: u64) -> String {
    const UNITS: &[(&str, u64)] = &[
        ("GB/s", 1_000_000_000),
        ("MB/s", 1_000_000),
        ("KB/s", 1_000),
    ];
    for (unit, threshold) in UNITS {
        if bytes_per_sec >= *threshold {
            return format!("{:.1}{}", bytes_per_sec as f64 / *threshold as f64, unit);
        }
    }
    let mut buf = itoa::Buffer::new();
    format!("{}B/s", buf.format(bytes_per_sec))
}

#[must_use]
pub fn format_run_time(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let mins = (seconds % 3600) / 60;
    let secs = seconds % 60;
    let mut buf_d = itoa::Buffer::new();
    let mut buf_h = itoa::Buffer::new();
    let mut buf_m = itoa::Buffer::new();
    let mut buf_s = itoa::Buffer::new();
    if days > 0 {
        format!("{}d{}h", buf_d.format(days), buf_h.format(hours))
    } else if hours > 0 {
        format!("{}h{}m", buf_h.format(hours), buf_m.format(mins))
    } else if mins > 0 {
        format!("{}m{}s", buf_m.format(mins), buf_s.format(secs))
    } else {
        format!("{}s", buf_s.format(secs))
    }
}

// ── snapshot export ──

/// ISO-8601 timestamp with local offset (e.g. "2026-06-13T10:23:45+08:00").
/// Uses `local_offset_hours` + `epoch_secs_to_ymd` so we don't pull in chrono.
#[must_use]
pub fn local_iso_timestamp(epoch_secs: u64) -> String {
    let offset_hours = crate::local_offset_hours();
    let local_secs = (epoch_secs as i64 + offset_hours * 3600).max(0) as u64;
    let (y, m, d) = crate::epoch_secs_to_ymd(local_secs);
    let h = (local_secs / 3600) % 24;
    let min = (local_secs / 60) % 60;
    let s = local_secs % 60;
    let sign = if offset_hours >= 0 { '+' } else { '-' };
    let off = offset_hours.unsigned_abs();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:00",
        y, m, d, h, min, s, sign, off
    )
}

fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        field.to_string()
    }
}

/// Export a sorted slice of processes as JSON (curated fields, ISO-8601 timestamp).
#[must_use]
pub fn export_processes_as_json(procs: &[crate::collect::ProcessInfo], epoch_secs: u64) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!(
        "  \"timestamp\": \"{}\",\n",
        local_iso_timestamp(epoch_secs)
    ));
    out.push_str(&format!("  \"total\": {},\n", procs.len()));
    out.push_str("  \"processes\": [\n");
    for (i, p) in procs.iter().enumerate() {
        let exe = p.exe.as_deref().unwrap_or("");
        out.push_str("    {");
        out.push_str(&format!("\"pid\": {}, ", p.pid));
        out.push_str(&format!("\"name\": \"{}\", ", json_escape_string(&p.name)));
        out.push_str(&format!("\"cpu_usage\": {}, ", json_float(p.cpu_usage)));
        out.push_str(&format!("\"memory_bytes\": {}, ", p.memory));
        out.push_str(&format!("\"exe\": \"{}\"", json_escape_string(exe)));
        out.push_str(if i + 1 == procs.len() {
            " }\n"
        } else {
            " },\n"
        });
    }
    out.push_str("  ]\n");
    out.push('}');
    out
}

/// Export a sorted slice of processes as CSV with the columns pid,name,cpu_usage,memory_bytes,exe.
#[must_use]
pub fn export_processes_as_csv(procs: &[crate::collect::ProcessInfo]) -> String {
    let mut out = String::new();
    out.push_str("pid,name,cpu_usage,memory_bytes,exe\n");
    for p in procs {
        let exe = p.exe.as_deref().unwrap_or("");
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            p.pid,
            csv_escape(&p.name),
            format_cpu(p.cpu_usage),
            p.memory,
            csv_escape(exe)
        ));
    }
    out
}

fn format_cpu(value: f32) -> String {
    format!("{:.2}", value)
}

fn json_float(value: f32) -> String {
    if value.is_finite() {
        format!("{:.2}", value)
    } else {
        "0".to_string()
    }
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_escape_plain_field() {
        assert_eq!(csv_escape("chrome.exe"), "chrome.exe");
    }

    #[test]
    fn csv_escape_quoted_field() {
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape("a\nb"), "\"a\nb\"");
    }

    #[test]
    fn json_escape_handles_special_chars() {
        assert_eq!(json_escape_string("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(json_escape_string("a\nb"), "a\\nb");
    }

    #[test]
    fn local_iso_timestamp_includes_offset() {
        let ts = local_iso_timestamp(0);
        // Same calendar date as the local day that contains 1970-01-01 UTC.
        // For positive offsets (e.g. +8) it stays on the 1st; for negative offsets
        // (e.g. -5) it slides back to 1969-12-31. Either is valid — what we care
        // about is that the timestamp carries a timezone suffix.
        let has_offset = ts.rfind('+').is_some() || ts[ts.len() - 6..].contains('-');
        assert!(has_offset, "missing offset in {}", ts);
        assert!(ts.ends_with(":00"), "expected :00 minute suffix in {}", ts);
    }

    // v0.17 stage 3 TD-44：itoa 路径与 std format! 输出等价性回归测试。

    #[test]
    fn format_bytes_itoa_equivalence_b_tier() {
        // B 档走 itoa，与 std format! 输出一致
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(1), "1B");
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(1023), "1023B");
    }

    #[test]
    fn format_bytes_itoa_equivalence_upper_tiers_unchanged() {
        // MB / KB / GB 档保留 f64 路径，行为不变
        assert_eq!(format_bytes(1024), "1KB");
        assert_eq!(format_bytes(1048576), "1MB");
        assert_eq!(format_bytes(1_073_741_824), "1.0GB");
    }

    #[test]
    fn format_speed_itoa_equivalence_b_tier() {
        assert_eq!(format_speed(0), "0B/s");
        assert_eq!(format_speed(100), "100B/s");
        assert_eq!(format_speed(999), "999B/s");
    }

    #[test]
    fn format_speed_itoa_equivalence_upper_tiers_unchanged() {
        assert_eq!(format_speed(1_000), "1.0KB/s");
        assert_eq!(format_speed(1_000_000), "1.0MB/s");
        assert_eq!(format_speed(1_000_000_000), "1.0GB/s");
    }

    #[test]
    fn format_uptime_itoa_equivalence() {
        // 单分钟档
        assert_eq!(format_uptime(60), "1分钟");
        assert_eq!(format_uptime(600), "10分钟");
        // 小时档
        assert_eq!(format_uptime(3600), "1小时0分");
        assert_eq!(format_uptime(3660), "1小时1分");
        // 天档
        assert_eq!(format_uptime(86400), "1天0小时");
        assert_eq!(format_uptime(90000), "1天1小时");
    }

    #[test]
    fn format_run_time_itoa_equivalence() {
        assert_eq!(format_run_time(0), "0s");
        assert_eq!(format_run_time(45), "45s");
        assert_eq!(format_run_time(60), "1m0s");
        assert_eq!(format_run_time(125), "2m5s");
        assert_eq!(format_run_time(3600), "1h0m");
        assert_eq!(format_run_time(3665), "1h1m");
        assert_eq!(format_run_time(86400), "1d0h");
        assert_eq!(format_run_time(90000), "1d1h");
    }
}
