//! `proc kill` 与 `proc pkill` — 终止进程（按 PID / 按名称）。

use colored::Colorize;

use crate::kill;

pub fn run_kill(pid: u32, force: bool) {
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

pub fn run_pkill(name: &str, force: bool, dry_run: bool) {
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
