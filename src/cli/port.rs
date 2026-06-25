//! `proc port` — 端口映射查询、占用进程终止、TCP 传输质量摘要。

use colored::Colorize;

use crate::collect;
use crate::kill;
use crate::port_map;

pub fn run_port(port: &Option<u16>, do_kill: &bool, stats: &bool) {
    if *stats {
        let tcp = collect::SystemSnapshot::tcp_stats();
        println!("{}", "TCP 传输质量摘要".cyan());
        println!("已建立连接: {}", tcp.established);
        println!("LISTEN:     {}", tcp.listen);
        println!("TIME_WAIT:  {}", tcp.time_wait);
        println!("CLOSE_WAIT: {}", tcp.close_wait);
        println!("重传段数:   {}", tcp.retransmitted_segs);
        println!("RST 段数:   {}", tcp.reset_segs);
        println!("失败连接:   {}", tcp.failed_connections);
        println!("输出段数:   {}", tcp.out_segs);
        if tcp.out_segs > 0 {
            println!(
                "重传率:     {:.2}%",
                tcp.retransmitted_segs as f64 / tcp.out_segs as f64 * 100.0
            );
            println!(
                "RST 率:     {:.2}%",
                tcp.reset_segs as f64 / tcp.out_segs as f64 * 100.0
            );
        } else {
            println!(
                "{} 当前无 TCP 传输计数（非 Windows / Linux 平台降级）",
                "提示:".yellow()
            );
        }
        return;
    }
    if let Some(p) = port {
        match port_map::find_pid_by_port(*p) {
            Ok(entries) if entries.is_empty() => {
                println!("{}", format!("端口 {} 未被占用", p).yellow());
            }
            Ok(entries) => {
                if *do_kill {
                    for entry in &entries {
                        if entry.pid > 0 {
                            match kill::kill_process(entry.pid, false) {
                                Ok(kill::KillResult::Killed) => {
                                    println!(
                                        "{}",
                                        format!(
                                            "✓ 已终止 {} (PID {}) 占用端口 {}",
                                            entry.process_name, entry.pid, p
                                        )
                                        .green()
                                    );
                                }
                                Ok(kill::KillResult::AlreadyGone) => {
                                    println!("{}", "进程已不存在".yellow());
                                }
                                Ok(kill::KillResult::AccessDenied) => {
                                    eprintln!(
                                        "{}",
                                        format!(
                                            "✗ 权限不足，无法终止 {} (PID {})",
                                            entry.process_name, entry.pid
                                        )
                                        .red()
                                    );
                                }
                                Ok(kill::KillResult::Failed(e)) => {
                                    eprintln!("{} {}", "✗ 终止失败:".red(), e);
                                }
                                Err(e) => {
                                    eprintln!("{} {}", "✗ 错误:".red(), e);
                                }
                            }
                        }
                    }
                } else {
                    let mut table = comfy_table::Table::new();
                    table.set_header(vec![
                        "协议",
                        "本地地址",
                        "远程地址",
                        "状态",
                        "PID",
                        "进程名",
                    ]);
                    for entry in &entries {
                        let remote = match (entry.remote_addr, entry.remote_port) {
                            (Some(addr), Some(port)) => format!("{}:{}", addr, port),
                            _ => "-".to_string(),
                        };
                        let state = entry.state.as_deref().unwrap_or("-");
                        table.add_row(vec![
                            entry.protocol.to_string(),
                            format!("{}:{}", entry.local_addr, entry.local_port),
                            remote,
                            state.to_string(),
                            entry.pid.to_string(),
                            entry.process_name.clone(),
                        ]);
                    }
                    println!("{table}");
                }
            }
            Err(e) => {
                eprintln!("{} {}", "端口扫描错误:".red(), e);
                std::process::exit(1);
            }
        }
    } else {
        match port_map::scan_ports() {
            Ok(entries) => {
                let mut table = comfy_table::Table::new();
                table.set_header(vec![
                    "协议",
                    "本地地址",
                    "远程地址",
                    "状态",
                    "PID",
                    "进程名",
                ]);
                for entry in &entries {
                    let remote = match (entry.remote_addr, entry.remote_port) {
                        (Some(addr), Some(port)) => format!("{}:{}", addr, port),
                        _ => "-".to_string(),
                    };
                    let state = entry.state.as_deref().unwrap_or("-");
                    table.add_row(vec![
                        entry.protocol.to_string(),
                        format!("{}:{}", entry.local_addr, entry.local_port),
                        remote,
                        state.to_string(),
                        entry.pid.to_string(),
                        entry.process_name.clone(),
                    ]);
                }
                println!("{table}");
            }
            Err(e) => {
                eprintln!("{} {}", "端口扫描错误:".red(), e);
                std::process::exit(1);
            }
        }
    }
}
