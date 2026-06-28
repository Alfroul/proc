pub mod alert;
pub mod anomaly;
pub mod app;
pub mod app_group;
pub mod app_panel;
pub mod classify;
pub mod cli;
pub mod collect;
pub mod diag;
pub mod disk_io_etw;
pub mod dns_log;
pub mod docker;
pub mod ebpf;
pub mod eject;
pub mod error;
pub mod estats;
pub mod filter;
pub mod format;
pub mod gpu;
pub mod inspect;
pub mod kill;
pub mod mcp;
pub mod metrics;
pub mod monitor;
pub mod net_flow;
pub mod port_map;
pub mod port_worker;
pub mod process_control;
pub mod psi;
pub mod record;
pub mod replay;
pub mod schannel_etw;
pub mod search;
pub mod security;
pub mod shutdown;
pub mod smart;
pub mod throttle;
pub mod tree;
pub mod tui;
pub mod ui_state;
pub mod view_models;
pub mod worker;
pub mod workers;

/// Returns the local timezone offset from UTC in hours (e.g. +8 for CST).
/// Uses Win32 `GetTimeZoneInformation` to avoid chrono dependency.
#[cfg(target_os = "windows")]
#[must_use]
pub fn local_offset_hours() -> i64 {
    use windows::Win32::System::Time::GetTimeZoneInformation;
    use windows::Win32::System::Time::TIME_ZONE_INFORMATION;
    unsafe {
        let mut tz: TIME_ZONE_INFORMATION = std::mem::zeroed();
        let result = GetTimeZoneInformation(&mut tz);
        // Bias fields are i32 minutes (negative = east of UTC). result:
        // 0=unknown, 1=standard, 2=daylight.
        let bias_total = if result == 2 {
            tz.Bias
                .wrapping_add(tz.StandardBias)
                .wrapping_add(tz.DaylightBias)
        } else {
            tz.Bias.wrapping_add(tz.StandardBias)
        };
        bias_minutes_to_offset_hours(bias_total)
    }
}

/// Pure conversion from Win32 bias minutes to UTC-offset hours. Separated so
/// we can unit-test the i32::MIN corner case without going through Win32.
///
/// Bias convention: `bias_total` is `UTC − local` in minutes, so the offset
/// we want (east-positive, e.g. +8 for CST) is `-bias_total / 60`. We promote
/// to i64 *before* negating — `-(i32::MIN)` is UB in release Rust.
#[must_use]
pub fn bias_minutes_to_offset_hours(bias_total: i32) -> i64 {
    -(bias_total as i64) / 60
}

#[cfg(not(target_os = "windows"))]
pub fn local_offset_hours() -> i64 {
    // Use libc::localtime_r to get the platform-aware UTC offset (seconds east of UTC).
    // Returns 0 only if the underlying call fails — keep behavior deterministic for tests.
    unsafe {
        let mut now: libc::time_t = 0;
        if libc::time(&mut now as *mut _) == -1 {
            return 0;
        }
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return 0;
        }
        (tm.tm_gmtoff / 3600) as i64
    }
}

/// Returns the proc config directory (~/.config/proc).
#[must_use]
pub fn dirs_config_dir() -> std::path::PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".config").join("proc")
}

/// 删除 `dir` 下 mtime 早于 `keep_days` 天的 `proc*.log` 文件。
///
/// v0.6.0 阶段 3：启动时调一次，避免长期运行后日志目录无限增长。
/// 测试通过传入临时目录 + 预置文件验证。
pub fn cleanup_old_logs(dir: &std::path::Path, keep_days: u32) -> usize {
    use std::time::{Duration, SystemTime};
    let cutoff = SystemTime::now() - Duration::from_secs(u64::from(keep_days) * 86400);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // 匹配 proc.log / proc.YYYY-MM-DD.log，跳过 crashes/ 子目录里的 crash-*.txt。
        if !name.starts_with("proc") || !name.ends_with(".log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.modified().map(|t| t < cutoff).unwrap_or(false)
            && std::fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Convert UTC epoch seconds to (year, month, day, hour, min, sec) using
/// Howard Hinnant's civil_from_days algorithm. Takes seconds since 1970-01-01
/// UTC and treats the input as a UTC value — callers should add
/// `local_offset_hours() * 3600` first if they want local-calendar output.
///
/// v0.6.0 阶段 3：crash report 文件名 + 内容时间戳需要本地时分秒。
#[must_use]
pub fn epoch_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let (year, month, day) = epoch_secs_to_ymd(secs);
    let h = ((secs / 3600) % 24) as u32;
    let m = ((secs / 60) % 60) as u32;
    let s = (secs % 60) as u32;
    (year, month, day, h, m, s)
}

/// Convert UTC epoch seconds to (year, month, day) using Howard Hinnant's
/// civil_from_days algorithm. Takes seconds since 1970-01-01 UTC and treats the
/// input as a UTC value — callers should add `local_offset_hours() * 3600`
/// first if they want local-calendar output.
#[must_use]
pub fn epoch_secs_to_ymd(secs: u64) -> (u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = (if m <= 2 { y + 1 } else { y }) as u32;
    (year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_secs_to_ymd_known_dates() {
        // 2026-06-13 00:00:00 UTC
        //   = 20617 days from 1970-01-01
        //   = 1_781_308_800 seconds
        assert_eq!(epoch_secs_to_ymd(1_781_308_800), (2026, 6, 13));
        // 2026-06-13 23:00:00 UTC still lands on the 13th
        assert_eq!(epoch_secs_to_ymd(1_781_308_800 + 23 * 3600), (2026, 6, 13));
        // 2026-06-14 00:00:00 UTC
        assert_eq!(epoch_secs_to_ymd(1_781_308_800 + 86400), (2026, 6, 14));
        // 1970-01-01 00:00:00 UTC
        assert_eq!(epoch_secs_to_ymd(0), (1970, 1, 1));
        // 2000-02-29 (leap day) — known daylight-saving-free reference
        // 30 years * 365 + 7 leap days (1972..1996 inclusive of every 4 except 1900)
        // = 10957 days => 946_684_800 seconds
        assert_eq!(epoch_secs_to_ymd(951_782_400), (2000, 2, 29));
    }

    /// P1.24 regression: `-(i32::MIN)` is UB in release Rust. The Win32 path
    /// now routes through `bias_minutes_to_offset_hours`, which casts to i64
    /// before negation. We can't easily inject i32::MIN via `local_offset_hours`
    /// (it reads the real OS timezone), so the regression is anchored here
    /// against the pure helper. If this test triggers UB, Miri / sanitizers
    /// catch it; if it triggers a panic, debug assertions catch it.
    #[test]
    fn bias_minutes_to_offset_hours_survives_i32_min() {
        // i32::MIN as minutes is ~ -4084 years — nonsense in practice, but
        // the arithmetic must be defined.
        let _ = bias_minutes_to_offset_hours(i32::MIN);
        // Sanity: a CST-like bias of -480 min gives +8 hours.
        assert_eq!(bias_minutes_to_offset_hours(-480), 8);
        // And UTC (bias 0) gives 0.
        assert_eq!(bias_minutes_to_offset_hours(0), 0);
    }
}
