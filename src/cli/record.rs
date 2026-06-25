//! `proc record` / `proc replay` — VT100 录制与回放（v2 format）。

use std::path::Path;

use colored::Colorize;

use crate::app;
use crate::record::vt100::{VtPlayer, is_vt100_file};
use crate::tui;

pub fn run_record(_output: &Option<std::path::PathBuf>) {
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

pub fn run_replay(file: &Path) {
    if is_vt100_file(file) {
        run_vt100_replay(file);
    } else {
        run_legacy_replay(file);
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
