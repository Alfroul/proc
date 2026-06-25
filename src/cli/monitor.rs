//! `proc monitor` — 进程监控（PID / 端口 / 命令带自动重启）。
//!
//! 无参数时进入交互式 TUI 监控面板（与不传子命令的 `proc` 同款入口）。

use colored::Colorize;

use crate::app;
use crate::error;
use crate::monitor;
use crate::shutdown;
use crate::tui;

pub fn run_monitor(
    _add: bool,
    remove: &Option<u32>,
    port: &Option<u16>,
    pid: &Option<u32>,
    command: &Option<String>,
) {
    let mut mgr = monitor::MonitorManager::new();

    if let Some(id) = remove {
        match mgr.remove_monitor(*id) {
            Ok(()) => println!("{}", format!("✓ 已删除监控 ID {}", id).green()),
            Err(e) => {
                eprintln!("{} {}", "✗ 错误:".red(), e);
                std::process::exit(1);
            }
        }
        return;
    }

    if let Some(p) = pid {
        match mgr.add_monitor(
            monitor::MonitorTarget::ByPid { pid: *p },
            monitor::RestartPolicy::NotifyOnly,
        ) {
            Ok(id) => println!(
                "{}",
                format!("✓ 已添加 PID {} 监控 (ID: {})", p, id).green()
            ),
            Err(e) => {
                eprintln!("{} {}", "✗ 错误:".red(), e);
                std::process::exit(1);
            }
        }
        println!("{}", "按 Ctrl+C 停止监控".yellow());
        // 不再用 stdin().read_line() 阻塞 — Ctrl+C 由 shutdown::requested() 捕获
        while !shutdown::requested() {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        println!("{}", "停止监控".yellow());
        return;
    }

    if let Some(p) = port {
        match mgr.add_monitor(
            monitor::MonitorTarget::ByPort { port: *p },
            monitor::RestartPolicy::NotifyOnly,
        ) {
            Ok(id) => {
                println!(
                    "{}",
                    format!("✓ 已添加端口 {} 监控 (ID: {})", p, id).green()
                );
                let handle = monitor::port_watcher::spawn_port_watcher(*p, 5);
                println!("{}", "监控中... 按 Ctrl+C 停止".yellow());
                loop {
                    if shutdown::requested() {
                        println!("{}", "停止监控".yellow());
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    while let Some(event) = handle.try_recv() {
                        match event {
                            monitor::port_watcher::PortEvent::Occupied {
                                port,
                                pid,
                                process_name,
                            } => {
                                println!(
                                    "{}",
                                    format!("端口 {} 被 {} (PID {}) 占用", port, process_name, pid)
                                        .cyan()
                                );
                            }
                            monitor::port_watcher::PortEvent::Released { port } => {
                                println!("{}", format!("端口 {} 已释放", port).yellow());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("{} {}", "✗ 错误:".red(), e);
                std::process::exit(1);
            }
        }
    }

    if let Some(cmd) = command {
        let args: Vec<String> = cmd
            .split_whitespace()
            .skip(1)
            .map(|s| s.to_string())
            .collect();
        let cmd_bin = cmd.split_whitespace().next().unwrap_or(cmd);
        match mgr.add_monitor(
            monitor::MonitorTarget::ByCommand {
                cmd: cmd_bin.to_string(),
                args,
                cwd: None,
            },
            monitor::RestartPolicy::AutoRestart {
                max_retries: 5,
                base_backoff: 1,
                max_backoff: 30,
            },
        ) {
            Ok(id) => {
                println!(
                    "{}",
                    format!("✓ 已添加命令监控: {} (ID: {})", cmd, id).green()
                );
                let handle = monitor::watchdog::spawn_watchdog(
                    id,
                    cmd_bin,
                    &cmd.split_whitespace()
                        .skip(1)
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>(),
                    None,
                    monitor::RestartPolicy::AutoRestart {
                        max_retries: 5,
                        base_backoff: 1,
                        max_backoff: 30,
                    },
                );
                println!("{}", "监控中... 按 Ctrl+C 停止".yellow());
                loop {
                    if shutdown::requested() {
                        println!("{}", "停止监控".yellow());
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    while let Some(event) = handle.try_recv() {
                        match event {
                            monitor::watchdog::WatchdogEvent::Started { pid, .. } => {
                                println!("{}", format!("进程已启动 (PID {})", pid).green());
                            }
                            monitor::watchdog::WatchdogEvent::Crashed {
                                exit_code,
                                attempt,
                                restarting,
                                ..
                            } => {
                                println!(
                                    "{}",
                                    format!(
                                        "进程崩溃 (code: {:?})，第 {} 次{}",
                                        exit_code,
                                        attempt,
                                        if restarting { "，正在重启..." } else { "" }
                                    )
                                    .red()
                                );
                            }
                            monitor::watchdog::WatchdogEvent::Stopped { reason, .. } => {
                                println!("{}", format!("监控停止: {}", reason).yellow());
                                return;
                            }
                            monitor::watchdog::WatchdogEvent::Running { pid, .. } => {
                                println!("{}", format!("进程运行中 (PID {})", pid).cyan());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("{} {}", "✗ 错误:".red(), e);
                std::process::exit(1);
            }
        }
    }

    // 无参数 → 启动 TUI 监控面板
    if let Err(e) = run_tui() {
        eprintln!("{} {}", "错误:".red(), e);
        std::process::exit(1);
    }
}

fn run_tui() -> error::Result<()> {
    let mut app = app::App::new()?;
    let mut terminal = tui::setup_terminal()?;
    let result = tui::run_app(&mut terminal, &mut app);
    tui::restore_terminal(&mut terminal)?;
    result
}
