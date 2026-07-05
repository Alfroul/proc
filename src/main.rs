//! `proc` 可执行入口 — v0.6.0 阶段 5 #6 后 main.rs 只保留顶层运行时：
//! - `main()`：self-mitigation + init_tracing + panic_hook + .prec/.cast 直跑 + dispatch；
//! - `init_tracing()`（阶段 3 改造为 RollingFileAppender::daily）；
//! - `install_panic_hook()`（阶段 3 链式 panic hook 写 crash report）；
//! - `run_tui()`：默认入口（无子命令时进入交互式 TUI）。
//!
//! 每个 CLI 子命令的 dispatch 实现见 [`proc::cli`] 各子模块
//! （`ls::run_ls` / `kill::run_kill` / `docker_cmd::run_docker` ...）。

use clap::Parser;
use colored::Colorize;

use proc::cli;
use proc::error;
use proc::tui;

fn main() {
    // Install Ctrl+C / SIGINT handler before anything else so every code path
    // (TUI, replay, CLI subcommand loops) can poll `shutdown::requested()`.
    proc::shutdown::init();

    // v0.6.0 阶段 2：最早调用 self-mitigation，早于 tracing init / 任何 worker / FFI。
    // tracing 此时还没初始化 → warn 会丢失，所以函数返回失败的策略名列表，
    // 这里直接 eprintln!。
    {
        let failed = proc::security::self_mitigation::apply_self_mitigations();
        if !failed.is_empty() {
            eprintln!(
                "warning: self-mitigation policies failed: {}",
                failed.join(", "),
            );
        }
    }

    // Quick check: if first arg is a .prec file, replay directly
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1].to_lowercase().ends_with(".prec") {
        let path = std::path::PathBuf::from(&args[1]);
        if path.exists() {
            let _log_guard = init_tracing();
            install_panic_hook();
            cli::record::run_replay(&path, false);
            return;
        }
    }

    // Also accept .cast files
    if args.len() == 2 && args[1].to_lowercase().ends_with(".cast") {
        let path = std::path::PathBuf::from(&args[1]);
        if path.exists() {
            let _log_guard = init_tracing();
            install_panic_hook();
            cli::record::run_replay(&path, false);
            return;
        }
    }

    let cli_args = cli::Cli::parse();

    let _log_guard = init_tracing();
    install_panic_hook();

    if let Some(cmd) = &cli_args.command {
        cli::run_subcommand(cmd);
        return;
    }

    if let Err(e) = run_tui() {
        eprintln!("{} {}", "错误:".red(), e);
        std::process::exit(1);
    }
}

/// v0.6.0 阶段 3：在 init_tracing 之后、业务逻辑之前注册 panic hook。
///
/// 时序：`init_tracing` → `install_panic_hook` → `tui::setup_terminal`（TUI 模式）。
/// `tui::setup_terminal` 内部 `take_hook` 会把我们 chain 进去，最终 panic 时
/// 先 restore terminal → 写 crash report → 系统默认 hook。
fn install_panic_hook() {
    proc::metrics::crash::install_panic_hook();
}

/// v0.6.0 阶段 3：tracing 改造为 `RollingFileAppender::daily`。
///
/// 返回 `WorkerGuard` — 必须 hold 到程序退出，drop 时 flush 残留日志。
/// 路径：`~/.config/proc/proc.YYYY-MM-DD.log`，保留 7 天（启动时清理更早的）。
///
/// 失败（创建目录失败 / 已有 global subscriber）时返回 `None`，tracing 完全
/// 禁用 — 业务逻辑不应依赖 tracing 必有输出。
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let config_dir = proc::dirs_config_dir();
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        eprintln!("警告: 创建配置目录失败: {} (日志不可用)", e);
        return None;
    }

    // 启动时清理 7 天前的旧日志。
    let removed = proc::cleanup_old_logs(&config_dir, 7);
    if removed > 0 {
        tracing::info!(removed, "cleaned up old log files");
    }

    // `RollingFileAppender::daily` 实际文件名：`proc.YYYY-MM-DD.log`。
    // 同一天再次启动 → 追加（不再 truncate，与 v0.5.0 行为不同）。
    let file_appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_appender::rolling::Rotation::DAILY,
        &config_dir,
        "proc.log",
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .finish();
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        eprintln!("警告: 初始化日志失败 (日志不可用)");
        return None;
    }

    Some(guard)
}

/// 默认入口（`proc` 不带子命令）— 启动交互式 TUI。
fn run_tui() -> error::Result<()> {
    let mut app = proc::app::App::new()?;
    let mut terminal = tui::setup_terminal()?;
    let result = tui::run_app(&mut terminal, &mut app);
    tui::restore_terminal(&mut terminal)?;
    result
}
