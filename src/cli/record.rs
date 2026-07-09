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
        // v0.17 stage 5：VT100 自动转码到临时 v3 文件，让用户享受 search / 倒放 / 书签能力。
        // 转码失败 fallback 走 VtPlayer 正向 replay（保留 v0.6 既有路径）。
        match try_transcode_vt100_for_replay(file) {
            Ok(tmp_path) => {
                run_legacy_replay(&tmp_path);
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    format!("VT100 转码失败，回退 VtPlayer 路径: {e}").yellow()
                );
                run_vt100_replay(file);
            }
        }
    } else {
        run_legacy_replay(file);
    }
}

/// v0.17 stage 5：VT100 → 临时 v3 文件转码 + RAII 清理。
///
/// 返回临时文件路径（`<file>.prec.tmp.v3`），调用方用它走 v3 Player 路径。
/// 临时文件生命周期由 [`crate::record::TranscodedTempFile`] Drop 管理——本函数
/// leak 一个 wrapper 到 'static 让它进程退出时才清理（replay 场景进程退出 =
/// 用户结束 replay）。
fn try_transcode_vt100_for_replay(file: &Path) -> anyhow::Result<std::path::PathBuf> {
    let tmp_path = file.with_extension("prec.tmp.v3");
    let stats = crate::record::convert_vt100_to_v3_file(file, &tmp_path)
        .map_err(|e| anyhow::anyhow!("转码失败: {e}"))?;
    tracing::info!(
        frame_count = stats.frame_count,
        unique_processes = stats.unique_process_count,
        tmp_path = %tmp_path.display(),
        "VT100 → v3 转码完成",
    );
    // RAII wrapper：leak 到 'static 让进程退出时才删（replay 单次会话场景，
    // run_legacy_replay 不返 wrapper，Box::leak 是最简方案）
    Box::leak(Box::new(crate::record::TranscodedTempFile::new(
        tmp_path.clone(),
    )));
    Ok(tmp_path)
}

/// `proc replay recording.prec --info`：不开 TUI，输出 footer 元数据。
///
/// v0.17 stage 5：VT100 路径走透明转码 + 输出 v3 footer 元数据（含 hostname /
/// anomaly_count / max_cpu 等 VT100 header 不携带的字段）。转码失败 fallback
/// 输出原 VT100 header 信息（保留 v0.6 既有行为）。
fn run_replay_info(file: &Path) -> anyhow::Result<()> {
    if is_vt100_file(file) {
        // 尝试转码输出 v3 footer 元数据；失败 fallback 走 VT100 header 路径
        return run_replay_info_vt100_transcoded(file)
            .or_else(|_| run_replay_info_vt100_legacy(file));
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

/// VT100 录制 → 转码后输出 v3 footer 元数据（v0.17 stage 5）。
///
/// 临时文件 `<file>.info.tmp.v3` 在函数结束时手动清理（不用 RAII wrapper，
/// 因为 info 路径是 one-shot，函数结束就完成）。
fn run_replay_info_vt100_transcoded(file: &Path) -> anyhow::Result<()> {
    let tmp_path = file.with_extension("info.tmp.v3");
    let result = (|| -> anyhow::Result<()> {
        let stats = crate::record::convert_vt100_to_v3_file(file, &tmp_path)
            .map_err(|e| anyhow::anyhow!("转码失败: {e}"))?;

        use crate::record::Player;
        let player = Player::open(tmp_path.to_path_buf())?;
        let header = player.header();
        let meta = player.meta();
        println!("{}", format!("录制文件:   {}", file.display()).cyan());
        println!("格式:       VT100 (v2 转码 v3)");
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
        println!(
            "{} VT100 转码统计：{} unique 进程",
            "转码:".cyan(),
            stats.unique_process_count
        );
        Ok(())
    })();
    // 无论成功 / 失败都清理临时文件（info 路径 one-shot）
    let _ = std::fs::remove_file(&tmp_path);
    result
}

/// VT100 录制 → fallback 输出 header 信息（v0.17 stage 5 保留 v0.6 既有行为）。
fn run_replay_info_vt100_legacy(file: &Path) -> anyhow::Result<()> {
    let player = VtPlayer::open(file.to_path_buf())?;
    println!("{}", format!("录制文件:   {}", file.display()).cyan());
    println!("格式:       VT100 v2");
    let total = player.total_frames();
    let (start_ms, end_ms) = player.time_range_ms();
    println!("帧数:       {total}");
    println!("开始 (ms):  {start_ms}");
    println!("结束 (ms):  {end_ms}");
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
