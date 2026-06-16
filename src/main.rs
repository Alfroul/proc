use std::path::Path;

use clap::Parser;
use colored::Colorize;

use proc::app;
use proc::classify;
use proc::cli;
use proc::collect::{self, SortField};
use proc::docker;
use proc::eject;
use proc::error;
use proc::kill;
use proc::monitor;
use proc::port_map;
use proc::record::vt100::{VtPlayer, is_vt100_file};
use proc::shutdown;
use proc::tree;
use proc::tui;

fn main() {
    // Install Ctrl+C / SIGINT handler before anything else so every code path
    // (TUI, replay, CLI subcommand loops) can poll `shutdown::requested()`.
    shutdown::init();

    // Quick check: if first arg is a .prec file, replay directly
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1].to_lowercase().ends_with(".prec") {
        let path = std::path::PathBuf::from(&args[1]);
        if path.exists() {
            init_tracing();
            run_replay(&path);
            return;
        }
    }

    // Also accept .cast files
    if args.len() == 2 && args[1].to_lowercase().ends_with(".cast") {
        let path = std::path::PathBuf::from(&args[1]);
        if path.exists() {
            init_tracing();
            run_replay(&path);
            return;
        }
    }

    let cli_args = cli::Cli::parse();

    init_tracing();

    if let Some(cmd) = &cli_args.command {
        run_subcommand(cmd);
        return;
    }

    if let Err(e) = run_tui() {
        eprintln!("{} {}", "错误:".red(), e);
        std::process::exit(1);
    }
}

fn init_tracing() {
    let config_dir = proc::dirs_config_dir();
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        eprintln!("警告: 创建配置目录失败: {} (日志不可用)", e);
        return;
    }
    // File::create 默认 truncate，启动时覆盖旧日志 —— 防止长期运行后 proc.log 无限增长。
    // 如需保留历史，请用 RUST_LOG + 外部 logrotate 或改接 tracing-appender（见 ADR-0006）。
    let log_path = config_dir.join("proc.log");
    match std::fs::File::create(&log_path) {
        Ok(file) => {
            // 默认 info 级别；用户可用 RUST_LOG=proc=debug 提级。
            // from_default_env 在 RUST_LOG 未设置时返回空 filter → with_env_filter
            // 退回到我们给的 default。
            let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
            let subscriber = tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(file)
                .with_ansi(false)
                .finish();
            if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
                eprintln!("警告: 初始化日志失败: {} (日志不可用)", e);
            }
        }
        Err(e) => eprintln!("警告: 创建日志文件失败: {} (日志不可用)", e),
    }
}

fn run_subcommand(cmd: &cli::Command) {
    match cmd {
        cli::Command::Ls { sort, limit } => run_ls(sort, limit),
        cli::Command::Kill { pid, force } => run_kill(*pid, *force),
        cli::Command::Pkill {
            name,
            force,
            dry_run,
        } => run_pkill(name, *force, *dry_run),
        cli::Command::Tree => run_tree(),
        cli::Command::Port {
            port,
            kill: do_kill,
        } => run_port(port, do_kill),
        cli::Command::Eject { drive, find_locks } => run_eject(drive, find_locks),
        cli::Command::Monitor {
            add,
            remove,
            port,
            pid,
            command,
        } => run_monitor(*add, remove, port, pid, command),
        cli::Command::Docker { watch, container } => run_docker(watch, container),
        cli::Command::Record { output } => run_record(output),
        cli::Command::Replay { file } => run_replay(file),
        cli::Command::Export {
            format,
            output,
            sort,
            limit,
        } => run_export(format, output, sort, limit),
    }
}

fn run_ls(sort: &str, limit: &Option<usize>) {
    let mut snapshot = match collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };

    if let Err(e) = snapshot.refresh() {
        eprintln!("{} {}", "刷新失败:".red(), e);
        std::process::exit(1);
    }

    let _ = snapshot.refresh_heavy_incremental();
    let mut processes = snapshot.cached_processes_vec();
    let sort_field = match sort {
        "mem" | "memory" => SortField::Memory,
        "name" => SortField::Name,
        "pid" => SortField::Pid,
        _ => SortField::Cpu,
    };

    processes.sort_by(|a, b| match sort_field {
        SortField::Cpu => b
            .cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal),
        SortField::Memory => b.memory.cmp(&a.memory),
        SortField::Pid => a.pid.cmp(&b.pid),
        SortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortField::Security => std::cmp::Ordering::Equal,
        SortField::DiskRead => b.disk_read_speed.cmp(&a.disk_read_speed),
        SortField::DiskWrite => b.disk_write_speed.cmp(&a.disk_write_speed),
    });

    if let Some(n) = limit {
        processes.truncate(*n);
    }

    let mut table = comfy_table::Table::new();
    table.set_header(vec!["PID", "CPU%", "MEM%", "内存", "分类", "名称"]);

    for proc in &processes {
        let class = classify::classify_process(proc);
        let mem_str = format_bytes(proc.memory);
        let cpu_str = format!("{:.1}", proc.cpu_usage);
        let mem_pct = if proc.virtual_memory > 0 {
            format!(
                "{:.1}",
                proc.memory as f64 / proc.virtual_memory as f64 * 100.0
            )
        } else {
            "0.0".to_string()
        };

        table.add_row(vec![
            proc.pid.to_string(),
            cpu_str,
            mem_pct,
            mem_str,
            class.label().to_string(),
            proc.name.clone(),
        ]);
    }

    println!("{table}");
}

