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
            stats,
        } => run_port(port, do_kill, stats),
        cli::Command::Eject { drive, find_locks } => run_eject(drive, find_locks),
        cli::Command::Who { target_path } => run_who(target_path),
        cli::Command::Handles { pid, file } => run_handles(pid, file),
        cli::Command::Priority { pid, set } => run_priority(*pid, set),
        cli::Command::Affinity { pid, set } => run_affinity(*pid, set),
        cli::Command::Monitor {
            add,
            remove,
            port,
            pid,
            command,
        } => run_monitor(*add, remove, port, pid, command),
        cli::Command::Docker { sub } => run_docker(sub),
        cli::Command::Smart { device } => run_smart(device.as_deref()),
        cli::Command::Dns { tail, since } => run_dns(*tail, since.as_deref()),
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
        "disk_read" | "diskread" => SortField::DiskRead,
        "disk_write" | "diskwrite" => SortField::DiskWrite,
        "net_sent" | "netsent" => SortField::NetSent,
        "net_recv" | "netrecv" => SortField::NetRecv,
        _ => SortField::Cpu,
    };

    crate::collect::sort_processes(&mut processes, sort_field);

    if let Some(n) = limit {
        processes.truncate(*n);
    }

    let show_net = matches!(sort_field, SortField::NetSent | SortField::NetRecv);

    let mut table = comfy_table::Table::new();
    if show_net {
        table.set_header(vec![
            "PID",
            "CPU%",
            "MEM%",
            "内存",
            "↑网络/s",
            "↓网络/s",
            "分类",
            "名称",
        ]);
    } else {
        table.set_header(vec!["PID", "CPU%", "MEM%", "内存", "分类", "名称"]);
    }

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

        if show_net {
            table.add_row(vec![
                proc.pid.to_string(),
                cpu_str,
                mem_pct,
                mem_str,
                proc::format::format_speed(proc.net_sent_rate),
                proc::format::format_speed(proc.net_recv_rate),
                class.label().to_string(),
                proc.name.clone(),
            ]);
        } else {
            table.add_row(vec![
                proc.pid.to_string(),
                cpu_str,
                mem_pct,
                mem_str,
                class.label().to_string(),
                proc.name.clone(),
            ]);
        }
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
        // 部分成功 → exit(2)，便于脚本区分"全失败 (1)"与"部分成功 (2)"。
        std::process::exit(if killed > 0 { 2 } else { 1 });
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

fn run_port(port: &Option<u16>, do_kill: &bool, stats: &bool) {
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
        "disk_read" | "diskread" => SortField::DiskRead,
        "disk_write" | "diskwrite" => SortField::DiskWrite,
        "net_sent" | "netsent" => SortField::NetSent,
        "net_recv" | "netrecv" => SortField::NetRecv,
        _ => SortField::Cpu,
    };
    crate::collect::sort_processes(&mut processes, sort_field);

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

fn run_docker(sub: &cli::DockerSub) {
    let monitor = match docker::DockerMonitor::connect() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            eprintln!("{}", "请确认 Docker 正在运行".yellow());
            std::process::exit(1);
        }
    };

    match sub {
        cli::DockerSub::Ps => run_docker_ps(&monitor),
        cli::DockerSub::Inspect { name } => run_docker_inspect(&monitor, name),
        cli::DockerSub::Top { name } => run_docker_top(&monitor, name),
        cli::DockerSub::Logs { name, follow, tail } => {
            run_docker_logs(&monitor, name, *follow, tail.as_deref())
        }
        cli::DockerSub::Images => run_docker_images(&monitor),
        cli::DockerSub::Volumes => run_docker_volumes(&monitor),
        cli::DockerSub::ImageRm { id, force } => run_docker_image_rm(&monitor, id, *force),
        cli::DockerSub::VolumeRm { name, force } => run_docker_volume_rm(&monitor, name, *force),
        cli::DockerSub::Compose { args } => run_docker_compose(args),
        cli::DockerSub::Events => run_docker_events(&monitor),
        cli::DockerSub::Exec { container, cmd } => run_docker_exec(&monitor, container, cmd),
    }
}

