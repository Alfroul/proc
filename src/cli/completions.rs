//! `proc completions --shell <SHELL>` — v0.7.0 阶段 3 入口。
//!
//! 在线生成 shell 补全脚本到 stdout，供用户 `source` 或发布 CI 打包到
//! Release artifact。详见 `docs/adr/0010-shell-completion-and-palette.md`。
//!
//! 为什么不用 `build.rs`：build-time 耦合 `clap_complete` 到编译依赖、
//! `cargo check` 也重跑、产物落到 `OUT_DIR` 用户拿不到。在线子命令更灵活。

use clap::CommandFactory;
use colored::Colorize;

/// 入口：根据 `shell` 生成对应补全脚本，写到 stdout。
pub fn run_completions(shell: clap_complete::Shell) {
    // 用 `Cli::command()` 重新构造一份 `Command`，不消耗调用方 parser。
    // bin name 必须显式设置，否则 clap_complete 会用 package name (`proc`)
    // 来自二进制名，但有些 shell（zsh）需要明确指定。
    let mut cmd = crate::cli::Cli::command();
    let bin = "proc";
    clap_complete::generate(shell, &mut cmd, bin, &mut std::io::stdout());
    eprintln!(
        "{}",
        format!("✓ generated {shell} completions for `{bin}`").green()
    );
}