fn run_kill(pid: u32, force: bool) {
    let force_label = if force { "（强制）" } else { "" };
    println!("{}进程 PID {} {}...", "终止".cyan(), pid, force_label);

    match kill::kill_process(pid, force) {
        Ok(kill::KillResult::Killed) => println!("{}", "✓ 进程已终止".green()),
        Ok(kill::KillResult::AlreadyGone) => println!("{}", "进程已不存在".yellow()),
        Ok(kill::KillResult::AccessDenied) => {
            eprintln!("{}", "✗ 权限不足，请尝试以管理员身份运行".red());
            std::process::exit(1);
        }
        Ok(kill::KillResult::Failed(e)) => {
            eprintln!("{} {}", "✗ 终止失败:".red(), e);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{} {}", "✗ 错误:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_pkill(name: &str, force: bool, dry_run: bool) {
    let mode = if dry_run {
        "预演"
    } else if force {
        "强制"
    } else {
        ""
    };
    println!(
        "{}名称为 '{}' 的进程{}...",
        "查找并".cyan(),
        name,
        if mode.is_empty() {
            "".to_string()
        } else {
            format!("（{}）", mode)
        }
    );

    let results = match kill::kill_by_name(name, force, dry_run) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {}", "✗ 错误:".red(), e);
            std::process::exit(1);
        }
    };

    if results.is_empty() {
        println!("{}", format!("未找到名称匹配 '{}' 的进程", name).yellow());
        return;
    }

    let mut killed = 0u32;
    let mut failed = 0u32;
    for r in &results {
        match &r.outcome {
            None => println!(
                "{}  PID {} ({}) — 不终止",
                "[dry-run]".yellow(),
                r.pid,
                r.name
            ),
            Some(kill::KillResult::Killed) => {
                println!("{}", format!("✓ PID {} ({}) 已终止", r.pid, r.name).green());
                killed += 1;
            }
            Some(kill::KillResult::AlreadyGone) => {
                println!(
                    "{}",
                    format!("  PID {} ({}) 已退出", r.pid, r.name).yellow()
                );
            }
            Some(kill::KillResult::AccessDenied) => {
                eprintln!("{}", format!("✗ PID {} ({}) 权限不足", r.pid, r.name).red());
                failed += 1;
            }
            Some(kill::KillResult::Failed(e)) => {
                eprintln!(
                    "{}",
                    format!("✗ PID {} ({}) 失败: {}", r.pid, r.name, e).red()
                );
                failed += 1;
            }
        }
    }

    let total = results.len();
    println!(
        "{}",
        format!(
            "共匹配 {} 个进程{}",
            total,
            if dry_run {
                "".to_string()
            } else {
                format!("，已终止 {} 个，失败 {} 个", killed, failed)
            }
        )
        .cyan()
    );

    if failed > 0 {
        std::process::exit(1);
    }
}

fn run_tree() {
    let mut snapshot = match collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };

    if let Err(e) = snapshot.refresh() {
        eprintln!("{} {}", "刷新失败:".red(), e);
        std::process::exit(1);
    }

    let _ = snapshot.refresh_heavy_incremental();
    let processes = snapshot.cached_processes_vec();
    let (_, total_mem) = snapshot.memory_usage();
    let tree_nodes = tree::build_process_tree(&processes, total_mem);
    let output = tree::format_tree_text(&tree_nodes);
    println!("{output}");
}

