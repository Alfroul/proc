//! `proc record` / `proc replay` — VT100 录制与回放（v3 format）。
//!
//! v0.14 stage 1：UiFrame 录制升级到 v3（按需加载 + footer + v1/v2 sidecar）。
//! `proc replay recording.prec --info` 不开 TUI 输出 footer 元数据。

use std::path::Path;

use colored::Colorize;

use crate::app;
use crate::record::vt100::{VtPlayer, is_vt100_file};
use crate::tui;

pub fn run_record(_output: &Option<std::path::PathBuf>, no_tui: bool) {
    // v0.17 stage 1 Spike：--no-tui flag 已注册但 headless 路径尚未落地。
    // stage 6 Slice 实装 spawn 子进程 + recorder + bookmark + anomaly detection
    // 复用 v0.6 落地的 R 键 TUI 路径业务逻辑（绕过 TUI attach）。
    if no_tui {
        eprintln!(
            "{} v0.17-stage-6 未实装：--no-tui flag 已注册但 headless 路径尚未落地",
            "错误:".red()
        );
        std::process::exit(1);
    }

    let mut app = match app::App::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} {}", "初始化失败:".red(), e);
            std::process::exit(1);
        }
    };
    app.set_recording_wanted(true);

    let mut terminal = match tui::setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{} {}", "TUI 初始化失败:".red(), e);
            std::process::exit(1);
        }
    };

    let result = tui::run_app(&mut terminal, &mut app);
    tui::restore_terminal(&mut terminal).ok();

    if let Err(e) = result {
        eprintln!("{} {}", "错误:".red(), e);
    }
}

pub fn run_replay(file: &Path, info: bool) {
    if info {
        if let Err(e) = run_replay_info(file) {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
        return;
    }
    if is_vt100_file(file) {
        run_vt100_replay(file);
    } else {
        run_legacy_replay(file);
    }
}

/// `proc replay recording.prec --info`：不开 TUI，输出 footer 元数据。
/// VT100 录制走单独路径（无 footer，只输出 start_time / total_frames）。
fn run_replay_info(file: &Path) -> anyhow::Result<()> {
    if is_vt100_file(file) {
        let player = VtPlayer::open(file.to_path_buf())?;
        println!("{}", format!("录制文件:   {}", file.display()).cyan());
        println!("格式:       VT100 v2");
        let total = player.total_frames();
        let (start_ms, end_ms) = player.time_range_ms();
        println!("帧数:       {total}");
        println!("开始 (ms):  {start_ms}");
        println!("结束 (ms):  {end_ms}");
        return Ok(());
    }
    use crate::record::Player;
    let player = Player::open(file.to_path_buf())?;
    let header = player.header();
    let meta = player.meta();
    println!("{}", format!("录制文件:   {}", file.display()).cyan());
    println!("格式版本:   v{}", header.version);
    println!("主机名:     {}", header.hostname);
    println!(
        "开始时间:   {} (unix {})",
        format_iso(header.start_time),
        header.start_time
    );
    println!(
        "结束时间:   {} (unix {})",
        format_iso(meta.end_time),
        meta.end_time
    );
    let duration = meta.end_time.saturating_sub(meta.start_time);
    println!("时长:       {}", format_duration_human(duration));
    println!("帧数:       {}", meta.frame_count);
    println!("异常事件:   {}", meta.anomaly_count);
    println!("docker/操作: {}", meta.event_count);
    println!("最高 CPU:   {:.1}%", meta.max_cpu);
    println!("最高内存:   {}", format_bytes(meta.max_mem));
    Ok(())
}

fn format_iso(ts: u64) -> String {
    // UTC 时间显示（与本机时区无关，footer 存 unix epoch 秒）
    let days_from_epoch = (ts / 86_400) as i64;
    let secs_today = ts % 86_400;
    let h = secs_today / 3600;
    let m = (secs_today % 3600) / 60;
    let s = secs_today % 60;

    // 1970-01-01 + days_from_epoch → YYYY-MM-DD（不引入 chrono，用近似公式）
    let (year, month, day) = epoch_days_to_ymd(days_from_epoch);
    format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02} UTC")
}

/// `epoch_days → (year, month, day)`（基于 civil calendar 算法，1800-2400 准确）。
fn epoch_days_to_ymd(days: i64) -> (i64, u32, u32) {
    // Howard Hinnant 的 civil_from_days 算法
    let z = days + 719_468; // 1970-01-01 → 0 起算
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64; // [0, 146097)
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn format_duration_human(secs: u64) -> String {
    if secs < 60 {
        format!("{secs} 秒")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m} 分 {s} 秒")
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        format!("{h} 时 {m} 分 {s} 秒")
    }
}

fn format_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut f = n as f64;
    let mut unit_idx = 0;
    while f >= 1024.0 && unit_idx + 1 < UNITS.len() {
        f /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{:.2} {}", f, UNITS[unit_idx])
    }
}

fn run_vt100_replay(file: &Path) {
    let player = match VtPlayer::open(file.to_path_buf()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{} {}", "打开录制文件失败:".red(), e);
            std::process::exit(1);
        }
    };

    let total = player.total_frames();
    println!(
        "{}",
        format!("加载 VT100 录制: {} ({} 帧)", file.display(), total).cyan()
    );

    let mut terminal = match tui::setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{} {}", "TUI 初始化失败:".red(), e);
            std::process::exit(1);
        }
    };

    let result = tui::run_vt_replay(&mut terminal, player);
    tui::restore_terminal(&mut terminal).ok();

    if let Err(e) = result {
        eprintln!("{} {}", "错误:".red(), e);
    }
}

fn run_legacy_replay(file: &Path) {
    use crate::record::Player;

    let player = match Player::open(file.to_path_buf()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{} {}", "打开录制文件失败:".red(), e);
            std::process::exit(1);
        }
    };

    let total = player.total_frames();
    println!(
        "{}",
        format!("加载 VT100 录制: {} ({} 帧)", file.display(), total).cyan()
    );

    let mut app = match app::App::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} {}", "初始化失败:".red(), e);
            std::process::exit(1);
        }
    };
    app.start_replay(player);

    let mut terminal = match tui::setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{} {}", "TUI 初始化失败:".red(), e);
            std::process::exit(1);
        }
    };

    let result = tui::run_app(&mut terminal, &mut app);
    tui::restore_terminal(&mut terminal).ok();

    if let Err(e) = result {
        eprintln!("{} {}", "错误:".red(), e);
    }
}
