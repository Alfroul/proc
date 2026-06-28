//! `proc throttle <pid> on|off` — Windows 11 EcoQoS / Efficiency Mode（阶段 6）。
//!
//! 与 priority / affinity 同款风格：成功绿字、失败红字 + exit 1。
//! 非 Windows 平台调用直接报错退出（ADR-0014 cfg-gate）。

use colored::Colorize;

pub fn run_throttle(pid: u32, state: &str) {
    let eco = match state {
        "on" => true,
        "off" => false,
        // clap value_parser 已经限制为 on/off，理论不会到这里
        _ => {
            eprintln!(
                "{}",
                format!("参数错误: '{}'（合法值: on / off）", state).red()
            );
            std::process::exit(1);
        }
    };

    match crate::throttle::set_throttle(pid, eco) {
        Ok(()) => {
            let label = if eco { "Eco (🍃)" } else { "Normal" };
            println!(
                "{}",
                format!("PID {} EcoQoS 已设置为 {}", pid, label).green()
            );
        }
        Err(e) => {
            eprintln!("{} {}", "切换失败:".red(), e);
            std::process::exit(1);
        }
    }
}