fn run_port(port: &Option<u16>, do_kill: &bool) {
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

use proc::format::format_bytes;

fn run_eject(drive: &Option<String>, find_locks: &bool) {
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

fn run_monitor(
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

fn run_record(_output: &Option<std::path::PathBuf>) {
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

fn run_export(
    format: &str,
    output: &Option<std::path::PathBuf>,
    sort: &str,
    limit: &Option<usize>,
) {
    let mut snapshot = match collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };
    if let Err(e) = snapshot.refresh() {
        eprintln!("{} {}", "刷新失败:".red(), e);
        std::process::exit(1);
    }
    let _ = snapshot.refresh_heavy_incremental();
    let mut processes = snapshot.cached_processes_vec();

    let sort_field = match sort {
        "mem" | "memory" => SortField::Memory,
        "name" => SortField::Name,
        "pid" => SortField::Pid,
        _ => SortField::Cpu,
    };
    processes.sort_by(|a, b| match sort_field {
        SortField::Cpu => b
            .cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal),
        SortField::Memory => b.memory.cmp(&a.memory),
        SortField::Pid => a.pid.cmp(&b.pid),
        SortField::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SortField::Security => std::cmp::Ordering::Equal,
        SortField::DiskRead => b.disk_read_speed.cmp(&a.disk_read_speed),
        SortField::DiskWrite => b.disk_write_speed.cmp(&a.disk_write_speed),
    });

    if let Some(n) = limit {
        processes.truncate(*n);
    }

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let payload = match format.to_lowercase().as_str() {
        "csv" => proc::format::export_processes_as_csv(&processes),
        _ => proc::format::export_processes_as_json(&processes, epoch_secs),
    };

    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &payload) {
                eprintln!("{} {}", "写入失败:".red(), e);
                std::process::exit(1);
            }
            println!(
                "{}",
                format!("✓ 已导出 {} 个进程到 {}", processes.len(), path.display()).green()
            );
        }
        None => println!("{}", payload),
    }
}

fn run_replay(file: &Path) {
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
    use proc::record::Player;

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

fn run_docker(watch: &bool, container: &Option<String>) {
    let monitor = match docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            eprintln!("{}", "请确认 Docker 正在运行".yellow());
            std::process::exit(1);
        }
    };

    if let Some(name) = container {
        match monitor.list_containers(true) {
            Ok(containers) => {
                let found = containers
                    .iter()
                    .find(|c| c.name == *name || c.id.starts_with(name));
                match found {
                    Some(c) => {
                        println!("{}", format!("容器: {} ({})", c.name, c.id).cyan());
                        println!("镜像: {}", c.image);
                        println!("状态: {}", c.status);
                        println!("健康: {}", c.health);

                        match monitor.inspect_health(name) {
                            Ok(health) => println!("健康详情: {}", health),
                            Err(e) => println!("{} 健康检查失败: {}", "⚠".yellow(), e),
                        }

                        match monitor.get_stats(name) {
                            Ok(stats) => {
                                println!("CPU:  {:.1}%", stats.cpu_percent);
                                println!(
                                    "内存: {} / {}",
                                    format_bytes(stats.memory_usage),
                                    format_bytes(stats.memory_limit)
                                );
                                println!(
                                    "网络: ↓{} ↑{}",
                                    format_bytes(stats.network_in),
                                    format_bytes(stats.network_out)
                                );
                            }
                            Err(e) => println!("{} 获取统计失败: {}", "⚠".yellow(), e),
                        }
                    }
                    None => {
                        eprintln!("{}", format!("容器 '{}' 未找到", name).red());
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("{} {}", "获取容器列表失败:".red(), e);
                std::process::exit(1);
            }
        }
        return;
    }

    if *watch {
        let docker_client = monitor.docker();
        let receiver = docker::events::spawn_event_watcher(docker_client);
        println!("{}", "监听 Docker 事件中... (Ctrl+C 停止)".cyan());

        loop {
            if shutdown::requested() {
                println!("{}", "停止事件监听".yellow());
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            while let Some(event) = receiver.try_recv() {
                let name = event
                    .container_name
                    .as_deref()
                    .unwrap_or(&event.container_id);
                let style = match event.action.as_str() {
                    "die" | "stop" => "red",
                    "start" => "green",
                    _ => "yellow",
                };
                let styled = match style {
                    "red" => format!("{} {} ({})", event.action, name, event.container_id).red(),
                    "green" => {
                        format!("{} {} ({})", event.action, name, event.container_id).green()
                    }
                    _ => format!("{} {} ({})", event.action, name, event.container_id).yellow(),
                };
                println!("{}", styled);
            }
        }
    }

    match monitor.list_containers(true) {
        Ok(containers) => {
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["状态", "名称", "镜像", "健康", "运行时长"]);
            for c in &containers {
                let status_icon = match c.state.as_str() {
                    "running" => "▲ 运行",
                    "exited" | "dead" => "■ 停止",
                    _ => &c.state,
                };
                let uptime = c
                    .running_since
                    .map(|s| {
                        let elapsed = s.elapsed().unwrap_or(std::time::Duration::ZERO);
                        let secs = elapsed.as_secs();
                        if secs < 60 {
                            format!("{}秒", secs)
                        } else if secs < 3600 {
                            format!("{}分", secs / 60)
                        } else if secs < 86400 {
                            format!("{}时", secs / 3600)
                        } else {
                            format!("{}天", secs / 86400)
                        }
                    })
                    .unwrap_or_else(|| "-".to_string());
                table.add_row(vec![
                    status_icon.to_string(),
                    c.name.clone(),
                    c.image.clone(),
                    c.health.to_string(),
                    uptime,
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取容器列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}
