//! `proc agent <sub>` — v0.20 内置 AI agent CLI 入口（ADR-0030）。
//!
//! stage 1 Spike：subcommand 已注册但 ask / models 都返占位错误；
//! stage 2 实装 `models`（GGUF scanner + ModelRegistry）；
//! stage 3b 实装 `ask`（LlmProvider 构造 + ToolRegistry + AgentRunner）。

use colored::Colorize;

use super::def::AgentSub;

/// 入口：dispatch agent 子命令。stage 1 Spike 占位错误（exit 1）。
pub fn run_agent(sub: &AgentSub) {
    let msg = match sub {
        AgentSub::Models { refresh: _ } => {
            "proc agent models 将在 v0.20 stage 2 落地（GGUF scanner 实装后）".to_string()
        }
        AgentSub::Ask { query, .. } => {
            format!(
                "proc agent ask '{query}' 将在 v0.20 stage 3 落地（LlamaCppProvider + Agent loop 实装后）"
            )
        }
    };
    eprintln!("{} {}", "错误:".red(), msg);
    std::process::exit(1);
}
