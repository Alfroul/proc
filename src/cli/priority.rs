//! `proc priority` / `proc affinity` — 进程优先级与 CPU affinity（阶段 4 A4）。

use colored::Colorize;

use crate::process_control::{
    PriorityClass, get_affinity, get_priority, set_affinity, set_priority,
};

pub fn run_priority(pid: u32, set: &Option<String>) {
    match set {
        None => match get_priority(pid) {
            Ok(class) => println!("PID {} 优先级: {}", pid, class.label()),
            Err(e) => {
                eprintln!("{} {}", "查询失败:".red(), e);
                std::process::exit(1);
            }
        },
        Some(class_str) => {
            let class = match parse_priority_class(class_str) {
                Ok(c) => c,
                Err(msg) => {
                    eprintln!("{} {}", "参数错误:".red(), msg);
                    std::process::exit(1);
                }
            };
            match set_priority(pid, class) {
                Ok(()) => println!(
                    "{}",
                    format!("PID {} 优先级已设置为 {}", pid, class.label()).green()
                ),
                Err(e) => {
                    eprintln!("{} {}", "设置失败:".red(), e);
                    std::process::exit(1);
                }
            }
        }
    }
}

fn parse_priority_class(s: &str) -> std::result::Result<PriorityClass, String> {
    match s.to_lowercase().as_str() {
        "idle" => Ok(PriorityClass::Idle),
        "belownormal" | "below_normal" | "below" => Ok(PriorityClass::BelowNormal),
        "normal" => Ok(PriorityClass::Normal),
        "abovenormal" | "above_normal" | "above" => Ok(PriorityClass::AboveNormal),
        "high" => Ok(PriorityClass::High),
        "realtime" => Ok(PriorityClass::Realtime),
        _ => Err(format!(
            "未知优先级 '{}'（合法值：idle / belownormal / normal / abovenormal / high / realtime）",
            s
        )),
    }
}

pub fn run_affinity(pid: u32, set: &Option<String>) {
    match set {
        None => match get_affinity(pid) {
            Ok(mask) => println!(
                "PID {} affinity: 0x{:X} ({} 核)",
                pid,
                mask,
                u64::count_ones(mask)
            ),
            Err(e) => {
                eprintln!("{} {}", "查询失败:".red(), e);
                std::process::exit(1);
            }
        },
        Some(hex_str) => {
            let trimmed = hex_str.trim_start_matches("0x").trim_start_matches("0X");
            let mask = match u64::from_str_radix(trimmed, 16) {
                Ok(v) => v,
                Err(_) => {
                    eprintln!(
                        "{}",
                        format!("--set 期望 16 进制（如 0xFF），实际 '{}'", hex_str).red()
                    );
                    std::process::exit(1);
                }
            };
            match set_affinity(pid, mask) {
                Ok(()) => println!(
                    "{}",
                    format!("PID {} affinity 已设置为 0x{:X}", pid, mask).green()
                ),
                Err(e) => {
                    eprintln!("{} {}", "设置失败:".red(), e);
                    std::process::exit(1);
                }
            }
        }
    }
}
