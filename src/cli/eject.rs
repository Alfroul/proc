//! `proc eject [drive]` — U 盘助手（查占用 / 弹出）。

use colored::Colorize;

use crate::eject;

pub fn run_eject(drive: &Option<String>, find_locks: &bool) {
    match drive {
        Some(drive_str) => {
            if let Err(e) = eject::cli_check_drive(drive_str, *find_locks) {
                eprintln!("{} {}", "错误:".red(), e);
                std::process::exit(1);
            }
        }
        None => {
            if let Err(e) = eject::cli_list_devices() {
                eprintln!("{} {}", "错误:".red(), e);
                std::process::exit(1);
            }
        }
    }
}