fn run_docker_ps(monitor: &docker::DockerMonitor) {
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

fn run_docker_inspect(monitor: &docker::DockerMonitor, name: &str) {
    let container = monitor.list_containers(true).ok().and_then(|cs| {
        cs.into_iter()
            .find(|c| c.name == name || c.id.starts_with(name))
    });

    let Some(c) = container else {
        eprintln!("{}", format!("容器 '{}' 未找到", name).red());
        std::process::exit(1);
    };
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

fn run_docker_top(monitor: &docker::DockerMonitor, name: &str) {
    match monitor.container_top(name) {
        Ok(procs) => {
            if procs.is_empty() {
                println!("{}", "容器内无进程（可能未运行）".yellow());
                return;
            }
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["PID", "USER", "START", "TIME", "CMD"]);
            for p in &procs {
                table.add_row(vec![
                    p.pid.clone(),
                    p.user.clone(),
                    p.started.clone(),
                    p.cpu_time.clone(),
                    p.command.clone(),
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取进程列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_logs(monitor: &docker::DockerMonitor, name: &str, follow: bool, tail: Option<&str>) {
    if follow {
        // follow 模式：用 logs_worker 同样的策略（spawn thread + runtime）。
        let docker_client = monitor.docker();
        let worker =
            docker::logs_worker::spawn(docker_client, name.to_string(), tail.map(str::to_string));
        println!("{}", format!("跟随 {} 日志（Ctrl+C 停止）", name).cyan());
        loop {
            if shutdown::requested() {
                println!();
                return;
            }
            for chunk in worker.drain() {
                for line in chunk.lines {
                    let prefix = if line.is_stderr { "[stderr] " } else { "" };
                    println!("{}{}", prefix, line.message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    match monitor.collect_logs(name, tail) {
        Ok(logs) => {
            for line in logs {
                let prefix = if line.is_stderr { "[stderr] " } else { "" };
                println!("{}{}", prefix, line.message);
            }
        }
        Err(e) => {
            eprintln!("{} {}", "获取日志失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_images(monitor: &docker::DockerMonitor) {
    match monitor.list_images() {
        Ok(images) => {
            if images.is_empty() {
                println!("{}", "暂无镜像".yellow());
                return;
            }
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["ID", "Tags", "大小", "容器数", "创建"]);
            for img in &images {
                let tags = if img.repo_tags.is_empty() {
                    "<none>".to_string()
                } else {
                    img.repo_tags.join(", ")
                };
                table.add_row(vec![
                    img.short_id.clone(),
                    tags,
                    format_bytes(img.size),
                    img.containers.to_string(),
                    format!("{}", img),
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取镜像列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_volumes(monitor: &docker::DockerMonitor) {
    match monitor.list_volumes() {
        Ok(volumes) => {
            if volumes.is_empty() {
                println!("{}", "暂无 volume".yellow());
                return;
            }
            let mut table = comfy_table::Table::new();
            table.set_header(vec!["名称", "驱动", "挂载点", "大小", "使用"]);
            for v in &volumes {
                let size = if v.size > 0 {
                    format_bytes(v.size)
                } else {
                    "-".to_string()
                };
                let used = if v.in_use { "使用中" } else { "未使用" };
                table.add_row(vec![
                    v.name.clone(),
                    v.driver.clone(),
                    v.mountpoint.clone(),
                    size,
                    used.to_string(),
                ]);
            }
            println!("{table}");
        }
        Err(e) => {
            eprintln!("{} {}", "获取 volume 列表失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_image_rm(monitor: &docker::DockerMonitor, id: &str, force: bool) {
    match monitor.remove_image(id, force) {
        Ok(()) => println!("{}", format!("镜像 {} 已删除", id).green()),
        Err(e) => {
            eprintln!("{} {}", "删除失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_volume_rm(monitor: &docker::DockerMonitor, name: &str, force: bool) {
    match monitor.remove_volume(name, force) {
        Ok(()) => println!("{}", format!("volume {} 已删除", name).green()),
        Err(e) => {
            eprintln!("{} {}", "删除失败:".red(), e);
            std::process::exit(1);
        }
    }
}

fn run_docker_compose(args: &[String]) {
    use std::process::Command;
    let bin = std::env::var("PROC_DOCKER_COMPOSE").unwrap_or_else(|_| "docker-compose".to_string());
    let status = Command::new(&bin).args(args).status().unwrap_or_else(|e| {
        eprintln!(
            "{} 调用 {} 失败: {}（请确认已安装 docker-compose）",
            "错误:".red(),
            bin,
            e
        );
        std::process::exit(127);
    });
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn run_docker_events(monitor: &docker::DockerMonitor) {
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
                "green" => format!("{} {} ({})", event.action, name, event.container_id).green(),
                _ => format!("{} {} ({})", event.action, name, event.container_id).yellow(),
            };
            println!("{}", styled);
        }
    }
}

/// CLI `proc docker exec <container> [cmd...]`（阶段 9 E2）。
///
/// 直接 spawn `docker exec -it <container> <cmd>`，docker CLI 接管 stdio，
/// 用户的终端 = 远端 PTY（无需 proc 自身的 PTY 桥接）。
///
/// TUI 内按 `e` 走另一条路：[`crate::tui::container_exec_view`] 嵌入式 PTY 视图。
fn run_docker_exec(monitor: &docker::DockerMonitor, container: &str, cmd: &[String]) {
    use std::process::Command;

    // 容器存在性检查：友好错误优于 docker CLI 的晦涩报错。
    let containers = monitor.list_containers(true).unwrap_or_default();
    let found = containers
        .iter()
        .find(|c| c.name == container || c.id.starts_with(container));
    let Some(found) = found else {
        eprintln!("{}", format!("容器 '{}' 未找到", container).red());
        std::process::exit(1);
    };

    // cmd 为空时根据 image 推断 shell；非空时透传用户命令。
    let inferred_shell = if cmd.is_empty() {
        docker::exec::detect_default_shell(&found.image)
    } else {
        ""
    };

    let mut command = Command::new("docker");
    command.arg("exec").arg("-it").arg(container);
    if cmd.is_empty() {
        for token in inferred_shell.split_whitespace() {
            command.arg(token);
        }
    } else {
        for token in cmd {
            command.arg(token);
        }
    }

    match command.status() {
        Ok(status) => {
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Err(e) => {
            eprintln!("{} {}", "exec 失败（确认 PATH 有 docker）:".red(), e);
            std::process::exit(1);
        }
    }
}

// ── 阶段 4 CLI：who / handles / priority / affinity ─────────────────────────

fn run_who(path: &std::path::Path) {
    let handles = match proc::inspect::handles::find_lockers(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{} {}", "反查失败:".red(), e);
            std::process::exit(1);
        }
    };
    if handles.is_empty() {
        // filelocksmith 在非管理员账户下看不到系统进程句柄；空结果绝大多数是这个原因。
        println!(
            "{}",
            "未发现占用此路径的进程（提示：枚举系统进程句柄需要管理员权限）".yellow()
        );
        return;
    }
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["PID", "进程名", "类型", "路径"]);
    for h in &handles {
        // find_lockers 反查路径下 raw_handle 字段被借用来存 PID（见模块注释）。
        let pid = h.raw_handle;
        let name = pid_to_name(pid as u32);
        table.add_row(vec![
            pid.to_string(),
            name,
            h.kind.label().to_string(),
            h.name.clone(),
        ]);
    }
    println!("{table}");
}

fn run_handles(pid: &Option<u32>, file: &Option<std::path::PathBuf>) {
    match (pid, file) {
        (Some(pid), None) => run_handles_pid(*pid),
        (None, Some(path)) => run_who(path),
        (Some(_), Some(_)) => {
            eprintln!("{}", "--pid 与 --file 互斥，请二选一".red());
            std::process::exit(1);
        }
        (None, None) => {
            eprintln!(
                "{}",
                "用法: proc handles --pid <PID>   或   proc handles --file <PATH>".red()
            );
            std::process::exit(1);
        }
    }
}

fn run_handles_pid(pid: u32) {
    let handles = match proc::inspect::handles::collect_handles(pid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{} {}", "枚举句柄失败:".red(), e);
            std::process::exit(1);
        }
    };
    if handles.is_empty() {
        println!(
            "{}",
            format!("PID {} 当前无可见句柄（权限不足或进程已退出）", pid).yellow()
        );
        return;
    }
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["类型", "名称", "句柄", "访问"]);
    for h in &handles {
        let name = if h.name.is_empty() {
            "-".to_string()
        } else {
            h.name.clone()
        };
        let access = if h.granted_access == 0 {
            "-".to_string()
        } else {
            format!("0x{:08X}", h.granted_access)
        };
        table.add_row(vec![
            h.kind.label().to_string(),
            name,
            format!("0x{:X}", h.raw_handle),
            access,
        ]);
    }
    println!("{table}");
}

fn run_priority(pid: u32, set: &Option<String>) {
    use proc::process_control::{get_priority, set_priority};
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

fn parse_priority_class(
    s: &str,
) -> std::result::Result<proc::process_control::PriorityClass, String> {
    use proc::process_control::PriorityClass;
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

fn run_affinity(pid: u32, set: &Option<String>) {
    use proc::process_control::{get_affinity, set_affinity};
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

/// 用 sysinfo 反查 PID → 进程名。失败时返回 "?"。
fn pid_to_name(pid: u32) -> String {
    proc::collect::sysinfo_with(|sys| {
        sys.process(sysinfo::Pid::from_u32(pid))
            .map(|p| p.name().to_string_lossy().to_string())
            .unwrap_or_else(|| "?".to_string())
    })
}

// ── 阶段 8 CLI:dns ───────────────────────────────────────────────────────

/// `proc dns` 子命令：流式输出 DNS 查询日志。仅 Windows 平台（其它平台
/// [`proc::dns_log::detect_collector`] 返回 None，给出降级提示）。
fn run_dns(tail: bool, since: Option<&str>) {
    if let Some(s) = since {
        // 隐私约束：DNS 查询不持久化；`--since` 需要从持久化源（Windows EventLog）
        // 读历史。本阶段未实现历史读取（需要单独的 Get-WinEvent 一次性查询路径），
        // 留作未来工作 —— stage-8.md §7 明确「需要持久化？本阶段不做，留 TODO」。
        eprintln!(
            "{}",
            "--since 暂未实现：DNS 日志仅内存缓冲，不持久化。请用 --tail 实时跟随。".yellow()
        );
        let _ = s;
        return;
    }

    let Some(collector) = proc::dns_log::detect_collector() else {
        eprintln!(
            "{}",
            "DNS 日志采集在此平台不可用（Windows 走 PowerShell Get-WinEvent，其它见 ADR-0006）"
                .yellow()
        );
        return;
    };

    println!("{}", "DNS 日志跟随中（仅内存 · Ctrl+C 退出）...".cyan());
    let mut collector = collector;

    // tail 模式：每 500ms drain collector，新事件打 stdout。
    // 非 tail 模式：drain 一次拿现有事件，然后退出（与 --since 互补）。
    let poll = std::time::Duration::from_millis(500);
    let mut printed_any = false;
    loop {
        let queries = collector.drain();
        for q in &queries {
            println!("{q}");
            printed_any = true;
        }
        if proc::shutdown::requested() {
            break;
        }
        if !tail {
            if !printed_any {
                eprintln!(
                    "{}",
                    "当前暂无 DNS 查询日志（启动浏览器或 curl 触发，或用 --tail 持续跟随）"
                        .yellow()
                );
            }
            break;
        }
        std::thread::sleep(poll);
    }
}

// ── 阶段 5 CLI:smart ──────────────────────────────────────────────────────

fn run_smart(device: Option<&str>) {
    match device {
        Some(dev) => run_smart_detail(dev),
        None => run_smart_list(),
    }
}

fn run_smart_list() {
    let disks = proc::smart::list_disks();
    if disks.is_empty() {
        println!(
            "{}",
            "未发现可查询的磁盘(Linux 看 /sys/block,Windows 走 WMI Win32_DiskDrive)".yellow()
        );
        return;
    }
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["设备", "型号", "序列号", "健康", "温度", "属性数"]);
    let mut any_data = false;
    for dev in &disks {
        match proc::smart::read_smart(dev) {
            Ok(data) => {
                any_data = true;
                let temp = data
                    .temperature
                    .map(|t| format!("{:.1}\u{00B0}C", t))
                    .unwrap_or_else(|| "-".to_string());
                table.add_row(vec![
                    data.device.clone(),
                    data.model.clone(),
                    data.serial.clone(),
                    format!("{} {:?}", data.health.badge(), data.health),
                    temp,
                    data.attributes.len().to_string(),
                ]);
            }
            Err(e) => {
                table.add_row(vec![
                    dev.clone(),
                    "-".to_string(),
                    "-".to_string(),
                    "无数据".to_string(),
                    "-".to_string(),
                    format!("（{}）", e),
                ]);
            }
        }
    }
    println!("{table}");
    if !any_data {
        println!(
            "{}",
            "提示: 多数 Linux 装包带 smartmontools,Windows 装上 smartmontools 后 JSON 解析更完整"
                .yellow()
        );
    }
}

fn run_smart_detail(device: &str) {
    let data = match proc::smart::read_smart(device) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{} 读取 {} SMART 数据失败: {}", "错误:".red(), device, e);
            std::process::exit(1);
        }
    };
    println!("{}", format!("磁盘: {}", data.device).cyan());
    println!("型号: {}", data.model);
    println!("序列号: {}", data.serial);
    println!(
        "温度: {}",
        data.temperature
            .map(|t| format!("{:.1}\u{00B0}C", t))
            .unwrap_or_else(|| "未知".to_string())
    );
    println!("健康: {} {:?}", data.health.badge(), data.health);
    if data.attributes.is_empty() {
        println!(
            "{}",
            "（无详细 SMART 属性 —— Windows 走 WMI 降级时常见,装 smartmontools 可拿完整表）"
                .yellow()
        );
        return;
    }
    println!();
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["ID", "名称", "当前值", "阈值", "原始值", "失败"]);
    for attr in &data.attributes {
        table.add_row(vec![
            format!("{:3}", attr.id),
            attr.name.clone(),
            attr.value.to_string(),
            attr.threshold.to_string(),
            attr.raw_value.to_string(),
            if attr.failing { "✗" } else { "-" }.to_string(),
        ]);
    }
    println!("{table}");
}
