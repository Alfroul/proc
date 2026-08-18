//! `proc agent <sub>` — v0.20 内置 AI agent CLI 入口（ADR-0030）。
//!
//! - `models`（stage 2）：GGUF scanner + ModelRegistry 表格输出
//! - `ask`（stage 3b）：单轮 query → AgentRunner ReAct loop → Markdown 输出。
//!   provider 构造链（决策 H）：CLI flag > agent.toml > 代码默认（llama-cpp）。

use colored::Colorize;

use super::def::AgentSub;
use crate::agent::config::AgentConfig;
use crate::agent::model_registry::{ModelRegistry, ModelStatus};
use crate::agent::runner::StepEvent;

/// 入口：dispatch agent 子命令。失败时打印错误并 exit 1（与既有 CLI 子命令同款）。
pub fn run_agent(sub: &AgentSub) {
    match sub {
        AgentSub::Models { refresh: _ } => {
            // 当前 scan 每次全量重扫，--refresh 仅是语义占位（无缓存层）。
            if let Err(e) = run_agent_models() {
                eprintln!("{} {}", "错误:".red(), e);
                std::process::exit(1);
            }
        }
        AgentSub::Ask {
            query,
            provider,
            model,
            max_steps,
        } => {
            if let Err(e) = run_agent_ask(query, provider.as_deref(), model.as_deref(), *max_steps)
            {
                eprintln!("{} {}", "错误:".red(), e);
                std::process::exit(1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ask（stage 3b）
// ---------------------------------------------------------------------------

fn run_agent_ask(
    query: &str,
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
    max_steps: u32,
) -> Result<(), String> {
    // provider 构造链抽共享（v0.21 stage 2）：CLI 与 TUI AgentSession 共用
    // `agent::builder::build_runner`（CLI flag > agent.toml > 代码默认）。
    let (runner, spec) = crate::agent::builder::build_runner(provider_flag, model_flag, max_steps)?;
    eprintln!("{} provider: {}", "·".dimmed(), spec.detail);

    // CLI 自建 current_thread runtime（agent 不走 MCP server runtime，风险 6 规避）。
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime 创建失败: {e}"))?;

    let outcome = rt
        .block_on(runner.run_with_progress(query, &|ev| match ev {
            StepEvent::LlmTurn(n) => {
                eprintln!("{} LLM 第 {} 轮", "·".dimmed(), n + 1);
            }
            StepEvent::ToolStart(name, args) => {
                eprintln!("{} {name} {}", "→".cyan(), args.to_string().dimmed());
            }
        }))
        .map_err(|e| format!("agent 运行失败: {e}"))?;

    if outcome.stop != crate::agent::runner::StopCause::EndTurn {
        eprintln!(
            "{} 终止原因: {}（{} 步 / {} tool call）",
            "⚠".yellow(),
            outcome.stop.label(),
            outcome.steps.len(),
            outcome.steps.len()
        );
    }
    println!("{}", outcome.final_text);
    Ok(())
}

fn run_agent_models() -> Result<(), String> {
    let config = AgentConfig::load();
    // 默认扫描路径 + agent.toml 自定义路径（ModelRegistry::scan 只扫传入路径，
    // 占位符在 scan 内展开）。
    let mut paths: Vec<String> = crate::agent::gguf_scan::default_scan_paths()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    paths.extend(config.llama_cpp.search_paths.iter().cloned());
    let mut registry = ModelRegistry::new();
    registry
        .scan(&paths)
        .map_err(|e| format!("模型扫描失败: {e}"))?;

    let models = registry.models();
    if models.is_empty() {
        println!("{}", "未检测到本地 GGUF 模型".yellow());
        println!();
        println!("默认扫描路径：");
        for path in crate::agent::gguf_scan::default_scan_paths() {
            println!("  {}", path.display());
        }
        println!();
        println!(
            "可在 {} 的 [llama-cpp] search_paths 中追加自定义路径（支持 %VAR% 占位符）。",
            "~/.config/proc/agent.toml".bold()
        );
        return Ok(());
    }

    println!("检测到 {} 个本地模型：\n", models.len().to_string().green());
    println!(
        "{:<36} {:>10}  {:<8} {:<12} {}",
        "NAME".bold(),
        "SIZE".bold(),
        "QUANT".bold(),
        "ARCH".bold(),
        "PATH".bold()
    );
    for model in models {
        let status_mark = match model.status {
            ModelStatus::Available => String::new(),
            _ => " [metadata 解析失败]".red().to_string(),
        };
        println!(
            "{:<36} {:>10}  {:<8} {:<12} {}{}",
            model.name,
            format_size(model.size_bytes),
            model.quantization.as_deref().unwrap_or("-"),
            model.architecture.as_deref().unwrap_or("-"),
            model.path.display(),
            status_mark
        );
    }
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1}G", b / GIB)
    } else if b >= MIB {
        format!("{:.0}M", b / MIB)
    } else {
        format!("{bytes}B")
    }
}
